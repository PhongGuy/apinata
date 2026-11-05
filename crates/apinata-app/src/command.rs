//! The messages a `CollectionModel` sends its actor, and the notices the actor reports back.

use std::path::PathBuf;
use std::sync::Arc;

use apinata_core::import::{
    ImportBundle, ImportNote, ImportReport, ImportedEnvironment, ImportedRequest, UpdateSummary,
};
use apinata_core::{
    Assertion, AuthSpec, BodySpec, ClientCertificate, CollectionProxy, DropTarget, EnvVariable,
    GlobalEnvironments, ItemPath, KeyValue, Param, PostResponseVar, RequestKind, RequestSettings,
    RunPlan, ScriptPhase, VarContext, Variable, VariableSet,
};

/// The document a command addresses.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Target {
    /// A request or opaque file.
    Request(ItemPath),
    /// A folder's `folder.yml`.
    Folder(ItemPath),
    /// The collection's `opencollection.yml`.
    Collection,
    /// An environment file, by name.
    Environment(String),
}

/// An editable field; the unit of in-flight tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(missing_docs)]
pub enum Field {
    Name,
    Method,
    UrlParams,
    Headers,
    Body,
    Auth,
    Vars,
    PostResponseVars,
    Assertions,
    Settings,
    /// The default headers a request omits, a settings key of its own.
    OmitHeaders,
    Docs,
    /// One field per script phase, so an edit of one editor never coalesces with another's.
    ScriptPreRequest,
    ScriptPostResponse,
    ScriptTests,
    Tags,
    Variables,
    Color,
    Extends,
    /// The collection's proxy and client certificates.
    Network,
    /// The collection's request presets.
    Presets,
}

/// A new value for one field of a target.
#[derive(Clone, Debug)]
#[allow(missing_docs)]
#[expect(
    clippy::large_enum_variant,
    reason = "an edit is sent once per change; an oauth2 auth is carried unboxed"
)]
pub enum Edit {
    Name(String),
    Method(String),
    Url(String),
    Params(Vec<Param>),
    /// Settle a URL and params block that disagree by filling the table from the URL.
    KeepUrlQuery,
    /// Settle a URL and params block that disagree by rebuilding the URL query from the table.
    KeepParamsQuery,
    Headers(Vec<KeyValue>),
    Body(BodySpec),
    Auth(AuthSpec),
    Vars(Vec<Variable>),
    PostResponseVars(Vec<PostResponseVar>),
    Assertions(Vec<Assertion>),
    Settings(RequestSettings),
    /// The names of the default headers the request omits; empty omits none.
    OmitHeaders(Vec<String>),
    Docs(String),
    /// A script phase's code; blank removes it.
    Script(ScriptPhase, String),
    Tags(Vec<String>),
    Variables(Vec<EnvVariable>),
    Color(Option<String>),
    Extends(Option<String>),
    /// The collection's proxy; a new literal password moves to `.env`.
    Proxy(CollectionProxy),
    /// The collection's client certificates; a new literal passphrase moves to `.env`.
    ClientCertificates(Vec<ClientCertificate>),
    /// The collection's host overrides.
    HostOverrides(Vec<apinata_core::HostOverride>),
    /// The collection's request type and URL presets for new requests.
    Presets(apinata_core::CollectionPresets),
    /// Move every literal network secret of the collection file to `.env`.
    MoveNetworkSecrets,
    /// Move the target's literal OAuth 2.0 client secret and password to `.env`.
    MoveAuthSecrets,
}

impl Edit {
    /// The field this edit writes; the URL and its params share [`Field::UrlParams`], since
    /// each rewrites the other.
    pub fn field(&self) -> Field {
        match self {
            Edit::Name(_) => Field::Name,
            Edit::Method(_) => Field::Method,
            Edit::Url(_) | Edit::Params(_) | Edit::KeepUrlQuery | Edit::KeepParamsQuery => {
                Field::UrlParams
            }
            Edit::Headers(_) => Field::Headers,
            Edit::Body(_) => Field::Body,
            Edit::Auth(_) | Edit::MoveAuthSecrets => Field::Auth,
            Edit::Vars(_) => Field::Vars,
            Edit::PostResponseVars(_) => Field::PostResponseVars,
            Edit::Assertions(_) => Field::Assertions,
            Edit::Settings(_) => Field::Settings,
            Edit::OmitHeaders(_) => Field::OmitHeaders,
            Edit::Docs(_) => Field::Docs,
            Edit::Script(ScriptPhase::PreRequest, _) => Field::ScriptPreRequest,
            Edit::Script(ScriptPhase::PostResponse, _) => Field::ScriptPostResponse,
            Edit::Script(..) => Field::ScriptTests,
            Edit::Tags(_) => Field::Tags,
            Edit::Variables(_) => Field::Variables,
            Edit::Color(_) => Field::Color,
            Edit::Extends(_) => Field::Extends,
            Edit::Presets(_) => Field::Presets,
            Edit::Proxy(_)
            | Edit::ClientCertificates(_)
            | Edit::HostOverrides(_)
            | Edit::MoveNetworkSecrets => Field::Network,
        }
    }
}

/// A change to the collection's tree of folders and requests.
#[allow(missing_docs)]
pub enum TreeOp {
    CreateRequest {
        parent: ItemPath,
        name: String,
    },
    CreateFolder {
        parent: ItemPath,
        name: String,
    },
    Clone {
        item: ItemPath,
        name: String,
    },
    Rename {
        item: ItemPath,
        name: String,
        rename_file: bool,
    },
    Delete {
        item: ItemPath,
    },
    /// Delete `item` into the platform trash; answers `Trashed` with a ticket that `Restore`
    /// takes until `ForgetTrash` drops it.
    Trash {
        item: ItemPath,
    },
    /// Undo the `Trash` that answered `ticket`; answers `Restored`, a failure that says why,
    /// or nothing when the ticket is unknown or forgotten.
    Restore {
        ticket: u64,
    },
    /// Drop `ticket`, so a later `Restore` of it does nothing. The item stays in the trash.
    ForgetTrash {
        ticket: u64,
    },
    Move {
        item: ItemPath,
        to: DropTarget,
    },
    /// Renumber the items of `folder` by name; answers with no notice, or `Failed`.
    SortByName {
        folder: ItemPath,
    },
    /// Start an unsaved request of `kind` in the folder `parent`; answers `Created` with its path.
    CreateDraft {
        parent: ItemPath,
        kind: RequestKind,
    },
    /// Start an unsaved request in `parent` with the kind and URL the collection's presets
    /// name; answers `Created` with its path.
    CreateDraftFromPresets {
        parent: ItemPath,
    },
    /// Start a draft in `parent` holding `snapshot`, unsaved until saved when `unsaved`;
    /// answers `Created` with its path.
    CreateDraftFrom {
        parent: ItemPath,
        snapshot: apinata_core::DraftSnapshot,
        unsaved: bool,
    },
    /// Write a draft as the request `name` in `parent`; answers `Moved` to the new file.
    SaveDraft {
        draft: ItemPath,
        parent: ItemPath,
        name: String,
    },
    /// Write `snapshot` as the request `name` in `parent`; answers `Created` with the new path.
    SaveSnapshot {
        parent: ItemPath,
        name: String,
        snapshot: apinata_core::DraftSnapshot,
    },
    /// Drop a draft and its edits; answers nothing.
    DiscardDraft {
        draft: ItemPath,
    },
    /// Write a copy of `source`, taken from any open collection, into the folder `target`;
    /// answers `Created` with the new path.
    Paste {
        target: ItemPath,
        source: apinata_core::PasteSource,
    },
    /// Save `response` as a new example of the request `item`; answers `ExampleAdded`.
    AddExample {
        item: ItemPath,
        name: String,
        description: String,
        response: apinata_core::ExampleResponse,
    },
    /// Rename the request's example `index`; answers nothing.
    RenameExample {
        item: ItemPath,
        index: usize,
        name: String,
    },
    /// Delete the request's example `index`; answers `ExampleDeleted`.
    DeleteExample {
        item: ItemPath,
        index: usize,
    },
}

/// A change to the set of environments or a variable's secret flag.
#[allow(missing_docs)]
pub enum EnvOp {
    Create {
        name: String,
    },
    Rename {
        from: String,
        to: String,
    },
    /// Copy `from` to a new environment; answers `EnvironmentCreated`.
    Duplicate {
        from: String,
    },
    Delete {
        name: String,
    },
    SetSecret {
        env: String,
        var: String,
        secret: bool,
    },
    /// Store a secret variable's value; the value never reaches a file or a snapshot.
    SetSecretValue {
        /// The environment holding the variable.
        env: String,
        /// The secret variable.
        var: String,
        /// The value to store.
        value: String,
    },
}

/// A run plan for one item with the variable context to resolve it in.
pub struct Planned {
    /// The requests to run.
    pub plan: RunPlan,
    /// The variables for the selected environment.
    pub context: VarContext,
}

/// Why a `Plan` command produced no plan.
#[derive(Debug)]
pub enum PlanError {
    /// No item at this path.
    NotFound(ItemPath),
    /// Selecting the requests failed.
    Select(String),
    /// Building the variable context failed.
    Context(String),
}

/// What a `Command::Network` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkOp {
    /// Record the saved settings, as the selected environment resolves them, as seen.
    Acknowledge,
    /// Move an unreadable record aside so a new one starts.
    Reset,
    /// Look again, after something else changed the record.
    Recheck,
}

/// Where a list of fileless groups, or why it failed, goes.
pub type FilelessReply =
    async_channel::Sender<Result<Vec<crate::orphan_section::FilelessGroup>, String>>;

/// One instruction to the collection actor.
pub enum Command {
    /// Act on the collection's record of acknowledged network settings.
    Network(NetworkOp),
    /// Set one field of a target.
    Edit {
        /// The document to edit.
        target: Target,
        /// The new value.
        edit: Edit,
    },
    /// Write a target to disk.
    Save(Target),
    /// Drop a target's draft for what the disk holds.
    Revert(Target),
    /// Write a target over a conflicting file.
    Overwrite(Target),
    /// Forget the edits held back as stale for a target.
    DiscardStale(Target),
    /// Change the tree.
    Tree(TreeOp),
    /// Change the environments.
    Env(EnvOp),
    /// Reload these files or directories from disk.
    Reload(Vec<PathBuf>),
    /// Reload the whole collection from disk.
    ReloadAll,
    /// Choose the active environment, or none.
    SelectEnvironment(Option<String>),
    /// Choose the global environment every context of this collection is built with, or none.
    SetGlobalEnvironment(Option<(Arc<GlobalEnvironments>, String)>),
    /// Build a run plan for an item and send it on `reply`.
    Plan {
        /// The request or folder to plan.
        item: ItemPath,
        /// Where the result goes.
        reply: async_channel::Sender<Result<Planned, PlanError>>,
    },
    /// Like `Plan`, but from a fresh open of the saved files, so open drafts never run.
    PlanSaved {
        /// The request or folder to plan.
        item: ItemPath,
        /// Where the result goes.
        reply: async_channel::Sender<Result<Planned, PlanError>>,
    },
    /// Like `PlanSaved`, for a set of items. Stale or non-runnable ones are skipped by core.
    PlanSavedItems {
        /// The items to plan.
        items: Vec<ItemPath>,
        /// Where the result goes.
        reply: async_channel::Sender<Result<Planned, PlanError>>,
    },
    /// Take the saved content of each of `items` (see `Collection::paste_source`) and send it,
    /// item by item, on `reply`. Changes nothing.
    CopyOut {
        /// The requests and folders to copy.
        items: Vec<ItemPath>,
        /// Where the copies, or why one failed, go.
        reply: async_channel::Sender<Vec<(ItemPath, Result<apinata_core::PasteSource, String>)>>,
    },
    /// Send every variable a request can see, with scope and value, on `reply`.
    Variables {
        /// The request.
        item: ItemPath,
        /// Where the result goes.
        reply: async_channel::Sender<Result<VariableSet, PlanError>>,
    },
    /// Send every secret value the collection's variable context holds
    /// (`VarContext::secret_values`) on `reply`, or why the context cannot be built.
    SecretValues {
        /// Where the values, or the context error's text, go.
        reply: async_channel::Sender<Result<Vec<String>, String>>,
    },
    /// Search the collection's request text for `query` and send the hits on `reply`, or the
    /// text of why the query cannot run (an invalid regular expression). The search reads only
    /// the request files held in memory, never variables or secrets.
    Search {
        /// The text to find.
        query: String,
        /// Case and regular expression switches.
        options: apinata_core::SearchOptions,
        /// Where the hits, or the error's text, go.
        reply: async_channel::Sender<Result<apinata_core::SearchResults, String>>,
    },
    /// Replace `query` with `replacement` in the collection's saved request files, within
    /// `scope`, and send the report on `reply`, or the text of why the query cannot run.
    /// Requests holding unsaved edits are skipped by the core and named in the report.
    Replace {
        /// The text to find.
        query: String,
        /// What to put in its place.
        replacement: String,
        /// Case and regular expression switches.
        options: apinata_core::SearchOptions,
        /// Every match, or the one hit.
        scope: apinata_core::ReplaceScope,
        /// Where the report, or the error's text, goes.
        reply: async_channel::Sender<Result<apinata_core::ReplaceReport, String>>,
    },
    /// Read a secret variable's stored value and send it on `reply` only, never in a notice.
    RevealSecret {
        /// The environment holding the variable.
        env: String,
        /// The secret variable.
        var: String,
        /// Where the stored value, or why there is none to read, goes.
        reply: async_channel::Sender<Result<Option<String>, String>>,
    },
    /// Read the value stored under a variable's name, whether the variable is secret, plain or
    /// not yet in the file, and send it on `reply` only. It tells the lock what a change loses.
    StoredSecret {
        /// The environment.
        env: String,
        /// The variable's name.
        var: String,
        /// Where the stored value, or why it could not be read, goes.
        reply: async_channel::Sender<Result<Option<String>, String>>,
    },
    /// List the orphaned secrets of an environment on `reply` only, and with `fileless` the
    /// stored secrets of environments with no file, from the same one look at the store.
    OrphanedSecrets {
        /// The environment.
        env: String,
        /// Where the names, or why they could not be listed, go.
        reply: async_channel::Sender<Result<Vec<String>, String>>,
        /// Where the fileless groups, or why they could not be listed, go.
        fileless: Option<FilelessReply>,
    },
    /// List the stored secrets of environments with no file on `reply` only.
    FilelessSecrets {
        /// Where the groups, or why they could not be listed, go.
        reply: FilelessReply,
    },
    /// Remove named stored secrets of an environment with no file, each named by a person's
    /// confirmed action. Refused if the environment has a file again.
    RemoveFilelessSecrets {
        /// The environment.
        env: String,
        /// The variables to remove.
        names: Vec<String>,
        /// Each warning's text, or why nothing was removed.
        reply: async_channel::Sender<Result<Vec<String>, String>>,
    },
    /// Remove named orphans of an environment, each named by a person's confirmed action.
    RemoveOrphanedSecrets {
        /// The environment.
        env: String,
        /// The orphans to remove.
        names: Vec<String>,
        /// Each warning's text, or why nothing was removed.
        reply: async_channel::Sender<Result<Vec<String>, String>>,
    },
    /// Replace a request draft's method, URL, params, headers, body, auth, docs and settings
    /// with an import through `ImportedRequest::apply_to`; its name stays and the draft stays
    /// unsaved.
    ApplyImport {
        /// The request whose draft takes the import.
        item: ItemPath,
        /// The parsed request.
        request: ImportedRequest,
    },
    /// Keep the secrets of an imported request as secret variables of the selected environment,
    /// else the only one, through `store_request_secrets`. The notes come back to `reply` with
    /// each stored secret's needs-input note turned into a converted one.
    StoreImportSecrets {
        /// The request whose draft took the import. Nothing is stored unless the
        /// `ApplyImport` just before it for this item landed.
        item: ItemPath,
        /// The imported request whose `secrets` are kept.
        request: ImportedRequest,
        /// The import's notes so far.
        notes: Vec<ImportNote>,
        /// Receives the updated notes.
        reply: async_channel::Sender<Vec<ImportNote>>,
    },
    /// Write an imported environment through `add_environment` and store its secrets; the
    /// report, or the error's text, goes to `reply`.
    ImportEnvironment {
        /// The parsed environment.
        env: ImportedEnvironment,
        /// Where the report, or why the import failed, goes.
        reply: async_channel::Sender<Result<ImportReport, String>>,
    },
    /// The disk text and the serialized draft of `target`'s file, for a comparison; `None` when
    /// the target has no open file. Changes nothing.
    FileTexts {
        /// The request, folder, collection or environment to read.
        target: Target,
        /// Where the texts go.
        reply: async_channel::Sender<Option<apinata_core::FileTexts>>,
    },
    /// Whether this collection was imported from a spec and so can be updated from one.
    UpdateInfo {
        /// Where the answer goes.
        reply: async_channel::Sender<bool>,
    },
    /// The folders (never the root) that carry a spec baseline of their own, in tree order.
    UpdateFolders {
        /// Where the answer goes.
        reply: async_channel::Sender<Vec<ItemPath>>,
    },
    /// Report what updating from `bundle` would do, writing nothing. Fails like `ApplyUpdate`
    /// except that unsaved edits do not block a plan.
    PlanUpdate {
        /// The freshly parsed spec.
        bundle: Arc<ImportBundle>,
        /// Whether requests missing from the spec would be removed.
        prune: bool,
        /// The folder whose spec is planned; `None` is the collection root.
        folder: Option<ItemPath>,
        /// Where the summary, or why there is none, goes.
        reply: async_channel::Sender<Result<UpdateSummary, String>>,
    },
    /// Update the collection from `bundle`, trashing pruned requests, then reload it. Refused
    /// while the actor holds unsaved edits, and when the collection has no spec baseline.
    ApplyUpdate {
        /// The freshly parsed spec.
        bundle: Arc<ImportBundle>,
        /// Whether requests missing from the spec are removed (into the trash).
        prune: bool,
        /// The folder whose spec is applied; `None` is the collection root.
        folder: Option<ItemPath>,
        /// Where the applied summary, or why nothing was applied, goes.
        reply: async_channel::Sender<Result<UpdateSummary, String>>,
    },
}

/// A command numbered by the model and stamped with the last snapshot it had seen.
pub struct Envelope {
    /// Rises by one per command.
    pub id: u64,
    /// The sequence of the snapshot the sender last received.
    pub seen: u64,
    /// The snapshot the command's content was built from, at most `seen`: an edit built from
    /// content a later reload replaced is held back as stale.
    pub basis: u64,
    /// The instruction.
    pub command: Command,
}

/// What became of a command, reported in the next snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    /// The id of the command this notice is about.
    pub command: u64,
    /// What happened.
    pub kind: NoticeKind,
}

/// The outcome a [`Notice`] reports.
#[derive(Debug, Clone, PartialEq)]
pub enum NoticeKind {
    /// The target no longer exists.
    NotFound(Target),
    /// The target has no such field.
    Unsupported {
        /// The addressed document.
        target: Target,
        /// The field it lacks.
        field: Field,
    },
    /// The command failed.
    Failed {
        /// The document involved, if any.
        target: Option<Target>,
        /// The error, for display.
        message: String,
        /// The header row a refused header name belongs to, when the failure is that.
        row: Option<usize>,
    },
    /// The edit was held back because the target changed on disk after the sender saw it.
    Stale(Target),
    /// A request or folder was created at this path.
    Created(ItemPath),
    /// An environment was created.
    EnvironmentCreated(String),
    /// An item moved, by rename or move.
    Moved {
        /// The old path.
        from: ItemPath,
        /// The new path.
        to: ItemPath,
    },
    /// A request or folder was deleted from the app.
    Deleted(ItemPath),
    /// A request or folder went to the trash; `ticket` undoes it.
    Trashed {
        /// Where it was in the collection.
        item: ItemPath,
        /// The id to pass to `TreeOp::Restore` or `TreeOp::ForgetTrash`.
        ticket: u64,
    },
    /// A trashed request or folder was put back.
    Restored(ItemPath),
    /// A request's example was deleted from the app.
    ExampleDeleted {
        /// The request holding it.
        item: ItemPath,
        /// Its index before the delete.
        index: usize,
    },
    /// An example was added to a request from the app.
    ExampleAdded {
        /// The request now holding it.
        item: ItemPath,
        /// The example's name.
        name: String,
    },
    /// The command succeeded with warnings.
    Warnings(Vec<String>),
}
