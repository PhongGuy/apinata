//! The background actor that owns one collection: it applies commands in send order and
//! publishes one snapshot per batch of queued commands.

use std::collections::{HashMap, HashSet};
use std::mem;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use apinata_core::import::{
    ImportBundle, ImportReport, ImportedEnvironment, ImportedRequest, PrunedRequest, UpdateOptions,
    UpdateSummary, add_environment, apply_update, plan_update, store_request_secrets,
};
use apinata_core::{
    AuthSpec, Change, ChangeKind, Collection, DiskState, DropTarget, EnvOverlay, EnvWarning,
    IgnorePolicy, ItemKind, ItemPath, OpenError, OpenOptions, RuntimeVars, SaveError, SecretKey,
    SecretStore, TokenCache, Trash, Trashed, TreeError, VarContext, VariableSet,
};
use async_channel::{Receiver, Sender};

use crate::auth_secrets;
use crate::command::{
    Command, Edit, EnvOp, Envelope, Field, NetworkOp, Notice, NoticeKind, PlanError, Planned,
    Target, TreeOp,
};
use crate::context_cache::ContextCache;
use crate::env_table;
use crate::model::OPEN_PANIC_MESSAGE;
use crate::network_secrets;
use crate::orphan_section::{FilelessGroup, ListOnce, fileless_groups};
use crate::snapshot::{CollectionSnapshot, SnapshotCache};
use crate::stale::{Reloaded, StaleLedger, is_within, moved};

/// The notice for an import sent before a reload of its request.
const RELOADED_MESSAGE: &str = "the request was reloaded from disk; import it again";

/// What the actor tells its model.
pub enum Update {
    /// A new published state.
    Snapshot(Arc<CollectionSnapshot>),
    /// The collection could not be opened; the actor has stopped.
    OpenFailed(String),
    /// The collection is in the legacy .bru format; the actor has stopped.
    LegacyFormat(String),
    /// A panic stopped the actor; it applies nothing more.
    Stopped,
}

/// Test-only fault injection: each hook runs just before its real step, on the actor.
#[doc(hidden)]
#[derive(Debug, Default, Clone)]
pub struct Faults {
    /// Runs before the collection opens.
    pub open: Option<fn()>,
    /// Runs before each publish, given the new snapshot's sequence.
    pub publish: Option<fn(u64)>,
    /// Read after each command arrives; while it is set the actor applies nothing. Each test
    /// owns its flag, so parallel tests never share stall state.
    pub stall: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// While set, creating a draft from a snapshot fails, as when its folder vanished between
    /// the restore's snapshot and the command.
    pub refuse_draft_from: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// The trash the actor deletes into; without one, a `Trash` command fails.
    pub trash: Option<Arc<dyn Trash>>,
}

/// Open the collection at `root`, publish it, then apply `commands` until the channel closes
/// or nobody listens on `updates`. A panic ends the actor with `OpenFailed` while opening and
/// `Stopped` afterwards.
#[expect(
    clippy::too_many_arguments,
    reason = "each is a distinct shared handle or channel"
)]
pub async fn run_actor(
    root: PathBuf,
    store: Arc<dyn SecretStore>,
    commands: Receiver<Envelope>,
    updates: Sender<Update>,
    faults: Faults,
    runtime: RuntimeVars,
    overlay: EnvOverlay,
    tokens: TokenCache,
    network_ack: Option<PathBuf>,
) {
    // gpui's pool threads run tasks with no unwind guard, so every synchronous step is caught
    // here. The payload is never reported: it may quote file content.
    let options = OpenOptions::new().ignore(IgnorePolicy::BrunoApp);
    let opened = catch_unwind(AssertUnwindSafe(|| {
        if let Some(hook) = faults.open {
            hook();
        }
        Collection::open_with(&root, &options)
            .map(|c| Actor::new(c, store, runtime, overlay, tokens))
    }));
    let mut actor = match opened {
        Ok(Ok(mut actor)) => {
            actor.refuse_draft_from = faults.refuse_draft_from.clone();
            actor.network_ack = network_ack;
            if let Some(trash) = &faults.trash {
                actor.trash = trash.clone();
            }
            actor
        }
        Ok(Err(e @ OpenError::LegacyFormat { .. })) => {
            let _ = updates.send(Update::LegacyFormat(e.to_string())).await;
            return;
        }
        Ok(Err(e)) => {
            let _ = updates.send(Update::OpenFailed(e.to_string())).await;
            return;
        }
        Err(_) => {
            let message = OPEN_PANIC_MESSAGE.to_owned();
            let _ = updates.send(Update::OpenFailed(message)).await;
            return;
        }
    };
    let Some(opening) = guarded_publish(&mut actor, &faults) else {
        let _ = updates.send(Update::Stopped).await;
        return;
    };
    if updates.send(Update::Snapshot(opening)).await.is_err() {
        return;
    }
    while let Ok(first) = commands.recv().await {
        if faults
            .stall
            .as_ref()
            .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::SeqCst))
        {
            std::future::pending::<()>().await;
        }
        // Drain what queued meanwhile, so a burst of keystrokes costs one snapshot.
        let applied = catch_unwind(AssertUnwindSafe(|| {
            actor.apply(first);
            while let Ok(next) = commands.try_recv() {
                actor.apply(next);
            }
        }));
        let published = applied
            .ok()
            .and_then(|()| guarded_publish(&mut actor, &faults));
        let Some(snapshot) = published else {
            let _ = updates.send(Update::Stopped).await;
            return;
        };
        if updates.send(Update::Snapshot(snapshot)).await.is_err() {
            break;
        }
    }
}

/// Publish under an unwind guard; `None` means the publish panicked.
fn guarded_publish(actor: &mut Actor, faults: &Faults) -> Option<Arc<CollectionSnapshot>> {
    catch_unwind(AssertUnwindSafe(|| {
        if let Some(hook) = faults.publish {
            hook(actor.seq + 1);
        }
        actor.publish()
    }))
    .ok()
}

/// The collection and the bookkeeping between two snapshots.
struct Actor {
    collection: Collection,
    store: Arc<dyn SecretStore>,
    /// The active environment's name, if any.
    selected: Option<String>,
    // The item the last `ApplyImport` landed on, for the `StoreImportSecrets` that follows it.
    applied_import: Option<ItemPath>,
    cache: SnapshotCache,
    context: ContextCache,
    seq: u64,
    acked: u64,
    notices: Vec<Notice>,
    ledger: StaleLedger,
    /// Edits held back per target until the user resends or discards them.
    stale: Arc<HashMap<Target, Vec<Edit>>>,
    /// Drafts whose file changed underneath them and still loads.
    changed: HashSet<Target>,
    /// Per environment name, the snapshot seq of the last reload that replaced its file's
    /// content from outside. Kept past removal, so a recreated namesake stamps higher.
    env_reloads: HashMap<String, u64>,
    /// Per request, folder and the collection, the snapshot seq of the last outside change
    /// to its file, so an unsaved document tells each change apart.
    reloads: HashMap<Target, u64>,
    /// How many times `.env` changed on disk, published for views that depend on its values.
    dotenv_seq: u64,
    /// Test-only: see `Faults::refuse_draft_from`.
    refuse_draft_from: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// Where `Trash` commands send items.
    trash: Arc<dyn Trash>,
    /// Items trashed and not yet forgotten, by the id of the command that trashed them.
    trashed: HashMap<u64, Trashed>,
    /// Requests a spec update removed and not yet forgotten, by a ticket from `prune_ticket`.
    pruned: HashMap<u64, PrunedRequest>,
    /// The next ticket for an item a spec update removed. They count down from the top, so they
    /// never meet the command ids a delete's tickets are, which count up from 1.
    prune_ticket: u64,
    /// The record of acknowledged network settings, when the app keeps one.
    network_ack: Option<PathBuf>,
    /// What the last Network notice showed, recorded as is by Acknowledge.
    network_shown: Option<apinata_core::ShownNetwork>,
}

/// Apply a configuration edit to a `FolderFile` or `CollectionFile`, which share their
/// setters by macro rather than by trait.
macro_rules! config_edit {
    ($file:expr, $edit:expr, $unsupported:expr, $failed:expr) => {
        match $edit {
            Edit::Name(v) => $file.set_name(v),
            Edit::Headers(v) => $file.set_headers(v).map_err($failed)?,
            Edit::Auth(v) => $file.set_auth(v),
            Edit::Vars(v) => $file.set_vars(v),
            Edit::PostResponseVars(v) => $file.set_post_response_vars(v),
            Edit::Docs(v) => $file.set_docs(v),
            Edit::Script(phase, v) => $file.set_script(phase, v),
            _ => return Err($unsupported),
        }
    };
}

impl Actor {
    fn new(
        collection: Collection,
        store: Arc<dyn SecretStore>,
        runtime: RuntimeVars,
        overlay: EnvOverlay,
        tokens: TokenCache,
    ) -> Self {
        Actor {
            collection,
            store,
            selected: None,
            applied_import: None,
            cache: SnapshotCache::new(),
            context: ContextCache::new(runtime, overlay, tokens),
            seq: 0,
            acked: 0,
            notices: Vec::new(),
            ledger: StaleLedger::new(),
            stale: Arc::default(),
            changed: HashSet::new(),
            env_reloads: HashMap::new(),
            reloads: HashMap::new(),
            dotenv_seq: 0,
            refuse_draft_from: None,
            trash: Arc::new(crate::workspace::NoTrash),
            trashed: HashMap::new(),
            pruned: HashMap::new(),
            prune_ticket: u64::MAX,
            network_ack: None,
            network_shown: None,
        }
    }

    /// Apply one command; whatever goes wrong becomes a notice in the next snapshot.
    fn apply(&mut self, envelope: Envelope) {
        let (id, seen, basis) = (envelope.id, envelope.seen, envelope.basis);
        self.acked = id;
        let command = self.remap(envelope.command, seen);
        if shapes_context(&command) {
            self.context.invalidate();
        }
        let outcome = match command {
            // Checked before the lookup, so an edit to an item a reload removed is held too.
            Command::Edit { target, edit } if self.ledger.is_stale(&target, basis) => {
                Arc::make_mut(&mut self.stale)
                    .entry(target.clone())
                    .or_default()
                    .push(edit);
                Err(NoticeKind::Stale(target))
            }
            // Network edits may write `.env`, so they bypass the generic setters.
            Command::Edit {
                target: Target::Collection,
                edit,
            } if edit.field() == Field::Network => self.network_edit(id, edit),
            // So may an OAuth 2.0 auth edit, for its client secret and password.
            Command::Edit {
                target,
                edit: edit @ (Edit::Auth(AuthSpec::OAuth2(_)) | Edit::MoveAuthSecrets),
            } if !matches!(target, Target::Environment(_)) => self.auth_edit(id, &target, edit),
            Command::Edit { target, edit } => self.edit(&target, edit),
            // An import is never held: the user re-imports against what the reload loaded.
            Command::ApplyImport { item, .. }
                if self.ledger.is_stale(&Target::Request(item.clone()), seen) =>
            {
                self.applied_import = None;
                Err(NoticeKind::Failed {
                    target: Some(Target::Request(item)),
                    message: RELOADED_MESSAGE.to_owned(),
                    row: None,
                })
            }
            Command::ApplyImport { item, request } => {
                let applied = self.apply_import(&item, &request);
                self.applied_import = applied.is_ok().then_some(item);
                applied
            }
            Command::StoreImportSecrets {
                item,
                request,
                mut notes,
                reply,
            } => {
                // Only the import that just applied to this item may store its secrets.
                let applied = self.applied_import.take().as_ref() == Some(&item);
                if applied && !request.secrets.is_empty() {
                    self.cache.touch_all();
                }
                if applied {
                    store_request_secrets(
                        &mut self.collection,
                        &request,
                        self.selected.as_deref(),
                        &*self.store,
                        &mut notes,
                    );
                }
                let _ = reply.try_send(notes);
                Ok(())
            }
            Command::ImportEnvironment { env, reply } => match self.import_environment(&env) {
                Ok((name, report)) => {
                    let _ = reply.try_send(Ok(report));
                    let kind = NoticeKind::EnvironmentCreated(name);
                    self.notices.push(Notice { command: id, kind });
                    Ok(())
                }
                Err(message) => {
                    let _ = reply.try_send(Err(message.clone()));
                    Err(NoticeKind::Failed {
                        target: None,
                        message,
                        row: None,
                    })
                }
            },
            Command::FileTexts { target, reply } => {
                let texts = self
                    .file_path(&target)
                    .ok()
                    .and_then(|path| self.collection.file_texts(&path));
                let _ = reply.try_send(texts);
                Ok(())
            }
            Command::UpdateInfo { reply } => {
                let _ = reply.try_send(self.collection.file().spec_baseline().is_some());
                Ok(())
            }
            Command::UpdateFolders { reply } => {
                let _ = reply.try_send(self.collection.baseline_folders());
                Ok(())
            }
            Command::PlanUpdate {
                bundle,
                prune,
                folder,
                reply,
            } => {
                let _ = reply.try_send(self.plan_spec_update(&bundle, prune, folder));
                Ok(())
            }
            Command::ApplyUpdate {
                bundle,
                prune,
                folder,
                reply,
            } => {
                let done = self.apply_spec_update(id, &bundle, prune, folder);
                let _ = reply.try_send(done);
                Ok(())
            }
            Command::Save(target) => {
                self.discard_stale(&target);
                self.save(id, &target)
            }
            Command::Revert(target) => {
                self.discard_stale(&target);
                self.revert(&target)
            }
            Command::Overwrite(target) => {
                self.discard_stale(&target);
                self.overwrite(id, &target)
            }
            Command::DiscardStale(target) => {
                self.discard_stale(&target);
                Ok(())
            }
            Command::Network(op) => {
                self.network_op(op);
                Ok(())
            }
            Command::Reload(paths) => {
                let names = self.environment_names();
                for path in paths {
                    let changes = self.collection.reload_path(&path);
                    self.absorb(&changes, &names);
                }
                Ok(())
            }
            Command::ReloadAll => {
                let names = self.environment_names();
                let changes = self.collection.reload();
                self.absorb(&changes, &names);
                Ok(())
            }
            Command::Tree(op) => self.tree(id, op).map(|done| {
                let done = done.map(|kind| Notice { command: id, kind });
                self.notices.extend(done);
            }),
            Command::Env(EnvOp::SetSecretValue { env, var, value }) => {
                self.set_secret_value(&env, &var, &value)
            }
            Command::Env(op) => self.env(op).map(|done| {
                let done = done.into_iter().map(|kind| Notice { command: id, kind });
                self.notices.extend(done);
            }),
            Command::SelectEnvironment(name) => {
                // Script-set environment values stay with the environment they were set under.
                if self.selected != name {
                    self.context.select_environment(name.as_deref());
                }
                self.selected = name;
                self.cache.touch_all();
                Ok(())
            }
            Command::SetGlobalEnvironment(globals) => {
                self.context.set_globals(globals);
                self.cache.touch_all();
                Ok(())
            }
            Command::Plan { item, reply } => {
                // The sender may have stopped waiting; a reply nobody reads is dropped.
                let _ = reply.try_send(self.plan(&item));
                Ok(())
            }
            Command::PlanSaved { item, reply } => {
                let _ = reply.try_send(self.plan_saved(&item));
                Ok(())
            }
            Command::PlanSavedItems { items, reply } => {
                let _ = reply.try_send(self.plan_saved_items(&items));
                Ok(())
            }
            Command::Variables { item, reply } => {
                let _ = reply.try_send(self.variables(&item));
                Ok(())
            }
            Command::MockSource { reply } => {
                let source = self.open_saved().and_then(|collection| {
                    let context = self.saved_context(&collection)?;
                    Ok(crate::command::MockSource {
                        collection,
                        context,
                    })
                });
                let _ = reply.try_send(source.map_err(|e| match e {
                    PlanError::Select(m) | PlanError::Context(m) => m,
                    PlanError::NotFound(item) => format!("{} not found", item.as_str()),
                }));
                Ok(())
            }
            Command::SecretValues { reply } => {
                let values = self
                    .context
                    .get(&self.collection, self.selected.as_deref(), &self.store)
                    .map(|context| context.secret_values())
                    .map_err(|e| e.to_string());
                let _ = reply.try_send(values);
                Ok(())
            }
            Command::ConvertPreview { item, kind, reply } => {
                let preview = self.collection.conversion_preview(&item, kind);
                let _ = reply.try_send(preview.map_err(|e| e.to_string()));
                Ok(())
            }
            Command::ConvertKind {
                item,
                kind,
                expected,
                reply,
            } => {
                self.cache.touch_all();
                let done = self.collection.convert_request(&item, kind, &expected);
                let _ = reply.try_send(done.map_err(|e| e.to_string()));
                Ok(())
            }
            Command::UndoConvert {
                item,
                conversion,
                reply,
            } => {
                self.cache.touch_all();
                let done = self.collection.undo_conversion(&item, &conversion);
                let _ = reply.try_send(done.map_err(|e| e.to_string()));
                Ok(())
            }
            Command::CopyOut { items, reply } => {
                let copies = items
                    .into_iter()
                    .map(|i| {
                        let copy = self.collection.paste_source(&i).map_err(|e| e.to_string());
                        (i, copy)
                    })
                    .collect();
                let _ = reply.try_send(copies);
                Ok(())
            }
            Command::Search {
                query,
                options,
                reply,
            } => {
                let found = self
                    .collection
                    .search_with(&query, &options)
                    .map_err(|e| e.to_string());
                let _ = reply.try_send(found);
                Ok(())
            }
            Command::Replace {
                query,
                replacement,
                options,
                scope,
                reply,
            } => {
                let done = self
                    .collection
                    .replace(&query, &replacement, &options, scope)
                    .map_err(|e| e.to_string());
                // Files were rewritten in memory and on disk, so every view is rebuilt for
                // the next snapshot; open tabs read their request from it.
                self.cache.touch_all();
                let _ = reply.try_send(done);
                Ok(())
            }
            Command::RevealSecret { env, var, reply } => {
                let value = self
                    .secret_key(&env, &var)
                    .and_then(|key| self.store.get(&key).map_err(|e| e.to_string()));
                let _ = reply.try_send(value);
                Ok(())
            }
            Command::StoredSecret { env, var, reply } => {
                let value = match self.collection.environment(&env) {
                    Some(_) => {
                        let key = self.collection.secret_key(&env, &var);
                        self.store.get(&key).map_err(|e| e.to_string())
                    }
                    None => Err(self.collection.environment_not_found(&env).to_string()),
                };
                let _ = reply.try_send(value);
                Ok(())
            }
            Command::OrphanedSecrets {
                env,
                reply,
                fileless,
            } => {
                let once = ListOnce::new(&*self.store);
                let names = self
                    .collection
                    .orphaned_secrets(&env, &once)
                    .map_err(|e| e.to_string());
                let _ = reply.try_send(names);
                if let Some(fileless) = fileless {
                    let _ = fileless.try_send(self.fileless_groups(&once));
                }
                Ok(())
            }
            Command::FilelessSecrets { reply } => {
                let _ = reply.try_send(self.fileless_groups(&*self.store));
                Ok(())
            }
            Command::RemoveFilelessSecrets { env, names, reply } => {
                let warnings = self
                    .collection
                    .remove_fileless_secrets(&env, &names, &*self.store)
                    .map(|warnings| warnings.iter().map(ToString::to_string).collect())
                    .map_err(|e| e.to_string());
                let _ = reply.try_send(warnings);
                Ok(())
            }
            Command::RemoveOrphanedSecrets { env, names, reply } => {
                let warnings = self
                    .collection
                    .remove_orphaned_secrets(&env, &names, &*self.store)
                    .map(|warnings| warnings.iter().map(ToString::to_string).collect())
                    .map_err(|e| e.to_string());
                let _ = reply.try_send(warnings);
                Ok(())
            }
        };
        if let Err(kind) = outcome {
            self.notices.push(Notice { command: id, kind });
        }
        // Commands arrive with non-decreasing `seen`, so no later one can need older moves.
        self.ledger.prune(seen);
    }

    /// A run plan for `item` from the current drafts, with the cached variable context.
    fn plan(&mut self, item: &ItemPath) -> Result<Planned, PlanError> {
        let c = &self.collection;
        if c.folder(item).is_none() && c.request(item).is_none() {
            return Err(PlanError::NotFound(item.clone()));
        }
        let plan = c
            .plan(&item.to_path(c.root()))
            .map_err(|e| PlanError::Select(e.to_string()))?;
        let context = self
            .context
            .get(c, self.selected.as_deref(), &self.store)
            .map_err(|e| PlanError::Context(e.to_string()))?;
        Ok(Planned { plan, context })
    }

    /// A run plan for `item` from the files on disk, with a context built for this plan
    /// alone: the actor's drafts and cached context stay as they are.
    fn plan_saved(&mut self, item: &ItemPath) -> Result<Planned, PlanError> {
        let c = self.open_saved()?;
        if c.folder(item).is_none() && c.request(item).is_none() {
            return Err(PlanError::NotFound(item.clone()));
        }
        let plan = c
            .plan(&item.to_path(c.root()))
            .map_err(|e| PlanError::Select(e.to_string()))?;
        let context = self.saved_context(&c)?;
        Ok(Planned { plan, context })
    }

    /// Plan a set of items from a fresh open of the saved files; stale items plan to nothing.
    fn plan_saved_items(&mut self, items: &[ItemPath]) -> Result<Planned, PlanError> {
        let c = self.open_saved()?;
        let plan = c.plan_selected(items);
        let context = self.saved_context(&c)?;
        Ok(Planned { plan, context })
    }

    /// Reopen the collection from disk so open drafts never reach a saved plan.
    fn open_saved(&self) -> Result<Collection, PlanError> {
        let options = OpenOptions::new().ignore(IgnorePolicy::BrunoApp);
        Collection::open_with(self.collection.root(), &options)
            .map_err(|e| PlanError::Select(e.to_string()))
    }

    /// The variable context for a freshly opened collection, from the actor's options.
    fn saved_context(&mut self, c: &Collection) -> Result<VarContext, PlanError> {
        let options = self.context.options(self.selected.as_deref(), &self.store);
        VarContext::new(c, options).map_err(|e| PlanError::Context(e.to_string()))
    }

    /// The variables request `item` sees, from its draft and the cached variable context.
    fn variables(&mut self, item: &ItemPath) -> Result<VariableSet, PlanError> {
        let c = &self.collection;
        let req = c
            .request(item)
            .ok_or_else(|| PlanError::NotFound(item.clone()))?;
        let context = self
            .context
            .get(c, self.selected.as_deref(), &self.store)
            .map_err(|e| PlanError::Context(e.to_string()))?;
        Ok(context.variables(req))
    }

    /// Redirect every path in `command` past the moves its sender had not yet seen.
    fn remap(&self, command: Command, seen: u64) -> Command {
        let t = |target: Target| self.ledger.remap(&target, seen);
        let p = |path: ItemPath| self.remap_path(path, seen);
        let e = |name: String| match self.ledger.remap(&Target::Environment(name.clone()), seen) {
            Target::Environment(renamed) => renamed,
            _ => name,
        };
        match command {
            Command::Edit { target, edit } => Command::Edit {
                target: t(target),
                edit,
            },
            Command::Save(target) => Command::Save(t(target)),
            Command::Revert(target) => Command::Revert(t(target)),
            Command::Overwrite(target) => Command::Overwrite(t(target)),
            Command::DiscardStale(target) => Command::DiscardStale(t(target)),
            Command::Tree(op) => Command::Tree(match op {
                TreeOp::CreateRequest { parent, name } => TreeOp::CreateRequest {
                    parent: p(parent),
                    name,
                },
                TreeOp::CreateFolder { parent, name } => TreeOp::CreateFolder {
                    parent: p(parent),
                    name,
                },
                TreeOp::Clone { item, name } => TreeOp::Clone {
                    item: p(item),
                    name,
                },
                TreeOp::Rename {
                    item,
                    name,
                    rename_file,
                } => TreeOp::Rename {
                    item: p(item),
                    name,
                    rename_file,
                },
                TreeOp::Delete { item } => TreeOp::Delete { item: p(item) },
                TreeOp::Trash { item } => TreeOp::Trash { item: p(item) },
                other @ (TreeOp::Restore { .. } | TreeOp::ForgetTrash { .. }) => other,
                TreeOp::Paste { target, source } => TreeOp::Paste {
                    target: p(target),
                    source,
                },
                TreeOp::Move { item, to } => TreeOp::Move {
                    item: p(item),
                    to: match to {
                        DropTarget::Inside(x) => DropTarget::Inside(p(x)),
                        DropTarget::Before(x) => DropTarget::Before(p(x)),
                        DropTarget::After(x) => DropTarget::After(p(x)),
                        other => other,
                    },
                },
                TreeOp::SortByName { folder } => TreeOp::SortByName { folder: p(folder) },
                TreeOp::CreateDraft { parent, kind } => TreeOp::CreateDraft {
                    parent: p(parent),
                    kind,
                },
                TreeOp::CreateDraftFromPresets { parent } => {
                    TreeOp::CreateDraftFromPresets { parent: p(parent) }
                }
                TreeOp::CreateDraftFrom {
                    parent,
                    snapshot,
                    unsaved,
                } => TreeOp::CreateDraftFrom {
                    parent: p(parent),
                    snapshot,
                    unsaved,
                },
                TreeOp::SaveDraft {
                    draft,
                    parent,
                    name,
                } => TreeOp::SaveDraft {
                    draft: p(draft),
                    parent: p(parent),
                    name,
                },
                TreeOp::SaveSnapshot {
                    parent,
                    name,
                    snapshot,
                } => TreeOp::SaveSnapshot {
                    parent: p(parent),
                    name,
                    snapshot,
                },
                TreeOp::DiscardDraft { draft } => TreeOp::DiscardDraft { draft: p(draft) },
                TreeOp::AddExample {
                    item,
                    name,
                    description,
                    response,
                } => TreeOp::AddExample {
                    item: p(item),
                    name,
                    description,
                    response,
                },
                TreeOp::RenameExample { item, index, name } => TreeOp::RenameExample {
                    item: p(item),
                    index,
                    name,
                },
                TreeOp::DeleteExample { item, index } => TreeOp::DeleteExample {
                    item: p(item),
                    index,
                },
            }),
            Command::FileTexts { target, reply } => Command::FileTexts {
                target: t(target),
                reply,
            },
            Command::Plan { item, reply } => Command::Plan {
                item: p(item),
                reply,
            },
            Command::ConvertPreview { item, kind, reply } => Command::ConvertPreview {
                item: p(item),
                kind,
                reply,
            },
            Command::ConvertKind {
                item,
                kind,
                expected,
                reply,
            } => Command::ConvertKind {
                item: p(item),
                kind,
                expected,
                reply,
            },
            Command::UndoConvert {
                item,
                conversion,
                reply,
            } => Command::UndoConvert {
                item: p(item),
                conversion,
                reply,
            },
            Command::PlanSaved { item, reply } => Command::PlanSaved {
                item: p(item),
                reply,
            },
            Command::PlanSavedItems { items, reply } => Command::PlanSavedItems {
                items: items.into_iter().map(p).collect(),
                reply,
            },
            Command::Variables { item, reply } => Command::Variables {
                item: p(item),
                reply,
            },
            Command::ApplyImport { item, request } => Command::ApplyImport {
                item: p(item),
                request,
            },
            Command::Env(op) => Command::Env(match op {
                EnvOp::Rename { from, to } => EnvOp::Rename { from: e(from), to },
                EnvOp::Duplicate { from } => EnvOp::Duplicate { from: e(from) },
                EnvOp::Delete { name } => EnvOp::Delete { name: e(name) },
                EnvOp::SetSecret { env, var, secret } => EnvOp::SetSecret {
                    env: e(env),
                    var,
                    secret,
                },
                EnvOp::SetSecretValue { env, var, value } => EnvOp::SetSecretValue {
                    env: e(env),
                    var,
                    value,
                },
                create @ EnvOp::Create { .. } => create,
            }),
            Command::RevealSecret { env, var, reply } => Command::RevealSecret {
                env: e(env),
                var,
                reply,
            },
            Command::StoredSecret { env, var, reply } => Command::StoredSecret {
                env: e(env),
                var,
                reply,
            },
            Command::OrphanedSecrets {
                env,
                reply,
                fileless,
            } => Command::OrphanedSecrets {
                env: e(env),
                reply,
                fileless,
            },
            Command::RemoveOrphanedSecrets { env, names, reply } => {
                Command::RemoveOrphanedSecrets {
                    env: e(env),
                    names,
                    reply,
                }
            }
            Command::SelectEnvironment(name) => Command::SelectEnvironment(name.map(e)),
            other => other,
        }
    }

    /// A bare path remapped as a request target, which both folder moves (by prefix) and
    /// request moves (exactly) rewrite, so it serves for folders too.
    fn remap_path(&self, path: ItemPath, seen: u64) -> ItemPath {
        match self.ledger.remap(&Target::Request(path.clone()), seen) {
            Target::Request(p) => p,
            _ => path,
        }
    }

    /// Apply a tree operation; the notice it reports on success, if any.
    fn tree(&mut self, id: u64, op: TreeOp) -> Result<Option<NoticeKind>, NoticeKind> {
        self.cache.touch_all();
        let c = &mut self.collection;
        let created = |r: Result<ItemPath, TreeError>| match r {
            Ok(path) => Ok(Some(NoticeKind::Created(path))),
            Err(e) => Err(NoticeKind::Failed {
                target: None,
                message: e.to_string(),
                row: None,
            }),
        };
        match op {
            TreeOp::CreateRequest { parent, name } => created(c.create_request(&parent, &name)),
            TreeOp::CreateFolder { parent, name } => created(c.create_folder(&parent, &name)),
            TreeOp::Clone { item, name } if c.folder(&item).is_some() => {
                created(c.clone_folder(&item, &name))
            }
            TreeOp::Clone { item, name } if c.opaque_item(&item).is_some() => {
                created(c.clone_opaque(&item, &name))
            }
            TreeOp::Clone { item, name } => created(c.clone_request(&item, &name)),
            TreeOp::Rename {
                item,
                name,
                rename_file,
            } => {
                let from = self.item_target(&item);
                let to = self.collection.rename(&item, &name, rename_file);
                self.moved_to(from, to)
            }
            TreeOp::Paste { target, source } => created(c.paste(&target, &source)),
            TreeOp::Move { item, to } => {
                let from = self.item_target(&item);
                let to = self.collection.move_item(&item, to);
                self.moved_or_renumber_failed(id, from, to)
            }
            TreeOp::SortByName { folder } => match c.sort_by_name(&folder) {
                Ok(_) => Ok(None),
                Err(e) => Err(failed(&Target::Folder(folder), e)),
            },
            TreeOp::Delete { item } => {
                let target = self.item_target(&item);
                let result = self.delete(&item);
                self.deleted_or_renumber_failed(id, &target, result)?;
                self.forget(
                    |t| matches!(t, Target::Request(p) | Target::Folder(p) if is_within(p, &item)),
                );
                Ok(Some(NoticeKind::Deleted(item)))
            }
            TreeOp::Trash { item } => self.trash_item(id, &item),
            TreeOp::Restore { ticket } => self.restore(id, ticket),
            TreeOp::ForgetTrash { ticket } => {
                self.trashed.remove(&ticket);
                self.pruned.remove(&ticket);
                Ok(None)
            }
            TreeOp::CreateDraft { parent, kind } => created(c.create_draft_of_kind(&parent, kind)),
            TreeOp::CreateDraftFromPresets { parent } => {
                created(c.create_draft_from_presets(&parent))
            }
            TreeOp::CreateDraftFrom {
                parent,
                snapshot,
                unsaved,
            } => {
                let refused = (self.refuse_draft_from.as_ref())
                    .is_some_and(|f| f.load(std::sync::atomic::Ordering::SeqCst));
                let made = if refused {
                    Err(TreeError::NotFound { item: parent })
                } else {
                    c.create_draft_from(&parent, &snapshot)
                };
                // A restored draft stays dirty until saved, so it is kept and prompted for.
                if unsaved && let Ok(draft) = &made {
                    c.mark_draft_unsaved(draft);
                }
                created(made)
            }
            TreeOp::SaveSnapshot {
                parent,
                name,
                snapshot,
            } => created(c.save_snapshot(&parent, &name, &snapshot)),
            TreeOp::SaveDraft {
                draft,
                parent,
                name,
            } => {
                let to = self.collection.save_draft(&draft, &parent, &name);
                self.moved_to(Target::Request(draft), to)
            }
            TreeOp::DiscardDraft { draft } => {
                c.remove_draft(&draft);
                self.forget(|t| *t == Target::Request(draft.clone()));
                Ok(None)
            }
            TreeOp::AddExample {
                item,
                name,
                description,
                response,
            } => match c.add_example(&item, &name, &description, &response) {
                Ok(_) => Ok(Some(NoticeKind::ExampleAdded { item, name })),
                Err(e) => Err(failed(&Target::Request(item), e)),
            },
            TreeOp::RenameExample { item, index, name } => c
                .rename_example(&item, index, &name)
                .map(|()| None)
                .map_err(|e| failed(&Target::Request(item), e)),
            TreeOp::DeleteExample { item, index } => match c.delete_example(&item, index) {
                Ok(()) => Ok(Some(NoticeKind::ExampleDeleted { item, index })),
                Err(e) => Err(failed(&Target::Request(item), e)),
            },
        }
    }

    /// The target kind of a tree item, decided before the operation changes it.
    fn item_target(&self, item: &ItemPath) -> Target {
        match self.collection.folder(item) {
            Some(_) => Target::Folder(item.clone()),
            None => Target::Request(item.clone()),
        }
    }

    /// Record a move whose result may say the item moved but a sibling could not be renumbered.
    /// The move still counts and its tabs follow; the renumber failure is one warning notice,
    /// queued before the move so a batch can fold it into its summary.
    fn moved_or_renumber_failed(
        &mut self,
        id: u64,
        from: Target,
        result: Result<ItemPath, TreeError>,
    ) -> Result<Option<NoticeKind>, NoticeKind> {
        let root = self.collection.root().to_path_buf();
        let relative = |e: &dyn std::fmt::Display| relative_to_root(&e.to_string(), &root);
        match result {
            Err(TreeError::Renumber { moved, source }) => {
                let warning = NoticeKind::Warnings(vec![format!(
                    "Moved, but could not renumber the siblings: {}",
                    relative(&source)
                )]);
                self.notices.push(Notice {
                    command: id,
                    kind: warning,
                });
                self.moved_to(from, Ok(moved))
            }
            Err(e) => Err(failed(&from, relative(&e))),
            ok => self.moved_to(from, ok),
        }
    }

    /// Settle a delete whose result may say the item is gone but a sibling could not be
    /// renumbered. The delete still counts; the renumber failure is one warning notice.
    fn deleted_or_renumber_failed(
        &mut self,
        id: u64,
        target: &Target,
        result: Result<(), DeleteFailure>,
    ) -> Result<(), NoticeKind> {
        match result {
            Ok(()) => Ok(()),
            Err(DeleteFailure::Tree(TreeError::DeletedRenumber { source })) => {
                let text = relative_to_root(&source.to_string(), self.collection.root());
                self.notices.push(Notice {
                    command: id,
                    kind: NoticeKind::Warnings(vec![format!(
                        "Deleted, but could not renumber the siblings: {text}"
                    )]),
                });
                Ok(())
            }
            Err(DeleteFailure::Tree(e)) => Err(failed(target, e)),
            Err(DeleteFailure::Text(e)) => Err(failed(target, e)),
        }
    }

    /// Record a rename or move, carrying the changed-on-disk flags and held edits of the item
    /// and everything under it to the new key.
    fn moved_to(
        &mut self,
        from: Target,
        result: Result<ItemPath, TreeError>,
    ) -> Result<Option<NoticeKind>, NoticeKind> {
        let path = result.map_err(|e| failed(&from, e))?;
        let to = match &from {
            Target::Folder(_) => Target::Folder(path),
            _ => Target::Request(path),
        };
        let (Target::Request(old) | Target::Folder(old)) = &from else {
            unreachable!("tree items are requests or folders")
        };
        let (Target::Request(new) | Target::Folder(new)) = &to else {
            unreachable!("tree items are requests or folders")
        };
        // A rename that kept the file name, or a drop where the item already was, moved nothing.
        if old == new {
            return Ok(None);
        }
        let notice = NoticeKind::Moved {
            from: old.clone(),
            to: new.clone(),
        };
        self.record_move(from, to);
        Ok(Some(notice))
    }

    /// Carry the changed-on-disk flags and held edits under `from` to `to`, and record the
    /// move for commands sent before the next snapshot shows it.
    fn record_move(&mut self, from: Target, to: Target) {
        let rekey = |t: Target| moved(&t, &from, &to).unwrap_or(t);
        self.changed = mem::take(&mut self.changed)
            .into_iter()
            .map(rekey)
            .collect();
        if self.stale.keys().any(|k| moved(k, &from, &to).is_some()) {
            let held = mem::take(Arc::make_mut(&mut self.stale));
            *Arc::make_mut(&mut self.stale) =
                held.into_iter().map(|(k, v)| (rekey(k), v)).collect();
        }
        self.ledger.record_move(from, to, self.seq + 1);
    }

    /// Drop the changed-on-disk flags and held edits of deleted targets, so an item created
    /// later under the same key starts clean.
    fn forget(&mut self, gone: impl Fn(&Target) -> bool) {
        self.changed.retain(|t| !gone(t));
        if self.stale.keys().any(&gone) {
            Arc::make_mut(&mut self.stale).retain(|t, _| !gone(t));
        }
    }

    /// Apply an environment operation with the actor's store; the notices it reports on
    /// success. Store failures after the file change are one `Warnings` notice.
    fn env(&mut self, op: EnvOp) -> Result<Vec<NoticeKind>, NoticeKind> {
        self.cache.touch_all();
        let store = &*self.store;
        let c = &mut self.collection;
        let fail = |target: Option<Target>| {
            move |e: apinata_core::EnvError| NoticeKind::Failed {
                target,
                message: e.to_string(),
                row: None,
            }
        };
        let mut done = Vec::new();
        let warnings = match op {
            EnvOp::Create { name } => {
                let (used, warnings) = c.create_environment(&name, store).map_err(fail(None))?;
                done.push(NoticeKind::EnvironmentCreated(used));
                warnings
            }
            EnvOp::Duplicate { from } => {
                let target = Target::Environment(from.clone());
                let (used, warnings) = c
                    .duplicate_environment(&from, store)
                    .map_err(fail(Some(target)))?;
                done.push(NoticeKind::EnvironmentCreated(used));
                warnings
            }
            EnvOp::Rename { from, to } => {
                let target = Target::Environment(from.clone());
                let warnings = c
                    .rename_environment(&from, &to, store)
                    .map_err(fail(Some(target.clone())))?;
                self.context.rename_environment(&from, &to);
                if self.selected.as_deref() == Some(from.as_str()) {
                    self.selected = Some(to.clone());
                }
                self.record_move(target, Target::Environment(to));
                warnings
            }
            EnvOp::Delete { name } => {
                let target = Target::Environment(name.clone());
                let warnings = c
                    .delete_environment(&name, store)
                    .map_err(fail(Some(target.clone())))?;
                self.context.forget_environment(&name);
                if self.selected.as_deref() == Some(name.as_str()) {
                    self.context.select_environment(None);
                    self.selected = None;
                }
                self.forget(|t| t == &target);
                warnings
            }
            EnvOp::SetSecret { env, var, secret } => {
                let target = Target::Environment(env.clone());
                c.set_variable_secret(&env, &var, secret, store)
                    .map_err(fail(Some(target)))?
            }
            // `apply` stores values itself, since they change no file and so rebuild no view.
            EnvOp::SetSecretValue { .. } => unreachable!("applied without `env`"),
        };
        if !warnings.is_empty() {
            done.push(NoticeKind::Warnings(display(&warnings)));
        }
        Ok(done)
    }

    /// Store a secret variable's value. It changes no file, so no view is rebuilt and nothing
    /// turns dirty.
    fn set_secret_value(&mut self, env: &str, var: &str, value: &str) -> Result<(), NoticeKind> {
        let target = Target::Environment(env.to_owned());
        if self.collection.environment(env).is_none() {
            return Err(NoticeKind::NotFound(target));
        }
        let key = self.secret_key(env, var).map_err(|e| failed(&target, e))?;
        self.store.set(&key, value).map_err(|e| failed(&target, e))
    }

    /// The store key of a secret variable in the current drafts; a missing environment or
    /// variable, or a plain one, has none.
    fn secret_key(&self, env: &str, var: &str) -> Result<SecretKey, String> {
        let environment = self
            .collection
            .environment(env)
            .ok_or_else(|| self.collection.environment_not_found(env).to_string())?;
        match environment.variables().iter().find(|v| v.name == var) {
            Some(v) if v.secret => Ok(self.collection.secret_key(env, var)),
            Some(_) => Err(format!("{var} in {env} is not a secret")),
            None => Err(format!("no variable named {var} in {env}")),
        }
    }

    /// Delete an item; one whose file is already gone, kept only for its drafts, is dropped by
    /// reverting those drafts, since core cannot delete what the disk no longer holds.
    fn delete(&mut self, item: &ItemPath) -> Result<(), DeleteFailure> {
        let c = &self.collection;
        let kept: Vec<PathBuf> = match (c.request(item), c.folder(item)) {
            (Some(r), _) if r.disk_state() == DiskState::Removed => vec![r.path().to_path_buf()],
            (_, Some(f)) if f.disk_state() == DiskState::Removed => documents(f),
            _ => return self.collection.delete(item).map_err(DeleteFailure::Tree),
        };
        for path in kept {
            self.collection
                .revert(&path)
                .map_err(|e| DeleteFailure::Text(e.to_string()))?;
        }
        Ok(())
    }

    /// Delete an item into the trash and remember how to bring it back under command `id`. An
    /// item whose file is already gone has nothing to trash, so it is dropped as `delete` does.
    fn trash_item(&mut self, id: u64, item: &ItemPath) -> Result<Option<NoticeKind>, NoticeKind> {
        let target = self.item_target(item);
        let c = &self.collection;
        let gone = c
            .request(item)
            .is_some_and(|r| r.disk_state() == DiskState::Removed)
            || c.folder(item)
                .is_some_and(|f| f.disk_state() == DiskState::Removed);
        if gone {
            let result = self.delete(item);
            self.deleted_or_renumber_failed(id, &target, result)?;
            self.forget(
                |t| matches!(t, Target::Request(p) | Target::Folder(p) if is_within(p, item)),
            );
            return Ok(Some(NoticeKind::Deleted(item.clone())));
        }
        let trashed = self
            .collection
            .trash(item, &*self.trash)
            .map_err(|e| failed(&target, e))?;
        if let Some(why) = trashed.renumber_failure() {
            let text = relative_to_root(why, self.collection.root());
            self.notices.push(Notice {
                command: id,
                kind: NoticeKind::Warnings(vec![format!(
                    "Deleted, but could not renumber the siblings: {text}"
                )]),
            });
        }
        self.forget(|t| matches!(t, Target::Request(p) | Target::Folder(p) if is_within(p, item)));
        self.trashed.insert(id, trashed);
        Ok(Some(NoticeKind::Trashed {
            item: item.clone(),
            ticket: id,
        }))
    }

    /// Undo a trash. An unknown ticket is a no-op, since the user can only have lost it by
    /// letting the toast go; a restore never overwrites, so a taken name fails with its reason.
    fn restore(&mut self, id: u64, ticket: u64) -> Result<Option<NoticeKind>, NoticeKind> {
        if let Some(pruned) = self.pruned.remove(&ticket) {
            return self.restore_pruned(id, &pruned);
        }
        let Some(trashed) = self.trashed.remove(&ticket) else {
            return Ok(None);
        };
        let target = self.item_target(trashed.item());
        self.collection
            .restore_trashed(&trashed, &*self.trash)
            .map_err(|e| failed(&target, e))?;
        Ok(Some(NoticeKind::Restored(trashed.item().clone())))
    }

    /// Undo a spec update's removal of one request, which also records it as generated again.
    fn restore_pruned(
        &mut self,
        id: u64,
        pruned: &PrunedRequest,
    ) -> Result<Option<NoticeKind>, NoticeKind> {
        let target = self.item_target(pruned.item());
        let warning = pruned
            .restore(&mut self.collection, &*self.trash)
            .map_err(|e| failed(&target, e))?;
        self.cache.touch(&Target::Collection);
        if let Some(text) = warning {
            let kind = NoticeKind::Warnings(vec![text]);
            self.notices.push(Notice { command: id, kind });
        }
        Ok(Some(NoticeKind::Restored(pruned.item().clone())))
    }

    /// Apply an import to a clone of the request's draft, kept only when every field took, since
    /// `apply_to` has already written the method and URL when a header stops it.
    fn apply_import(
        &mut self,
        item: &ItemPath,
        request: &ImportedRequest,
    ) -> Result<(), NoticeKind> {
        let target = Target::Request(item.clone());
        let draft = self
            .collection
            .request_mut(item)
            .ok_or_else(|| NoticeKind::NotFound(target.clone()))?;
        let mut file = draft.clone();
        request
            .apply_to(&mut file)
            .map_err(|e| failed(&target, e))?;
        *draft = file;
        self.cache.touch(&target);
        Ok(())
    }

    /// Write an imported environment and store its secrets; the name used and the report, or
    /// the error's alert text, which names a failing path once as other import failures do.
    fn import_environment(
        &mut self,
        env: &ImportedEnvironment,
    ) -> Result<(String, ImportReport), String> {
        self.cache.touch_all();
        add_environment(&mut self.collection, env, &*self.store)
            .map_err(|e| crate::import_report::open_failure(self.collection.root(), &e.to_string()))
    }

    /// What a spec update would do; the error text when the collection cannot be updated.
    fn plan_spec_update(
        &self,
        bundle: &ImportBundle,
        prune: bool,
        folder: Option<ItemPath>,
    ) -> Result<UpdateSummary, String> {
        if !has_baseline(&self.collection, folder.as_ref()) {
            return Err(NO_BASELINE.to_owned());
        }
        let options = update_options(prune, folder);
        let summary = plan_update(&self.collection, bundle, &options).map_err(|e| e.to_string())?;
        self.need_trash(&summary)?;
        Ok(summary)
    }

    /// Removal goes to the trash; when it is unavailable the update is refused before any
    /// file is written, so nothing is half-removed.
    fn need_trash(&self, summary: &UpdateSummary) -> Result<(), String> {
        if summary.pruned.is_empty() || self.trash.available() {
            return Ok(());
        }
        Err(TRASH_UNAVAILABLE.to_owned())
    }

    /// Update from a spec. The actor holds the unsaved edits,
    /// so the refusal is decided here and no caller flag is needed; pruned files go to `trash`
    /// and each is answered as a `Trashed` notice for command `id`, which an undo restores.
    fn apply_spec_update(
        &mut self,
        id: u64,
        bundle: &ImportBundle,
        prune: bool,
        folder: Option<ItemPath>,
    ) -> Result<UpdateSummary, String> {
        if !has_baseline(&self.collection, folder.as_ref()) {
            return Err(NO_BASELINE.to_owned());
        }
        if self.collection.file().is_dirty() || folder_unsaved(self.collection.tree()) {
            return Err(UNSAVED_UPDATE.to_owned());
        }
        let options = update_options(prune, folder);
        if prune {
            let planned =
                plan_update(&self.collection, bundle, &options).map_err(|e| e.to_string())?;
            self.need_trash(&planned)?;
        }
        let summary = apply_update(&mut self.collection, bundle, &options, Some(&*self.trash))
            .map_err(|e| e.to_string());
        // apply_update mutates the open collection itself; only the view cache needs a nudge.
        self.cache.touch_all();
        let mut summary = summary?;
        for trashed in mem::take(&mut summary.trashed) {
            let item = trashed.item().clone();
            self.forget(
                |t| matches!(t, Target::Request(p) | Target::Folder(p) if is_within(p, &item)),
            );
            let ticket = self.prune_ticket;
            self.prune_ticket -= 1;
            self.pruned.insert(ticket, trashed);
            let kind = NoticeKind::Trashed { item, ticket };
            self.notices.push(Notice { command: id, kind });
        }
        Ok(summary)
    }

    /// The next snapshot, carrying every notice since the last one.
    fn publish(&mut self) -> Arc<CollectionSnapshot> {
        self.seq += 1;
        Arc::new(CollectionSnapshot {
            seq: self.seq,
            acked: self.acked,
            content: self.cache.build_with(
                &self.collection,
                &self.changed,
                &self.env_reloads,
                &self.reloads,
            ),
            selected_environment: self.selected.clone(),
            context_generation: self.context.generation(),
            dotenv_seq: self.dotenv_seq,
            network_notice: self.network_notice(),
            stale: self.stale.clone(),
            notices: mem::take(&mut self.notices).into(),
        })
    }

    /// Apply a collection network edit; its warnings become one notice for command `id`.
    fn network_edit(&mut self, id: u64, edit: Edit) -> Result<(), NoticeKind> {
        let warnings = network_secrets::apply(&mut self.collection, edit).map_err(|message| {
            NoticeKind::Failed {
                target: Some(Target::Collection),
                message,
                row: None,
            }
        })?;
        self.cache.touch(&Target::Collection);
        if !warnings.is_empty() {
            let kind = NoticeKind::Warnings(warnings);
            self.notices.push(Notice { command: id, kind });
        }
        Ok(())
    }

    /// Apply an OAuth 2.0 auth edit, moving a typed secret to `.env` first; its warnings
    /// become one notice for command `id`.
    fn auth_edit(&mut self, id: u64, target: &Target, edit: Edit) -> Result<(), NoticeKind> {
        // "Move to .env" rewrites the auth the draft holds, moving every literal it has.
        let all = matches!(edit, Edit::MoveAuthSecrets);
        let mut auth = match edit {
            Edit::Auth(auth) => auth,
            Edit::MoveAuthSecrets => auth_secrets::current(&self.collection, target)
                .ok_or_else(|| NoticeKind::NotFound(target.clone()))?,
            other => return self.edit(target, other),
        };
        let before = all.then(|| auth.clone());
        // A target that is gone writes no `.env`; the edit reports it as usual.
        let exists = match target {
            Target::Request(path) => self.collection.request(path).is_some(),
            Target::Folder(path) => self.collection.folder_file_mut(path).is_some(),
            _ => true,
        };
        if !exists {
            return self.edit(target, Edit::Auth(auth));
        }
        let warnings = auth_secrets::apply(&mut self.collection, target, &mut auth, all).map_err(
            |message| NoticeKind::Failed {
                target: Some(target.clone()),
                message,
                row: None,
            },
        )?;
        // A move with nothing to move leaves the draft clean.
        if before.as_ref() == Some(&auth) {
            return Ok(());
        }
        self.edit(target, Edit::Auth(auth))?;
        if !warnings.is_empty() {
            let kind = NoticeKind::Warnings(warnings);
            self.notices.push(Notice { command: id, kind });
        }
        Ok(())
    }

    fn edit(&mut self, target: &Target, edit: Edit) -> Result<(), NoticeKind> {
        let unsupported = NoticeKind::Unsupported {
            target: target.clone(),
            field: edit.field(),
        };
        let not_found = || NoticeKind::NotFound(target.clone());
        let failed = |e: apinata_core::EditError| edit_failed(target, e);
        match target {
            Target::Request(path) => {
                let r = self.collection.request_mut(path).ok_or_else(not_found)?;
                match edit {
                    Edit::Name(v) => r.set_name(v),
                    // No path may store a method that is not a token.
                    Edit::Method(v) => {
                        let m =
                            apinata_core::normalize_method(&v).map_err(|e| NoticeKind::Failed {
                                target: Some(target.clone()),
                                message: e.to_string(),
                                row: None,
                            })?;
                        r.set_method(m);
                    }
                    Edit::Url(v) => r.set_url(v),
                    Edit::Params(v) => r.set_params(v),
                    Edit::KeepUrlQuery => r.use_url_query(),
                    Edit::KeepParamsQuery => r.use_params_query(),
                    Edit::Headers(v) => r.set_headers(v).map_err(failed)?,
                    Edit::Body(v) => r.set_body(v),
                    Edit::Auth(v) => r.set_auth(v),
                    Edit::Vars(v) => r.set_vars(v),
                    Edit::PostResponseVars(v) => r.set_post_response_vars(v),
                    Edit::Assertions(v) => r.set_assertions(v),
                    Edit::Settings(v) => r.set_settings(v),
                    Edit::OmitHeaders(v) => r.set_omitted_headers(v),
                    Edit::Docs(v) => r.set_docs(v),
                    Edit::Script(phase, v) => r.set_script(phase, v),
                    Edit::Tags(v) => r.set_tags(v),
                    Edit::Variables(_)
                    | Edit::Color(_)
                    | Edit::Extends(_)
                    | Edit::Presets(_)
                    | Edit::Proxy(_)
                    | Edit::ClientCertificates(_)
                    | Edit::HostOverrides(_)
                    | Edit::MoveNetworkSecrets
                    | Edit::MoveAuthSecrets => {
                        return Err(unsupported);
                    }
                }
            }
            Target::Folder(path) => {
                let f = self
                    .collection
                    .folder_file_mut(path)
                    .ok_or_else(not_found)?;
                match edit {
                    Edit::Tags(v) => f.set_tags(v),
                    edit => config_edit!(f, edit, unsupported, failed),
                }
            }
            Target::Collection => {
                let f = self.collection.file_mut();
                if let Edit::Presets(v) = &edit {
                    f.set_presets(v);
                } else {
                    config_edit!(f, edit, unsupported, failed);
                }
            }
            Target::Environment(name) => {
                let e = self
                    .collection
                    .environment_mut(name)
                    .ok_or_else(not_found)?;
                match edit {
                    Edit::Variables(v) => e.set_variables(v),
                    Edit::Color(v) => e.set_color(v),
                    Edit::Extends(v) => e.set_extends(v),
                    _ => return Err(unsupported),
                }
            }
        }
        self.cache.touch(target);
        Ok(())
    }

    fn save(&mut self, id: u64, target: &Target) -> Result<(), NoticeKind> {
        self.refuse_colliding_secrets(target)?;
        let not_found = || NoticeKind::NotFound(target.clone());
        let before = self.saved_secrets(target);
        let own_network = self.own_network_save(target);
        let c = &mut self.collection;
        // A draft has no file yet: only the Save dialog's `SaveDraft` can name one.
        if let Target::Request(path) = target
            && c.is_draft(path)
        {
            return Err(failed(
                target,
                "Choose a name and folder to save this request",
            ));
        }
        let result = match target {
            Target::Request(path) => c.request_mut(path).ok_or_else(not_found)?.save(),
            Target::Folder(path) => c.folder_file_mut(path).ok_or_else(not_found)?.save(),
            Target::Collection => c.file_mut().save(),
            Target::Environment(name) => c.environment_mut(name).ok_or_else(not_found)?.save(),
        };
        // A file changed under the draft marks the target changed on disk, after the
        // settle step, so the editor offers its banner instead of a failure notice.
        self.settled(target, result.is_ok());
        if result.is_ok() {
            self.drop_orphaned_secrets(id, target, &before);
            self.acknowledge_network_save(own_network);
        }
        match result {
            Err(SaveError::Conflict { .. }) => {
                self.changed.insert(target.clone());
                Ok(())
            }
            result => result.map_err(|e| failed(target, e)),
        }
    }

    // Whether a save of the collection file would only add the person's own edits to what
    // they last saw. Asked before the write, while the file still holds the old settings.
    fn own_network_save(&mut self, target: &Target) -> bool {
        matches!(target, Target::Collection) && self.disk_network_acknowledged()
    }

    // Whether the collection file on disk holds the settings last acknowledged, as the
    // selected environment resolves them.
    fn disk_network_acknowledged(&mut self) -> bool {
        let Some(file) = self.network_ack.clone() else {
            return false;
        };
        let root = self.collection.root().to_path_buf();
        let Some(network) = crate::network_gate::saved(&root) else {
            return false;
        };
        let ctx = self
            .context
            .get(&self.collection, self.selected.as_deref(), &self.store)
            .ok();
        let make = |salt: &str| network_snapshot(&network, ctx.as_ref(), salt);
        crate::network_gate::may_acknowledge_save(&file, &root, &make)
    }

    // Records the settings just written, from the model rather than the file, so an outside
    // write that lands right after the save cannot be acknowledged by mistake.
    fn acknowledge_network_save(&mut self, own: bool) {
        let (true, Some(file)) = (own, self.network_ack.clone()) else {
            return;
        };
        let root = self.collection.root().to_path_buf();
        let network = self.collection.file().network();
        let ctx = self
            .context
            .get(&self.collection, self.selected.as_deref(), &self.store)
            .ok();
        let make = |salt: &str| network_snapshot(&network, ctx.as_ref(), salt);
        crate::network_gate::acknowledge(&file, &root, &make);
    }

    // The Network section's notice, from the settings in memory (what a send uses) as the
    // selected environment resolves them. Read at every snapshot, so an outside edit shows as
    // soon as the watcher reloads it; what it shows is kept for Acknowledge. The person's own
    // unsaved edits are no outside change, so a dirty file over acknowledged disk shows none.
    fn network_notice(&mut self) -> Option<crate::network_gate::Notice> {
        let file = self.network_ack.clone()?;
        let root = self.collection.root().to_path_buf();
        let network = self.collection.file().network();
        let ctx = self
            .context
            .get(&self.collection, self.selected.as_deref(), &self.store)
            .ok();
        let make = |salt: &str| network_snapshot(&network, ctx.as_ref(), salt);
        let mut notice = crate::network_gate::notice_for(&file, &root, &make);
        // Disk is read only when memory differs and holds unsaved edits.
        if notice.is_some() && self.collection.file().is_dirty() && self.disk_network_acknowledged()
        {
            notice = None;
        }
        self.network_shown = notice.as_ref().and_then(|n| n.shown.clone());
        notice
    }

    fn network_op(&mut self, op: NetworkOp) {
        let Some(file) = self.network_ack.clone() else {
            return;
        };
        match op {
            // What the notice showed, never a fresh read that may hold a later edit.
            NetworkOp::Acknowledge => {
                let root = self.collection.root().to_path_buf();
                if let Some(shown) = &self.network_shown {
                    crate::network_gate::acknowledge_shown(&file, &root, shown);
                }
            }
            NetworkOp::Reset => crate::network_gate::reset(&file),
            NetworkOp::Recheck => {}
        }
    }

    fn revert(&mut self, target: &Target) -> Result<(), NoticeKind> {
        let path = self.file_path(target)?;
        let result = self.collection.revert(&path);
        self.settled(target, result.is_ok());
        result.map_err(|e| failed(target, e))
    }

    fn overwrite(&mut self, id: u64, target: &Target) -> Result<(), NoticeKind> {
        self.refuse_colliding_secrets(target)?;
        let path = self.file_path(target)?;
        let before = self.saved_secrets(target);
        let result = self.collection.save_overwrite(&path);
        self.settled(target, result.is_ok());
        if result.is_ok() {
            self.drop_orphaned_secrets(id, target, &before);
        }
        result.map_err(|e| failed(target, e))
    }

    /// Every write of an environment draft (Save, Overwrite, autosave, Save All) is refused
    /// when it would write two secrets of one name the file does not already hold as they are.
    fn refuse_colliding_secrets(&self, target: &Target) -> Result<(), NoticeKind> {
        let Target::Environment(name) = target else {
            return Ok(());
        };
        let Some(env) = self.collection.environment(name) else {
            return Ok(());
        };
        let draft = env_table::secret_records(&env.variables());
        let saved = (self.collection.try_saved_variables(name))
            .map(|vars| env_table::secret_records(&vars))
            .unwrap_or_default();
        if env_table::writes_colliding_secrets(&draft, &saved) {
            return Err(failed(target, env_table::DUPLICATE_SECRET_NAMES));
        }
        Ok(())
    }

    /// The secret names of an environment target's file on disk, read just before a save so
    /// no draft, reload or revert can make them stale; other targets have none.
    fn saved_secrets(&self, target: &Target) -> Vec<String> {
        match target {
            Target::Environment(name) => self.collection.saved_secret_names(name),
            _ => Vec::new(),
        }
    }

    // The stored secrets of environments with no file, grouped by environment.
    fn fileless_groups(&self, store: &dyn SecretStore) -> Result<Vec<FilelessGroup>, String> {
        let report = self.collection.all_orphaned_secrets(store);
        report
            .map(|r| fileless_groups(&r))
            .map_err(|e| e.to_string())
    }

    /// After a successful save, delete the stored values of names that were secret on disk
    /// before it and are not after it, as Bruno's save does; a save that wrote nothing, or a
    /// file that cannot be read back, deletes nothing. Failed deletes are one `Warnings`
    /// notice; the save stands.
    fn drop_orphaned_secrets(&mut self, id: u64, target: &Target, before: &[String]) {
        let Target::Environment(env) = target else {
            return;
        };
        let Some(after) = self.collection.try_saved_secret_names(env) else {
            return;
        };
        let mut warnings = Vec::new();
        for var in before.iter().filter(|v| !after.contains(v)) {
            let key = self.collection.secret_key(env, var);
            if let Err(error) = self.store.delete(&key) {
                let warning = EnvWarning::Secret {
                    environment: env.clone(),
                    variable: var.clone(),
                    error,
                };
                warnings.push(warning.to_string());
            }
        }
        if !warnings.is_empty() {
            let kind = NoticeKind::Warnings(warnings);
            self.notices.push(Notice { command: id, kind });
        }
    }

    /// After a save, revert or overwrite: the draft and the disk agree again on success.
    fn settled(&mut self, target: &Target, ok: bool) {
        if ok {
            self.changed.remove(target);
        }
        self.cache.touch(target);
    }

    fn discard_stale(&mut self, target: &Target) {
        if self.stale.contains_key(target) {
            Arc::make_mut(&mut self.stale).remove(target);
        }
    }

    /// Each environment file's name, read before a reload so a removed file still maps.
    fn environment_names(&self) -> HashMap<PathBuf, String> {
        let envs = self.collection.environments();
        envs.iter()
            .map(|e| (e.path().to_path_buf(), e.name().to_owned()))
            .collect()
    }

    /// Record what a reload replaced, stamped with the first snapshot that will show it, and
    /// update the changed-on-disk marks.
    fn absorb(&mut self, changes: &[Change], names: &HashMap<PathBuf, String>) {
        let at = self.seq + 1;
        for change in changes {
            if change.item == ItemKind::DotEnv {
                self.dotenv_seq += 1;
            }
            let Some(what) = self.reloaded(change, names) else {
                continue;
            };
            // Conflicts and restores are drafts under a banner, which owns their typing.
            if let Reloaded::Item(Target::Environment(name)) = &what
                && matches!(
                    change.kind,
                    ChangeKind::Modified | ChangeKind::Added | ChangeKind::Removed
                )
            {
                self.env_reloads.insert(name.clone(), at);
            }
            match &what {
                Reloaded::Item(target) => match change.kind {
                    ChangeKind::Conflict if self.disk_state(target) == Some(DiskState::Present) => {
                        self.changed.insert(target.clone());
                    }
                    ChangeKind::SeqOnly | ChangeKind::Added => {}
                    _ => {
                        self.changed.remove(target);
                    }
                },
                // A directory that went or came back reports its own state for what it holds.
                Reloaded::Subtree(dir) => self.changed.retain(|t| match t {
                    Target::Request(p) | Target::Folder(p) => !is_within(p, dir),
                    _ => true,
                }),
            }
            if matches!(
                change.kind,
                ChangeKind::Modified | ChangeKind::Conflict | ChangeKind::Removed
            ) {
                if let Reloaded::Item(target) = &what {
                    self.reloads.insert(target.clone(), at);
                }
                self.ledger.record(what, at);
            }
        }
        self.cache.touch_all();
    }

    /// The staleness key of a change; `None` for `.env` and paths outside the tree.
    fn reloaded(&self, change: &Change, names: &HashMap<PathBuf, String>) -> Option<Reloaded> {
        let path = &change.path;
        let item = || self.collection.item_path(path);
        let what = match change.item {
            ItemKind::Request | ItemKind::Opaque => Reloaded::Item(Target::Request(item()?)),
            ItemKind::Folder if path.file_name().is_some_and(|n| n == "folder.yml") => {
                Reloaded::Item(Target::Folder(item()?.parent()?))
            }
            ItemKind::Folder => Reloaded::Subtree(item()?),
            ItemKind::Collection => Reloaded::Item(Target::Collection),
            ItemKind::Environment => {
                let name = names.get(path).cloned().or_else(|| file_stem(path))?;
                Reloaded::Item(Target::Environment(name))
            }
            _ => return None,
        };
        Some(what)
    }

    /// What the disk holds for `target`'s document; an opaque file is present while it exists.
    fn disk_state(&self, target: &Target) -> Option<DiskState> {
        let c = &self.collection;
        match target {
            Target::Request(p) => c.request(p).map(|r| r.disk_state()).or_else(|| {
                c.opaque_item(p).map(|o| match o.path().exists() {
                    true => DiskState::Present,
                    false => DiskState::Removed,
                })
            }),
            Target::Folder(p) => c.folder(p)?.file().map(|f| f.disk_state()),
            Target::Collection => Some(c.file().disk_state()),
            Target::Environment(name) => c.environment(name).map(|e| e.disk_state()),
        }
    }

    /// The file behind `target`, which core's conflict operations address by path.
    fn file_path(&self, target: &Target) -> Result<PathBuf, NoticeKind> {
        let c = &self.collection;
        let path = match target {
            Target::Request(path) => c
                .request(path)
                .map(|r| r.path().to_path_buf())
                .or_else(|| c.opaque_item(path).map(|o| o.path().to_path_buf())),
            Target::Folder(path) => c.folder(path).map(|f| f.path().join("folder.yml")),
            Target::Collection => Some(c.file().path().to_path_buf()),
            Target::Environment(name) => c.environment(name).map(|e| e.path().to_path_buf()),
        };
        path.ok_or_else(|| NoticeKind::NotFound(target.clone()))
    }
}

// The snapshot of `network` as `ctx` resolves it, or as written when no context can be built.
fn network_snapshot(
    network: &apinata_core::CollectionNetwork,
    ctx: Option<&apinata_core::VarContext>,
    salt: &str,
) -> apinata_core::NetworkSnapshot {
    match ctx {
        Some(ctx) => apinata_core::NetworkSnapshot::resolve(network, ctx, salt),
        None => apinata_core::NetworkSnapshot::from_network(network, salt),
    }
}

/// Whether `command` may change what the variable context holds: anything but a request's
/// own edits, imports and conflict actions, held-edit discards, plans, variable reads,
/// secret-value lists, mock sources, reveals and orphan lists and removals.
fn shapes_context(command: &Command) -> bool {
    match command {
        Command::Edit { target, .. }
        | Command::Save(target)
        | Command::Revert(target)
        | Command::Overwrite(target) => !matches!(target, Target::Request(_)),
        Command::DiscardStale(_)
        | Command::Plan { .. }
        | Command::PlanSaved { .. }
        | Command::PlanSavedItems { .. }
        | Command::FileTexts { .. }
        | Command::Variables { .. }
        | Command::SecretValues { .. }
        | Command::MockSource { .. }
        | Command::Search { .. }
        | Command::CopyOut { .. }
        | Command::ConvertPreview { .. }
        | Command::RevealSecret { .. }
        | Command::StoredSecret { .. }
        | Command::OrphanedSecrets { .. }
        | Command::RemoveOrphanedSecrets { .. }
        | Command::FilelessSecrets { .. }
        | Command::RemoveFilelessSecrets { .. }
        | Command::Network(_)
        | Command::ApplyImport { .. } => false,
        _ => true,
    }
}

const NO_BASELINE: &str = "Not imported from a spec, so it cannot be updated";
const TRASH_UNAVAILABLE: &str = "The trash is not available, so nothing was removed. Turn off removing missing requests, or fix the system trash.";
const UNSAVED_UPDATE: &str = "Save or discard your unsaved edits in this collection first";

fn update_options(prune: bool, folder: Option<ItemPath>) -> UpdateOptions {
    let mut options = UpdateOptions::default();
    options.prune = prune;
    options.folder = folder;
    // The app undoes a removal in session, from the tickets its notices carry.
    options.undo_in_session = true;
    options
}

/// Whether the spec imported into `folder` (the root for `None`) can be updated.
fn has_baseline(c: &Collection, folder: Option<&ItemPath>) -> bool {
    match folder {
        None => c.file().spec_baseline().is_some(),
        Some(f) => c.folder(f).is_some() && c.file().spec_baseline_in(f).is_some(),
    }
}

/// Whether any request or folder file under `folder` has edits not yet saved.
fn folder_unsaved(folder: &apinata_core::Folder) -> bool {
    folder.requests().iter().any(|r| r.is_dirty())
        || folder.file().is_some_and(|f| f.is_dirty())
        || folder.folders().iter().any(folder_unsaved)
}

/// Every document file under `folder`: its requests and folder files, depth first.
fn documents(folder: &apinata_core::Folder) -> Vec<PathBuf> {
    let requests = folder.requests().iter().map(|r| r.path().to_path_buf());
    let own = folder.file().map(|f| f.path().to_path_buf());
    let nested = folder.folders().iter().flat_map(documents);
    requests.chain(nested).chain(own).collect()
}

/// Each warning's message, for one `Warnings` notice.
fn display(warnings: &[impl std::fmt::Display]) -> Vec<String> {
    warnings.iter().map(ToString::to_string).collect()
}

fn file_stem(path: &Path) -> Option<String> {
    path.file_stem()?.to_str().map(str::to_owned)
}

/// Why a delete failed: a core tree error, kept typed so a renumber failure can be told apart,
/// or the text of a draft revert failure.
enum DeleteFailure {
    Tree(TreeError),
    Text(String),
}

fn failed(target: &Target, error: impl std::fmt::Display) -> NoticeKind {
    NoticeKind::Failed {
        target: Some(target.clone()),
        message: error.to_string(),
        row: None,
    }
}

/// `failed` for a refused edit, carrying the header row core named as typed data.
fn edit_failed(target: &Target, error: apinata_core::EditError) -> NoticeKind {
    let row = match &error {
        apinata_core::EditError::InvalidHeaderName { row } => Some(*row),
        _ => None,
    };
    NoticeKind::Failed {
        target: Some(target.clone()),
        message: error.to_string(),
        row,
    }
}

/// Strips the collection `root` and its separator from every path in `text`, so a toast names a
/// file relative to the collection. Matches the root as given and in its canonical form (a
/// symlinked root), with either `/` or `\` as the separator.
pub fn relative_to_root(text: &str, root: &Path) -> String {
    let mut roots = vec![root.display().to_string()];
    if let Ok(real) = root.canonicalize() {
        roots.push(real.display().to_string());
    }
    let mut out = text.to_owned();
    for root in roots {
        let root = root.trim_end_matches(['/', '\\']);
        if root.is_empty() {
            continue;
        }
        let spellings = [
            root.to_owned(),
            root.replace('/', "\\"),
            root.replace('\\', "/"),
        ];
        for spelling in spellings {
            for separator in ['/', '\\'] {
                out = out.replace(&format!("{spelling}{separator}"), "");
            }
        }
    }
    out
}
