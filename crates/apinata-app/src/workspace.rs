//! The open collections of the window: opening, closing and restoring them, the only path
//! that sends them commands, and the session's open and recent lists.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use apinata_core::import::ImportedRequest;
use apinata_core::{
    CommitError, DraftSnapshot, EnvOverlay, EnvVariable, GitError, GitOptions, GitState,
    GlobalEnvironments, ItemPath, OpenError, RequestKind, RuntimeVars, ScriptPhase, SecretStore,
    StageState, WorkspaceFile, collection_status_with, commit, is_workspace_dir, stage, unstage,
};
use gpui_kit::{App, AppContext, Context, Entity, EventEmitter, SharedString, Subscription, Task};

use crate::actor::Faults;
use crate::busy::{BusyState, BusyTracker};
use crate::closing::{Unsaved, dirty_targets};
use crate::command::{Command, Field, Notice, NoticeKind, Target, TreeOp};
use crate::draft_store;
use crate::git_status::{
    GitChanges, RefreshGate, after_refresh, commit_failure_text, staged_change_note,
};
use crate::history_key::new_history_key;
use crate::last_response::{StoreOp, enqueue};
use crate::model::CollectionModel;
use crate::persisted::Persisted;
use crate::prompt_vars::PromptMemory;
use crate::session::{SavedExample, SavedPanel, SavedPanelKind, SavedTab, Session};
use crate::snapshot::{CollectionSnapshot, ConfigView, Content, DiskState, RequestView};
use crate::tabs::{TabEntry, TabId, TabKind, Tabs};
use crate::watch::rules::{WatchMsg, translate};
use crate::watch::set::{EventHandler, Watch, WatchError, WatchFactory};
use crate::watch::watcher::CollectionWatcher;

/// What happened to the workspace's collections.
#[derive(Debug, Clone, PartialEq)]
pub enum WorkspaceEvent {
    /// The collection at this root delivered its first snapshot.
    Opened(PathBuf),
    /// The collection at `root` could not be opened and was dropped.
    OpenFailed {
        /// The root that failed.
        root: PathBuf,
        /// Why, for an error dialog.
        message: String,
    },
    /// The collection at `root` is in the legacy .bru format and was dropped unopened.
    LegacyFormat {
        /// The legacy root.
        root: PathBuf,
    },
    /// The actor of the collection at `root` reported these notices, each exactly once.
    Notices {
        /// The collection's root.
        root: PathBuf,
        /// The notices, in command order.
        notices: Vec<Notice>,
        /// The open generation that reported them; see [`CollectionEntry::generation`].
        generation: u64,
    },
    /// Every notice of `root` up to command `acked` has been delivered; emitted after each
    /// snapshot, so editors drop their bookkeeping of those commands here and never sooner.
    Delivered {
        /// The collection's root.
        root: PathBuf,
        /// The snapshot's acknowledged command id.
        acked: u64,
        /// The open generation the snapshot belongs to.
        generation: u64,
    },
    /// Request or folder settings tabs of `root` closed because their items vanished from the
    /// collection.
    TabsClosed {
        /// The collection's root.
        root: PathBuf,
        /// The items whose tabs closed, in tab order.
        items: Vec<ItemPath>,
        /// How many tabs closed; a runner tab holds several items, so this can be fewer.
        tabs: usize,
    },
    /// A draft opened as the active tab with this id; its editor should take focus.
    DraftOpened(TabId),
    /// Members of the run-selection or folder runner tab with this id left or came back, so
    /// its list is stale.
    SelectionChanged(TabId),
    /// The breadcrumb asked to show this folder of `root` in the sidebar.
    RevealFolder {
        /// The collection's root.
        root: PathBuf,
        /// The folder to reveal.
        folder: ItemPath,
    },
    /// The shared request split ratio changed, so every open editor should lay out again.
    SplitRatio,
    /// A Revert of environment `name` of `root` was just sent, from anywhere; editors drop
    /// secret typing for it before any later input can store it.
    EnvReverted {
        /// The collection's root.
        root: PathBuf,
        /// The environment reverted.
        name: String,
    },
    /// Opening a folder as a reference workspace failed or skipped something; the text says what.
    FolderNotice(String),
    /// Watching the collection at `root`, or the global directory, stopped; changes on disk
    /// need a manual reload.
    WatchFailed {
        /// The collection's root, or the global directory.
        root: PathBuf,
        /// Why, for display.
        message: String,
    },
}

// Why the global directory is re-read: asked for (opening the editor, the picker) or the
// watcher noticing another program's change.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ReloadCause {
    Asked,
    Watched,
}

/// One change to the global environments, applied by [`Workspace::edit_globals`].
#[non_exhaustive]
#[derive(Clone)]
pub enum GlobalOp {
    /// Create an environment with this name.
    Create(String),
    /// Copy an environment, its stored secret values included, under a fresh name.
    Duplicate(String),
    /// Rename an environment; those extending it follow, and its secrets move.
    Rename {
        /// The current name.
        from: String,
        /// The new name.
        to: String,
    },
    /// Delete an environment and its stored secrets.
    Delete(String),
    /// Make a saved variable secret or plain, moving its value between file and store.
    SetSecret {
        /// The environment.
        name: String,
        /// The variable.
        variable: String,
        /// The requested state.
        secret: bool,
    },
    /// Replace `name`'s variables, save the file, then store each (variable, value) secret.
    Save {
        /// The environment.
        name: String,
        /// Its new variables, in file order.
        variables: Vec<EnvVariable>,
        /// Secret values to store, as (variable, value).
        secrets: Vec<(String, String)>,
    },
    /// Remove named orphans of an environment. Warnings become the error text.
    RemoveOrphans {
        /// The environment.
        name: String,
        /// The orphans to remove.
        names: Vec<String>,
    },
    /// Remove named stored secrets of an environment with no file. Warnings become the error
    /// text; refused if the environment has a file again.
    RemoveFileless {
        /// The environment.
        name: String,
        /// The variables to remove.
        names: Vec<String>,
    },
}

// Save's secret values are plaintext, so only their variable names are printed.
impl std::fmt::Debug for GlobalOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GlobalOp::Create(name) => f.debug_tuple("Create").field(name).finish(),
            GlobalOp::Rename { from, to } => (f.debug_struct("Rename"))
                .field("from", from)
                .field("to", to)
                .finish(),
            GlobalOp::Duplicate(name) => f.debug_tuple("Duplicate").field(name).finish(),
            GlobalOp::Delete(name) => f.debug_tuple("Delete").field(name).finish(),
            GlobalOp::SetSecret {
                name,
                variable,
                secret,
            } => (f.debug_struct("SetSecret"))
                .field("name", name)
                .field("variable", variable)
                .field("secret", secret)
                .finish(),
            GlobalOp::Save {
                name,
                variables,
                secrets,
            } => {
                let secrets: Vec<&str> = secrets.iter().map(|(v, _)| v.as_str()).collect();
                (f.debug_struct("Save"))
                    .field("name", name)
                    .field("variables", variables)
                    .field("secrets", &secrets)
                    .finish_non_exhaustive()
            }
            GlobalOp::RemoveOrphans { name, names } => (f.debug_struct("RemoveOrphans"))
                .field("name", name)
                .field("names", names)
                .finish(),
            GlobalOp::RemoveFileless { name, names } => (f.debug_struct("RemoveFileless"))
                .field("name", name)
                .field("names", names)
                .finish(),
        }
    }
}

impl GlobalOp {
    // Where the active name goes after this op succeeds: a rename moves it, a delete drops it.
    fn carry(&self, name: Option<String>) -> Option<String> {
        match self {
            GlobalOp::Rename { from, to } if name.as_ref() == Some(from) => Some(to.clone()),
            GlobalOp::Delete(gone) if name.as_ref() == Some(gone) => None,
            _ => name,
        }
    }

    // Applies the op to a fresh open of `dir`, then reopens it so the published set is what
    // the directory now holds; secrets are stored only after the file saved.
    fn apply(&self, dir: PathBuf, store: &dyn SecretStore) -> Result<GlobalEnvironments, String> {
        fn text(e: impl std::fmt::Display) -> String {
            e.to_string()
        }
        let mut globals = GlobalEnvironments::open(&dir).map_err(text)?;
        match self {
            GlobalOp::Create(name) => drop(globals.create_environment(name, store).map_err(text)?),
            GlobalOp::Duplicate(from) => {
                drop(globals.duplicate_environment(from, store).map_err(text)?)
            }
            GlobalOp::Rename { from, to } => {
                drop(globals.rename_environment(from, to, store).map_err(text)?)
            }
            GlobalOp::Delete(name) => drop(globals.delete_environment(name, store).map_err(text)?),
            GlobalOp::SetSecret {
                name,
                variable,
                secret,
            } => drop(
                globals
                    .set_variable_secret(name, variable, *secret, store)
                    .map_err(text)?,
            ),
            GlobalOp::Save {
                name,
                variables,
                secrets,
            } => {
                let Some(env) = globals.environment_mut(name) else {
                    return Err(globals.environment_not_found(name).to_string());
                };
                env.set_variables(variables.clone());
                env.save().map_err(text)?;
                for (variable, value) in secrets {
                    let key = globals.secret_key(name, variable);
                    store.set(&key, value).map_err(text)?;
                }
            }
            GlobalOp::RemoveOrphans { name, names } | GlobalOp::RemoveFileless { name, names } => {
                let warnings = match self {
                    GlobalOp::RemoveOrphans { .. } => {
                        globals.remove_orphaned_secrets(name, names, store)
                    }
                    _ => globals.remove_fileless_secrets(name, names, store),
                }
                .map_err(text)?;
                if !warnings.is_empty() {
                    let warnings: Vec<String> = warnings.iter().map(text).collect();
                    return Err(warnings.join("; "));
                }
            }
        }
        GlobalEnvironments::open(dir).map_err(text)
    }
}

/// The answer of [`Workspace::global_secrets_to_tidy`]: the orphans, when an environment was
/// named, and the fileless groups.
pub type GlobalTidy = (
    Option<Result<Vec<String>, String>>,
    Result<Vec<crate::orphan_section::FilelessGroup>, String>,
);

/// One collection the workspace holds open.
pub struct CollectionEntry {
    root: PathBuf,
    model: Entity<CollectionModel>,
    opened: bool,
    busy: bool,
    tracker: BusyTracker,
    recheck: Option<Task<()>>,
    watcher: Option<CollectionWatcher>,
    dirs: BTreeSet<PathBuf>,
    watch_failed: Option<String>,
    pump: Option<Task<()>>,
    env_sent: u64,
    // Git status: the latest state, the one-run gate, finished runs, and the largest write
    // command whose reply should refresh it.
    git: Option<Arc<GitChanges>>,
    git_gate: RefreshGate,
    git_runs: u64,
    git_write: Option<u64>,
    // Unique per open in the session, so it tells this entry from a reopened one.
    generation: u64,
    _observer: Subscription,
}

impl CollectionEntry {
    /// The collection's absolute root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether the collection's actor has stalled on pending commands.
    pub fn busy(&self) -> bool {
        self.busy
    }

    /// The open generation: unique for every open of a collection in this session, so
    /// state kept for one instance is never taken for a reopened one's.
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

/// A pending request to show an environment with a new variable row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvIntent {
    /// The environment to select; `None` with `create`.
    pub select: Option<String>,
    /// The variable to add, or to focus when the environment already has it.
    pub add_var: String,
    /// The collection has no environment: start naming a new one first.
    pub create: bool,
}

// A root's kept drafts reopening: those still to answer, the wanted draft's history key, the
// saved active tab and the tab active when they were sent, which picks the active tab at the end.
struct Restoring {
    pending: usize,
    active_draft: Option<String>,
    saved_active: Option<TabId>,
    at_send: Option<TabId>,
}

/// A tab a delete into the trash closed: what it showed, where it stood and how.
struct TrashedTab {
    index: usize,
    kind: TabKind,
    preview: bool,
    pinned: bool,
    active: bool,
}

/// Script texts typed in the app, approved once the write of `command` is confirmed.
struct AwaitedApproval {
    root: PathBuf,
    command: u64,
    // The file the write is meant to leave present; `None` for a draft, whose write is
    // confirmed by the actor's `Moved` notice.
    target: Option<Target>,
    texts: Vec<String>,
}

/// One git write for `Workspace::run_git_write`; the path `"."` stages the collection.
#[derive(Clone, Debug)]
pub enum GitWrite {
    /// Stage these paths, relative to the collection.
    Stage(Vec<PathBuf>),
    /// Unstage these paths, relative to the collection.
    Unstage(Vec<PathBuf>),
    /// Commit what is staged with this message.
    Commit(String),
}

/// How a git write ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitWriteDone {
    /// A stage or unstage went through.
    Done,
    /// The commit landed.
    Committed {
        /// The abbreviated id of the new commit.
        short_id: String,
        /// The commit's subject line.
        subject: String,
    },
    /// The write was refused or failed; the text is shown to the user as is.
    Failed(String),
}

// A workspace folder opened read-only: the collections it listed, its environments, and the
// one of them that is active as the global. The active name is never persisted.
struct ReferenceWorkspace {
    name: String,
    dir: PathBuf,
    roots: Vec<PathBuf>,
    envs: Option<Arc<GlobalEnvironments>>,
    active: Option<String>,
}

const WORKSPACE_SUFFIX: &str = " (workspace)";

/// A trash that is never there: the default of a workspace given none, so deletes ask first
/// and are permanent.
#[derive(Debug, Default)]
pub struct NoTrash;

impl apinata_core::Trash for NoTrash {
    fn available(&self) -> bool {
        false
    }

    fn trash(
        &self,
        _: &std::path::Path,
    ) -> Result<apinata_core::TrashHandle, apinata_core::TrashError> {
        Err(apinata_core::TrashError::Unavailable(
            "no trash is configured".into(),
        ))
    }

    fn restore(&self, _: &apinata_core::TrashHandle) -> Result<(), apinata_core::TrashError> {
        Err(apinata_core::TrashError::Unavailable(
            "no trash is configured".into(),
        ))
    }
}

/// The window's open collections and its session.
pub struct Workspace {
    store: Arc<dyn SecretStore>,
    // Where deleted items go; none by default, so a delete asks and is permanent.
    trash: Arc<dyn apinata_core::Trash>,
    open_faults: Faults,
    session: Entity<Persisted<Session>>,
    collections: Vec<CollectionEntry>,
    tabs: Tabs,
    factory: Option<WatchFactory>,
    // `CreateDraft` commands sent by `new_draft`, by root and command id, awaiting `Created`;
    // ids restart with each actor, so a closed root's entries are dropped.
    drafts: HashSet<(PathBuf, u64)>,
    // Reopened drafts among `drafts`: the history key and kept send their tab takes on open.
    draft_keys: HashMap<(PathBuf, u64), (ItemPath, Option<u64>)>,
    // Saved drafts by root and old history key, to the file they became, so a send that was
    // in flight at the save is kept under the saved request.
    saved_drafts: HashMap<(PathBuf, ItemPath), ItemPath>,
    // Saves of a draft into another collection, by target root and command id, to the source
    // root, draft and tab; the target's `Created` notice finishes the move.
    cross_saves: HashMap<(PathBuf, u64), (PathBuf, ItemPath, TabId)>,
    // Per open run-selection tab, the members already missing after the last prune, so a
    // close only weighs the members that leave in its own delivery.
    selection_gone: HashMap<TabId, HashSet<ItemPath>>,
    // Per open runner or run-selection tab, what its plan lists after the last prune of its
    // root, so a request that leaves, returns, or changes a shown field re-plans the list.
    runner_members: HashMap<TabId, RunnerRecord>,
    // Typed script texts waiting for the write they ride on to be confirmed.
    awaiting: Vec<AwaitedApproval>,
    // Imports among `drafts`: applied to the draft once it is created.
    draft_imports: HashMap<(PathBuf, u64), ImportedRequest>,
    // Kept sends a reopened draft's editor shows once it is built.
    kept_views: HashMap<TabId, u64>,
    // The tab a root's restore activated from its saved active tab, which a later root's
    // restore leaves active.
    restored_active: Option<TabId>,
    // Roots whose kept drafts are reopening, until the last one answers.
    restoring: HashMap<PathBuf, Restoring>,
    // The `CreateDraftFrom` commands a restore sent, by root and command id, with the file slot
    // of the kept draft each reopens.
    restore_ids: HashMap<(PathBuf, u64), u32>,
    // Commands of a sidebar batch, by root and id, whose notices the batch summarises instead
    // of the shell toasting each one.
    batched: HashSet<(PathBuf, u64)>,
    // The tabs each delete into the trash closed, by root and trash ticket, so restoring
    // that ticket puts them back; `restores` maps a sent Restore's command id to its ticket.
    trashed_tabs: HashMap<(PathBuf, u64), Vec<TrashedTab>>,
    restores: HashMap<(PathBuf, u64), u64>,
    // The global environments directory, its latest loaded snapshot and the active name.
    global_dir: Option<PathBuf>,
    // The hidden scratch collection's directory, once the app names it.
    scratch_dir: Option<PathBuf>,
    globals: Option<Arc<GlobalEnvironments>>,
    selected_global: Option<String>,
    // Bumped by every load and edit; a load result older than this is dropped.
    global_gen: u64,
    // Whether a load has succeeded, which is when the session's saved global applies.
    global_loaded: bool,
    // Edits waiting for the one drain task, which runs them in order while `global_draining`;
    // each carries where its outcome goes.
    global_queue: VecDeque<(GlobalOp, async_channel::Sender<Result<(), String>>)>,
    global_draining: bool,
    // A reload asked for while edits drain; it runs once the queue empties, so its read
    // never precedes an edit's write.
    global_reload_waiting: Option<ReloadCause>,
    // Whether the latest published global snapshot came from the directory's watcher.
    globals_watched: bool,
    global_error: Option<String>,
    // The watch on the global directory and the task reloading it after each batch.
    global_watch: Option<(Box<dyn Watch>, Task<()>)>,
    // The reference workspace opened last, read-only, whose environments join the picker.
    reference: Option<ReferenceWorkspace>,
    // Counts the commands that change a document's content, so autosave can tell an edit
    // from a save, reload or conflict.
    edit_generation: u64,
    // How git runs for status, `None` while git is off.
    git_options: Option<GitOptions>,
    // Roots a branch switch holds read-only: edits and writes to them are refused.
    switching: BTreeSet<PathBuf>,
    // The next open generation; see `CollectionEntry::generation`.
    generations: u64,
    // The request pane's share of every opened split in this window; in memory only.
    split_ratio: f32,
    // The script phase last chosen per request, by absolute root and item; in memory only.
    script_sides: HashMap<(PathBuf, ItemPath), ScriptPhase>,
    // "Add to environment" requests waiting for an environment editor, by absolute root.
    env_intents: HashMap<PathBuf, EnvIntent>,
}

impl EventEmitter<WorkspaceEvent> for Workspace {}

impl Workspace {
    /// An empty workspace that reads secrets from `store` and records roots in `session`.
    pub fn new(
        store: Arc<dyn SecretStore>,
        session: Entity<Persisted<Session>>,
        _cx: &mut Context<Self>,
    ) -> Self {
        Workspace {
            store,
            trash: Arc::new(NoTrash),
            open_faults: Faults::default(),
            session,
            collections: Vec::new(),
            tabs: Tabs::new(),
            factory: None,
            drafts: HashSet::new(),
            draft_imports: HashMap::new(),
            draft_keys: HashMap::new(),
            saved_drafts: HashMap::new(),
            cross_saves: HashMap::new(),
            selection_gone: HashMap::new(),
            runner_members: HashMap::new(),
            awaiting: Vec::new(),
            kept_views: HashMap::new(),
            restored_active: None,
            restoring: HashMap::new(),
            restore_ids: HashMap::new(),
            batched: HashSet::new(),
            trashed_tabs: HashMap::new(),
            restores: HashMap::new(),
            global_dir: None,
            scratch_dir: None,
            globals: None,
            selected_global: None,
            global_gen: 0,
            global_loaded: false,
            global_queue: VecDeque::new(),
            global_draining: false,
            global_reload_waiting: None,
            globals_watched: false,
            global_error: None,
            global_watch: None,
            reference: None,
            edit_generation: 0,
            git_options: None,
            switching: BTreeSet::new(),
            generations: 0,
            split_ratio: crate::style::DEFAULT_SPLIT_RATIO,
            script_sides: HashMap::new(),
            env_intents: HashMap::new(),
        }
    }

    /// The request pane's share of an opened request split, 0.5 until a divider is dragged.
    pub fn split_ratio(&self) -> f32 {
        self.split_ratio
    }

    /// Records a dragged split's request share, clamped to 0.05..=0.95; a NaN is ignored. A
    /// changed ratio emits [`WorkspaceEvent::SplitRatio`] so every open editor redraws.
    pub fn set_split_ratio(&mut self, ratio: f32, cx: &mut Context<Self>) {
        if ratio.is_nan() {
            return;
        }
        let ratio = ratio.clamp(0.05, 0.95);
        if ratio != self.split_ratio {
            self.split_ratio = ratio;
            cx.emit(WorkspaceEvent::SplitRatio);
        }
    }

    /// The script phase last chosen for the request `item` of `root`, if one was.
    pub fn script_side(&self, root: &Path, item: &ItemPath) -> Option<ScriptPhase> {
        self.script_sides
            .get(&(absolute(root), item.clone()))
            .copied()
    }

    /// Records the script phase chosen for the request `item` of `root`.
    pub fn set_script_side(&mut self, root: &Path, item: ItemPath, side: ScriptPhase) {
        self.script_sides.insert((absolute(root), item), side);
    }

    /// Delete into `trash` from now on. Collections opened earlier keep the trash they opened with.
    pub fn set_trash(&mut self, trash: Arc<dyn apinata_core::Trash>) {
        self.open_faults.trash = Some(trash.clone());
        self.trash = trash;
    }

    /// Whether a delete can go to a trash with an Undo, rather than being permanent.
    pub fn trash_available(&self) -> bool {
        self.trash.available()
    }

    /// The secret store every collection of this workspace reads and writes.
    pub fn store(&self) -> &Arc<dyn SecretStore> {
        &self.store
    }

    /// The session this workspace records into.
    pub fn session(&self) -> &Entity<Persisted<Session>> {
        &self.session
    }

    /// The open collections, in the order they were opened.
    pub fn collections(&self) -> &[CollectionEntry] {
        &self.collections
    }

    /// Test-only: the actor faults given to collections opened from now on.
    #[doc(hidden)]
    pub fn set_open_faults(&mut self, faults: Faults) {
        self.open_faults = faults;
    }

    /// Opens a folder from the Open collection prompt: a reference workspace when it holds
    /// `workspace.yml` and no `opencollection.yml`, else the collection at `path`.
    pub fn open_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if is_workspace_dir(&path) && !path.join("opencollection.yml").is_file() {
            self.open_reference(path, cx);
        } else {
            self.open(path, cx);
        }
    }

    /// Opens each launch root with `open_path`, so a workspace folder works as well as a
    /// collection; roots already open are skipped.
    pub fn open_roots(&mut self, roots: Vec<PathBuf>, cx: &mut Context<Self>) {
        for root in roots {
            self.open_path(root, cx);
        }
    }

    // Opens every listed collection that is a directory and loads the workspace's
    // environments, replacing any reference workspace open before.
    fn open_reference(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        let file = match WorkspaceFile::open(&dir) {
            Ok(file) => file,
            Err(e) => {
                cx.emit(WorkspaceEvent::FolderNotice(e.to_string()));
                return;
            }
        };
        let mut roots = Vec::new();
        let mut missing = Vec::new();
        for c in file.collections() {
            if c.root().is_dir() {
                roots.push(absolute(c.root()));
            } else {
                missing.push(c.name().to_owned());
            }
        }
        if file.skipped() > 0 {
            missing.push(format!("{} invalid entries", file.skipped()));
        }
        if !missing.is_empty() {
            cx.emit(WorkspaceEvent::FolderNotice(format!(
                "Workspace {} skipped: {}",
                file.name(),
                missing.join(", ")
            )));
        }
        let had_active = self.reference.as_ref().is_some_and(|r| r.active.is_some());
        if roots.is_empty() {
            self.reference = None;
            self.save_workspace(None, cx);
            if had_active {
                self.broadcast_globals(cx);
            }
            cx.emit(WorkspaceEvent::FolderNotice(format!(
                "Workspace {} lists no collection that can be opened",
                file.name()
            )));
            cx.notify();
            return;
        }
        self.reference = Some(ReferenceWorkspace {
            name: file.name().to_owned(),
            dir: absolute(&dir),
            roots: roots.clone(),
            envs: None,
            active: None,
        });
        self.save_workspace(Some(absolute(&dir)), cx);
        if had_active {
            self.broadcast_globals(cx);
        }
        for root in roots {
            self.open(root, cx);
        }
        let key = absolute(&dir);
        let loaded = cx.background_spawn(async move { file.environments() });
        cx.spawn(async move |this, cx| {
            let loaded = loaded.await;
            this.update(cx, |w, cx| w.apply_reference_envs(key, loaded, cx))
        })
        .detach();
        cx.notify();
    }

    // Keeps the loaded environments only while the workspace they came from is still open.
    fn apply_reference_envs(
        &mut self,
        dir: PathBuf,
        loaded: Result<GlobalEnvironments, OpenError>,
        cx: &mut Context<Self>,
    ) {
        let Some(r) = self.reference.as_mut().filter(|r| r.dir == dir) else {
            return;
        };
        match loaded {
            Ok(envs) => r.envs = Some(Arc::new(envs)),
            Err(e) => cx.emit(WorkspaceEvent::FolderNotice(format!(
                "Workspace environments not loaded: {e}"
            ))),
        }
        cx.notify();
    }

    /// The name of the open reference workspace.
    pub fn reference_workspace_name(&self) -> Option<&str> {
        self.reference.as_ref().map(|r| r.name.as_str())
    }

    /// What the global picker lists: the profile names, then `<name> (workspace)` for each
    /// environment of the open reference workspace.
    pub fn global_picker_items(&self) -> Vec<String> {
        let mut items = self.global_environments();
        let envs = self.reference.as_ref().and_then(|r| r.envs.as_ref());
        for e in envs.into_iter().flat_map(|g| g.environments()) {
            items.push(format!("{}{WORKSPACE_SUFFIX}", e.name()));
        }
        items
    }

    /// The picker item of the active global.
    pub fn selected_picker_item(&self) -> Option<String> {
        match self.reference.as_ref().and_then(|r| r.active.as_ref()) {
            Some(name) => Some(format!("{name}{WORKSPACE_SUFFIX}")),
            None => self.selected_global.clone(),
        }
    }

    /// Activates the picker item `item` (a profile name, or `<name> (workspace)`), and
    /// broadcasts it; anything not listed selects none.
    pub fn select_picker_item(&mut self, item: &str, cx: &mut Context<Self>) {
        // A workspace label wins a clash with a profile name, so both stay selectable by
        // their own plain names.
        let workspace = item.strip_suffix(WORKSPACE_SUFFIX).filter(|n| {
            (self.reference.as_ref().and_then(|r| r.envs.as_ref()))
                .is_some_and(|g| g.environment(n).is_some())
        });
        let profile = self.is_global(item).then(|| item.to_owned());
        if let Some(name) = workspace.map(str::to_owned) {
            if let Some(r) = self.reference.as_mut() {
                r.active = Some(name);
            }
            self.broadcast_globals(cx);
            cx.notify();
        } else {
            self.select_global_environment(profile, cx);
        }
    }

    /// Opens the collection at `root`, made absolute; does nothing when it is already open.
    pub fn open(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        let root = absolute(&root);
        if self.entry(&root).is_some() {
            return;
        }
        let model = CollectionModel::open_with_faults(
            root.clone(),
            self.store.clone(),
            self.open_faults.clone(),
            cx,
        );
        // Each instance's events carry its own generation, since a subscriber that looked the
        // root up when the event arrives could find a reopened instance instead.
        let (key, generation) = (root.clone(), self.generations);
        let observer = cx.observe(&model, move |this, model, cx| {
            this.changed(&key, generation, model, cx)
        });
        self.collections.push(CollectionEntry {
            root: root.clone(),
            model,
            opened: false,
            busy: false,
            tracker: BusyTracker::default(),
            recheck: None,
            watcher: None,
            dirs: BTreeSet::new(),
            watch_failed: None,
            pump: None,
            env_sent: 0,
            git: None,
            git_gate: RefreshGate::default(),
            git_runs: 0,
            git_write: None,
            generation,
            _observer: observer,
        });
        self.generations += 1;
        if let Some(globals) = self.global_selection() {
            self.send(&root, Command::SetGlobalEnvironment(Some(globals)), cx);
        }
        cx.notify();
    }

    /// Closes the collection at `root`, ending its actor, and drops it from the session's
    /// open list; it stays in recents.
    pub fn close(&mut self, root: &Path, cx: &mut Context<Self>) {
        let root = absolute(root);
        self.collections.retain(|c| c.root != root);
        self.tabs.retain(|e| e.root != root);
        self.drafts.retain(|(r, _)| *r != root);
        self.draft_keys.retain(|(r, _), _| *r != root);
        self.saved_drafts.retain(|(r, _), _| *r != root);
        self.awaiting.retain(|a| a.root != root);
        self.cross_saves
            .retain(|(target, _), (from, _, _)| *target != root && *from != root);
        self.draft_imports.retain(|(r, _), _| *r != root);
        self.env_intents.remove(&root);
        self.script_sides.retain(|(r, _), _| *r != root);
        self.restoring.remove(&root);
        self.restore_ids.retain(|(r, _), _| *r != root);
        self.batched.retain(|(r, _)| *r != root);
        self.trashed_tabs.retain(|(r, _), _| *r != root);
        self.restores.retain(|(r, _), _| *r != root);
        let tabs = &self.tabs;
        self.kept_views.retain(|t, _| tabs.get(*t).is_some());
        self.forget_open(&root, cx);
        draft_store::return_root(&root, cx);
        self.release_reference(&root, cx);
        cx.notify();
    }

    // Drops the reference workspace once the last collection it listed is closed.
    fn release_reference(&mut self, root: &Path, cx: &mut Context<Self>) {
        let Some(r) = self.reference.as_mut() else {
            return;
        };
        if !r.roots.iter().any(|x| x == root) {
            return;
        }
        let open = |x: &PathBuf| self.collections.iter().any(|c| &c.root == x);
        if r.roots.iter().any(open) {
            return;
        }
        let was_active = r.active.is_some();
        self.reference = None;
        self.save_workspace(None, cx);
        if was_active {
            self.broadcast_globals(cx);
        }
    }

    /// Marks these command ids of `root` as part of a batch, so the shell leaves their notices
    /// to the batch's summary.
    pub fn track_batch(&mut self, root: &Path, ids: &[u64]) {
        let root = absolute(root);
        self.batched
            .extend(ids.iter().map(|id| (root.clone(), *id)));
    }

    /// Whether `command` of `root` was tracked by `track_batch`; each id answers true once.
    pub fn take_batched(&mut self, root: &Path, command: u64) -> bool {
        self.batched.remove(&(absolute(root), command))
    }

    /// Reopens the saved workspace folder, then every collection root the session had open.
    /// The workspace goes first so its environments are listed; a folder that no longer opens
    /// drops out of the session. The workspace's environment selection is not restored.
    pub fn restore(&mut self, cx: &mut Context<Self>) {
        let (dirs, roots) = {
            let s = self.session.read(cx).get();
            (s.workspaces.clone(), s.open.clone())
        };
        // Opening a workspace records it again, so only the ones that still open survive.
        self.save_workspace(None, cx);
        let (live, gone): (Vec<PathBuf>, Vec<PathBuf>) =
            dirs.into_iter().partition(|d| is_workspace_dir(d));
        for dir in &gone {
            cx.emit(WorkspaceEvent::FolderNotice(format!(
                "Workspace folder {} no longer exists, so it was not reopened",
                dir.display()
            )));
        }
        // A workspace folder is never a collection root, and nothing under a vanished one can open.
        let roots: Vec<PathBuf> = roots
            .into_iter()
            .filter(|r| !gone.iter().any(|g| r.starts_with(g)))
            .filter(|r| !(is_workspace_dir(r) && !r.join("opencollection.yml").is_file()))
            .collect();
        self.session.update(cx, |p, cx| {
            p.update(cx, |s| s.open.retain(|o| roots.contains(o)))
        });
        self.open_roots(live, cx);
        self.open_roots(roots, cx);
        // Unsent drafts of the scratch survive a restart, so it reopens when the store has any.
        let scratch = self.scratch_dir.clone();
        if let Some(scratch) = scratch
            && cx
                .try_global::<draft_store::Drafts>()
                .is_some_and(|d| d.0.read(cx).holds(&scratch))
        {
            self.open_scratch(cx);
        }
    }

    /// Names the directory of the hidden scratch collection; it is not opened until needed.
    pub fn set_scratch_dir(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        self.scratch_dir = Some(absolute(&dir));
        cx.set_global(crate::script_gate::ScratchDir(absolute(&dir)));
    }

    /// The hidden scratch collection's directory, as `is_scratch` compares it.
    pub fn scratch_dir(&self) -> Option<PathBuf> {
        self.scratch_dir.clone()
    }

    /// Whether `root` is the hidden scratch collection.
    pub fn is_scratch(&self, root: &Path) -> bool {
        self.scratch_dir.as_deref() == Some(absolute(root).as_path())
    }

    /// The open collections the user opened, without the scratch.
    pub fn user_collections(&self) -> Vec<&CollectionEntry> {
        self.collections
            .iter()
            .filter(|c| !self.is_scratch(&c.root))
            .collect()
    }

    /// Creates or repairs the scratch collection and opens it; `None`, with a notice when the
    /// directory could not be made, if no scratch directory is set.
    pub fn open_scratch(&mut self, cx: &mut Context<Self>) -> Option<PathBuf> {
        let dir = self.scratch_dir.clone()?;
        if let Err(message) = crate::scratch::ensure(&dir) {
            cx.emit(WorkspaceEvent::FolderNotice(format!(
                "The scratch pad could not be opened: {message}"
            )));
            return None;
        }
        self.open(dir.clone(), cx);
        Some(dir)
    }

    /// Opens the scratch and asks it for a new untitled draft at its root; `None` when the
    /// scratch could not be opened.
    pub fn new_scratch_request(&mut self, cx: &mut Context<Self>) -> Option<u64> {
        let root = self.open_scratch(cx)?;
        self.new_draft(&root, ItemPath::root(), cx)
    }

    /// Whether the scratch holds a tab or an unsaved draft, which the user would lose.
    pub fn scratch_has_work(&self, cx: &App) -> bool {
        let Some(dir) = &self.scratch_dir else {
            return false;
        };
        self.tabs.entries().iter().any(|t| t.root == *dir)
            || self
                .snapshot(dir, cx)
                .is_some_and(|s| s.content.requests.keys().any(ItemPath::is_draft))
    }

    /// The collections the sidebar lists: the user's, and the scratch only while it holds work.
    pub fn listed_collections(&self, cx: &App) -> Vec<&CollectionEntry> {
        let scratch = self.scratch_has_work(cx);
        (self.collections.iter())
            .filter(|c| scratch || !self.is_scratch(&c.root))
            .collect()
    }

    /// Whether the window shows the welcome state: no user collection and no scratch work.
    pub fn shows_empty_state(&self, cx: &App) -> bool {
        self.user_collections().is_empty() && !self.scratch_has_work(cx)
    }

    // Keeps the session's single workspace folder in step with `self.reference`.
    fn save_workspace(&mut self, dir: Option<PathBuf>, cx: &mut Context<Self>) {
        let want: Vec<PathBuf> = dir.into_iter().collect();
        if self.session.read(cx).get().workspaces == want {
            return;
        }
        self.session
            .update(cx, |p, cx| p.update(cx, |s| s.workspaces = want));
    }

    /// Sends `command` to the collection at `root` and returns its id, or `None` when that
    /// collection is not open or a branch switch holds it read-only.
    pub fn send(&mut self, root: &Path, command: Command, cx: &mut Context<Self>) -> Option<u64> {
        self.send_from(root, command, None, cx)
    }

    /// As [`send`](Self::send), for content built from the snapshot `basis`; see
    /// [`CollectionModel::send_from`].
    pub fn send_from(
        &mut self,
        root: &Path,
        command: Command,
        basis: Option<u64>,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        let held = matches!(
            command,
            Command::Edit { .. }
                | Command::Env(_)
                | Command::ApplyImport { .. }
                | Command::StoreImportSecrets { .. }
                | Command::Save(_)
                | Command::Overwrite(_)
                | Command::Revert(_)
                | Command::DiscardStale(_)
        ) || match &command {
            // A draft restored as unsaved only adds in-memory work and writes no file, so it
            // may land mid-switch; every other tree change writes files and waits.
            Command::Tree(TreeOp::CreateDraftFrom { unsaved: true, .. }) => false,
            Command::Tree(_) => true,
            _ => false,
        };
        if held && self.is_switching(root) {
            return None;
        }
        let model = self.entry(root)?.model.clone();
        let changes_env = matches!(command, Command::SelectEnvironment(_) | Command::Env(_));
        let edits = matches!(
            command,
            Command::Edit { .. } | Command::Env(_) | Command::ApplyImport { .. }
        );
        let writes = matches!(
            command,
            Command::Save(_) | Command::Overwrite(_) | Command::Tree(_)
        );
        let reverts_env = match &command {
            Command::Revert(Target::Environment(name)) => Some(name.clone()),
            _ => None,
        };
        let ticket_op = match &command {
            Command::Tree(TreeOp::Restore { ticket }) => Some((true, *ticket)),
            Command::Tree(TreeOp::ForgetTrash { ticket }) => Some((false, *ticket)),
            _ => None,
        };
        let id = model.update(cx, |m, _| m.send_from(command, basis));
        // A restore reopens its ticket's tabs once it lands; a forgotten ticket never will.
        match ticket_op {
            Some((true, ticket)) => {
                self.restores.insert((absolute(root), id), ticket);
            }
            Some((false, ticket)) => {
                self.trashed_tabs.remove(&(absolute(root), ticket));
            }
            None => {}
        }
        if writes && let Some(entry) = self.entry_mut(root) {
            entry.git_write = entry.git_write.max(Some(id));
        }
        if edits {
            self.edit_generation += 1;
            cx.notify();
        }
        if let Some(name) = reverts_env {
            let root = absolute(root);
            cx.emit(WorkspaceEvent::EnvReverted { root, name });
        }
        if changes_env && let Some(entry) = self.entry_mut(root) {
            entry.env_sent = id;
        }
        self.check_busy(root, cx);
        Some(id)
    }

    /// Writes the draft at `draft` of the collection `from` into `target` as the request `name`
    /// in `parent`, then discards the draft and closes its tab once `target` confirms; a failed
    /// write keeps the draft. Returns the command id in `target`, or `None` when the draft,
    /// its tab or the target is gone.
    #[allow(clippy::too_many_arguments)]
    pub fn save_draft_into(
        &mut self,
        from: &Path,
        draft: ItemPath,
        target: &Path,
        parent: ItemPath,
        name: String,
        texts: Vec<String>,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        let snapshot = self.request(from, &draft, cx)?.snapshot.clone()?;
        let kind = TabKind::Request(draft.clone());
        let tab = (self.tabs.entries().iter())
            .find(|e| e.root == from && e.kind == kind)?
            .id;
        let op = TreeOp::SaveSnapshot {
            parent,
            name,
            snapshot: (*snapshot).clone(),
        };
        let id = self.send(target, Command::Tree(op), cx)?;
        // The typed scripts are vouched for in the target, once its write is confirmed.
        self.approve_when_saved(target, Some(id), None, texts);
        self.cross_saves
            .insert((target.to_path_buf(), id), (from.to_path_buf(), draft, tab));
        Some(id)
    }

    // The target wrote the snapshot: the source draft and its tab have served.
    fn finish_cross_save(
        &mut self,
        from: &Path,
        draft: ItemPath,
        tab: TabId,
        cx: &mut Context<Self>,
    ) {
        self.send(from, Command::Tree(TreeOp::DiscardDraft { draft }), cx);
        let key = self.tabs.history_key(tab).cloned();
        self.update_tabs(cx, |t| {
            t.close(tab);
        });
        if let Some(key) = key
            && !self.draft_key_held_dirty(&key, cx)
        {
            draft_store::discard(&key, cx);
        }
    }

    /// Approves `texts`, typed in the app, once the write `command` asked for is confirmed
    /// (see `settle_approvals`); a failed or held write approves nothing.
    pub fn approve_when_saved(
        &mut self,
        root: &Path,
        command: Option<u64>,
        target: Option<Target>,
        texts: Vec<String>,
    ) {
        if let Some(command) = command {
            self.awaiting.push(AwaitedApproval {
                root: root.to_path_buf(),
                command,
                target,
                texts,
            });
        }
    }

    // Settles the waiting approvals the snapshot just acknowledged, against its notices.
    fn settle_approvals(&mut self, root: &Path, notices: &[Notice], cx: &mut Context<Self>) {
        let Some(snapshot) = self.snapshot(root, cx) else {
            self.awaiting.retain(|a| a.root != root);
            return;
        };
        let (done, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.awaiting)
            .into_iter()
            .partition(|a| a.root == root && snapshot.acked >= a.command);
        self.awaiting = rest;
        for a in done {
            let about = |n: &&Notice| n.command == a.command;
            let refused = notices.iter().filter(about).any(|n| {
                matches!(
                    n.kind,
                    NoticeKind::Failed { .. }
                        | NoticeKind::NotFound(_)
                        | NoticeKind::Stale(_)
                        | NoticeKind::Unsupported { .. }
                )
            });
            let written = match &a.target {
                Some(t) => {
                    crate::autosave::disk_of(&snapshot.content, t) == Some(DiskState::Present)
                }
                None => notices
                    .iter()
                    .filter(about)
                    .any(|n| matches!(n.kind, NoticeKind::Moved { .. } | NoticeKind::Created(_))),
            };
            if written && !refused {
                crate::script_approve::approve_saved(root, a.texts, cx);
            }
        }
    }

    /// Rises by one for each `Command::Edit`, `Command::Env` or `Command::ApplyImport` sent;
    /// nothing else moves it.
    pub fn edit_generation(&self) -> u64 {
        self.edit_generation
    }

    /// Makes `name` the active environment of the collection at `root`, or clears it with
    /// `None`; the session follows once the actor acknowledges. `None` when not open.
    pub fn select_environment(
        &mut self,
        root: &Path,
        name: Option<String>,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        self.send(root, Command::SelectEnvironment(name), cx)
    }

    /// The environment names of the collection at `root` in its latest snapshot, in order.
    pub fn environments(&self, root: &Path, cx: &App) -> Vec<String> {
        self.snapshot(root, cx).map_or_else(Vec::new, |s| {
            s.content
                .environments
                .iter()
                .map(|e| e.name.clone())
                .collect()
        })
    }

    /// The active environment of the collection at `root` in its latest snapshot.
    pub fn selected_environment(&self, root: &Path, cx: &App) -> Option<String> {
        self.snapshot(root, cx)?.selected_environment.clone()
    }

    /// Use `dir` for global environments and load it off the main thread; once loaded, the
    /// session's saved global becomes active, or none (and the session forgets it) when gone.
    pub fn set_global_dir(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        self.global_dir = Some(dir);
        self.sync_global_watch(cx);
        self.reload_globals(cx);
    }

    // Watches the global directory while there is a factory, so another program's edit is
    // reloaded as a collection's is; one that cannot be watched is read on demand only.
    fn sync_global_watch(&mut self, cx: &mut Context<Self>) {
        self.global_watch = None;
        let (Some(factory), Some(dir)) = (self.factory.clone(), self.global_dir.clone()) else {
            return;
        };
        // The platform watcher needs an existing directory, and the CLI or another window may
        // write the first file into this config directory; an empty one costs nothing.
        let _ = std::fs::create_dir_all(&dir);
        let (tx, batches) = async_channel::unbounded();
        // Batches are read as a collection's are, so the reload's own opens and closes ask for
        // nothing and only the watch limit stops watching.
        let (root, real_root) = (
            dir.clone(),
            std::fs::canonicalize(&dir).unwrap_or(dir.clone()),
        );
        let handler: EventHandler =
            Box::new(move |result| match translate(result, &root, &real_root) {
                None => {}
                Some(WatchMsg::Failed(message)) => drop(tx.try_send(Some(message))),
                Some(_) => drop(tx.try_send(None)),
            });
        let placed = factory(handler).and_then(|mut watch| match watch.watch(&dir) {
            Ok(()) => Ok(watch),
            Err(WatchError::Failed(message)) => Err(message),
            Err(WatchError::Unwatchable) => Err(format!("cannot watch {}", dir.display())),
        });
        let watch = match placed {
            Ok(watch) => watch,
            Err(message) => {
                cx.emit(WorkspaceEvent::WatchFailed { root: dir, message });
                cx.notify();
                return;
            }
        };
        let failed_root = dir.clone();
        let pump = cx.spawn(async move |this, cx| {
            // A failure drained with pending changes still reloads them before it is reported.
            while let Ok(first) = batches.recv().await {
                let (mut failed, mut changed) = (None, false);
                for batch in
                    std::iter::once(first).chain(std::iter::from_fn(|| batches.try_recv().ok()))
                {
                    match batch {
                        Some(message) => failed = failed.or(Some(message)),
                        None => changed = true,
                    }
                }
                let go_on = this.update(cx, |w, cx| {
                    if changed {
                        w.load_globals(ReloadCause::Watched, cx);
                    }
                    let Some(message) = failed else {
                        return true;
                    };
                    w.stop_global_watch(failed_root.clone(), message, cx);
                    false
                });
                if !go_on.unwrap_or(false) {
                    break;
                }
            }
        });
        self.global_watch = Some((watch, pump));
        // An edit made before the watch was placed has no event of its own.
        self.load_globals(ReloadCause::Watched, cx);
    }

    // Drops a global watch that failed after it started and says so. This runs inside the
    // watch's own pump, so the pump is let finish rather than dropped.
    fn stop_global_watch(&mut self, dir: PathBuf, message: String, cx: &mut Context<Self>) {
        if let Some((_watch, pump)) = self.global_watch.take() {
            pump.detach();
        }
        cx.emit(WorkspaceEvent::WatchFailed { root: dir, message });
        cx.notify();
    }

    /// Re-read the directory off the main thread and publish it; nothing without a directory.
    pub fn reload_globals(&mut self, cx: &mut Context<Self>) {
        self.load_globals(ReloadCause::Asked, cx);
    }

    /// The global environments directory, once the app names it.
    pub fn global_dir(&self) -> Option<&Path> {
        self.global_dir.as_deref()
    }

    /// Whether the latest published global snapshot came from the watcher noticing another
    /// program's change, rather than from an asked-for reload or an edit.
    pub fn globals_from_watch(&self) -> bool {
        self.globals_watched
    }

    // A reload asked for while edits drain waits for them; an asked-for one outranks a
    // waiting watched one, so the snapshot it publishes counts as asked for.
    fn load_globals(&mut self, cause: ReloadCause, cx: &mut Context<Self>) {
        let Some(dir) = self.global_dir.clone() else {
            return;
        };
        if self.global_draining {
            self.global_reload_waiting = Some(match self.global_reload_waiting {
                Some(ReloadCause::Asked) => ReloadCause::Asked,
                _ => cause,
            });
            return;
        }
        self.global_gen += 1;
        let generation = self.global_gen;
        let load = cx.background_spawn(async move { GlobalEnvironments::open(dir) });
        cx.spawn(async move |this, cx| {
            let loaded = load.await;
            this.update(cx, |w, cx| w.apply_globals(generation, cause, loaded, cx))
        })
        .detach();
    }

    /// The loaded global environment names, in the core's order.
    pub fn global_environments(&self) -> Vec<String> {
        self.globals.as_ref().map_or_else(Vec::new, |g| {
            g.environments()
                .iter()
                .map(|e| e.name().to_owned())
                .collect()
        })
    }

    /// The active global environment.
    pub fn selected_global_environment(&self) -> Option<String> {
        self.selected_global.clone()
    }

    /// Make `name` active (or none), record it in the session and send it to every open
    /// collection. A name not loaded is treated as none.
    pub fn select_global_environment(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        self.selected_global = name.filter(|n| self.is_global(n));
        if let Some(r) = self.reference.as_mut() {
            r.active = None;
        }
        self.save_global(cx);
        self.broadcast_globals(cx);
        cx.notify();
    }

    /// The loaded snapshot, if any.
    pub fn globals(&self) -> Option<Arc<GlobalEnvironments>> {
        self.globals.clone()
    }

    /// Apply `op` off the main thread on a fresh open of the directory, publish the result,
    /// and carry the selection through a rename or a delete. Edits run one at a time in the
    /// order given; a failure keeps the previous snapshot and sets [`Self::global_error`].
    /// The returned receiver gets the edit's outcome once it is published.
    pub fn edit_globals(
        &mut self,
        op: GlobalOp,
        cx: &mut Context<Self>,
    ) -> async_channel::Receiver<Result<(), String>> {
        let (done, outcome) = async_channel::bounded(1);
        if self.global_dir.is_none() {
            let message = "Global environments are unavailable here.".to_owned();
            self.global_error = Some(message.clone());
            let _ = done.try_send(Err(message));
            cx.notify();
            return outcome;
        }
        self.global_gen += 1;
        self.global_queue.push_back((op, done));
        if !self.global_draining {
            self.drain_globals(cx);
        }
        outcome
    }

    /// Reads the stored value of `name`'s secret `variable` off the main thread; `None` when
    /// nothing is stored or nothing is loaded.
    pub fn reveal_global_secret(
        &self,
        name: &str,
        variable: &str,
        cx: &App,
    ) -> Task<Result<Option<String>, String>> {
        let Some(globals) = self.globals.clone() else {
            return Task::ready(Ok(None));
        };
        let (store, name, variable) = (self.store.clone(), name.to_owned(), variable.to_owned());
        cx.background_spawn(async move {
            let key = globals.secret_key(&name, &variable);
            store.get(&key).map_err(|e| e.to_string())
        })
    }

    /// The orphans of global `name`, off the main thread; an error when nothing is loaded,
    /// which says nothing about what is stored.
    pub fn orphaned_global_secrets(
        &self,
        name: &str,
        cx: &App,
    ) -> Task<Result<Vec<String>, String>> {
        let Some(globals) = self.globals.clone() else {
            return Task::ready(Err("Global environments are not loaded.".to_owned()));
        };
        let (store, name) = (self.store.clone(), name.to_owned());
        cx.background_spawn(async move {
            globals
                .orphaned_secrets(&name, &*store)
                .map_err(|e| e.to_string())
        })
    }

    /// The orphans of global `name`, when one is given, and the stored secrets of global
    /// environments with no file, off the main thread and from one look at the store.
    pub fn global_secrets_to_tidy(&self, name: Option<&str>, cx: &App) -> Task<GlobalTidy> {
        let Some(globals) = self.globals.clone() else {
            let unloaded = "Global environments are not loaded.";
            return Task::ready((
                name.map(|_| Err(unloaded.to_owned())),
                Err(unloaded.to_owned()),
            ));
        };
        let (store, name) = (self.store.clone(), name.map(str::to_owned));
        cx.background_spawn(async move {
            let once = crate::orphan_section::ListOnce::new(&*store);
            let orphans = name.map(|name| {
                globals
                    .orphaned_secrets(&name, &once)
                    .map_err(|e| e.to_string())
            });
            let fileless = globals
                .all_orphaned_secrets(&once)
                .map(|r| crate::orphan_section::fileless_groups(&r))
                .map_err(|e| e.to_string());
            (orphans, fileless)
        })
    }

    /// Whether an edit from [`Self::edit_globals`] is queued or running.
    pub fn global_edits_pending(&self) -> bool {
        self.global_draining
    }

    /// The last failed operation's message; cleared by the next success.
    pub fn global_error(&self) -> Option<&str> {
        self.global_error.as_deref()
    }

    /// Drops the last failed operation's message, once the dialog that showed it is gone.
    pub(crate) fn clear_global_error(&mut self, cx: &mut Context<Self>) {
        if self.global_error.take().is_some() {
            cx.notify();
        }
    }

    /// The collection the environment controls act on: the active tab's, else the first open.
    pub fn current_root(&self) -> Option<PathBuf> {
        self.tabs
            .active()
            .map(|t| t.root.clone())
            .or_else(|| self.user_collections().first().map(|c| c.root.clone()))
    }

    /// `current_root` unless it is the scratch, for actions that only make sense on a user's
    /// collection (close, export).
    pub fn current_user_root(&self) -> Option<PathBuf> {
        self.current_root().filter(|r| !self.is_scratch(r))
    }

    /// Where a new draft goes: the current collection or scratch tab's root, else the scratch,
    /// opened on demand; `None` when the scratch cannot be opened.
    pub fn draft_root(&mut self, cx: &mut Context<Self>) -> Option<PathBuf> {
        match self.current_root() {
            Some(root) => Some(root),
            None => self.open_scratch(cx),
        }
    }

    /// The unsaved work of every open collection, or only of the one at `only`, in open
    /// order: dirty documents of its latest snapshot and commands not yet applied. A draft
    /// kept for after restart is left out, and so is a collection with neither.
    pub fn unsaved(&self, only: Option<&Path>, cx: &App) -> Vec<Unsaved> {
        let only = only.map(absolute);
        self.collections
            .iter()
            .filter(|c| only.as_ref().is_none_or(|o| *o == c.root))
            .filter_map(|c| {
                let model = c.model.read(cx);
                let targets = model.snapshot().map_or_else(Vec::new, |s| {
                    let mut targets = dirty_targets(&s.content);
                    targets.retain(|t| !self.kept_draft(&c.root, &s.content, t, cx));
                    targets
                });
                let pending = model.pending();
                (!targets.is_empty() || pending > 0).then(|| Unsaved {
                    root: c.root.clone(),
                    targets,
                    pending,
                })
            })
            .collect()
    }

    // Whether `target` is a draft of `root` whose current content the draft store keeps.
    fn kept_draft(&self, root: &Path, content: &Content, target: &Target, cx: &App) -> bool {
        let Target::Request(item) = target else {
            return false;
        };
        let view = content.requests.get(item);
        let current = view.and_then(|v| v.snapshot.as_ref());
        match (self.draft_key(root, item), current) {
            (Some(key), Some(current)) => {
                let blanks = view.is_some_and(|v| v.blanks_credentials);
                draft_store::is_kept(&key, current, blanks, cx)
            }
            _ => false,
        }
    }

    /// Whether a branch switch holds the collection at `root` read-only.
    pub fn is_switching(&self, root: &Path) -> bool {
        self.switching.contains(&absolute(root))
    }

    /// Holds the collection at `root` read-only: until `end_switch`, `send` refuses every
    /// command that edits or writes it, so nothing lands while a branch switch runs.
    pub fn begin_switch(&mut self, root: &Path, cx: &mut Context<Self>) {
        self.switching.insert(absolute(root));
        cx.notify();
    }

    /// Lifts the hold `begin_switch` put on the collection at `root`.
    pub fn end_switch(&mut self, root: &Path, cx: &mut Context<Self>) {
        if self.switching.remove(&absolute(root)) {
            cx.notify();
        }
    }

    /// Whether the collection at `root` is open and its actor has stalled.
    pub fn is_busy(&self, root: &Path) -> bool {
        self.entry(root).is_some_and(CollectionEntry::busy)
    }

    /// The newest snapshot of the collection at `root`, or `None` when it is not open or
    /// has not loaded yet.
    pub fn snapshot(&self, root: &Path, cx: &App) -> Option<Arc<CollectionSnapshot>> {
        self.entry(root)?.model.read(cx).snapshot().cloned()
    }

    /// The runtime variables of the collection at `root`, or `None` when it is not open.
    pub fn runtime_vars(&self, root: &Path, cx: &App) -> Option<RuntimeVars> {
        Some(self.entry(root)?.model.read(cx).runtime_vars())
    }

    /// The prompt answers of the collection at `root`, or `None` when it is not open.
    pub fn prompt_memory(&self, root: &Path, cx: &App) -> Option<PromptMemory> {
        Some(self.entry(root)?.model.read(cx).prompt_memory())
    }

    /// The script-set variables of the open collection at `root`, or `None` when it is not open.
    pub fn env_overlay(&self, root: &Path, cx: &App) -> Option<EnvOverlay> {
        Some(self.entry(root)?.model.read(cx).env_overlay())
    }

    /// The name to show for the collection at `root`: the open collection's `info.name`,
    /// found by path or by canonical path, else the directory name.
    pub fn collection_name(&self, root: &Path, cx: &App) -> SharedString {
        // Agent prompts carry canonical paths, so a miss by path retries canonically.
        let entry = self.entry(root).or_else(|| {
            let wanted = std::fs::canonicalize(root).ok()?;
            (self.collections.iter())
                .find(|c| std::fs::canonicalize(&c.root).is_ok_and(|p| p == wanted))
        });
        let name = entry.and_then(|c| c.model.read(cx).snapshot()?.content.collection.name.clone());
        label_or_dir(name.as_deref(), root)
    }

    /// The open tabs of every collection.
    pub fn tabs(&self) -> &Tabs {
        &self.tabs
    }

    /// Opens a tab on the request `item` of the collection at `root` and returns it, or
    /// `None` with nothing changed when the root is not open or `item` is not a request.
    pub fn open_item(
        &mut self,
        root: &Path,
        item: ItemPath,
        preview: bool,
        cx: &mut Context<Self>,
    ) -> Option<TabId> {
        let root = absolute(root);
        self.request(&root, &item, cx)?;
        Some(self.update_tabs(cx, |t| t.open(&root, TabKind::Request(item), preview)))
    }

    /// Opens (preview) or activates example `index` of the request `item`; `None` with nothing
    /// changed when the root is not open or there is no such example.
    pub fn open_example(
        &mut self,
        root: &Path,
        item: ItemPath,
        index: usize,
        preview: bool,
        cx: &mut Context<Self>,
    ) -> Option<TabId> {
        let root = absolute(root);
        let kind = TabKind::Example { item, index };
        if !self.has_tab_target(&root, &kind, cx) {
            return None;
        }
        // Only a newly created tab is placed, so re-activating an open example keeps a drag.
        let existed = self
            .tabs
            .entries()
            .iter()
            .any(|e| e.root == root && e.kind == kind);
        Some(self.update_tabs(cx, |t| {
            let id = t.open(&root, kind, preview);
            if !existed {
                place_example(t, &root, id);
            }
            id
        }))
    }

    /// Sends `TreeOp::DeleteExample`; `None` when the root is not open. Tabs change only when the
    /// actor confirms with `NoticeKind::ExampleDeleted`, which `prune_tabs` turns into
    /// `Tabs::forget_example`; a failed delete leaves every tab alone.
    pub fn delete_example(
        &mut self,
        root: &Path,
        item: ItemPath,
        index: usize,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        let op = TreeOp::DeleteExample { item, index };
        self.send(&absolute(root), Command::Tree(op), cx)
    }

    /// Asks the collection at `root` for a new draft request under the folder `parent`, of the
    /// kind its presets name and with their URL; see `new_draft_of_kind`.
    pub fn new_draft(
        &mut self,
        root: &Path,
        parent: ItemPath,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        let root = absolute(root);
        let op = TreeOp::CreateDraftFromPresets { parent };
        let id = self.send(&root, Command::Tree(op), cx)?;
        self.drafts.insert((root, id));
        Some(id)
    }

    /// Asks the collection at `root` for a new draft request of `kind` under the folder
    /// `parent` and returns the command id; once created, the draft opens as a permanent active
    /// tab whose editor takes focus. `None` with nothing sent when the root is not open.
    pub fn new_draft_of_kind(
        &mut self,
        root: &Path,
        parent: ItemPath,
        kind: RequestKind,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        let root = absolute(root);
        let op = TreeOp::CreateDraft { parent, kind };
        let id = self.send(&root, Command::Tree(op), cx)?;
        self.drafts.insert((root, id));
        Some(id)
    }

    /// Asks for a draft at the collection root from `snapshot`; once it opens, its tab holds
    /// `key` and `take_kept_view` yields `seq`. None when the root is not open.
    pub fn reopen_draft(
        &mut self,
        root: &Path,
        key: ItemPath,
        seq: u64,
        snapshot: DraftSnapshot,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        let root = absolute(root);
        let op = TreeOp::CreateDraftFrom {
            parent: ItemPath::root(),
            snapshot,
            unsaved: false,
        };
        let id = self.send(&root, Command::Tree(op), cx)?;
        self.drafts.insert((root.clone(), id));
        self.draft_keys.insert((root, id), (key, Some(seq)));
        Some(id)
    }

    /// Asks for an HTTP draft at the collection root that, once it opens as a tab, takes
    /// `request` as a paste would; the command id, or `None` when the root is not open.
    pub fn new_draft_from_import(
        &mut self,
        root: &Path,
        request: ImportedRequest,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        let root = absolute(root);
        let id = self.new_draft_of_kind(&root, ItemPath::root(), RequestKind::Http, cx)?;
        self.draft_imports.insert((root, id), request);
        Some(id)
    }

    /// The history key of the open tab showing draft `item` of `root`; `None` when no such
    /// tab is open or `item` is not a draft.
    pub fn draft_key(&self, root: &Path, item: &ItemPath) -> Option<ItemPath> {
        if !item.is_draft() {
            return None;
        }
        let root = absolute(root);
        let tab = self
            .tabs
            .entries()
            .iter()
            .find(|e| e.root == root && e.item() == Some(item))?;
        self.tabs.history_key(tab.id).cloned()
    }

    /// The file a draft kept under history `key` was saved as, if it has been saved.
    pub fn saved_as(&self, root: &Path, key: &ItemPath) -> Option<ItemPath> {
        self.saved_drafts
            .get(&(root.to_path_buf(), key.clone()))
            .cloned()
    }

    /// Whether an open, dirty draft tab holds history `key`. A reopened history entry shares
    /// its draft's key, so the key's kept file is discarded only when this is false.
    pub fn draft_key_held_dirty(&self, key: &ItemPath, cx: &App) -> bool {
        (self.tabs.entries().iter())
            .filter(|e| self.tabs.history_key(e.id) == Some(key))
            .any(|e| {
                let view = e.item().and_then(|item| {
                    let snap = self.snapshot(&e.root, cx)?;
                    snap.content.requests.get(item).cloned()
                });
                view.is_some_and(|v| v.dirty)
            })
    }

    /// The kept send a reopened draft's tab should show, handed out once.
    pub fn take_kept_view(&mut self, tab: TabId) -> Option<u64> {
        self.kept_views.remove(&tab)
    }

    /// Opens a permanent settings tab of `kind` on the collection at `root`, or activates the
    /// one already open. `None` with nothing changed for a request kind, a folder the
    /// collection does not have, or a root that is not open.
    pub fn open_settings(
        &mut self,
        root: &Path,
        kind: TabKind,
        cx: &mut Context<Self>,
    ) -> Option<TabId> {
        let root = absolute(root);
        if matches!(kind, TabKind::Request(_)) || !self.has_tab_target(&root, &kind, cx) {
            return None;
        }
        Some(self.update_tabs(cx, |t| t.open(&root, kind, false)))
    }

    /// Targets the selected collection environment, else the first, else a new one; stores
    /// the intent for `root` and opens its Environments tab. Never a global environment.
    pub fn add_variable_to_environment(
        &mut self,
        root: &Path,
        name: &str,
        cx: &mut Context<Self>,
    ) -> Option<TabId> {
        let root = absolute(root);
        let select = self
            .selected_environment(&root, cx)
            .or_else(|| self.environments(&root, cx).into_iter().next());
        let intent = EnvIntent {
            create: select.is_none(),
            select,
            add_var: name.to_string(),
        };
        // Stored before the tab opens, so an open editor's observer sees it on the notify;
        // dropped again when no tab opens, so a later reopen cannot apply it.
        self.env_intents.insert(root.clone(), intent);
        let tab = self.open_settings(&root, TabKind::Environments, cx);
        if tab.is_none() {
            self.env_intents.remove(&root);
        }
        tab
    }

    /// The intent waiting for `root`'s environment editor.
    pub fn env_intent(&self, root: &Path) -> Option<&EnvIntent> {
        self.env_intents.get(&absolute(root))
    }

    /// Removes and returns that intent.
    pub fn take_env_intent(&mut self, root: &Path) -> Option<EnvIntent> {
        self.env_intents.remove(&absolute(root))
    }

    /// Opens the permanent runner tab of `target`, a folder or `ItemPath::root()` for the
    /// whole collection at `root`, or activates the one already open. `None` with nothing
    /// changed for a target that is neither, or a root that is not open.
    pub fn open_runner(
        &mut self,
        root: &Path,
        target: ItemPath,
        cx: &mut Context<Self>,
    ) -> Option<TabId> {
        let root = absolute(root);
        let kind = TabKind::Runner(target);
        if !self.has_tab_target(&root, &kind, cx) {
            return None;
        }
        Some(self.open_runner_tab(&root, kind, cx))
    }

    /// Opens the permanent run-selection tab of `items` (sorted, deduplicated), or activates
    /// the one already open. `None` for an empty list, an unopened root or no item that exists.
    pub fn open_runner_items(
        &mut self,
        root: &Path,
        mut items: Vec<ItemPath>,
        cx: &mut Context<Self>,
    ) -> Option<TabId> {
        items.sort();
        items.dedup();
        let root = absolute(root);
        let kind = TabKind::RunSelection(items);
        if !self.has_tab_target(&root, &kind, cx) {
            return None;
        }
        Some(self.open_runner_tab(&root, kind, cx))
    }

    // Records what a new runner tab will plan, so the next prune re-plans it only on a change.
    fn open_runner_tab(&mut self, root: &Path, kind: TabKind, cx: &mut Context<Self>) -> TabId {
        let snapshot = self.snapshot(root, cx);
        let record = snapshot.and_then(|s| runner_record(&s.content, &kind, None));
        let id = self.update_tabs(cx, |t| t.open(root, kind, false));
        if let Some(record) = record {
            self.runner_members.entry(id).or_insert(record);
        }
        id
    }

    /// Changes the tab list with `f`, then saves it in the session and redraws.
    pub fn update_tabs<R>(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Tabs) -> R) -> R {
        let result = f(&mut self.tabs);
        self.persist_tabs(cx);
        cx.notify();
        result
    }

    /// Reopens the most recently closed tab whose target still exists, as a permanent tab;
    /// closed tabs whose target is gone are dropped on the way. `None` when none is left.
    pub fn reopen_tab(&mut self, cx: &mut Context<Self>) -> Option<TabId> {
        while let Some((root, kind)) = self.tabs.pop_closed() {
            if self.has_tab_target(&root, &kind, cx) {
                return Some(self.update_tabs(cx, |t| t.open(&root, kind, false)));
            }
        }
        None
    }

    /// Asks the shell to reveal `folder` of the collection at `root` in the sidebar.
    pub fn reveal_folder(&mut self, root: &Path, folder: ItemPath, cx: &mut Context<Self>) {
        let root = root.to_path_buf();
        cx.emit(WorkspaceEvent::RevealFolder { root, folder });
    }

    /// The newest view of the request `item` in the collection at `root`.
    pub fn request(&self, root: &Path, item: &ItemPath, cx: &App) -> Option<Arc<RequestView>> {
        self.snapshot(root, cx)?.content.requests.get(item).cloned()
    }

    /// Whether `field` of `target` in the collection at `root` has an edit the actor has
    /// not acknowledged yet.
    pub fn in_flight(&self, root: &Path, target: &Target, field: Field, cx: &App) -> bool {
        self.entry(root)
            .is_some_and(|c| c.model.read(cx).in_flight(target, field))
    }

    /// Sets how collections are watched and restarts every watcher with it; `None` stops
    /// watching. A collection whose watching failed stays stopped.
    pub fn set_watch_factory(&mut self, factory: Option<WatchFactory>, cx: &mut Context<Self>) {
        self.factory = factory;
        let roots: Vec<PathBuf> = self.collections.iter().map(|c| c.root.clone()).collect();
        for entry in &mut self.collections {
            entry.watcher = None;
            entry.pump = None;
        }
        for root in roots {
            self.sync_watch(&root, cx);
        }
        self.sync_global_watch(cx);
    }

    /// Turns git status on with `options` and refreshes every open collection, or off with
    /// `None`, which clears every collection's state.
    pub fn set_git(&mut self, options: Option<GitOptions>, cx: &mut Context<Self>) {
        let on = options.is_some();
        self.git_options = options;
        if on {
            self.refresh_all_git(cx);
        } else {
            for entry in &mut self.collections {
                entry.git = None;
            }
            cx.notify();
        }
    }

    /// How git runs for status, or `None` while git is off.
    pub fn git_options(&self) -> Option<&GitOptions> {
        self.git_options.as_ref()
    }

    /// The latest git changes of the collection at `root`; `None` while git is off, before the
    /// first run, without git or a repository, or when `root` is not open.
    pub fn git_changes(&self, root: &Path) -> Option<Arc<GitChanges>> {
        self.entry(root)?.git.clone()
    }

    /// Re-reads the git status of the collection at `root` off the UI thread; nothing while
    /// git is off or `root` is not open. A request during a run becomes one more run.
    pub fn refresh_git(&mut self, root: &Path, cx: &mut Context<Self>) {
        if self.git_options.is_none() {
            return;
        }
        let Some(entry) = self.entry_mut(root) else {
            return;
        };
        if entry.git_gate.request() {
            let (root, epoch) = (entry.root.clone(), entry.generation);
            self.run_git(root, epoch, cx);
        }
    }

    /// Runs `op` for the collection at `root` off the UI thread, then refreshes its git state.
    /// `None` while git is off. A core error becomes `Failed(text)`: `commit_failure_text` for a
    /// `CommitError`, `to_string()` for a `GitError`.
    pub fn run_git_write(
        &mut self,
        root: &Path,
        op: GitWrite,
        cx: &mut Context<Self>,
    ) -> Option<Task<GitWriteDone>> {
        let options = self.git_options.clone()?;
        let root = absolute(root);
        let dir = root.clone();
        let work = cx.background_spawn(async move {
            match op {
                GitWrite::Stage(paths) => stage(&dir, &paths, &options)
                    .map(|()| GitWriteDone::Done)
                    .unwrap_or_else(|e| GitWriteDone::Failed(e.to_string())),
                GitWrite::Unstage(paths) => unstage(&dir, &paths, &options)
                    .map(|()| GitWriteDone::Done)
                    .unwrap_or_else(|e| GitWriteDone::Failed(e.to_string())),
                GitWrite::Commit(message) => {
                    let before = staged_paths(&dir, &options);
                    match commit(&dir, &message, &options) {
                        Ok(o) => GitWriteDone::Committed {
                            short_id: o.short_id,
                            subject: o.subject,
                        },
                        Err(e) => {
                            let text = commit_failure_text(&e);
                            // A hook that ran may have rewritten the index; say so first.
                            let note = matches!(e, CommitError::Failed { .. })
                                .then(|| {
                                    staged_change_note(&before?, &staged_paths(&dir, &options)?)
                                })
                                .flatten();
                            GitWriteDone::Failed(match note {
                                Some(note) => format!("{note}\n{text}"),
                                None => text,
                            })
                        }
                    }
                }
            }
        });
        // The refresh is requested before the result is handed back, so a view following the
        // workspace sees the new state; a collection closed meanwhile is simply skipped.
        Some(cx.spawn(async move |this, cx| {
            let done = work.await;
            this.update(cx, |w, cx| w.refresh_git(&root, cx)).ok();
            done
        }))
    }

    /// Re-reads the git status of every open collection.
    pub fn refresh_all_git(&mut self, cx: &mut Context<Self>) {
        let roots: Vec<PathBuf> = self.collections.iter().map(|c| c.root.clone()).collect();
        for root in roots {
            self.refresh_git(&root, cx);
        }
    }

    /// How many status runs of the open collection at `root` have finished; a view reloads
    /// what it read from disk when this advances.
    pub fn git_runs(&self, root: &Path) -> u64 {
        self.entry(root).map_or(0, |e| e.git_runs)
    }

    /// Why watching the collection at `root` stopped, or `None` while it is watched or not open.
    pub fn watch_failed(&self, root: &Path) -> Option<&str> {
        self.entry(root)?.watch_failed.as_deref()
    }

    /// Test seam: sets why watching `root` stopped without notifying, so a test can move this
    /// one input of a cached view.
    #[doc(hidden)]
    pub fn set_watch_failed_silently(&mut self, root: &Path, why: Option<String>) {
        if let Some(entry) = self.entry_mut(root) {
            entry.watch_failed = why;
        }
    }

    // The tab a saved panel restores as, while its target still exists; a selection keeps
    // only the members still on disk.
    fn restorable(&self, root: &Path, saved: &SavedPanelKind, cx: &App) -> Option<TabKind> {
        let mut kind = saved.to_tab_kind()?;
        if let (TabKind::RunSelection(items), Some(s)) = (&mut kind, self.snapshot(root, cx)) {
            items.retain(|i| shows_member(&s.content, i));
        }
        self.has_tab_target(root, &kind, cx).then_some(kind)
    }

    // Whether the latest snapshot of `root` still has what a tab of `kind` shows.
    fn has_tab_target(&self, root: &Path, kind: &TabKind, cx: &App) -> bool {
        self.snapshot(root, cx)
            .is_some_and(|s| shows(&s.content, kind))
    }

    // A load result from before the latest load or edit started is stale and dropped; a
    // failed load keeps the previous snapshot.
    fn apply_globals(
        &mut self,
        generation: u64,
        cause: ReloadCause,
        loaded: Result<GlobalEnvironments, OpenError>,
        cx: &mut Context<Self>,
    ) {
        if generation < self.global_gen {
            return;
        }
        if let Ok(globals) = loaded {
            self.publish_globals(globals, cause == ReloadCause::Watched, cx);
        }
    }

    // Runs the next queued edit in the background, publishes or reports it, then takes the
    // next, so each op reopens the directory after the previous one's write.
    fn drain_globals(&mut self, cx: &mut Context<Self>) {
        let (Some(dir), Some((op, done))) =
            (self.global_dir.clone(), self.global_queue.pop_front())
        else {
            self.global_draining = false;
            if let Some(cause) = self.global_reload_waiting.take() {
                self.load_globals(cause, cx);
            }
            return;
        };
        self.global_draining = true;
        let store = self.store.clone();
        let task = cx.background_spawn(async move {
            let applied = op.apply(dir, &*store);
            (op, applied)
        });
        cx.spawn(async move |this, cx| {
            let (op, applied) = task.await;
            this.update(cx, |w, cx| {
                let outcome = applied.as_ref().map(drop).map_err(String::clone);
                w.finish_edit(&op, applied, cx);
                let _ = done.try_send(outcome);
                w.drain_globals(cx);
            })
        })
        .detach();
    }

    // An edit is never dropped as stale. Before any load succeeded the carry applies to the
    // session's saved name, which `publish_globals` then restores.
    fn finish_edit(
        &mut self,
        op: &GlobalOp,
        applied: Result<GlobalEnvironments, String>,
        cx: &mut Context<Self>,
    ) {
        let globals = match applied {
            Ok(globals) => globals,
            Err(message) => {
                self.global_error = Some(message);
                cx.notify();
                return;
            }
        };
        self.global_error = None;
        if self.global_loaded {
            self.selected_global = op.carry(self.selected_global.take());
        } else {
            let saved = self.session.read(cx).get().global_environment.clone();
            let moved = op.carry(saved.clone());
            if moved != saved {
                self.session
                    .update(cx, |p, cx| p.update(cx, |s| s.global_environment = moved));
            }
        }
        self.publish_globals(globals, false, cx);
    }

    // Every successful load or edit lands here. The first one applies the session's saved
    // name, so an edit that outran a dropped first load still restores it; later ones keep
    // the current selection while it is listed, else select none.
    fn publish_globals(
        &mut self,
        globals: GlobalEnvironments,
        watched: bool,
        cx: &mut Context<Self>,
    ) {
        let wanted = if self.global_loaded {
            self.selected_global.take()
        } else {
            self.global_loaded = true;
            self.session.read(cx).get().global_environment.clone()
        };
        self.globals = Some(Arc::new(globals));
        self.globals_watched = watched;
        self.selected_global = wanted.filter(|n| self.is_global(n));
        self.save_global(cx);
        self.broadcast_globals(cx);
        cx.notify();
    }

    fn is_global(&self, name: &str) -> bool {
        (self.globals.as_ref()).is_some_and(|g| g.environment(name).is_some())
    }

    /// The active global with its snapshot, whether profile or workspace: what actors receive.
    pub fn effective_global(&self) -> Option<(Arc<GlobalEnvironments>, String)> {
        self.global_selection()
    }

    fn global_selection(&self) -> Option<(Arc<GlobalEnvironments>, String)> {
        if let Some(r) = &self.reference
            && let (Some(envs), Some(name)) = (&r.envs, &r.active)
        {
            return Some((envs.clone(), name.clone()));
        }
        Some((self.globals.clone()?, self.selected_global.clone()?))
    }

    // Every actor gets the selection, even an unchanged one, so a fresh snapshot rebuilds
    // their contexts.
    fn broadcast_globals(&mut self, cx: &mut Context<Self>) {
        let roots: Vec<PathBuf> = self.collections.iter().map(|c| c.root.clone()).collect();
        for root in roots {
            let globals = self.global_selection();
            self.send(&root, Command::SetGlobalEnvironment(globals), cx);
        }
    }

    // Writes the active global as the session's when it differs; before any load succeeded
    // the session's entry is kept, so a transient failure never erases it.
    fn save_global(&mut self, cx: &mut Context<Self>) {
        let name = self.selected_global.clone();
        if !self.global_loaded || self.session.read(cx).get().global_environment == name {
            return;
        }
        self.session
            .update(cx, |p, cx| p.update(cx, |s| s.global_environment = name));
    }

    // Runs one status call in the background for the entry of `root` created at `epoch`. Its
    // result lands only on that same entry: closed or reopened since, it is dropped untouched.
    fn run_git(&mut self, root: PathBuf, epoch: u64, cx: &mut Context<Self>) {
        let Some(options) = self.git_options.clone() else {
            return;
        };
        let dir = root.clone();
        // The Changes dialog names where a gone row's copy went, which only this call supplies.
        let options = options.check_kept(true);
        let status = cx.background_spawn(async move { collection_status_with(&dir, &options) });
        cx.spawn(async move |this, cx| {
            let result = status.await;
            this.update(cx, |w, cx| w.finish_git(root, epoch, result, cx))
        })
        .detach();
    }

    fn finish_git(
        &mut self,
        root: PathBuf,
        epoch: u64,
        result: Result<GitState, GitError>,
        cx: &mut Context<Self>,
    ) {
        let on = self.git_options.is_some();
        let Some(entry) = self.entry_mut(&root).filter(|e| e.generation == epoch) else {
            return;
        };
        entry.git_runs += 1;
        // Git was turned off during the run: its result is stale and nothing is remembered.
        if !on {
            entry.git_gate = RefreshGate::default();
            return;
        }
        // An equal state keeps its Arc, so views key a list rebuild on pointer identity; every
        // finished run still notifies, as file contents may change under an unchanged status.
        let next = after_refresh(entry.git.clone(), result);
        if next != entry.git {
            entry.git = next;
        }
        let again = entry.git_gate.finish();
        cx.notify();
        if again {
            self.run_git(root, epoch, cx);
        }
    }

    fn entry(&self, root: &Path) -> Option<&CollectionEntry> {
        let root = absolute(root);
        self.collections.iter().find(|c| c.root == root)
    }

    fn entry_mut(&mut self, root: &Path) -> Option<&mut CollectionEntry> {
        let root = absolute(root);
        self.collections.iter_mut().find(|c| c.root == root)
    }

    // Each model update is either its open failing (legacy or not), which drops the entry, or
    // snapshots: the first one records the root, and every one hands over its notices exactly once.
    // Views redraw from the model on every update, so each one notifies.
    fn changed(
        &mut self,
        root: &Path,
        generation: u64,
        model: Entity<CollectionModel>,
        cx: &mut Context<Self>,
    ) {
        // Notify before emitting notices: the sidebar's example-delete handling relies on it.
        cx.notify();
        if let Some(error) = model.read(cx).open_error().map(str::to_owned) {
            self.collections.retain(|c| c.root != root);
            self.awaiting.retain(|a| a.root != root);
            self.drafts.retain(|(r, _)| r != root);
            self.draft_keys.retain(|(r, _), _| r != root);
            self.saved_drafts.retain(|(r, _), _| r != root);
            self.draft_imports.retain(|(r, _), _| r != root);
            self.restoring.remove(root);
            self.restore_ids.retain(|(r, _), _| r != root);
            self.trashed_tabs.retain(|(r, _), _| r != root);
            self.restores.retain(|(r, _), _| r != root);
            let tabs = &self.tabs;
            self.kept_views.retain(|t, _| tabs.get(*t).is_some());
            self.forget_open(root, cx);
            let root = root.to_path_buf();
            cx.emit(if model.read(cx).is_legacy() {
                WorkspaceEvent::LegacyFormat { root }
            } else {
                WorkspaceEvent::OpenFailed {
                    root,
                    message: error,
                }
            });
            return;
        }
        let first = model.read(cx).snapshot().is_some()
            && self
                .collections
                .iter_mut()
                .find(|c| c.root == root)
                .is_some_and(|c| !std::mem::replace(&mut c.opened, true));
        if first {
            if !self.is_scratch(root) {
                self.remember(root, cx);
            }
            cx.emit(WorkspaceEvent::Opened(root.to_path_buf()));
            self.restore_tabs(root, cx);
            self.restore_environment(root, cx);
        }
        // Refresh git on the first snapshot and once the reply covering our own write lands.
        let acked = model.read(cx).snapshot().map_or(0, |s| s.acked);
        let mut wrote = false;
        if let Some(entry) = self.collections.iter_mut().find(|c| c.root == root)
            && entry.git_write.is_some_and(|id| acked >= id)
        {
            entry.git_write = None;
            wrote = true;
        }
        if first || wrote {
            self.refresh_git(root, cx);
        }
        self.mirror_environment(root, cx);
        self.check_busy(root, cx);
        let notices = model.update(cx, |m, _| m.take_notices());
        self.settle_approvals(root, &notices, cx);
        self.prune_tabs(root, &notices, cx);
        if !notices.is_empty() {
            cx.emit(WorkspaceEvent::Notices {
                root: root.to_path_buf(),
                notices,
                generation,
            });
        }
        if model.read(cx).snapshot().is_some() {
            cx.emit(WorkspaceEvent::Delivered {
                root: root.to_path_buf(),
                acked,
                generation,
            });
        }
        self.sync_watch(root, cx);
        self.persist_tabs(cx);
    }

    // Starts the watcher of a loaded collection when a factory is set and it has neither a
    // watcher nor a failure, then hands it the snapshot's directories whenever they changed.
    fn sync_watch(&mut self, root: &Path, cx: &mut Context<Self>) {
        let Some(snapshot) = self.snapshot(root, cx) else {
            return;
        };
        let factory = self.factory.clone();
        let Some(entry) = self.collections.iter_mut().find(|c| c.root == root) else {
            return;
        };
        if let Some(factory) = factory
            && entry.watcher.is_none()
            && entry.watch_failed.is_none()
        {
            let (out, messages) = async_channel::unbounded();
            entry.watcher = Some(CollectionWatcher::start(
                root.to_path_buf(),
                factory,
                out,
                cx,
            ));
            entry.dirs.clear();
            let key = root.to_path_buf();
            entry.pump = Some(cx.spawn(async move |this, cx| {
                while let Ok(msg) = messages.recv().await {
                    if this.update(cx, |w, cx| w.on_watch(&key, msg, cx)).is_err() {
                        break;
                    }
                }
            }));
        }
        let Some(watcher) = &entry.watcher else {
            return;
        };
        let dirs: BTreeSet<PathBuf> = snapshot.content.dirs.iter().cloned().collect();
        if dirs != entry.dirs {
            watcher.set_dirs(dirs.clone());
            entry.dirs = dirs;
        }
    }

    // Turns a watcher message into a reload through `send`. A failure drops only the watcher:
    // this runs inside the pump, which ends by itself once the finished loop closes its channel.
    fn on_watch(&mut self, root: &Path, msg: WatchMsg, cx: &mut Context<Self>) {
        match msg {
            WatchMsg::Paths(paths) => {
                self.send(root, Command::Reload(paths), cx);
                self.refresh_git(root, cx);
            }
            WatchMsg::Git => self.refresh_git(root, cx),
            WatchMsg::Rescan => {
                self.send(root, Command::ReloadAll, cx);
                self.refresh_git(root, cx);
            }
            WatchMsg::Failed(message) => {
                let Some(entry) = self.collections.iter_mut().find(|c| c.root == root) else {
                    return;
                };
                entry.watcher = None;
                entry.watch_failed = Some(message.clone());
                cx.emit(WorkspaceEvent::WatchFailed {
                    root: root.to_path_buf(),
                    message,
                });
                cx.notify();
            }
        }
    }

    // Reopens the tabs the session saved for `root`, skipping items that are no longer
    // requests. The saved preview comes back as a preview only while no other root's preview
    // is open, since opening a preview replaces the one there is.
    fn restore_tabs(&mut self, root: &Path, cx: &mut Context<Self>) {
        let before = self.tabs.active().map(|e| e.id);
        let (saved, preview, examples, panels, saved_active, active_draft, pinned) = self
            .session
            .read(cx)
            .get()
            .collections
            .get(root)
            .map(|c| {
                let (tabs, preview) = (c.tabs.clone(), c.preview.clone());
                let (active, draft) = (c.active.clone(), c.active_draft.clone());
                (
                    tabs,
                    preview,
                    c.examples.clone(),
                    c.panels.clone(),
                    active,
                    draft,
                    c.pinned.clone(),
                )
            })
            .unwrap_or_default();
        for name in &saved {
            let Some(item) = ItemPath::new(name) else {
                continue;
            };
            if self.request(root, &item, cx).is_some() {
                let as_preview = preview.as_ref() == Some(name)
                    && !self.tabs.entries().iter().any(|e| e.preview);
                self.tabs.open(root, TabKind::Request(item), as_preview);
            }
        }
        // Example tabs come back as permanent tabs, each only while it still exists, placed
        // after its request and the examples of that request restored before it.
        let active = self.tabs.active().map(|e| e.id);
        for saved in examples {
            let Some(item) = ItemPath::new(&saved.item) else {
                continue;
            };
            let kind = TabKind::Example {
                item: item.clone(),
                index: saved.index,
            };
            if !self.has_tab_target(root, &kind, cx) {
                continue;
            }
            let id = self.tabs.open(root, kind, false);
            place_example(&mut self.tabs, root, id);
        }
        // Panel tabs come back at their saved position among this root's tabs, lowest first,
        // each only while its folder still exists.
        let mut panels = panels;
        panels.sort_by_key(|p| p.position);
        // A selection may lose members, so its pin is matched by what was saved.
        let mut pinned_panels = Vec::new();
        for saved in panels {
            let Some(kind) = self.restorable(root, &saved.kind, cx) else {
                continue;
            };
            // A runner plans when its view opens, so this delivery's prune must not re-plan it.
            let snapshot = self.snapshot(root, cx);
            let record = snapshot.and_then(|s| runner_record(&s.content, &kind, None));
            let id = self.tabs.open(root, kind, false);
            if let Some(record) = record {
                self.runner_members.insert(id, record);
            }
            if pinned.contains(&SavedTab::Panel(saved.kind)) {
                pinned_panels.push(id);
            }
            let anchor = self
                .tabs
                .entries()
                .iter()
                .filter(|e| e.root == root && e.id != id)
                .nth(saved.position)
                .map(|e| e.id);
            if let Some(anchor) = anchor {
                self.tabs.move_before(id, anchor);
            }
        }
        // Pinned tabs are pinned last, in their current order, so they end up first and
        // keep their relative order.
        let again: Vec<TabId> = self
            .tabs
            .entries()
            .iter()
            .filter(|e| e.root == root)
            .filter(|e| {
                pinned_panels.contains(&e.id)
                    || saved_tab_of(&e.kind).is_some_and(|t| pinned.contains(&t))
            })
            .map(|e| e.id)
            .collect();
        for id in again {
            self.tabs.set_pinned(id, true);
        }
        // The saved active tab wins; failing that, an active tab another root's restore
        // brought back stays active, else the tab active before the examples does.
        let wanted = saved_active.and_then(|t| match t {
            SavedTab::Request(name) => ItemPath::new(&name).map(TabKind::Request),
            SavedTab::Example(e) => ItemPath::new(&e.item).map(|item| TabKind::Example {
                item,
                index: e.index,
            }),
            SavedTab::Panel(kind) => self.restorable(root, &kind, cx),
        });
        let found = wanted.and_then(|kind| {
            self.tabs
                .entries()
                .iter()
                .find(|e| e.root == root && e.kind == kind)
                .map(|e| e.id)
        });
        if let Some(id) = found {
            self.tabs.activate(id);
            self.restored_active = Some(id);
        } else if let Some(id) = before.filter(|id| Some(*id) == self.restored_active) {
            self.tabs.activate(id);
        } else if let Some(id) = active {
            self.tabs.activate(id);
        }
        self.restore_drafts(root, active_draft, found, cx);
    }

    // Reopens the store's kept drafts of `root` unsaved, with their history keys, in their saved
    // folder or the root when it is gone; the last answer picks the active tab.
    fn restore_drafts(
        &mut self,
        root: &Path,
        active_draft: Option<String>,
        saved_active: Option<TabId>,
        cx: &mut Context<Self>,
    ) {
        let Some(store) = cx.try_global::<draft_store::Drafts>().map(|d| d.0.clone()) else {
            return;
        };
        let Some(content) = self.snapshot(root, cx).map(|s| s.content.clone()) else {
            return;
        };
        let files = store.update(cx, |s, cx| s.take_root(root, cx));
        let mut sent = 0;
        for file in files {
            let (Some(key), Some(snapshot)) = (file.item_key(), file.snapshot()) else {
                continue;
            };
            let mut parent = file.parent_path();
            if parent != ItemPath::root() && !content.folders.contains_key(&parent) {
                parent = ItemPath::root();
            }
            let op = TreeOp::CreateDraftFrom {
                parent,
                snapshot,
                unsaved: true,
            };
            let Some(id) = self.send(root, Command::Tree(op), cx) else {
                continue;
            };
            let at = (root.to_path_buf(), id);
            self.drafts.insert(at.clone());
            self.draft_keys.insert(at.clone(), (key, None));
            self.restore_ids.insert(at, file.slot);
            sent += 1;
        }
        if sent > 0 {
            let wanted = Restoring {
                pending: sent,
                active_draft,
                saved_active,
                at_send: self.tabs.active().map(|e| e.id),
            };
            self.restoring.insert(root.to_path_buf(), wanted);
        }
    }

    // Counts one restored draft of `root` answered; after the last, activates and focuses the
    // draft that was active, failing that the saved active tab, else another root's restored one
    // while it is still the tab that was active at send.
    fn restore_answered(&mut self, root: &Path, cx: &mut Context<Self>) {
        let Some(state) = self.restoring.get_mut(root) else {
            return;
        };
        state.pending = state.pending.saturating_sub(1);
        if state.pending > 0 {
            return;
        }
        let Some(Restoring {
            active_draft: wanted,
            saved_active,
            at_send,
            ..
        }) = self.restoring.remove(root)
        else {
            return;
        };
        let draft = wanted.and_then(|key| {
            (self.tabs.entries().iter())
                .find(|e| {
                    e.root == root
                        && self.tabs.history_key(e.id).map(ItemPath::as_str) == Some(key.as_str())
                })
                .map(|e| e.id)
        });
        let open = |id: &TabId| self.tabs.get(*id).is_some();
        let still_active = |id: &TabId| at_send == Some(*id) && open(id);
        let id = draft
            .or(saved_active.filter(open))
            .or(self.restored_active.filter(still_active));
        if let Some(id) = id {
            self.tabs.activate(id);
            self.restored_active = Some(id);
            cx.emit(WorkspaceEvent::DraftOpened(id));
        }
    }

    // Reselects the environment the session saved for `root` when the collection still has
    // it, through `send` so the mirror waits for it; a vanished one is dropped from the session.
    fn restore_environment(&mut self, root: &Path, cx: &mut Context<Self>) {
        let session = self.session.read(cx).get();
        let Some(name) = session
            .collections
            .get(root)
            .and_then(|c| c.environment.clone())
        else {
            return;
        };
        if self.environments(root, cx).contains(&name) {
            self.send(root, Command::SelectEnvironment(Some(name)), cx);
        } else {
            self.save_environment(root, None, cx);
        }
    }

    // Once the actor has acknowledged every environment command sent for `root`, its
    // selection is the user's choice, renames and deletes included, so the session copies it.
    fn mirror_environment(&mut self, root: &Path, cx: &mut Context<Self>) {
        let Some(entry) = self.entry(root).filter(|c| c.opened) else {
            return;
        };
        let Some(snapshot) = entry.model.read(cx).snapshot() else {
            return;
        };
        if snapshot.acked < entry.env_sent {
            return;
        }
        let selected = snapshot.selected_environment.clone();
        self.save_environment(root, selected, cx);
    }

    // Writes `name` as the session's environment of `root` when it differs; a root without a
    // UTF-8 path is not stored, as `Session::remember` does.
    fn save_environment(&mut self, root: &Path, name: Option<String>, cx: &mut Context<Self>) {
        let session = self.session.read(cx).get();
        let stored = session
            .collections
            .get(root)
            .and_then(|c| c.environment.as_ref());
        if root.to_str().is_none() || stored == name.as_ref() {
            return;
        }
        self.session.update(cx, |p, cx| {
            p.update(cx, |s| {
                s.collections
                    .entry(root.to_path_buf())
                    .or_default()
                    .environment = name;
            })
        });
    }

    // A draft saved to a file moves its kept sends from the draft's history key to the
    // file, so History lists them under the saved request; the tab drops the draft key,
    // which is returned.
    fn rekey_draft(
        &mut self,
        root: &Path,
        from: &ItemPath,
        to: &ItemPath,
        cx: &App,
    ) -> Option<ItemPath> {
        let kind = TabKind::Request(from.clone());
        let tab = self
            .tabs
            .entries()
            .iter()
            .find(|e| e.root == root && e.kind == kind);
        let id = tab.map(|e| e.id)?;
        let key = self.tabs.take_history_key(id)?;
        let (root, to) = (root.to_path_buf(), to.clone());
        self.saved_drafts
            .insert((root.clone(), key.clone()), to.clone());
        enqueue(
            StoreOp::Rename {
                root,
                from: key.clone(),
                to,
            },
            cx,
        );
        Some(key)
    }

    // Moves and example deletes are applied before the prune so a renamed item's tab follows
    // it and later examples keep their tabs, and a draft `new_draft` asked for opens; then
    // tabs whose target left the snapshot close, and all but example tabs are reported.
    // Collection and environments tabs live as long as their collection.
    fn prune_tabs(&mut self, root: &Path, notices: &[Notice], cx: &mut Context<Self>) {
        forward_to_store(root, notices, cx);
        let Some(snapshot) = self.snapshot(root, cx) else {
            return;
        };
        // `send` borrows `self`, so imports for drafts created here go out after the loop.
        let mut imports = Vec::new();
        for notice in notices {
            let key = (root.to_path_buf(), notice.command);
            match &notice.kind {
                NoticeKind::Moved { from, to } => {
                    // A saved draft's sends move from its history key to the new file.
                    // Its kept file is dropped too, unless a dirty tab still holds its key.
                    if from.is_draft()
                        && let Some(key) = self.rekey_draft(root, from, to, cx)
                        && !self.draft_key_held_dirty(&key, cx)
                    {
                        draft_store::discard(&key, cx);
                    }
                    self.tabs.follow_move(root, from, to)
                }
                NoticeKind::ExampleDeleted { item, index } => {
                    self.tabs.forget_example(root, item, *index)
                }
                NoticeKind::Created(saved) if self.cross_saves.contains_key(&key) => {
                    if let Some((from, draft, tab)) = self.cross_saves.remove(&key) {
                        self.finish_cross_save(&from, draft, tab, cx);
                        // The draft's tab closed with the save, so show the request it became.
                        self.open_item(root, saved.clone(), false, cx);
                    }
                }
                NoticeKind::Created(item) if item.is_draft() && self.drafts.remove(&key) => {
                    let restored = self.restore_ids.remove(&key);
                    let tab = self.tabs.open(root, TabKind::Request(item.clone()), false);
                    // Every draft is keyed at creation; a reopen keeps its given key and view.
                    let (history, seq) = self
                        .draft_keys
                        .remove(&key)
                        .unwrap_or_else(|| (new_history_key(SystemTime::now()), None));
                    self.tabs.set_history_key(tab, history.clone());
                    // A restored tab owns the file it came from, not whichever slot is free.
                    if let Some(slot) = restored {
                        draft_store::bind_restored(root, item, &history, slot, cx);
                    }
                    if let Some(seq) = seq {
                        self.kept_views.insert(tab, seq);
                    }
                    if let Some(request) = self.draft_imports.remove(&key) {
                        imports.push((item.clone(), request));
                    }
                    // A restore focuses one tab once all its drafts have answered.
                    if restored.is_some() {
                        self.restore_answered(root, cx);
                    } else {
                        cx.emit(WorkspaceEvent::DraftOpened(tab));
                    }
                }
                NoticeKind::Failed { .. } => {
                    self.cross_saves.remove(&key);
                    self.drafts.remove(&key);
                    let history = self.draft_keys.remove(&key);
                    self.draft_imports.remove(&key);
                    if let Some(slot) = self.restore_ids.remove(&key) {
                        // No tab shows this file, so the store must not hand its slot out.
                        if let Some((history, _)) = history {
                            draft_store::reserve_failed(&history, slot, cx);
                        }
                        self.restore_answered(root, cx);
                    }
                }
                _ => {}
            }
        }
        for (item, request) in imports {
            // No banner shows for a draft from the dialog or a drop, so the reply is dropped.
            // The secrets follow only an import that was sent.
            let secrets = request.secrets.clone();
            let sent = self.send(
                root,
                Command::ApplyImport {
                    item: item.clone(),
                    request,
                },
                cx,
            );
            if sent.is_some() && !secrets.is_empty() {
                let mut stored = ImportedRequest::new("GET", "");
                stored.secrets = secrets;
                let (reply, _) = async_channel::bounded(1);
                let command = Command::StoreImportSecrets {
                    item,
                    request: stored,
                    notes: Vec::new(),
                    reply,
                };
                self.send(root, command, cx);
            }
        }
        let content = &snapshot.content;
        let before: Vec<TabId> = self.tabs.entries().iter().map(|e| e.id).collect();
        let active = self.tabs.active().map(|e| e.id);
        let gone = self
            .tabs
            .retain(|e| e.root != root || shows(content, &e.kind));
        self.remember_trashed(root, notices, &gone, &before, active);
        self.reopen_restored(root, notices, content);
        // The app's own delete already says what went, so the tabs it covers close silently.
        let removed: Vec<&ItemPath> = (notices.iter())
            .filter_map(|n| match &n.kind {
                NoticeKind::Deleted(item) | NoticeKind::Trashed { item, .. } => Some(item),
                _ => None,
            })
            .collect();
        let covered = |p: &ItemPath| removed.iter().any(|d| crate::stale::is_within(p, d));
        let gone: Vec<_> = (gone.into_iter())
            .filter(|e| match &e.kind {
                TabKind::Request(p)
                | TabKind::Folder(p)
                | TabKind::Runner(p)
                | TabKind::Example { item: p, .. } => !covered(p),
                // Quiet when this delivery covers what left now and the rest left earlier.
                TabKind::RunSelection(items) => {
                    let before = self.selection_gone.get(&e.id);
                    let earlier = |i: &ItemPath| before.is_some_and(|b| b.contains(i));
                    !items.iter().any(covered) || !items.iter().all(|i| earlier(i) || covered(i))
                }
                TabKind::Collection | TabKind::Environments => true,
            })
            .collect();
        // Recomputed from the new content, so a member restored by undo counts as present.
        let mut missing = HashMap::new();
        for e in self.tabs.entries() {
            if let TabKind::RunSelection(items) = &e.kind {
                let set: HashSet<ItemPath> = if e.root == root {
                    (items.iter())
                        .filter(|i| !shows_member(content, i))
                        .cloned()
                        .collect()
                } else {
                    self.selection_gone.get(&e.id).cloned().unwrap_or_default()
                };
                missing.insert(e.id, set);
            }
        }
        // A tab first seen here starts from an empty set, so only a real change is reported.
        let mut changed: Vec<TabId> = (missing.iter())
            .filter(|(id, set)| {
                self.selection_gone
                    .get(*id)
                    .map_or(!set.is_empty(), |b| b != *set)
            })
            .map(|(id, _)| *id)
            .collect();
        self.selection_gone = missing;
        // A tab with no record yet re-plans too: it may have planned before this delivery.
        // Another root's tabs keep their records, so its next delivery compares against them.
        let mut members = HashMap::new();
        for e in self.tabs.entries() {
            let before = self.runner_members.remove(&e.id);
            let record = (e.root == root)
                .then(|| runner_record(content, &e.kind, before.as_ref()))
                .flatten();
            let Some(record) = record else {
                if let Some(before) = before {
                    members.insert(e.id, before);
                }
                continue;
            };
            if before.as_ref() != Some(&record) && !changed.contains(&e.id) {
                changed.push(e.id);
            }
            members.insert(e.id, record);
        }
        self.runner_members = members;
        for id in changed {
            cx.emit(WorkspaceEvent::SelectionChanged(id));
        }
        if !gone.is_empty() {
            let tabs = gone.len();
            let items = gone.into_iter().filter_map(|e| match e.kind {
                TabKind::Request(path) | TabKind::Folder(path) | TabKind::Runner(path) => {
                    Some(vec![path])
                }
                TabKind::RunSelection(items) => Some(items),
                _ => None,
            });
            cx.emit(WorkspaceEvent::TabsClosed {
                root: root.to_path_buf(),
                items: items.flatten().collect(),
                tabs,
            });
        }
    }

    // Files the tabs each Trashed notice closed under its ticket, with their old positions.
    // Positions count against the strip as each delete in turn left it, so restores sent
    // newest first rebuild the strip exactly. A run tab goes with the last delete it covers.
    fn remember_trashed(
        &mut self,
        root: &Path,
        notices: &[Notice],
        gone: &[TabEntry],
        before: &[TabId],
        active: Option<TabId>,
    ) {
        let trashed: Vec<(&ItemPath, u64)> = (notices.iter())
            .filter_map(|n| match &n.kind {
                NoticeKind::Trashed { item, ticket } => Some((item, *ticket)),
                _ => None,
            })
            .collect();
        let owner = |e: &TabEntry| -> Option<usize> {
            let covers = |p: &ItemPath| {
                trashed
                    .iter()
                    .position(|(t, _)| crate::stale::is_within(p, t))
            };
            match &e.kind {
                TabKind::RunSelection(items) => items.iter().filter_map(covers).max(),
                kind => trashed_path(kind).and_then(covers),
            }
        };
        let owners: Vec<(&TabEntry, usize)> = (gone.iter())
            .filter_map(|e| owner(e).map(|o| (e, o)))
            .collect();
        // Tabs closed for no delete here never come back, so they hold no place.
        let mut strip: Vec<TabId> = (before.iter())
            .filter(|id| {
                owners.iter().any(|(e, _)| e.id == **id) || !gone.iter().any(|e| e.id == **id)
            })
            .copied()
            .collect();
        for (n, (_, ticket)) in trashed.iter().enumerate() {
            let mine: Vec<&TabEntry> = (owners.iter())
                .filter(|(_, o)| *o == n)
                .map(|(e, _)| *e)
                .collect();
            let tabs = (mine.iter())
                .map(|e| TrashedTab {
                    index: strip
                        .iter()
                        .position(|id| *id == e.id)
                        .unwrap_or(usize::MAX),
                    kind: e.kind.clone(),
                    preview: e.preview,
                    pinned: e.pinned,
                    active: Some(e.id) == active,
                })
                .collect();
            strip.retain(|id| !mine.iter().any(|e| e.id == *id));
            self.trashed_tabs.insert((absolute(root), *ticket), tabs);
        }
    }

    // Puts back, in their old order, the tabs of each ticket a Restored notice answers, and
    // activates the one that was active when the delete closed it.
    fn reopen_restored(&mut self, root: &Path, notices: &[Notice], content: &Content) {
        let root = absolute(root);
        for notice in notices {
            let Some(ticket) = self.restores.remove(&(root.clone(), notice.command)) else {
                continue;
            };
            let tabs = self.trashed_tabs.remove(&(root.clone(), ticket));
            if !matches!(notice.kind, NoticeKind::Restored(_)) {
                continue;
            }
            let mut tabs = tabs.unwrap_or_default();
            tabs.sort_by_key(|t| t.index);
            for t in tabs.into_iter().filter(|t| shows(content, &t.kind)) {
                let id = (self.tabs).reopen_at(&root, t.kind, t.index, t.preview, t.pinned);
                if t.active {
                    self.tabs.activate(id);
                }
            }
        }
    }

    // Writes each loaded root's tab list, preview tab, example tabs, active tab and active
    // draft's key to the session when any differs from the stored one; a root still opening
    // or restoring drafts is skipped so its saved tabs survive until restored.
    fn persist_tabs(&mut self, cx: &mut Context<Self>) {
        let active_id = self.tabs.active().map(|e| e.id);
        let lists: Vec<_> = self
            .collections
            .iter()
            .filter(|c| c.opened && !self.restoring.contains_key(&c.root))
            .map(|c| {
                let (mut names, mut preview, mut examples) = (Vec::new(), None, Vec::new());
                let (mut active, mut active_draft, mut panels) = (None, None, Vec::new());
                let mut pinned = Vec::new();
                // Counts the tabs that are saved, so a panel's position matches the list
                // restore rebuilds.
                let mut saved_count = 0;
                for e in self.tabs.entries().iter().filter(|e| e.root == c.root) {
                    let is_active = Some(e.id) == active_id;
                    if let Some(kind) = SavedPanelKind::from_tab_kind(&e.kind) {
                        if e.pinned {
                            pinned.push(SavedTab::Panel(kind.clone()));
                        }
                        if is_active {
                            active = Some(SavedTab::Panel(kind.clone()));
                        }
                        panels.push(SavedPanel {
                            kind,
                            position: saved_count,
                        });
                        saved_count += 1;
                        continue;
                    }
                    match &e.kind {
                        TabKind::Request(p) if p.is_draft() && is_active => {
                            active_draft =
                                self.tabs.history_key(e.id).map(|k| k.as_str().to_owned());
                        }
                        TabKind::Request(p) if !p.is_draft() => {
                            if e.preview {
                                preview = Some(p.as_str().to_owned());
                            }
                            if e.pinned {
                                pinned.push(SavedTab::Request(p.as_str().to_owned()));
                            }
                            if is_active {
                                active = Some(SavedTab::Request(p.as_str().to_owned()));
                            }
                            names.push(p.as_str().to_owned());
                            saved_count += 1;
                        }
                        TabKind::Example { item, index } if !item.is_draft() => {
                            let saved = SavedExample {
                                item: item.as_str().to_owned(),
                                index: *index,
                            };
                            if e.pinned {
                                pinned.push(SavedTab::Example(saved.clone()));
                            }
                            if is_active {
                                active = Some(SavedTab::Example(saved.clone()));
                            }
                            examples.push(saved);
                            saved_count += 1;
                        }
                        _ => {}
                    }
                }
                let active = (active, active_draft);
                (
                    c.root.clone(),
                    names,
                    preview,
                    examples,
                    (panels, pinned),
                    active,
                )
            })
            .collect();
        let session = self.session.read(cx).get();
        let changed: Vec<_> = lists
            .into_iter()
            .filter(
                |(root, tabs, preview, examples, (panels, pinned), (active, draft))| {
                    session.collections.get(root).map_or(
                        (&[][..], None, &[][..], (&[][..], &[][..]), None, None),
                        |c| {
                            (
                                &c.tabs[..],
                                c.preview.as_ref(),
                                &c.examples[..],
                                (&c.panels[..], &c.pinned[..]),
                                c.active.as_ref(),
                                c.active_draft.as_ref(),
                            )
                        },
                    ) != (
                        &tabs[..],
                        preview.as_ref(),
                        &examples[..],
                        (&panels[..], &pinned[..]),
                        active.as_ref(),
                        draft.as_ref(),
                    )
                },
            )
            .collect();
        if changed.is_empty() {
            return;
        }
        self.session.update(cx, |p, cx| {
            p.update(cx, |s| {
                for (root, tabs, preview, examples, (panels, pinned), (active, draft)) in changed {
                    let saved = s.collections.entry(root).or_default();
                    saved.tabs = tabs;
                    saved.preview = preview;
                    saved.examples = examples;
                    saved.panels = panels;
                    saved.pinned = pinned;
                    saved.active = active;
                    saved.active_draft = draft;
                }
            })
        });
    }

    // Feeds the tracker the model's progress, records the verdict, and while commands are
    // pending without progress schedules a look again at the moment they would turn busy.
    fn check_busy(&mut self, root: &Path, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let Some(entry) = self.collections.iter_mut().find(|c| c.root == root) else {
            return;
        };
        let model = entry.model.read(cx);
        let acked = model.snapshot().map_or(0, |s| s.acked);
        let state = entry.tracker.observe(model.pending(), acked, now);
        let busy = state == BusyState::Busy;
        entry.recheck = match state {
            BusyState::Waiting { until } => {
                let timer = cx.background_executor().timer(until - now);
                let key = root.to_path_buf();
                Some(cx.spawn(async move |this, cx| {
                    timer.await;
                    this.update(cx, |w, cx| w.check_busy(&key, cx)).ok();
                }))
            }
            _ => None,
        };
        if std::mem::replace(&mut entry.busy, busy) != busy {
            cx.notify();
        }
    }

    fn remember(&mut self, root: &Path, cx: &mut Context<Self>) {
        let stored = self.session.update(cx, |p, cx| {
            let mut stored = false;
            p.update(cx, |s| {
                stored = s.remember(root);
                if stored && !s.open.iter().any(|o| o == root) {
                    s.open.push(root.to_path_buf());
                }
            });
            stored
        });
        if !stored {
            eprintln!(
                "apinata: {} is not UTF-8, so it is not saved in the session",
                root.display()
            );
        }
    }

    fn forget_open(&mut self, root: &Path, cx: &mut Context<Self>) {
        if !self.session.read(cx).get().open.iter().any(|o| o == root) {
            return;
        }
        self.session
            .update(cx, |p, cx| p.update(cx, |s| s.open.retain(|o| o != root)));
    }
}

/// Whether `content` has what a tab of `kind` shows; collection-wide kinds always have it.
fn shows(content: &Content, kind: &TabKind) -> bool {
    match kind {
        TabKind::Request(item) => content.requests.contains_key(item),
        TabKind::Folder(path) => content.folders.contains_key(path),
        TabKind::Runner(path) => path.is_root() || content.folders.contains_key(path),
        TabKind::RunSelection(items) => items.iter().any(|i| shows_member(content, i)),
        TabKind::Example { item, index } => content
            .requests
            .get(item)
            .is_some_and(|r| *index < r.examples.len()),
        TabKind::Collection | TabKind::Environments => true,
    }
}

/// What a runner tab's plan depends on: per planned request the name and method its body
/// shows and the seq and tags it orders and filters by, and per folder holding one of them
/// the saved tags its requests inherit, since core filters on effective tags.
#[derive(Debug, PartialEq)]
struct RunnerRecord {
    requests: HashMap<ItemPath, PlanEntry>,
    folders: HashMap<ItemPath, Option<Vec<String>>>,
}

/// One planned request's seq, which no editor edit holds back, and its saved name, method
/// and tags (`None` when those saved values are unknown), and the stamp of the outside
/// change that last made them unknown, 0 otherwise.
type PlanEntry = (Option<u64>, Option<(String, String, Vec<String>)>, u64);

/// What a folder runner or run-selection tab plans from `content`; `None` for other tabs.
/// The plan is read from disk, so an unsaved request or folder whose file is still as loaded
/// keeps its saved values from `before`, and drafts, which no saved plan lists, are left out.
fn runner_record(
    content: &Content,
    kind: &TabKind,
    before: Option<&RunnerRecord>,
) -> Option<RunnerRecord> {
    let under = |p: &ItemPath, dir: &ItemPath| dir.is_root() || crate::stale::is_within(p, dir);
    let planned: Box<dyn Fn(&ItemPath) -> bool> = match kind {
        TabKind::Runner(dir) => Box::new(move |p| under(p, dir)),
        TabKind::RunSelection(items) => Box::new(move |p| items.iter().any(|i| under(p, i))),
        _ => return None,
    };
    let shown = |(p, v): (&ItemPath, &Arc<RequestView>)| {
        let (saved, stamp) = match (v.dirty, v.disk) {
            (false, _) => (Some((v.name.clone(), v.method.clone(), v.tags.clone())), 0),
            (true, DiskState::Present) => match before.and_then(|b| b.requests.get(p)) {
                Some((_, saved, stamp)) => (saved.clone(), *stamp),
                None => (None, 0),
            },
            (true, _) => (None, v.reloaded),
        };
        (planned(p) && !v.draft).then(|| (p.clone(), (v.seq, saved, stamp)))
    };
    let requests: HashMap<_, _> = content.requests.iter().filter_map(shown).collect();
    // Only the folders a planned request sits in pass tags on to the plan.
    let holds = |dir: &ItemPath| requests.keys().any(|p| crate::stale::is_within(p, dir));
    let folder = |(p, f): (&ItemPath, &Arc<ConfigView>)| {
        let saved = match (f.dirty, f.disk) {
            (false, _) => Some(f.tags.clone()),
            (true, DiskState::Present) => before.and_then(|b| b.folders.get(p)?.clone()),
            (true, _) => None,
        };
        holds(p).then(|| (p.clone(), saved))
    };
    let folders = content.folders.iter().filter_map(folder).collect();
    Some(RunnerRecord { requests, folders })
}

fn shows_member(content: &Content, item: &ItemPath) -> bool {
    item.is_root() || content.requests.contains_key(item) || content.folders.contains_key(item)
}

/// The directory name of `root`, or the whole path when it has none.
pub(crate) fn display_name(root: &Path) -> SharedString {
    root.file_name()
        .unwrap_or(root.as_os_str())
        .to_string_lossy()
        .into_owned()
        .into()
}

/// The one label rule for a collection: its `info.name`, a blank name counting as missing,
/// else the directory name of `root`. Pure, so render paths can call it without reading disk.
pub(crate) fn label_or_dir(name: Option<&str>, root: &Path) -> SharedString {
    match name.filter(|n| !n.trim().is_empty()) {
        Some(n) => n.to_owned().into(),
        None => display_name(root),
    }
}

/// What a collection at `root` is called in the recents, read off the disk. An unreadable
/// file is no error here, only the fallback.
pub(crate) fn collection_label(root: &Path) -> SharedString {
    label_or_dir(apinata_core::collection_name(root).as_deref(), root)
}

// The one item a tab shows, which a delete of it or a folder above takes with it.
fn trashed_path(kind: &TabKind) -> Option<&ItemPath> {
    match kind {
        TabKind::Request(p)
        | TabKind::Folder(p)
        | TabKind::Runner(p)
        | TabKind::Example { item: p, .. } => Some(p),
        _ => None,
    }
}

/// `root` made absolute, so roots given relative match the absolute ones `open` stores.
pub(crate) fn absolute(root: &Path) -> PathBuf {
    std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf())
}

// Kept responses follow in-app renames and moves and go with deletes, whatever the remember
// setting, since stale entries would otherwise resurface under a reused path.
fn forward_to_store(root: &Path, notices: &[Notice], cx: &App) {
    for notice in notices {
        let op = match &notice.kind {
            NoticeKind::Moved { from, to } => StoreOp::Rename {
                root: root.to_path_buf(),
                from: from.clone(),
                to: to.clone(),
            },
            NoticeKind::Deleted(item) | NoticeKind::Trashed { item, .. } => StoreOp::Remove {
                root: root.to_path_buf(),
                item: item.clone(),
            },
            _ => continue,
        };
        enqueue(op, cx);
    }
}

/// Moves the example tab `id` after the last other tab of `root` showing the same request or
/// one of its examples; nothing moves when `id` is not an example or has no such tab.
fn place_example(tabs: &mut Tabs, root: &Path, id: TabId) {
    let Some(TabKind::Example { item, .. }) = tabs.get(id).map(|e| e.kind.clone()) else {
        return;
    };
    let anchor = tabs.entries().iter().rev().find(|e| {
        e.id != id
            && e.root == root
            && match &e.kind {
                TabKind::Request(p) | TabKind::Example { item: p, .. } => *p == item,
                _ => false,
            }
    });
    if let Some(anchor) = anchor.map(|e| e.id) {
        tabs.move_after(id, anchor);
    }
}

/// The toast text for tabs closed by a reload, or `None` when nothing closed. One tab holding
/// one item names it; otherwise a single summary counts tabs, not items.
pub fn tabs_closed_message(items: &[ItemPath], tabs: usize) -> Option<String> {
    match (tabs, items) {
        (0, _) => None,
        (1, [item]) => {
            let name = item.file_name();
            let name = name.strip_suffix(".yml").unwrap_or(name);
            Some(format!("{name} is gone, so its tab closed"))
        }
        (1, _) => Some("1 tab closed because its requests are gone".to_string()),
        (n, _) => Some(format!("{n} tabs closed because their requests are gone")),
    }
}

// The session's name for a tab's target, `None` for a tab the session does not keep.
fn saved_tab_of(kind: &TabKind) -> Option<SavedTab> {
    match kind {
        TabKind::Request(p) if !p.is_draft() => Some(SavedTab::Request(p.as_str().to_owned())),
        TabKind::Example { item, index } if !item.is_draft() => {
            Some(SavedTab::Example(SavedExample {
                item: item.as_str().to_owned(),
                index: *index,
            }))
        }
        other => SavedPanelKind::from_tab_kind(other).map(SavedTab::Panel),
    }
}

// The collection-relative paths with something in the index, or `None` when git cannot say.
fn staged_paths(dir: &Path, options: &GitOptions) -> Option<std::collections::BTreeSet<PathBuf>> {
    match collection_status_with(dir, options).ok()? {
        GitState::Repo(r) => Some(
            r.stage
                .into_iter()
                .filter(|(_, s)| *s != StageState::Unstaged)
                .map(|(p, _)| p)
                .collect(),
        ),
        _ => None,
    }
}
