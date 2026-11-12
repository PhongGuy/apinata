//! Immutable views of a collection that the actor publishes, and the cache that rebuilds
//! only the views a command touched.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use apinata_core::{
    Assertion, AuthSpec, BodySpec, Collection, CollectionNetwork, EnvVariable, Environment, Folder,
    ItemPath, KeyValue, Param, PostResponseVar, RequestFile, RequestSettings, ScriptPhase, Sibling,
    Variable,
};

use crate::auth_section::inherit_text;
use crate::command::{Edit, Notice, Target};
use crate::network_secrets;

/// What the disk holds for a document, as the app shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiskState {
    /// The file loads as this document.
    #[default]
    Present,
    /// The file changed on disk while the document had a draft.
    Changed,
    /// The file is gone.
    Removed,
    /// The file no longer loads.
    Broken,
}

impl From<apinata_core::DiskState> for DiskState {
    fn from(state: apinata_core::DiskState) -> Self {
        match state {
            apinata_core::DiskState::Removed => DiskState::Removed,
            apinata_core::DiskState::Broken => DiskState::Broken,
            _ => DiskState::Present,
        }
    }
}

/// The kind of item a sidebar row shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// A sub-folder.
    Folder,
    /// An `http` request.
    Request,
    /// A request file of a type Apinata does not model.
    Opaque,
}

/// A request type whose files the app lists and keeps but cannot send yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unsupported {
    /// A `websocket` request.
    WebSocket,
    /// A `grpc` request.
    Grpc,
}

impl Unsupported {
    /// The kind a file's `info.type` names; other types, GraphQL among them, are `None`.
    pub fn from_type(kind: &str) -> Option<Self> {
        match kind {
            "websocket" => Some(Unsupported::WebSocket),
            "grpc" => Some(Unsupported::Grpc),
            _ => None,
        }
    }

    /// The short tag its row shows in the method column.
    pub fn tag(self) -> &'static str {
        match self {
            Unsupported::WebSocket => "WS",
            Unsupported::Grpc => "gRPC",
        }
    }

    /// What its row's tooltip and the note on opening it say.
    pub fn notice(self) -> &'static str {
        match self {
            Unsupported::WebSocket => {
                "WebSocket requests cannot be sent yet. The file is kept as it is."
            }
            Unsupported::Grpc => "gRPC requests cannot be sent yet. The file is kept as it is.",
        }
    }
}

/// One sidebar row.
#[derive(Debug, Clone)]
pub struct Row {
    /// The item's path.
    pub path: ItemPath,
    /// What the item is.
    pub kind: RowKind,
    /// The display name.
    pub name: String,
    /// Nesting below the root; top-level items are 0.
    pub depth: usize,
    /// Whether the item has unsaved changes.
    pub dirty: bool,
    /// What the disk holds for the item.
    pub disk: DiskState,
    /// The kind of an opaque row the app cannot send yet.
    pub unsupported: Option<Unsupported>,
}

/// A request's editable state.
#[derive(Clone)]
#[allow(missing_docs)]
pub struct RequestView {
    pub name: String,
    /// HTTP or GraphQL; a GraphQL request edits its body as a query and variables.
    pub kind: apinata_core::RequestKind,
    pub method: String,
    pub url: String,
    pub params: Vec<Param>,
    /// Where the file's URL query and params block disagree; in-app edits keep them agreeing.
    pub query_mismatch: Option<apinata_core::QueryMismatch>,
    pub headers: Vec<KeyValue>,
    pub body: BodySpec,
    pub auth: AuthSpec,
    /// What an inheriting auth inherits and from where, as the Auth section shows it.
    pub inherited_auth: String,
    /// The headers the collection and folders add, with the level that supplies each.
    pub inherited_headers: Vec<crate::inherited_headers_section::InheritedRow>,
    pub vars: Vec<Variable>,
    pub post_response_vars: Vec<PostResponseVar>,
    pub assertions: Vec<Assertion>,
    pub settings: RequestSettings,
    /// The default headers the request switches off, from `settings.omitHeaders`.
    pub omitted_headers: Vec<String>,
    pub docs: String,
    pub scripts: ScriptCode,
    pub tags: Vec<String>,
    pub seq: Option<u64>,
    pub dirty: bool,
    pub disk: DiskState,
    /// The seq of the last outside change to this request's file, 0 if none.
    pub reloaded: u64,
    /// An unsaved new request, held in memory with no file and no sidebar row.
    pub draft: bool,
    /// The saved examples, in file order.
    pub examples: Vec<apinata_core::Example>,
    /// A draft's mapping with literal credentials blanked; None for a saved request.
    pub snapshot: Option<Arc<apinata_core::DraftSnapshot>>,
    /// Whether `snapshot` blanked a literal credential.
    pub blanks_credentials: bool,
}

/// The code of each script phase, as the file holds it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct ScriptCode {
    pub pre_request: String,
    pub post_response: String,
    pub tests: String,
}

impl ScriptCode {
    /// Read each phase through `script`, the file's getter.
    fn of(script: impl Fn(ScriptPhase) -> String) -> ScriptCode {
        ScriptCode {
            pre_request: script(ScriptPhase::PreRequest),
            post_response: script(ScriptPhase::PostResponse),
            tests: script(ScriptPhase::Tests),
        }
    }

    /// The code of `phase`; a phase this view does not know reads as the tests script.
    pub fn get(&self, phase: ScriptPhase) -> &str {
        match phase {
            ScriptPhase::PreRequest => &self.pre_request,
            ScriptPhase::PostResponse => &self.post_response,
            _ => &self.tests,
        }
    }
}

/// A folder's or the collection's configuration.
#[derive(Clone)]
#[allow(missing_docs)]
pub struct ConfigView {
    pub name: Option<String>,
    pub headers: Vec<KeyValue>,
    pub auth: AuthSpec,
    /// What a folder's Inherit auth inherits and from where; empty for the collection.
    pub inherited_auth: String,
    pub vars: Vec<Variable>,
    pub post_response_vars: Vec<PostResponseVar>,
    pub docs: String,
    /// The folder's `info.tags`, normalised; a collection has none.
    pub tags: Vec<String>,
    pub scripts: ScriptCode,
    pub dirty: bool,
    pub disk: DiskState,
    /// The seq of the last outside change to this file, 0 if none.
    pub reloaded: u64,
    /// The proxy and client certificates; only the collection has them.
    pub network: Option<CollectionNetwork>,
    /// Whether `network` holds a literal password or passphrase.
    pub plain_secrets: bool,
    /// The request presets for new requests; only the collection has them.
    pub presets: Option<apinata_core::CollectionPresets>,
}

/// An environment's editable state; secret values stay in the store, never here.
#[derive(Clone)]
#[allow(missing_docs)]
pub struct EnvView {
    pub name: String,
    pub variables: Vec<EnvVariable>,
    pub color: Option<String>,
    pub extends: Option<String>,
    pub dirty: bool,
    pub disk: DiskState,
    /// The seq of the last outside reload of this environment's file, 0 if none.
    pub reloaded: u64,
}

/// Everything a snapshot shows about the collection's files.
#[derive(Clone)]
pub struct Content {
    /// Sidebar rows, depth-first, excluding the root.
    pub rows: Arc<[Row]>,
    /// Requests by path.
    pub requests: Arc<HashMap<ItemPath, Arc<RequestView>>>,
    /// Folders by path.
    pub folders: Arc<HashMap<ItemPath, Arc<ConfigView>>>,
    /// `opencollection.yml`.
    pub collection: Arc<ConfigView>,
    /// Environments in the collection's order.
    pub environments: Arc<[Arc<EnvView>]>,
    /// Files that failed to load.
    pub load_errors: Arc<[LoadErrorView]>,
    /// Directories to watch, as core's `Collection::watch_dirs` reports them.
    pub dirs: Arc<[PathBuf]>,
}

/// A file that failed to load.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct LoadErrorView {
    /// Relative to the collection root, or absolute when outside it.
    pub path: PathBuf,
    /// The error, for display.
    pub message: String,
}

/// One published state of a collection.
#[derive(Clone)]
pub struct CollectionSnapshot {
    /// Rises by one per published snapshot.
    pub seq: u64,
    /// The id of the last command this snapshot covers.
    pub acked: u64,
    /// The collection's files.
    pub content: Content,
    /// The active environment.
    pub selected_environment: Option<String>,
    /// Edits held back because their target changed on disk.
    pub stale: Arc<HashMap<Target, Vec<Edit>>>,
    /// What became of the commands this snapshot covers.
    pub notices: Arc<[Notice]>,
    /// Rises whenever the actor drops its variable context (environments, secrets, folder or
    /// collection edits, reloads), so views of variables know to ask again.
    pub context_generation: u64,
    /// Rises whenever `.env` changes on disk; it holds values no other part of the snapshot shows.
    pub dotenv_seq: u64,
    /// The keys `.env` defines.
    pub dotenv_stored: Arc<std::collections::BTreeSet<String>>,
    /// The `.env` names each target's unsaved draft holds a secret for, written when it is saved.
    pub dotenv_held: Arc<HashMap<Target, std::collections::BTreeSet<String>>>,
    /// What the collection's Network section says about settings changed outside the app.
    pub network_notice: Option<crate::network_gate::Notice>,
}

impl CollectionSnapshot {
    /// The `.env` names `target`'s secret fields may show as stored or held.
    pub fn dotenv_names(&self, target: &Target) -> DotenvNames {
        DotenvNames {
            stored: Arc::clone(&self.dotenv_stored),
            held: self.dotenv_held.get(target).cloned().unwrap_or_default(),
        }
    }
}

/// The `.env` names a target's secret fields know: those `.env` defines, and those its unsaved
/// draft holds a value for until it is saved.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DotenvNames {
    /// Keys `.env` defines.
    pub stored: Arc<std::collections::BTreeSet<String>>,
    /// Names held for the target's draft.
    pub held: std::collections::BTreeSet<String>,
}

impl DotenvNames {
    /// Where the secret `.env` name `name` lives, or None when it is neither held nor stored.
    pub fn home(&self, name: &str) -> Option<SecretHome> {
        if self.held.contains(name) {
            Some(SecretHome::Held)
        } else {
            self.stored.contains(name).then_some(SecretHome::Stored)
        }
    }
}

/// Where a secret field's `.env` value is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecretHome {
    /// Held by the unsaved draft; written to `.env` when it is saved.
    Held,
    /// In `.env`.
    Stored,
}

/// Views from the previous build, reused for every key not touched since.
#[derive(Default)]
pub struct SnapshotCache {
    requests: HashMap<ItemPath, Arc<RequestView>>,
    folders: HashMap<ItemPath, Arc<ConfigView>>,
    collection: Option<Arc<ConfigView>>,
    environments: HashMap<String, Arc<EnvView>>,
    touched: HashSet<Target>,
    all: bool,
}

impl SnapshotCache {
    /// An empty cache; the first build creates every view.
    pub fn new() -> Self {
        Self::default()
    }

    /// Rebuild `target`'s view at the next build. A folder or the collection also drops the
    /// request and folder views, whose inherited auth text names its auth and display name.
    pub fn touch(&mut self, target: &Target) {
        if matches!(target, Target::Folder(_) | Target::Collection) {
            self.requests.clear();
            self.folders.clear();
        }
        self.touched.insert(target.clone());
    }

    /// Rebuild every view at the next build.
    pub fn touch_all(&mut self) {
        self.all = true;
    }

    /// The content of `c`. Rows are always rebuilt; a view is reused unless touched, and the
    /// cache keeps only the views of this build, so removed items do not linger.
    pub fn build(&mut self, c: &Collection) -> Content {
        self.build_with(c, &HashSet::new(), &HashMap::new(), &HashMap::new())
    }

    /// As [`SnapshotCache::build`], showing every target in `changed` as
    /// [`DiskState::Changed`], each environment with its stamp in `env_reloads`, and each
    /// request, folder and the collection with its stamp in `reloads`. A caller that changes a
    /// target's membership or stamp touches it.
    pub fn build_with(
        &mut self,
        c: &Collection,
        changed: &HashSet<Target>,
        env_reloads: &HashMap<String, u64>,
        reloads: &HashMap<Target, u64>,
    ) -> Content {
        let mut walk = Walk {
            cache: self,
            collection: c,
            changed,
            reloads,
            rows: Vec::new(),
            requests: HashMap::new(),
            folders: HashMap::new(),
        };
        walk.folder(c.tree(), 0);
        // Drafts get views so their tabs can edit them, but no rows: the sidebar lists files.
        for draft in c.drafts() {
            if let Some(path) = c.item_path(draft.path()) {
                let view = walk.request(&path, draft);
                walk.requests.insert(path, view);
            }
        }
        let Walk {
            rows,
            requests,
            folders,
            ..
        } = walk;

        let collection = match &self.collection {
            Some(view) if self.fresh(&Target::Collection) => Arc::clone(view),
            _ => Arc::new(overlay(
                collection_view(c),
                changed,
                reloads,
                &Target::Collection,
            )),
        };
        let environments: Vec<Arc<EnvView>> = c
            .environments()
            .iter()
            .map(|env| {
                let target = Target::Environment(env.name().to_owned());
                match self.environments.get(env.name()) {
                    Some(view) if self.fresh(&target) => Arc::clone(view),
                    _ => {
                        let view = env_view(env);
                        let disk = mark(changed, &target, view.disk);
                        let reloaded = env_reloads.get(env.name()).copied().unwrap_or(0);
                        Arc::new(EnvView {
                            disk,
                            reloaded,
                            ..view
                        })
                    }
                }
            })
            .collect();

        let mut load_errors: Vec<LoadErrorView> = c
            .load_errors()
            .iter()
            .map(|e| LoadErrorView {
                path: e
                    .path()
                    .strip_prefix(c.root())
                    .unwrap_or(e.path())
                    .to_path_buf(),
                message: e.message().to_owned(),
            })
            .collect();
        load_errors.sort_by(|a, b| a.path.cmp(&b.path));

        self.requests = requests;
        self.folders = folders;
        self.collection = Some(Arc::clone(&collection));
        self.environments = environments
            .iter()
            .map(|v| (v.name.clone(), Arc::clone(v)))
            .collect();
        self.touched.clear();
        self.all = false;

        Content {
            rows: rows.into(),
            requests: Arc::new(self.requests.clone()),
            folders: Arc::new(self.folders.clone()),
            collection,
            environments: environments.into(),
            load_errors: load_errors.into(),
            dirs: c.watch_dirs().into(),
        }
    }

    /// Whether a cached view for `target` may be reused.
    fn fresh(&self, target: &Target) -> bool {
        !self.all && !self.touched.contains(target)
    }
}

/// One depth-first pass over the tree, collecting rows and the new view maps.
struct Walk<'a> {
    cache: &'a SnapshotCache,
    collection: &'a Collection,
    changed: &'a HashSet<Target>,
    reloads: &'a HashMap<Target, u64>,
    rows: Vec<Row>,
    requests: HashMap<ItemPath, Arc<RequestView>>,
    folders: HashMap<ItemPath, Arc<ConfigView>>,
}

impl Walk<'_> {
    /// The view of the request at `path`, reused from the cache while still fresh.
    fn request(&self, path: &ItemPath, request: &RequestFile) -> Arc<RequestView> {
        let target = Target::Request(path.clone());
        match self.cache.requests.get(path) {
            Some(v) if self.cache.fresh(&target) => Arc::clone(v),
            _ => {
                let view = request_view(path, request, self.collection);
                let disk = mark(self.changed, &target, view.disk);
                let reloaded = self.reloads.get(&target).copied().unwrap_or(0);
                Arc::new(RequestView {
                    disk,
                    reloaded,
                    ..view
                })
            }
        }
    }

    fn folder(&mut self, folder: &Folder, depth: usize) {
        for sibling in folder.siblings() {
            // An item whose path is not a valid ItemPath cannot be addressed by a command.
            let Some(path) = self.collection.item_path(sibling.path()) else {
                continue;
            };
            match sibling {
                Sibling::Folder(sub) => {
                    let target = Target::Folder(path.clone());
                    let view = match self.cache.folders.get(&path) {
                        Some(v) if self.cache.fresh(&target) => Arc::clone(v),
                        _ => Arc::new(overlay(
                            folder_view(sub, &path, self.collection),
                            self.changed,
                            self.reloads,
                            &target,
                        )),
                    };
                    self.rows.push(Row {
                        path: path.clone(),
                        kind: RowKind::Folder,
                        name: sub.name().to_owned(),
                        depth,
                        dirty: view.dirty,
                        disk: mark(self.changed, &target, sub.disk_state().into()),
                        unsupported: None,
                    });
                    self.folders.insert(path, view);
                    self.folder(sub, depth + 1);
                }
                Sibling::Request(request) => {
                    let view = self.request(&path, request);
                    self.rows.push(Row {
                        path: path.clone(),
                        kind: RowKind::Request,
                        name: view.name.clone(),
                        depth,
                        dirty: view.dirty,
                        disk: view.disk,
                        unsupported: None,
                    });
                    self.requests.insert(path, view);
                }
                Sibling::Opaque(item) => self.rows.push(Row {
                    disk: mark(
                        self.changed,
                        &Target::Request(path.clone()),
                        DiskState::Present,
                    ),
                    path,
                    kind: RowKind::Opaque,
                    name: item.name().to_owned(),
                    depth,
                    dirty: item.is_dirty(),
                    unsupported: Unsupported::from_type(item.kind()),
                }),
                _ => {}
            }
        }
    }
}

/// `state`, unless the target is marked as changed on disk.
fn mark(changed: &HashSet<Target>, target: &Target, state: DiskState) -> DiskState {
    if changed.contains(target) {
        DiskState::Changed
    } else {
        state
    }
}

// `view` with its changed-on-disk mark and reload stamp.
fn overlay(
    view: ConfigView,
    changed: &HashSet<Target>,
    reloads: &HashMap<Target, u64>,
    target: &Target,
) -> ConfigView {
    let disk = mark(changed, target, view.disk);
    let reloaded = reloads.get(target).copied().unwrap_or(0);
    ConfigView {
        disk,
        reloaded,
        ..view
    }
}

fn request_view(path: &ItemPath, r: &RequestFile, collection: &Collection) -> RequestView {
    // Only drafts are kept across restarts, so only they pay for the blanked copy.
    let (snapshot, blanks_credentials) = if path.is_draft() {
        let (snapshot, blanks) = r.snapshot_and_blanks();
        (Some(Arc::new(snapshot)), blanks)
    } else {
        (None, false)
    };
    RequestView {
        name: r.name().to_owned(),
        kind: r.kind(),
        method: r.method().to_owned(),
        url: r.url().to_owned(),
        params: r.shown_params(),
        query_mismatch: r.query_mismatch(),
        headers: r.headers(),
        body: r.body(),
        auth: r.auth(),
        inherited_auth: inherit_text(&collection.inherited_auth(path), collection),
        inherited_headers: crate::inherited_headers_section::inherited_rows(collection, path),
        vars: r.vars(),
        post_response_vars: r.post_response_vars(),
        assertions: r.assertions(),
        settings: r.settings(),
        omitted_headers: r.omitted_headers(),
        docs: r.docs(),
        scripts: ScriptCode::of(|p| r.script(p)),
        tags: r.tags(),
        seq: r.seq(),
        dirty: r.is_dirty(),
        disk: r.disk_state().into(),
        reloaded: 0,
        draft: path.is_draft(),
        examples: r.examples(),
        snapshot,
        blanks_credentials,
    }
}

/// A folder without `folder.yml` shows its directory name and empty settings.
fn folder_view(f: &Folder, path: &ItemPath, collection: &Collection) -> ConfigView {
    let inherited_auth = inherit_text(&collection.inherited_auth(path), collection);
    match f.file() {
        Some(file) => ConfigView {
            name: file.name().map(str::to_owned),
            headers: file.headers(),
            auth: file.auth(),
            inherited_auth,
            vars: file.vars(),
            post_response_vars: file.post_response_vars(),
            docs: file.docs(),
            tags: file.tags(),
            scripts: ScriptCode::of(|p| file.script(p)),
            dirty: file.is_dirty(),
            disk: file.disk_state().into(),
            reloaded: 0,
            network: None,
            plain_secrets: false,
            presets: None,
        },
        None => ConfigView {
            name: Some(f.name().to_owned()),
            headers: Vec::new(),
            auth: AuthSpec::None,
            inherited_auth,
            vars: Vec::new(),
            post_response_vars: Vec::new(),
            docs: String::new(),
            tags: Vec::new(),
            scripts: ScriptCode::default(),
            dirty: false,
            disk: f.disk_state().into(),
            reloaded: 0,
            network: None,
            plain_secrets: false,
            presets: None,
        },
    }
}

fn collection_view(c: &Collection) -> ConfigView {
    let file = c.file();
    let network = file.network();
    ConfigView {
        plain_secrets: network_secrets::plain_secrets(&network),
        presets: Some(file.presets()),
        network: Some(network),
        name: file.name().map(str::to_owned),
        headers: file.headers(),
        auth: file.auth(),
        inherited_auth: String::new(),
        vars: file.vars(),
        post_response_vars: file.post_response_vars(),
        docs: file.docs(),
        tags: Vec::new(),
        scripts: ScriptCode::of(|p| file.script(p)),
        dirty: file.is_dirty(),
        disk: file.disk_state().into(),
        reloaded: 0,
    }
}

fn env_view(e: &Environment) -> EnvView {
    EnvView {
        name: e.name().to_owned(),
        variables: e.variables(),
        color: e.color(),
        extends: e.extends(),
        dirty: e.is_dirty(),
        disk: e.disk_state().into(),
        reloaded: 0,
    }
}
