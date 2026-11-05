//! Importing collections from other tools. A parser turns foreign input into a neutral tree of
//! the core edit types, with a note for everything that could not be carried over unchanged;
//! the tree is then written through the same API every shell uses.

mod budget;
mod curl;
mod environment;
mod fetch;
mod format;
mod har;
mod hoppscotch;
mod insomnia;
mod load;
mod openapi;
mod place;
mod postman;
pub(crate) use postman::pm_chain_at;
mod request;
mod source;
mod update;
mod write;
pub(crate) mod yaml_prepass;

pub use curl::{CurlError, CurlSyntax, ParsedCurl, parse_curl};
pub use environment::add_environment;
pub use fetch::{FetchError, SourceFetch, default_import_name, fetch_source};
pub use format::{
    CollectionFormat, CollectionOptions, EnvironmentFormat, FormatError, GroupBy, ImportBundle,
    ParsedRequest, RequestFormat, parse_collection, parse_collections, parse_environment,
    parse_environments, parse_request,
};
pub use place::{Placed, ensure_folder, place_items};
pub use postman::{parse_postman, parse_postman_environment};
pub use request::{add_request, store_request_secrets};
pub use source::{ImportSource, SourceError, classify_source};
pub use update::{
    ChangedField, FieldHashes, PrunedRequest, RequestChange, SpecBaseline, UpdateOptions,
    UpdateSummary, apply_update, file_field_hashes, operation_key, plan_update, spec_field_hashes,
};
pub use write::{record_spec_baseline, write_collection};

use crate::edit::{AuthSpec, BodySpec, KeyValue, Param, RequestSettings, Variable};
use crate::env_file::EnvVariable;
use crate::error::{CreateError, EditError, EnvError, SaveError, TreeError};
use crate::item_path::ItemPath;
use crate::request::RequestKind;
use crate::script::ScriptPhase;
use std::path::PathBuf;

/// What happened to a piece of input that was not carried over as is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NoteKind {
    /// The input has no counterpart and was left out.
    Dropped,
    /// The input was carried over in a different form.
    Converted,
    /// An item was written under a different name than its source.
    Renamed,
    /// The input was carried over but needs a value from the user, such as a secret.
    NeedsInput,
}

/// One entry of an import report.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ImportNote {
    /// Where in the input the note comes from: item names joined by ` / ` and then the member
    /// path, such as `Users / Get user / request.auth`, `collection / variable[0].key`, or
    /// `values[3]` in an environment.
    pub source: String,
    /// What happened.
    pub kind: NoteKind,
    /// A human-readable explanation.
    pub detail: String,
    /// The written file the note concerns, `/`-separated and relative to the collection root;
    /// `None` until the writer places it.
    pub file: Option<String>,
}

impl ImportNote {
    /// A note not yet tied to a written file.
    pub(crate) fn new(
        source: impl Into<String>,
        kind: NoteKind,
        detail: impl Into<String>,
    ) -> ImportNote {
        ImportNote {
            source: source.into(),
            kind,
            detail: detail.into(),
            file: None,
        }
    }
}

/// What an import wrote and what it could not carry over.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct ImportReport {
    /// Every note: the collection's, then each written item's depth first, then those for input
    /// that produced no file, then per environment its notes and its secrets needing a value.
    pub notes: Vec<ImportNote>,
    /// The requests and folders written.
    pub items: Vec<ItemPath>,
    /// The environments written, by name.
    pub environments: Vec<String>,
    /// Secret variables left without a value, as `(environment, variable)`.
    pub secrets_to_enter: Vec<(String, String)>,
}

/// Scripts carried by a request, folder or collection root; blank code means none.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ImportedScripts {
    /// Code run before the request is sent.
    pub pre_request: String,
    /// Code run after the response arrives.
    pub post_response: String,
}

impl ImportedScripts {
    /// Write the non-blank scripts through `set`; a blank one leaves the file's own.
    pub(crate) fn write_with(&self, mut set: impl FnMut(ScriptPhase, &str)) {
        for (phase, code) in [
            (ScriptPhase::PreRequest, &self.pre_request),
            (ScriptPhase::PostResponse, &self.post_response),
        ] {
            if !code.trim().is_empty() {
                set(phase, code);
            }
        }
    }
}

/// A request ready to be written into a request file.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ImportedRequest {
    /// The kind of request file it is written as.
    pub kind: RequestKind,
    /// The HTTP method.
    pub method: String,
    /// The URL, written verbatim.
    pub url: String,
    /// Query and path params, written verbatim and in order.
    pub params: Vec<Param>,
    /// The headers.
    pub headers: Vec<KeyValue>,
    /// The body.
    pub body: BodySpec,
    /// The authentication.
    pub auth: AuthSpec,
    /// The documentation; empty means none.
    pub docs: String,
    /// The settings, or `None` to leave the file's own.
    pub settings: Option<RequestSettings>,
    /// Default headers the request omits; empty leaves the file's own.
    pub omitted_headers: Vec<String>,
    /// Request variables; empty leaves the file's own.
    pub vars: Vec<Variable>,
    /// Tags, in order; empty leaves the file's own.
    pub tags: Vec<String>,
    /// Scripts; blank ones leave the file's own.
    pub scripts: ImportedScripts,
    /// Secret values the request refers to by variable name. They are never written to the
    /// request file; [`store_request_secrets`] keeps them in an environment and the secret store.
    pub secrets: Vec<ImportedSecret>,
    /// Saved example responses; each one's request half is taken from the request as written.
    /// Empty leaves the file's own on [`ImportedRequest::apply_to`], but an update that
    /// refreshes the examples field removes the file's `examples`.
    pub examples: Vec<crate::Example>,
}

/// A secret value an imported request refers to as `{{name}}`. Its `Debug` hides the value.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ImportedSecret {
    /// The variable the request refers to.
    pub name: String,
    /// The secret value.
    pub value: String,
    /// The `source` of the note that asks the user to supply the value.
    pub source: String,
}

impl std::fmt::Debug for ImportedSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImportedSecret")
            .field("name", &self.name)
            .field("value", &format_args!("<{} bytes>", self.value.len()))
            .field("source", &self.source)
            .finish()
    }
}

/// A request and where it goes among its siblings.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ImportedRequestItem {
    /// The request's name.
    pub name: String,
    /// Its sequence number among its siblings.
    pub seq: u64,
    /// The request itself.
    pub request: ImportedRequest,
    /// Notes about this request.
    pub notes: Vec<ImportNote>,
}

/// A folder with its own settings and children.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ImportedFolder {
    /// The folder's name.
    pub name: String,
    /// Its sequence number among its siblings.
    pub seq: u64,
    /// The documentation; empty means none.
    pub docs: String,
    /// The folder's authentication.
    pub auth: AuthSpec,
    /// Headers sent with every request in the folder.
    pub headers: Vec<KeyValue>,
    /// Folder variables.
    pub vars: Vec<Variable>,
    /// Scripts run for every request in the folder.
    pub scripts: ImportedScripts,
    /// The children, in order.
    pub items: Vec<ImportedItem>,
    /// Notes about the folder itself.
    pub notes: Vec<ImportNote>,
}

/// One node of an imported tree.
// A tree is built once per import and walked by reference, so a boxed request saves nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ImportedItem {
    /// A folder.
    Folder(ImportedFolder),
    /// A request.
    Request(ImportedRequestItem),
}

/// A whole collection ready to be written.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ImportedCollection {
    /// The collection's name.
    pub name: String,
    /// The documentation; empty means none.
    pub docs: String,
    /// The collection's authentication.
    pub auth: AuthSpec,
    /// Headers sent with every request in the collection.
    pub headers: Vec<KeyValue>,
    /// Collection variables.
    pub vars: Vec<Variable>,
    /// Scripts run for every request in the collection.
    pub scripts: ImportedScripts,
    /// The top-level items, in order.
    pub items: Vec<ImportedItem>,
    /// Notes about the collection itself, which concern `opencollection.yml`.
    pub notes: Vec<ImportNote>,
    /// Notes about input that produced no file, such as a skipped item.
    pub unwritten: Vec<ImportNote>,
    /// Whether the collection came from an OpenAPI description, so a later update can find its requests.
    pub spec_source: bool,
}

/// An environment ready to be written.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ImportedEnvironment {
    /// The environment's name.
    pub name: String,
    /// Its variables, in order.
    pub variables: Vec<EnvVariable>,
    /// Notes about the environment.
    pub notes: Vec<ImportNote>,
    /// `sources[j]` is where `variables[j]` came from in the input, such as `values[3]`; a
    /// variable without an entry falls back to `variables[j]`.
    pub(crate) sources: Vec<String>,
}

/// Why a document in one of the loader-based formats could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DocumentError {
    /// The input is not valid JSON or YAML.
    #[error("cannot be read at byte {offset}: {message}")]
    Parse {
        /// The byte offset of the error in the input as given, a byte order mark included.
        offset: usize,
        /// What the parser reported.
        message: String,
    },
    /// The input is a version or variant of the format that cannot be imported.
    #[error("unsupported {found}")]
    Unsupported {
        /// What was found, such as a version.
        found: String,
    },
    /// The input parses but is not a document of the expected shape.
    #[error("{message}")]
    Invalid {
        /// What is wrong.
        message: String,
    },
    /// The input exceeds a size limit.
    #[error("larger than {limit} {unit}")]
    TooLarge {
        /// The limit exceeded.
        limit: u64,
        /// What the limit counts: `bytes`, `values`, `events` or `imported entries`.
        unit: &'static str,
    },
}

/// Why a Postman file could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PostmanError {
    /// The input is not valid JSON.
    #[error("not valid JSON at byte {offset}: {message}")]
    Parse {
        /// The byte offset of the error.
        offset: usize,
        /// What the JSON parser reported.
        message: String,
    },
    /// The input names a Postman schema other than v2.0 or v2.1.
    #[error("unsupported Postman schema `{schema}`; only v2.0 and v2.1 can be imported")]
    UnsupportedSchema {
        /// The schema URL found.
        schema: String,
    },
    /// The input is JSON but not a Postman collection or environment.
    #[error("not a Postman file: {message}")]
    Invalid {
        /// What is wrong.
        message: String,
    },
}

/// Why an import could not be written; nothing is left at the target.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ImportError {
    /// The target directory already exists.
    #[error("{} already exists", path.display())]
    Exists {
        /// The directory the collection would be written to.
        path: PathBuf,
    },
    /// A file system call failed.
    #[error("{}: {source}", path.display())]
    Io {
        /// The path that failed.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// The collection could not be created.
    #[error(transparent)]
    Create(#[from] CreateError),
    /// A folder or request could not be created.
    #[error(transparent)]
    Tree(#[from] TreeError),
    /// An environment could not be written.
    #[error(transparent)]
    Env(#[from] EnvError),
    /// A value cannot be written to a file.
    #[error(transparent)]
    Edit(#[from] EditError),
    /// A file could not be saved.
    #[error(transparent)]
    Save(#[from] SaveError),
    /// A folder path cannot be used as an import target.
    #[error("folder path {problem}")]
    Folder {
        /// What is wrong, phrased to follow the words `folder path`.
        problem: String,
    },
    /// An update would prune, and there is no trash to move the files to.
    #[error("pruning moves files to the trash and none is available, so nothing was changed")]
    TrashUnavailable,
}
