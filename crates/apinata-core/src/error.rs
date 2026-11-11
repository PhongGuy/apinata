//! Error types returned by the public API.

use crate::document::rebase;
use crate::item_path::ItemPath;
use crate::not_found::{env_text, not_found_text};
use crate::secrets::SecretError;
use std::path::{Path, PathBuf};

/// Why a collection directory could not be opened at all.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OpenError {
    /// The directory has no `opencollection.yml`.
    #[error("{} is not a collection: no opencollection.yml", root.display())]
    NotACollection {
        /// The directory that was tried.
        root: PathBuf,
    },
    /// The directory holds a collection in the legacy `.bru` format, which must be converted.
    #[error("{} is in the legacy .bru format; convert it to open it", root.display())]
    LegacyFormat {
        /// The legacy collection's root.
        root: PathBuf,
    },
    /// `opencollection.yml` exists but cannot be used.
    #[error("{}: {message}", path.display())]
    InvalidRoot {
        /// The root file.
        path: PathBuf,
        /// Why it was rejected.
        message: String,
    },
    /// The directory could not be read.
    #[error("{}: {source}", path.display())]
    Io {
        /// The path that failed.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
}

/// A single file that could not be loaded, with the reason.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{}: {message}", path.display())]
#[non_exhaustive]
pub struct FileError {
    path: PathBuf,
    message: String,
}

impl FileError {
    pub(crate) fn new(path: &Path, message: impl Into<String>) -> Self {
        Self {
            path: path.to_path_buf(),
            message: message.into(),
        }
    }

    /// Follow a rename on disk: a path under `from` moves under `to`.
    pub(crate) fn relocate(&mut self, from: &Path, to: &Path) {
        self.path = rebase(&self.path, from, to);
    }

    /// The file that failed to load.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Why the file failed to load.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Why a request file could not be saved.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SaveError {
    /// The file changed on disk since it was loaded or last saved.
    #[error("{} changed on disk since it was loaded", path.display())]
    Conflict {
        /// The file that changed.
        path: PathBuf,
    },
    /// Reading or writing the file failed.
    #[error("{}: {source}", path.display())]
    Io {
        /// The path that failed.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
}

/// Why a request could not be resolved against its variables.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ResolveError {
    /// Prompt variables `{{?label}}` the request needs have no answer.
    #[error("no value for prompt variables: {}", labels.join(", "))]
    PromptRequired {
        /// The unanswered labels, in scan order.
        labels: Vec<String>,
    },
    /// A placeholder names a variable that has no value.
    #[error("variable `{name}` is not defined")]
    UndefinedVariable {
        /// The placeholder text exactly as written between the braces.
        name: String,
        /// Why it is undefined, filled by a caller that knows the context.
        hint: Option<UndefinedHint>,
    },
    /// Expanding a variable led back to itself.
    #[error("variable `{name}` refers to itself")]
    CyclicVariable {
        /// The placeholder that was entered a second time.
        name: String,
    },
    /// The request body uses a mode Apinata cannot send.
    #[error("body type `{kind}` is not supported")]
    UnsupportedBody {
        /// The body type as written in the request file.
        kind: String,
    },
    /// A GraphQL request's variables, once expanded, are not valid JSON.
    #[error("Failed to parse GraphQL variables: {message}")]
    GraphqlVariables {
        /// The JSON parser's description of the problem.
        message: String,
    },
    /// The request auth uses a mode Apinata cannot apply.
    #[error("auth type `{kind}` is not supported")]
    UnsupportedAuth {
        /// The auth type as written in the request file.
        kind: String,
    },
    /// The request's oauth2 auth has no cached token to send.
    #[error(
        "OAuth 2.0 token \"{credentials_id}\": no token yet; send the request or get a token first"
    )]
    OAuth2NoToken {
        /// The `tokenConfig.id` of the credentials, `credentials` when unset.
        credentials_id: String,
    },
    /// A setting the oauth2 token placement needs is empty.
    #[error("{} is empty", crate::oauth2::oauth2_field_label(field))]
    OAuth2MissingField {
        /// The file key that is empty.
        field: &'static str,
    },
    /// The OAuth 1.0 auth cannot sign; `reason` never quotes a credential.
    #[error("OAuth 1.0: {reason}")]
    OAuth1 {
        /// What is wrong: the method, the placement or the private key.
        reason: String,
    },
    /// The EdgeGrid auth cannot sign; `reason` names the field, never its value.
    #[error("EdgeGrid: {reason}")]
    EdgeGrid {
        /// What is wrong: a missing credential or a baseURL that does not parse.
        reason: String,
    },
    /// The interpolated URL cannot be parsed as a WHATWG URL.
    #[error("`{url}` is not a valid URL")]
    InvalidUrl {
        /// The URL as parsed (interpolated, `http://` prepended when it had no scheme, encoded
        /// when `encodeUrl` is on), with userinfo and query values redacted. When a sensitive
        /// value was substituted into it, the URL as templated instead.
        url: String,
    },
    /// The URL is empty or only whitespace once its variables are filled.
    #[error("the URL is empty")]
    EmptyUrl,
    /// A host override row has a bad host or address.
    #[error("invalid host override `{host}` -> `{address}`: {reason}")]
    InvalidHostOverride {
        /// The host as written in the row.
        host: String,
        /// The address as written in the row.
        address: String,
        /// What is wrong and, once resolved for a request, where the row is set.
        reason: String,
    },
    /// Two enabled host override rows name the same host.
    #[error("host `{host}` is overridden twice: `{first}` and `{second}`")]
    DuplicateHostOverride {
        /// The host, normalised.
        host: String,
        /// The first row's address as written.
        first: String,
        /// The second row's address as written.
        second: String,
    },
    /// The collection's proxy has an unsupported protocol, or a bad host or port.
    #[error("invalid proxy: {reason}")]
    InvalidProxy {
        /// What is wrong, never quoting a field's value; for the collection's proxy, also
        /// where it is set.
        reason: String,
    },
}

/// Why an undefined variable has no value, from environment names only and never a value.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum UndefinedHint {
    /// The variable is a secret of the selected environment with no stored or supplied value.
    SecretWithoutValue {
        /// The selected environment that declares it.
        environment: String,
    },
    /// The variable is a secret of the selected environment whose stored value could not be read.
    SecretUnreadable {
        /// The selected environment that declares it.
        environment: String,
        /// What the secret store reported, which never holds a secret value.
        reason: String,
    },
    /// Other environments define it while the selected one, if any, does not.
    DefinedElsewhere {
        /// Those environments, sorted.
        environments: Vec<String>,
    },
}

/// Which front end a hint is worded for, since each offers different remedies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum HintShell {
    /// The command line, where a value comes from `--env-var` and an environment from `--env`.
    #[default]
    Cli,
    /// The MCP server, whose requests take an `environment` argument and read the keychain.
    Mcp,
}

impl UndefinedHint {
    /// The hint as one sentence for the command line, naming `name`.
    pub fn cli_text(&self, name: &str) -> String {
        self.text(name, HintShell::Cli)
    }

    /// The hint as one sentence for the MCP server, naming `name`.
    pub fn mcp_text(&self, name: &str) -> String {
        self.text(name, HintShell::Mcp)
    }

    /// The hint as one sentence worded for `shell`, offering only remedies that shell has.
    pub fn text(&self, name: &str, shell: HintShell) -> String {
        match (self, shell) {
            (UndefinedHint::SecretWithoutValue { environment }, HintShell::Cli) => format!(
                "{name} is a secret in environment {environment} with no value; pass --env-var {name}=VALUE or set it in the app's environment editor"
            ),
            (UndefinedHint::SecretWithoutValue { environment }, HintShell::Mcp) => format!(
                "{name} is a secret in environment {environment} with no value; set it in the app's environment editor"
            ),
            (
                UndefinedHint::SecretUnreadable {
                    environment,
                    reason,
                },
                HintShell::Cli,
            ) => format!(
                "{name} is a secret in environment {environment}; its stored value could not be read ({reason}); pass --env-var {name}=VALUE or set it in the app's environment editor"
            ),
            (
                UndefinedHint::SecretUnreadable {
                    environment,
                    reason,
                },
                HintShell::Mcp,
            ) => format!(
                "{name} is a secret in environment {environment}; its stored value could not be read ({reason}); set it in the app's environment editor"
            ),
            (UndefinedHint::DefinedElsewhere { environments }, HintShell::Cli) => {
                match environments.as_slice() {
                    [one] => format!("{name} is defined in environment {one}; pass --env {one}"),
                    many => format!(
                        "{name} is defined in environments {}; pass --env with one of them",
                        many.join(", ")
                    ),
                }
            }
            (UndefinedHint::DefinedElsewhere { environments }, HintShell::Mcp) => {
                match environments.as_slice() {
                    [one] => {
                        format!("{name} is defined in environment {one}; select that environment")
                    }
                    many => format!(
                        "{name} is defined in environments {}; select one of them",
                        many.join(", ")
                    ),
                }
            }
        }
    }
}

impl ResolveError {
    /// Why an undefined variable has no value, when a caller filled it in.
    pub fn hint(&self) -> Option<&UndefinedHint> {
        match self {
            ResolveError::UndefinedVariable { hint, .. } => hint.as_ref(),
            _ => None,
        }
    }

    /// The hint rendered by [`UndefinedHint::cli_text`] for this error's variable.
    pub fn cli_hint(&self) -> Option<String> {
        match self {
            ResolveError::UndefinedVariable {
                name,
                hint: Some(hint),
            } => Some(hint.cli_text(name)),
            _ => None,
        }
    }

    /// The hint rendered by [`UndefinedHint::mcp_text`] for this error's variable.
    pub fn mcp_hint(&self) -> Option<String> {
        match self {
            ResolveError::UndefinedVariable {
                name,
                hint: Some(hint),
            } => Some(hint.mcp_text(name)),
            _ => None,
        }
    }

    /// Fills the hint from the variable's name when this is an undefined
    /// variable without one; any other error, or an existing hint, is kept.
    #[must_use]
    pub fn with_hint(self, f: impl FnOnce(&str) -> Option<UndefinedHint>) -> Self {
        match self {
            ResolveError::UndefinedVariable { name, hint: None } => {
                let hint = f(&name);
                ResolveError::UndefinedVariable { name, hint }
            }
            other => other,
        }
    }
}

/// Why a variable context could not be built.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ContextError {
    /// No loadable environment file has the selected name.
    #[error(
        "{}",
        not_found_text("environment", "this collection has no environments", name, available)
    )]
    EnvironmentNotFound {
        /// The environment name that was asked for.
        name: String,
        /// The environments the collection does have, by name.
        available: Vec<String>,
    },
    /// The selected environment's file exists but failed to load.
    #[error("environment `{name}` failed to load: {message}")]
    EnvironmentFailedToLoad {
        /// The environment name that was asked for.
        name: String,
        /// The load error reported for its file.
        message: String,
    },
    /// The collection's `.env` exists but cannot be read, or is not UTF-8.
    #[error("{}: {message}", path.display())]
    DotEnv {
        /// The `.env` file.
        path: PathBuf,
        /// Why it could not be read.
        message: String,
    },
    /// The secret store failed to read a secret, under [`ContextOptions::strict_secrets`](crate::ContextOptions::strict_secrets).
    #[error("secret `{name}` of environment `{environment}` could not be read: {message}")]
    SecretUnavailable {
        /// The environment that declares the secret.
        environment: String,
        /// The secret variable's name.
        name: String,
        /// The store's error.
        message: String,
    },
    /// No loadable global environment file has the selected name.
    #[error(
        "{}",
        not_found_text("global environment", "no global environments", name, available)
    )]
    GlobalEnvironmentNotFound {
        /// The global environment name that was asked for.
        name: String,
        /// The global environments that do exist, by name.
        available: Vec<String>,
    },
    /// The selected global environment's file exists but failed to load.
    #[error("global environment `{name}` failed to load: {message}")]
    GlobalEnvironmentFailedToLoad {
        /// The global environment name that was asked for.
        name: String,
        /// The load error reported for its file.
        message: String,
    },
    /// The secret store failed to read a global secret, under
    /// [`ContextOptions::strict_secrets`](crate::ContextOptions::strict_secrets).
    #[error("secret `{name}` of global environment `{environment}` could not be read: {message}")]
    GlobalSecretUnavailable {
        /// The global environment that declares the secret.
        environment: String,
        /// The secret variable's name.
        name: String,
        /// The store's error.
        message: String,
    },
}

/// Why an edit was refused; the file is left unchanged.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum EditError {
    /// A value the format cannot hold in some other field. Header names use `InvalidHeaderName`.
    #[error("`{field}` cannot be written")]
    Invalid {
        /// The field that cannot be written.
        field: String,
    },
    /// The name of header `row` has a line break, which no header can carry.
    #[error("`headers[{row}].name` cannot be written")]
    InvalidHeaderName {
        /// The zero-based index of the refused header in the list that was set.
        row: usize,
    },
}

/// Why a new collection could not be created; nothing is left on disk.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CreateError {
    /// The target already exists and is not a directory.
    #[error("{} already exists", path.display())]
    Exists {
        /// The path asked for.
        path: PathBuf,
    },
    /// The target directory exists and has entries, hidden ones such as `.git` included.
    #[error("{} is not empty", path.display())]
    NotEmpty {
        /// The directory asked for.
        path: PathBuf,
    },
    /// The name cannot be a directory name, even after sanitising.
    #[error("`{name}` is not a valid collection name")]
    InvalidName {
        /// The name asked for.
        name: String,
    },
    /// A file system call failed.
    #[error("{}: {source}", path.display())]
    Io {
        /// The path that failed.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
}

/// Why an environment operation was refused or failed before any secret was moved.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EnvError {
    /// No loaded environment has this name.
    #[error("{}", env_text(*global, name, available))]
    NotFound {
        /// The name asked for.
        name: String,
        /// The environments that do exist, by name.
        available: Vec<String>,
        /// Whether the name was looked up among the global environments.
        global: bool,
    },
    /// The target name is taken by another environment file.
    #[error("environment `{name}` already exists")]
    Exists {
        /// The name asked for.
        name: String,
    },
    /// The name cannot be a file name, even after sanitising for a create.
    #[error("`{name}` is not a valid environment name")]
    InvalidName {
        /// The name asked for.
        name: String,
    },
    /// The environment has no variable of this name.
    #[error("environment `{environment}` has no variable `{variable}`")]
    VariableNotFound {
        /// The environment.
        environment: String,
        /// The variable asked for.
        variable: String,
    },
    /// The variable is secret, and the edit would write its value to the file or drop its
    /// stored value.
    #[error("variable `{variable}` of environment `{environment}` is secret")]
    SecretVariable {
        /// The environment.
        environment: String,
        /// The variable.
        variable: String,
    },
    /// The store refused a read or write that must succeed before the file changes; nothing
    /// was changed on disk.
    #[error("secret `{variable}` of environment `{environment}`: {error}")]
    SecretStore {
        /// The environment.
        environment: String,
        /// The variable.
        variable: String,
        /// What the store reported.
        error: SecretError,
    },
    /// Other environments extend this one, so deleting it would leave them extending nothing.
    #[error("{}", extended_text(*global, name, by))]
    ExtendedBy {
        /// The environment asked for.
        name: String,
        /// The environments whose `extends` names it, by name.
        by: Vec<String>,
        /// Whether these are global environments.
        global: bool,
    },
    /// The file rename failed and writing the old name back failed too, so the file at `path`
    /// still names the new environment.
    #[error("renaming failed ({rename}) and the old name could not be restored in {}: {rollback}", path.display())]
    RenameNotRestored {
        /// The file that still holds the new name.
        path: PathBuf,
        /// Why the rename failed.
        rename: SaveError,
        /// Why the write-back failed.
        rollback: SaveError,
    },
    /// The environment's file could not be read, so what it declares is unknown.
    #[error("cannot read environment `{environment}`: {message}")]
    Unreadable {
        /// The environment.
        environment: String,
        /// Why the file gave no answer.
        message: String,
    },
    /// The secret store could not list the environment's entries.
    #[error("cannot list the stored secrets of environment `{environment}`: {error}")]
    SecretList {
        /// The environment.
        environment: String,
        /// What the store reported.
        error: SecretError,
    },
    /// The secret store could not list what is stored under a collection or global directory.
    #[error("cannot list the stored secrets under {}: {error}", root.display())]
    SecretListRoot {
        /// The path the entries are stored under.
        root: PathBuf,
        /// What the store reported.
        error: SecretError,
    },
    /// A removal named something that is not an orphan now.
    #[error("not orphaned secrets of environment `{environment}`: {}", variables.join(", "))]
    NotOrphaned {
        /// The environment.
        environment: String,
        /// The names that are not orphans, in the order asked for.
        variables: Vec<String>,
    },
    /// A call for secrets with no file named an environment that has one.
    #[error("environment `{environment}` has a file, so its stored secrets may be in use")]
    HasFile {
        /// The environment.
        environment: String,
    },
    /// Which environments have a file could not be told, because a path on the way to them
    /// leads nowhere; nothing can be called fileless then.
    #[error("cannot tell which environments have a file: `{}` cannot be reached", path.display())]
    FilesUnknown {
        /// The dangling link or the gone root.
        path: PathBuf,
    },
    /// An old collection root was given as a relative path, which names nothing once it is gone.
    #[error("the old collection root `{}` is not an absolute path", path.display())]
    OldRootRelative {
        /// The path, without trailing slashes or `.`.
        path: PathBuf,
    },
    /// Something exists at an old collection root, so it has not gone.
    #[error("`{}` exists, so it is not a root that has gone", path.display())]
    OldRootExists {
        /// The path, without trailing slashes or `.`.
        path: PathBuf,
    },
    /// Whether anything exists at an old collection root could not be told.
    #[error("cannot tell whether `{}` still exists: {message}", path.display())]
    OldRootUnknown {
        /// The path, without trailing slashes or `.`.
        path: PathBuf,
        /// What the file system reported.
        message: String,
    },
    /// Writing, renaming or removing a file failed.
    #[error(transparent)]
    Save(#[from] SaveError),
}

/// Why a tree operation was refused or failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TreeError {
    /// No such item, or one the operation does not apply to.
    #[error("no such item: `{item}`")]
    NotFound {
        /// The path asked for.
        item: ItemPath,
    },
    /// The name cannot be a file or directory name.
    #[error("`{name}` is not a valid name")]
    InvalidName {
        /// The name asked for.
        name: String,
    },
    /// The request already has an example by that name.
    #[error("{item} already has an example named `{name}`")]
    ExampleNameTaken {
        /// The request.
        item: ItemPath,
        /// The name asked for, trimmed.
        name: String,
    },
    /// The root, or a move into itself or a non-folder.
    #[error("cannot move, rename or delete `{item}` that way")]
    InvalidMove {
        /// The item the operation was refused for.
        item: ItemPath,
    },
    /// A folder pasted into itself or one of its own subfolders.
    #[error("A folder cannot be pasted into itself or one of its subfolders")]
    IntoItself {
        /// The folder that was copied.
        item: ItemPath,
    },
    /// A file changed on disk since it was loaded.
    #[error("{} changed on disk since it was loaded", path.display())]
    Conflict {
        /// The file that changed.
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
    /// A move succeeded, but a sibling's sequence number could not be rewritten afterwards.
    #[error("moved, but could not renumber the siblings: {source}")]
    Renumber {
        /// Where the item is now.
        moved: ItemPath,
        /// Why the renumbering stopped.
        source: Box<TreeError>,
    },
    /// The trash refused the item, which is still where it was.
    #[error("could not move it to the trash: {message}")]
    Trash {
        /// Why, for display.
        message: String,
    },
    /// The item to restore is no longer in the bin, so nothing was restored.
    #[error("it is no longer in the trash")]
    NotInTrash,
    /// The item is in the system trash, but this platform cannot give it back.
    #[error(
        "it is in the system trash, but cannot be restored from here; \
         put it back from the file manager"
    )]
    RestoreUnsupported,
    /// Restoring would overwrite a file that is there now, so nothing was restored.
    #[error("{} already exists, so the restored copy was left in the trash", path.display())]
    RestoreClash {
        /// The path both the file and the trashed item want.
        path: PathBuf,
    },
    /// A delete succeeded, but a sibling's sequence number could not be rewritten afterwards.
    #[error("deleted, but could not renumber the siblings: {source}")]
    DeletedRenumber {
        /// Why the renumbering stopped.
        source: Box<TreeError>,
    },
}

impl From<SaveError> for TreeError {
    fn from(error: SaveError) -> TreeError {
        match error {
            SaveError::Conflict { path } => TreeError::Conflict { path },
            SaveError::Io { path, source } => TreeError::Io { path, source },
        }
    }
}

/// The refusal to delete an environment others extend, naming each of them.
fn extended_text(global: bool, name: &str, by: &[String]) -> String {
    let kind = if global {
        "global environment"
    } else {
        "environment"
    };
    let by: Vec<String> = by.iter().map(|b| format!("`{b}`")).collect();
    format!(
        "{kind} `{name}` is extended by {}; change `extends` in each first",
        by.join(", ")
    )
}
