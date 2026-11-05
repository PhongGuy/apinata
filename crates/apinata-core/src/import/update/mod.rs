//! Refreshing an imported collection from a changed spec.

mod baseline;
mod fields;

pub use baseline::SpecBaseline;
pub use fields::{ChangedField, FieldHashes, file_field_hashes, operation_key, spec_field_hashes};

use super::{
    ImportBundle, ImportError, ImportNote, ImportedItem, ImportedRequest, NoteKind, add_request,
};
use crate::collection::{Collection, Folder, Trashed};
use crate::item_path::ItemPath;
use crate::naming::sanitize_name;
use crate::request::{RequestFile, RequestKind};
use crate::script::ScriptPhase;
use crate::trash::Trash;
use fields::refresh_fields;
pub(crate) use fields::write_examples;
use std::collections::{BTreeMap, BTreeSet};

/// How an update treats vanished operations.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct UpdateOptions {
    /// Move to the trash requests whose operation left the spec, when they are still as generated.
    pub prune: bool,
    /// Update the spec imported into this folder instead of the collection root. `None` is the
    /// root; a folder that does not exist or has no entry reports the no-baseline note.
    pub folder: Option<ItemPath>,
    /// Where each pruned request is recorded so a later restore can bring it back, as a
    /// delete is; `None` records nothing.
    pub trash_log: Option<crate::trash_log::TrashLog>,
    /// The caller keeps no trash record by design (the app), so with no `trash_log` a prune's
    /// note says to restore it from the system trash rather than that it cannot be restored.
    pub restore_from_system_trash: bool,
}

impl UpdateOptions {
    fn scope(&self) -> ItemPath {
        self.folder.clone().unwrap_or_else(ItemPath::root)
    }
}

/// The baseline of `scope`, `None` when it has none or the folder is gone.
fn scope_baseline(c: &Collection, scope: &ItemPath) -> Option<SpecBaseline> {
    c.folder(scope)?;
    c.file().spec_baseline_in(scope)
}

/// One request whose spec-owned fields were, or would be, refreshed.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RequestChange {
    /// The request file.
    pub path: ItemPath,
    /// The fields rewritten from the spec, in [`ChangedField`] order.
    pub fields: Vec<ChangedField>,
}

/// What an update did, or with `plan_update` would do.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct UpdateSummary {
    /// New requests, by the path they are (or would be) written to.
    pub added: Vec<ItemPath>,
    /// Requests with refreshed fields.
    pub changed: Vec<RequestChange>,
    /// Matched requests with nothing to refresh.
    pub kept: Vec<ItemPath>,
    /// Requests the spec changed but a spec-owned field was edited locally; those fields stay.
    pub edited: Vec<ItemPath>,
    /// For each of those requests whose edited fields were found, which ones, in
    /// [`ChangedField`] order. A request skipped for unsaved edits has no entry.
    pub edited_fields: Vec<RequestChange>,
    /// Requests whose operation left the spec.
    pub orphaned: Vec<ItemPath>,
    /// Orphans removed, or that would be.
    pub pruned: Vec<ItemPath>,
    /// Everything that needs a human's attention.
    pub notes: Vec<ImportNote>,
}

/// Report what updating `c` from `bundle` would do, without writing anything.
pub fn plan_update(
    c: &Collection,
    bundle: &ImportBundle,
    o: &UpdateOptions,
) -> Result<UpdateSummary, ImportError> {
    Ok(plan(c, bundle, o)?.summary)
}

/// Update `c` from `bundle`: refresh unedited spec-owned fields, add new operations, and move
/// the recorded baseline to match. Returns the summary `plan_update` predicted, except that a
/// request that failed to write is moved out of `changed` and `added` into a note; its baseline
/// entry stays as it was and the run continues. With `o.prune`, orphans that are still as
/// generated go to `bin`. A collection with no recorded baseline is left untouched and the
/// summary carries the no-baseline note, whatever `bin` is. Otherwise, when any prune is planned
/// and `bin` is `None` or unavailable, the call returns [`ImportError::TrashUnavailable`]
/// before writing anything.
pub fn apply_update(
    c: &mut Collection,
    bundle: &ImportBundle,
    o: &UpdateOptions,
    bin: Option<&dyn Trash>,
) -> Result<UpdateSummary, ImportError> {
    let Plan {
        mut summary,
        actions,
    } = plan(c, bundle, o)?;
    // Without a baseline the plan is only its note, so that outcome wins over any refusal.
    let scope = o.scope();
    let Some(mut baseline) = scope_baseline(c, &scope) else {
        return Ok(summary);
    };
    // Pruned files are only ever moved to the trash, so refuse up front, before any write.
    let prunes = actions.iter().any(|a| matches!(a, Action::Prune { .. }));
    if prunes && !bin.is_some_and(|b| b.available()) {
        return Err(ImportError::TrashUnavailable);
    }
    // Every prune of this run shares one batch id, so one restore can bring them all back.
    let batch = crate::trash_log::new_batch();
    let planned_adds = std::mem::take(&mut summary.added);
    let mut planned_adds = planned_adds.into_iter();
    for action in actions {
        match action {
            Action::Refresh {
                path,
                key,
                fields,
                request,
                hashes,
            } => match refresh_request(c, &path, &request, &fields) {
                Ok(()) => baseline.insert(key, hashes),
                Err(e) => {
                    summary.changed.retain(|ch| ch.path != path);
                    summary
                        .notes
                        .push(note(&key, format!("not updated: {e}"), Some(&path)));
                    rollback(c, &path, &key, &mut summary);
                }
            },
            Action::Adopt { key, hashes } => baseline.insert(key, hashes),
            Action::Add {
                folders,
                name,
                request,
                key,
            } => {
                let predicted = planned_adds.next();
                // Hash first, so a failure cannot leave a written file without its entry.
                let hashes = spec_field_hashes(&request)?;
                match add_one(c, &scope, &folders, &name, &request) {
                    Ok(real) => {
                        baseline.insert(key.clone(), hashes);
                        baseline.set_file(key, real.as_str().to_owned());
                        summary.added.push(real);
                    }
                    Err(e) => {
                        let at = predicted.as_ref();
                        summary
                            .notes
                            .push(note(&key, format!("not added: {e}"), at));
                    }
                }
            }
            Action::Prune { path, key } => {
                // As a delete does: hold the log's lock and read the folder again first.
                let record = o.trash_log.as_ref().map(|log| log.locked(c.root()));
                if let Some(parent) = path.parent() {
                    c.refresh_folder(&parent);
                }
                match prune_one(c, &path, bin) {
                    Ok(trashed) => {
                        baseline.remove(&key);
                        if let Some(why) = trashed.renumber_failure() {
                            let detail =
                                format!("removed, but siblings were not renumbered: {why}");
                            summary.notes.push(note(&key, detail, Some(&path)));
                        }
                        // Each prune is its own entry in the run's batch, so a restore can bring back
                        // one or all of them.
                        let recorded = record
                            .as_ref()
                            .map(|record| record.record(&trashed, Some(&batch)));
                        let unrestorable = |why: &str| {
                            format!(
                                "moved to the trash, but it cannot be restored from here: {why}"
                            )
                        };
                        let detail = match recorded {
                            None if o.restore_from_system_trash => {
                                Some("moved to the system trash; restore it from there".to_owned())
                            }
                            None => Some(unrestorable("no data directory")),
                            Some(Ok(None)) => None,
                            Some(Ok(Some(replaced))) => Some(replaced),
                            Some(Err(why)) => Some(unrestorable(&why)),
                        };
                        if let Some(detail) = detail {
                            summary.notes.push(note(&key, detail, Some(&path)));
                        }
                    }
                    Err(e) => {
                        summary.pruned.retain(|p| *p != path);
                        summary
                            .notes
                            .push(note(&key, format!("not pruned: {e}"), Some(&path)));
                    }
                }
            }
            Action::DropStale { key } => {
                baseline.remove(&key);
            }
            Action::Relink { key, path } => baseline.set_file(key, path.as_str().to_owned()),
        }
    }
    if baseline != c.file().spec_baseline_in(&scope).unwrap_or_default() {
        c.file_mut().set_spec_baseline_in(&scope, &baseline);
    }
    if c.file().is_dirty() {
        c.file_mut().save()?;
    }
    Ok(summary)
}

/// Write `fields` into the request and save it.
fn refresh_request(
    c: &mut Collection,
    path: &ItemPath,
    request: &ImportedRequest,
    fields: &[ChangedField],
) -> Result<(), ImportError> {
    let file = c.request_mut(path).ok_or_else(|| {
        ImportError::Tree(crate::error::TreeError::NotFound { item: path.clone() })
    })?;
    refresh_fields(file, request, fields)?;
    file.save()?;
    Ok(())
}

/// Move an orphan to the trash through the tree writers, which renumber its siblings. The
/// result carries the reason the renumbering failed when the file is gone all the same.
fn prune_one(
    c: &mut Collection,
    path: &ItemPath,
    bin: Option<&dyn Trash>,
) -> Result<Trashed, crate::error::TreeError> {
    let Some(bin) = bin else {
        unreachable!("apply_update refuses a prune without a trash before this runs");
    };
    c.trash(path, bin)
}

/// After a failed write, take the disk's content back so a later plan on this collection sees
/// the file, not half-applied edits.
fn rollback(c: &mut Collection, path: &ItemPath, key: &str, summary: &mut UpdateSummary) {
    let Some(full) = c.request(path).map(|f| f.path().to_path_buf()) else {
        return;
    };
    if let Err(e) = c.revert(&full) {
        summary.notes.push(note(
            key,
            format!("could not reload after the failed update: {e}"),
            Some(path),
        ));
    }
}

/// Add one request under `base`, reusing an existing folder of the same name at each level.
fn add_one(
    c: &mut Collection,
    base: &ItemPath,
    folders: &[String],
    name: &str,
    request: &ImportedRequest,
) -> Result<ItemPath, ImportError> {
    let mut parent = base.clone();
    for folder in folders {
        let existing = c
            .folder(&parent)
            .and_then(|f| f.folders().iter().find(|d| d.name() == folder))
            .and_then(|d| c.item_path(d.path()));
        parent = match existing {
            Some(p) => p,
            None => c.create_folder(&parent, folder)?,
        };
    }
    add_request(c, &parent, name, request)
}

/// What one spec-owned field needs, from the spec's hash, the file's current hash and the
/// hash recorded at the last import or update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldState {
    /// Nothing to do, including "the user edited it but the spec did not change".
    Same,
    /// The file is as generated and the spec moved: write the spec value.
    Refresh,
    /// The user already made it equal to the new spec: only the recorded hash moves.
    Adopt,
    /// The user edited it and the spec moved too: leave it, list the request.
    Edited,
}

fn field_state(spec: &str, file: &str, recorded: &str) -> FieldState {
    if file == spec {
        return if spec == recorded {
            FieldState::Same
        } else {
            FieldState::Adopt
        };
    }
    if spec == recorded {
        return FieldState::Same;
    }
    if file == recorded {
        FieldState::Refresh
    } else {
        FieldState::Edited
    }
}

/// The method's own rule: the key already holds the spec's method, so a hand-changed method
/// leaves spec and recorded equal and `field_state` would call it Same. A method that is
/// neither the spec's nor the recorded one is Edited on every run. Without a recorded hash
/// (an older baseline) a matching method is adopted and a differing one is left unreported.
fn method_state(spec: &str, file: &str, recorded: Option<&str>) -> FieldState {
    match recorded {
        None if file == spec => FieldState::Adopt,
        None => FieldState::Same,
        Some(r) if file != spec && file != r => FieldState::Edited,
        Some(r) => field_state(spec, file, r),
    }
}

/// Tree order: a folder's requests in `Folder::requests()` order, then its subfolders.
fn walk<'a>(c: &Collection, folder: &'a Folder, out: &mut Vec<(ItemPath, &'a RequestFile)>) {
    for r in folder.requests() {
        if let Some(path) = c.item_path(r.path()) {
            out.push((path, r));
        }
    }
    for f in folder.folders() {
        walk(c, f, out);
    }
}

/// Why an orphan may not be pruned, or `None` when it may.
fn prune_blocker(f: &RequestFile, recorded: &FieldHashes) -> Option<&'static str> {
    if f.kind() != RequestKind::Http {
        Some("it is not an HTTP request")
    } else if f.is_dirty() {
        Some("it has unsaved edits")
    } else if !matches_recorded(f, recorded) {
        Some("a generated field was edited")
    } else if [
        ScriptPhase::PreRequest,
        ScriptPhase::PostResponse,
        ScriptPhase::Tests,
    ]
    .iter()
    .any(|p| !f.script(*p).trim().is_empty())
    {
        Some("it has scripts")
    } else if !f.assertions().is_empty() || !f.post_response_vars().is_empty() {
        Some("it has assertions or post-response variables")
    } else if !f.examples().is_empty() && recorded.get(ChangedField::Examples).is_none() {
        Some("it has examples")
    } else {
        None
    }
}

/// Whether every recorded hash still equals the file's; an older baseline has no method or
/// examples hash, so those are only compared when one was recorded.
fn matches_recorded(f: &RequestFile, recorded: &FieldHashes) -> bool {
    let now = file_field_hashes(f);
    ChangedField::ALL
        .into_iter()
        .all(|field| match recorded.get(field) {
            Some(h) => now.get(field) == Some(h),
            None => matches!(field, ChangedField::Method | ChangedField::Examples),
        })
}

/// One step of an update; `apply_update` replays exactly what the planner chose.
pub(crate) enum Action {
    Refresh {
        path: ItemPath,
        key: String,
        fields: Vec<ChangedField>,
        request: ImportedRequest,
        hashes: FieldHashes,
    },
    Adopt {
        key: String,
        hashes: FieldHashes,
    },
    Add {
        folders: Vec<String>,
        name: String,
        request: ImportedRequest,
        key: String,
    },
    Prune {
        path: ItemPath,
        key: String,
    },
    DropStale {
        key: String,
    },
    /// The operation's request was found somewhere other than the recorded file.
    Relink {
        key: String,
        path: ItemPath,
    },
}

pub(crate) struct Plan {
    pub(crate) summary: UpdateSummary,
    pub(crate) actions: Vec<Action>,
}

fn note(source: &str, detail: String, file: Option<&ItemPath>) -> ImportNote {
    let mut n = ImportNote::new(source, NoteKind::Dropped, detail);
    n.file = file.map(|p| p.as_str().to_owned());
    n
}

/// Flatten the bundle depth first into (folder names, request item).
fn bundle_requests<'a>(
    items: &'a [ImportedItem],
    folders: &mut Vec<String>,
    out: &mut Vec<(Vec<String>, &'a super::ImportedRequestItem)>,
) {
    for item in items {
        match item {
            ImportedItem::Request(r) => out.push((folders.clone(), r)),
            ImportedItem::Folder(f) => {
                folders.push(f.name.clone());
                bundle_requests(&f.items, folders, out);
                folders.pop();
            }
        }
    }
}

/// Where a new request in `folders` would be written: an existing same-name folder is reused,
/// otherwise the sanitised name stands in for the folder to be created.
fn predicted_path(
    c: &Collection,
    base: &ItemPath,
    folders: &[String],
    name: &str,
) -> Option<ItemPath> {
    let mut path = base.clone();
    let mut current = c.folder(base);
    for folder in folders {
        let existing = current.and_then(|f| f.folders().iter().find(|d| d.name() == folder));
        match existing {
            Some(d) => path = c.item_path(d.path())?,
            None => path = path.join(&sanitize_name(folder))?,
        }
        current = existing;
    }
    path.join(&format!("{}.yml", sanitize_name(name)))
}

/// The one planner behind `plan_update` and `apply_update`.
pub(crate) fn plan(
    c: &Collection,
    bundle: &ImportBundle,
    o: &UpdateOptions,
) -> Result<Plan, ImportError> {
    let mut summary = UpdateSummary::default();
    let mut actions = Vec::new();

    // Without a baseline nothing is known to be spec-owned, and adding would duplicate every
    // operation, so the whole update is a no-op with one explanation.
    let scope = o.scope();
    let Some(baseline) = scope_baseline(c, &scope) else {
        let detail = if scope.is_root() {
            "no update baseline: this collection was not imported from an OpenAPI spec, or by an \
             older version"
                .to_owned()
        } else {
            format!(
                "no update baseline for folder {}: it was not imported from an OpenAPI spec, or \
                 by an older version",
                scope.as_str()
            )
        };
        summary.notes.push(note("opencollection.yml", detail, None));
        return Ok(Plan { summary, actions });
    };

    let mut files = Vec::new();
    if let Some(start) = c.folder(&scope) {
        walk(c, start, &mut files);
    }
    // A scope must not claim requests that belong to a folder strictly inside it which has its
    // own spec entry; for the root that is every folder entry.
    let own = format!("{}/", scope.as_str());
    let owned: Vec<String> = c
        .file()
        .spec_baseline_folders()
        .iter()
        .filter(|f| {
            f.as_str() != scope.as_str() && (scope.is_root() || f.as_str().starts_with(&own))
        })
        .map(|f| format!("{}/", f.as_str()))
        .collect();
    files.retain(|(p, _)| !owned.iter().any(|f| p.as_str().starts_with(f.as_str())));
    let mut by_key: BTreeMap<String, Vec<(ItemPath, &RequestFile)>> = BTreeMap::new();
    let mut by_path: BTreeMap<String, (ItemPath, &RequestFile)> = BTreeMap::new();
    for (path, f) in files {
        if f.kind() == RequestKind::Http {
            by_path.insert(path.as_str().to_owned(), (path.clone(), f));
            by_key
                .entry(operation_key(f.method(), f.url()))
                .or_default()
                .push((path, f));
        }
    }
    for e in c.load_errors() {
        let file = c.item_path(e.path());
        summary.notes.push(note(
            &e.path().display().to_string(),
            format!("could not be read and was skipped: {}", e.message()),
            file.as_ref(),
        ));
    }

    let mut spec_requests = Vec::new();
    bundle_requests(
        &bundle.collection.items,
        &mut Vec::new(),
        &mut spec_requests,
    );
    let mut spec_keys = BTreeSet::new();
    // Examples were not spec-owned before, and older imports wrote none.
    let absent = fields::absent_hash();
    for (folders, item) in spec_requests {
        let request = &item.request;
        let key = operation_key(&request.method, &request.url);
        if !spec_keys.insert(key.clone()) {
            summary.notes.push(note(
                &key,
                "the spec lists this operation twice; only the first is used (duplicate)"
                    .to_owned(),
                None,
            ));
            continue;
        }
        let Some(matches) = locate(&baseline, &by_path, &by_key, &key) else {
            if let Some(path) = predicted_path(c, &scope, &folders, &item.name) {
                summary.added.push(path);
                actions.push(Action::Add {
                    folders,
                    name: item.name.clone(),
                    request: request.clone(),
                    key,
                });
            }
            continue;
        };
        let (path, file) = &matches[0];
        for (extra, _) in &matches[1..] {
            summary.notes.push(note(
                &key,
                format!("duplicate of {}; left alone", path.as_str()),
                Some(extra),
            ));
        }
        // A request found anywhere but its recorded file is relinked, so a renamed or moved
        // file keeps its identity through later edits of its path.
        if baseline.get(&key).is_some() && baseline.file(&key) != Some(path.as_str()) {
            actions.push(Action::Relink {
                key: key.clone(),
                path: path.clone(),
            });
        }
        let Some(recorded) = baseline.get(&key) else {
            summary.kept.push(path.clone());
            summary.notes.push(note(
                &key,
                "not generated by the spec (no baseline entry); left alone".to_owned(),
                Some(path),
            ));
            continue;
        };
        if file.is_dirty() {
            summary.edited.push(path.clone());
            continue;
        }
        let spec_hashes = spec_field_hashes(request)?;
        let file_hashes = file_field_hashes(file);
        let mut refresh = Vec::new();
        let mut edited_in = Vec::new();
        let mut adopted = false;
        let mut new_hashes = recorded.clone();
        let mut next = Vec::new();
        for field in ChangedField::ALL {
            let spec = spec_hashes.get(field).unwrap_or("");
            let on_file = file_hashes.get(field).unwrap_or("");
            let state = match (field, recorded.get(field)) {
                (ChangedField::Method, recorded) => method_state(spec, on_file, recorded),
                (ChangedField::Examples, None) => field_state(spec, on_file, &absent),
                (_, recorded) => field_state(spec, on_file, recorded.unwrap_or("")),
            };
            match state {
                FieldState::Same => {}
                FieldState::Refresh => refresh.push(field),
                FieldState::Adopt => adopted = true,
                FieldState::Edited => edited_in.push(field),
            }
            if matches!(state, FieldState::Refresh | FieldState::Adopt) {
                next.push((field, spec.to_owned()));
            }
        }
        for (field, hash) in next {
            new_hashes = replace_hash(&new_hashes, field, hash);
        }
        let edited = !edited_in.is_empty();
        if edited {
            summary.edited.push(path.clone());
            summary.edited_fields.push(RequestChange {
                path: path.clone(),
                fields: edited_in,
            });
        }
        if !refresh.is_empty() {
            summary.changed.push(RequestChange {
                path: path.clone(),
                fields: refresh.clone(),
            });
            actions.push(Action::Refresh {
                path: path.clone(),
                key,
                fields: refresh,
                request: request.clone(),
                hashes: new_hashes,
            });
        } else {
            if !edited {
                summary.kept.push(path.clone());
            }
            if adopted {
                actions.push(Action::Adopt {
                    key,
                    hashes: new_hashes,
                });
            }
        }
    }

    for key in baseline.keys().filter(|k| !spec_keys.contains(*k)) {
        let Some((path, file)) = locate(&baseline, &by_path, &by_key, key).map(|mut m| m.remove(0))
        else {
            if o.prune {
                actions.push(Action::DropStale {
                    key: key.to_owned(),
                });
            }
            continue;
        };
        summary.orphaned.push(path.clone());
        if !o.prune {
            continue;
        }
        let recorded = baseline.get(key).expect("key comes from the baseline");
        match prune_blocker(file, recorded) {
            None => {
                summary.pruned.push(path.clone());
                actions.push(Action::Prune {
                    path: path.clone(),
                    key: key.to_owned(),
                });
            }
            Some(why) => summary
                .notes
                .push(note(key, format!("not pruned: {why}"), Some(&path))),
        }
    }
    Ok(Plan { summary, actions })
}

/// The requests of operation `key`, best match first: the recorded file wherever its method and
/// URL have gone, else the files whose current key is `key`, skipping any file another
/// operation has recorded as its own.
fn locate<'a>(
    baseline: &SpecBaseline,
    by_path: &BTreeMap<String, (ItemPath, &'a RequestFile)>,
    by_key: &BTreeMap<String, Vec<(ItemPath, &'a RequestFile)>>,
    key: &str,
) -> Option<Vec<(ItemPath, &'a RequestFile)>> {
    // A file two keys both claim belongs to neither: a hand-edited baseline must not make one
    // path the target of two actions.
    let claimants = |file: &str| {
        baseline
            .keys()
            .filter(|k| baseline.file(k) == Some(file))
            .count()
    };
    let recorded = baseline
        .file(key)
        .filter(|f| claimants(f) == 1)
        .and_then(|f| by_path.get(f));
    let mut found: Vec<_> = recorded.cloned().into_iter().collect();
    for candidate in by_key.get(key).into_iter().flatten() {
        let file = candidate.0.as_str();
        // Uncontested ownership by another key keeps the file out of this key's fallback.
        let taken = baseline.file(key) != Some(file) && claimants(file) == 1;
        if !taken && recorded.is_none_or(|r| r.0 != candidate.0) {
            found.push(candidate.clone());
        }
    }
    (!found.is_empty()).then_some(found)
}

/// `hashes` with one field's hash replaced.
fn replace_hash(hashes: &FieldHashes, field: ChangedField, hash: String) -> FieldHashes {
    FieldHashes::new(
        hashes
            .iter()
            .filter(|(f, _)| *f != field)
            .map(|(f, h)| (f, h.to_owned()))
            .chain([(field, hash)]),
    )
}
