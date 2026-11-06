//! The `apinata` command line: argument parsing and the mapping of outcomes to exit codes.

#[macro_use]
mod out;
mod check_cmd;
mod collection_arg;
mod convert;
mod env_cmd;
mod example_cmd;
mod export;
mod gen_cmd;
mod git_cmd;
mod graphql_cmd;
mod graphql_query_cmd;
mod har_cmd;
mod hosts_cmd;
mod import;
mod item_cmd;
mod ls_cmd;
mod mcp;
mod mock_cmd;
mod network;
mod network_notice;
mod query_notice;
mod replace_cmd;
mod run;
mod search_cmd;
mod self_update;
mod send;
mod show;
mod trust;
mod watch;

use std::path::PathBuf;
use std::process::ExitCode;

use apinata_core::{HttpVersionPolicy, PacSource, StatusRule};
use clap::builder::{PossibleValuesParser, TypedValueParser as _};
use clap::error::{ContextKind, ErrorKind};
use clap::{Parser, Subcommand, ValueEnum};

/// Exit code for a usage, input or setup error, such as a collection that cannot be opened.
const EXIT_USAGE: u8 = 3;

/// `n` with `noun` in the singular for exactly one and with a regular `s` otherwise.
pub(crate) fn plural(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("{n} {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

#[derive(Parser)]
#[command(
    name = "apinata",
    version,
    about,
    after_help = "Licences of the bundled Rust crates: THIRD-PARTY-CRATES.md, shipped with each release."
)]
struct Cli {
    /// Hide notices that are not errors, such as a changed-network-settings warning.
    #[arg(short = 'q', long, global = true)]
    quiet: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Args)]
struct CompletionsArgs {
    /// The shell to generate the script for.
    #[arg(value_enum)]
    shell: clap_complete::Shell,
}

#[derive(Subcommand)]
enum Command {
    /// Run a collection, a folder in it, or one request file.
    #[command(after_help = "Exit codes:\n  \
        0  all passed\n  \
        1  an assertion or test failed\n  \
        2  a request errored (no response)\n  \
        3  usage error, collection or file that failed to load, a report file could not be written, or no request matched\n  \
        130  watch stopped with Ctrl+C before any run finished")]
    Run(Box<RunArgs>),
    /// Send one request from a URL and flags, or a curl command, without a collection.
    ///
    /// Prints the response body. Use `--env-var NAME=VALUE` for `{{variables}}`, `-i` to see the
    /// status and headers, and `--save PATH` to also keep the request as a file in a collection.
    #[command(after_help = "Exit codes:\n  \
        0  the request was sent and passed\n  \
        1  --fail-on-status judged the reply a failure\n  \
        2  the request errored (no response)\n  \
        3  usage error, an unusable URL or curl command, or a file that could not be written")]
    Send(Box<send::SendArgs>),
    /// Print a shell completion script to stdout.
    #[command(
        after_help = "Install (bash, zsh, fish; elvish and powershell source the output):\n  \
        apinata completions bash > ~/.local/share/bash-completion/completions/apinata\n  \
        apinata completions zsh > ~/.zfunc/_apinata   (~/.zfunc on fpath)\n  \
        apinata completions fish > ~/.config/fish/completions/apinata.fish\n  \
        apinata completions elvish >> ~/.config/elvish/rc.elv\n  \
        apinata completions powershell >> $PROFILE\n\n\
        Exit codes:\n  \
        0  script printed\n  \
        3  usage error (unknown shell)"
    )]
    Completions(CompletionsArgs),
    /// Serve a collection to an AI agent over MCP on stdin and stdout.
    ///
    /// The server offers tools, resources and prompts.
    #[command(after_help = "Exit codes:\n  \
        0  the client disconnected\n  \
        2  the stdio transport failed\n  \
        3  the server could not start (no collection, a bad option or setup failure)")]
    Mcp(McpArgs),
    /// Serve a collection's saved examples as a mock HTTP server on loopback.
    ///
    /// Hits are logged to stdout. If nothing reads stdout (a closed pipe), the server keeps
    /// serving without the log.
    #[command(after_help = "Exit codes:\n  \
        0  stopped by ctrl-c\n  \
        3  the server could not start (usage, collection or bind error)")]
    Mock(MockArgs),
    /// Manage the hosts a collection may send to from MCP and approve its scripts.
    #[command(after_help = "Exit codes:\n  \
        0  done\n  \
        3  any failure")]
    Trust(TrustArgs),
    /// Check for a newer release and replace this binary after confirmation.
    #[command(after_help = "Exit codes:\n  \
        0  up to date, installed, declined, or not replaceable\n  \
        2  an update is available (--check; the one use of 2 that is not a request failure)\n  \
        3  error")]
    SelfUpdate(SelfUpdateArgs),
    /// Import requests from other tools' formats into a collection.
    #[command(after_help = "Exit codes:\n  \
        0  imported (notes, if any, are on stderr)\n  \
        3  the import failed (usage, input, collection or write error)")]
    Import(import::ImportArgs),
    /// Print one request as code for another language or tool.
    #[command(after_help = "Exit codes:\n  \
        0  printed\n  \
        3  usage, collection or resolve error")]
    Gen(GenArgs),
    /// Work with a GraphQL request's server: fetch its schema by introspection.
    #[command(after_help = "Exit codes:\n  \
        0  printed or written\n  \
        1  the server answered without a schema (an error status, not JSON, GraphQL errors)\n  \
        2  the request or its token fetch errored (no response)\n  \
        3  usage, collection or resolve error, not a GraphQL request, or the file could not \
        be written")]
    Graphql(graphql_cmd::GraphqlArgs),
    /// Export a collection, and optionally its environments, to another tool's format.
    #[command(after_help = "Exit codes:\n  \
        0  exported (notes, if any, are on stderr)\n  \
        3  the export failed (usage, collection or write error)")]
    Export(export::ExportArgs),
    /// Convert a collection in the legacy .bru format to YAML.
    #[command(after_help = "Exit codes:\n  \
        0  converted, planned (--dry-run) or declined\n  \
        3  the conversion failed (usage, legacy format or write error)")]
    Convert(convert::ConvertArgs),
    /// Show what changed in a collection since the last commit.
    #[command(after_help = "Exit codes:\n  \
        0  printed, or no git or no repository (a note on stderr; commit, pull and push exit 3 instead)\n  \
        3  git failed, the file is not a request file or is outside the collection, or usage or collection error (clone: bad URL or folder name, or the folder exists)\n  \
        130  a clone, fetch, pull or push was cancelled with Ctrl+C")]
    Git(GitArgs),
    /// Search the text of a collection's requests.
    #[command(after_help = "Exit codes:\n  \
        0  searched, with or without hits\n  \
        3  usage or collection error, or the JSON could not be written")]
    Search(SearchArgs),
    /// Check a collection for undefined variables, missing files and script errors without sending.
    #[command(after_help = "Exit codes:\n  \
        0  no error findings (warnings do not count)\n  \
        1  at least one error finding\n  \
        3  usage error, collection cannot be opened, unknown --env, or the JSON could not be written")]
    Check(CheckArgs),
    /// Replace text across a collection's requests; preview first with --dry-run.
    #[command(after_help = "Exit codes:\n  \
        0  replaced or previewed, with or without matches\n  \
        3  usage or collection error, or an invalid regular expression")]
    Replace(ReplaceArgs),
    /// List a collection's folders and requests as a tree.
    #[command(after_help = "Exit codes:\n  \
        0  listed\n  \
        3  usage or collection error, the path is not a folder, or the JSON could not be written")]
    Ls(LsArgs),
    /// Manage a collection's network settings.
    #[command(after_help = "Exit codes:\n  \
        0  listed, set or removed\n  \
        3  usage or collection error, or a refused edit")]
    Network(NetworkCmdArgs),
    /// List, create, copy, rename and delete a collection's environments, and set or remove
    /// their variables.
    #[command(after_help = "Exit codes:\n  \
        0  listed, created, copied, renamed, set or removed\n  \
        3  usage or collection error, an unknown environment, a name that is invalid or taken, \
        an environment another extends, or a refused edit")]
    Env(EnvArgs),
    /// Create, copy, rename, move and remove a collection's requests and folders, and change a
    /// folder's or the collection's headers, variables, auth and docs.
    #[command(after_help = "Exit codes:\n  \
        0  created, copied, renamed, moved, removed (a taken name gets a number, as `Find 1`), \
        or a folder updated or left unchanged\n  \
        3  usage or collection error, an unknown path, an invalid name, \
        a move that is not allowed, no trash to remove into, a broken folder.yml, \
        an unknown header or variable to unset, or a refused edit")]
    Item(ItemArgs),
}

#[derive(clap::Args)]
#[command(override_usage = "apinata ls [OPTIONS] [COLLECTION] [PATH]")]
pub struct LsArgs {
    /// The collection directory, or the folder to list when it is the only argument and no
    /// `--collection` is given and it is not a collection directory (see PATH).
    #[arg(value_name = "COLLECTION")]
    pub first: Option<String>,
    /// The folder to list, relative to the collection root, each segment a directory name or a
    /// folder's display name; defaults to the root. A single argument is the folder unless it
    /// is a directory holding opencollection.yml that the working directory's collection has
    /// no folder of.
    #[arg(value_name = "PATH")]
    pub second: Option<String>,
    /// The collection directory or a directory inside it; defaults to the working directory.
    /// The same as the leading COLLECTION.
    #[arg(long)]
    pub collection: Option<PathBuf>,
    /// Show each request's tags.
    #[arg(long)]
    pub tags: bool,
    /// Print the tree as JSON.
    #[arg(long)]
    pub json: bool,
    /// Levels to list below the folder; 1 shows its direct items, each folder with its item count.
    #[arg(long, value_name = "N")]
    pub depth: Option<usize>,
    /// List at most N of the folder's direct items (1 to 500); more are cut to 500.
    #[arg(long, value_name = "N")]
    pub limit: Option<usize>,
    /// Skip the first N of the folder's direct items, to reach the next page.
    #[arg(long, value_name = "N")]
    pub offset: Option<usize>,
}

#[derive(clap::Args)]
pub struct EnvArgs {
    #[command(subcommand)]
    pub action: EnvAction,
    /// The collection directory or a directory inside it; defaults to the working directory.
    #[arg(long, global = true)]
    pub collection: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum EnvAction {
    /// List the environment names.
    Ls {
        /// Print the names as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show one environment's variables; a secret shows `<secret>`, never its value.
    Show {
        /// The environment's name.
        name: String,
        /// Print the variables as JSON.
        #[arg(long)]
        json: bool,
        /// Accepted for symmetry with `run`; never shown.
        #[arg(long = "env-var", value_name = "KEY[=VALUE]", value_parser = parse_env_var)]
        env_var: Vec<EnvVar>,
    },
    /// Create or change one variable in an environment file; the other variables stay as they are.
    ///
    /// A plain value that is already set writes nothing. With `--secret` the value is read from
    /// stdin (one line) or a hidden prompt and stored in the keychain, never in the file.
    Set {
        /// The environment's name.
        name: String,
        /// `KEY=VALUE`, or just `KEY` with `--secret`.
        assignment: String,
        /// Keep the value in the secret store, not the file.
        #[arg(long)]
        secret: bool,
        /// Do not use the platform keychain; a secret value then cannot be stored.
        #[arg(long)]
        no_keychain: bool,
    },
    /// Remove a variable from an environment file.
    Unset {
        /// The environment's name.
        name: String,
        /// The variable to remove.
        key: String,
        /// Allow removing a secret variable, and delete its stored value.
        #[arg(long)]
        secret: bool,
        /// Do not use the platform keychain.
        #[arg(long)]
        no_keychain: bool,
    },
    /// Create an empty environment; a taken name gets a suffix (`dev copy`, `dev copy 2`).
    New {
        /// The new environment's name.
        name: String,
        /// Print the name used as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Copy an environment to `<NAME> copy` (suffixed if taken), with its stored secret values.
    Cp {
        /// The environment to copy.
        name: String,
        /// Print the copy's name as JSON.
        #[arg(long)]
        json: bool,
        /// Do not use the platform keychain; the copy's secrets then have no values.
        #[arg(long)]
        no_keychain: bool,
    },
    /// Rename an environment, move its stored secret values and re-point every `extends` of it.
    Mv {
        /// The environment to rename.
        from: String,
        /// The new name; an existing environment's name is refused.
        to: String,
        /// Do not use the platform keychain; secret values stay under the old name.
        #[arg(long)]
        no_keychain: bool,
    },
    /// Delete an environment file and its stored secret values.
    ///
    /// Refused while another environment extends it.
    Rm {
        /// The environment to delete.
        name: String,
        /// Do not use the platform keychain; secret values are left in it.
        #[arg(long)]
        no_keychain: bool,
    },
    /// List stored secrets the environment's file no longer marks secret.
    ///
    /// `--all` looks across every environment and also lists stored secrets whose environment
    /// has no file, as after a deletion or a rename outside the app. `--old-roots` lists the
    /// collection roots that are gone but still hold stored secrets.
    Orphans {
        /// The environment's name.
        #[arg(required_unless_present_any = ["all", "old_roots"], conflicts_with = "all")]
        name: Option<String>,
        /// List the roots that no longer exist but still hold stored secrets, each with the
        /// command that shows them; works without a collection.
        #[arg(
            long,
            conflicts_with_all = ["name", "all", "no_file", "old_root", "global"]
        )]
        old_roots: bool,
        /// List every environment's, marking those with no file.
        #[arg(long)]
        all: bool,
        /// Take NAME as stored, for an environment that has no file.
        #[arg(long)]
        no_file: bool,
        /// A collection root that no longer exists; its stored names are taken as they are.
        ///
        /// Give the path exactly as it was, resolved, since a path that is gone cannot be
        /// canonicalised. Refused while anything exists there, and with `--collection`.
        #[arg(long, value_name = "PATH", conflicts_with = "global")]
        old_root: Option<PathBuf>,
        /// Print the names as JSON.
        #[arg(long)]
        json: bool,
        /// Use the global environments; `--collection` is ignored.
        #[arg(long)]
        global: bool,
    },
    /// Delete stored secrets the environment's file no longer marks secret.
    ///
    /// This cannot be undone, and another git branch may still use them. Asks first.
    Forget {
        /// The environment's name.
        name: String,
        /// The variables whose stored values to delete.
        #[arg(required_unless_present = "all", conflicts_with = "all")]
        variables: Vec<String>,
        /// Delete every stored secret the file no longer marks secret.
        #[arg(long)]
        all: bool,
        /// Take NAME as stored, for an environment that has no file.
        #[arg(long)]
        no_file: bool,
        /// A collection root that no longer exists; its stored names are taken as they are.
        ///
        /// Give the path exactly as it was, resolved, since a path that is gone cannot be
        /// canonicalised. Refused while anything exists there, and with `--collection`.
        #[arg(long, value_name = "PATH", conflicts_with = "global")]
        old_root: Option<PathBuf>,
        /// Use the global environments; `--collection` is ignored.
        #[arg(long)]
        global: bool,
        /// Do not ask first.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(clap::Args)]
pub struct ItemArgs {
    #[command(subcommand)]
    pub action: ItemAction,
    /// The collection directory or a directory inside it; defaults to the working directory.
    #[arg(long, global = true)]
    pub collection: Option<PathBuf>,
    /// Print the result as text or as JSON.
    #[arg(long, value_enum, global = true, default_value_t = Format::Text)]
    pub format: Format,
}

/// The item actions. Paths are relative to the collection root, such as `users/get.yml` or
/// `users`; a folder of `""` or `.` is the root.
#[derive(Subcommand)]
pub enum ItemAction {
    /// Create a request from the collection's presets; the file name follows the name.
    New {
        /// The request's display name.
        name: String,
        /// The folder to create it in; defaults to the root.
        #[arg(long = "in", value_name = "FOLDER")]
        parent: Option<String>,
        /// The request's kind; defaults to the collection's preset, else http.
        #[arg(long, value_enum)]
        kind: Option<ItemKindArg>,
        /// The HTTP method; GET for http and POST for graphql when absent.
        #[arg(long)]
        method: Option<String>,
        /// The URL; its query becomes params. Defaults to the collection's preset URL.
        #[arg(long)]
        url: Option<String>,
    },
    /// Create a folder; the directory name follows the name.
    Mkdir {
        /// The folder's display name.
        name: String,
        /// The folder to create it in; defaults to the root.
        #[arg(long = "in", value_name = "FOLDER")]
        parent: Option<String>,
    },
    /// Copy an item, or a folder with everything in it, beside itself under a new name.
    Cp {
        /// The item or folder to copy.
        path: String,
        /// The copy's display name.
        name: String,
    },
    /// Rename a request or folder, with its file or directory.
    Rename {
        /// The request or folder to rename.
        path: String,
        /// The new display name.
        name: String,
    },
    /// Move a request or folder into a folder or beside a sibling, renumbering both places.
    #[command(group(clap::ArgGroup::new("target").required(true).args(["into", "before", "after"])))]
    Mv {
        /// The request or folder to move.
        path: String,
        /// The folder to move it into; `.` or `""` for the root.
        #[arg(long, value_name = "FOLDER")]
        into: Option<String>,
        /// The sibling to place it above.
        #[arg(long, value_name = "SIBLING")]
        before: Option<String>,
        /// The sibling to place it below.
        #[arg(long, value_name = "SIBLING")]
        after: Option<String>,
    },
    /// Move a request or folder (with everything in it) to the trash and renumber its siblings.
    Rm {
        /// The request or folder to remove.
        path: String,
    },
    /// Put back the newest item `rm` (or the MCP `delete_item` tool) moved to the trash from
    /// this collection, in its place among its siblings, which end up numbered 1..n. Refused
    /// when something is at its path.
    Restore {
        /// Put back every request the newest `import --update --prune` removed, newest first,
        /// past any single delete recorded among them. A clash refuses the whole prune, changing
        /// nothing.
        #[arg(long)]
        pruned: bool,
        /// With --pruned: the prune to put back, by the id a refusal lists, wherever it is in
        /// the record.
        #[arg(long, value_name = "ID", requires = "pruned")]
        batch: Option<String>,
    },
    /// Remove the username and password from the URLs saved examples recorded. The requests'
    /// own URLs are left as they are. Lists what would change unless --write is given.
    ScrubUserinfo {
        /// Save the scrubbed examples.
        #[arg(long)]
        write: bool,
    },
    /// Change the headers, variables, auth or docs a folder (or the collection, with `.`)
    /// gives every request below it.
    ///
    /// Folder and collection values are written into the file as given. Keep a secret in an
    /// environment instead (`apinata env set --secret`, stored in the keychain) and refer to it
    /// as `{{name}}`.
    Folder(Box<FolderArgs>),
}

// `item folder`'s flags; the variant's doc comment is the command's help.
#[derive(clap::Args)]
#[command(group(clap::ArgGroup::new("auth_mode").args(["auth", "bearer", "basic", "digest", "api_key", "auth_json", "auth_json_file"])))]
pub struct FolderArgs {
    /// The folder; `.` or `""` for the collection itself.
    pub path: String,
    /// Set a header: the first one named NAME gets the value and is enabled, else it is
    /// added. The value lands in the git-tracked folder.yml (or opencollection.yml); put a
    /// secret in `apinata env set --secret` and use `{{name}}`.
    #[arg(long = "header", value_name = "NAME=VALUE")]
    pub headers: Vec<String>,
    /// Remove every header named NAME.
    #[arg(long = "unset-header", value_name = "NAME")]
    pub unset_headers: Vec<String>,
    /// Enable every header named NAME.
    #[arg(long = "enable-header", value_name = "NAME")]
    pub enable_headers: Vec<String>,
    /// Disable every header named NAME, applied after --header.
    #[arg(long = "disable-header", value_name = "NAME")]
    pub disable_headers: Vec<String>,
    /// Set a variable as --header sets a header; a typed variable keeps its type. The value
    /// lands in the git-tracked file too; a secret belongs in `apinata env set --secret`.
    #[arg(long = "var", value_name = "NAME=VALUE")]
    pub vars: Vec<String>,
    /// Remove every variable named NAME.
    #[arg(long = "unset-var", value_name = "NAME")]
    pub unset_vars: Vec<String>,
    /// Enable every variable named NAME.
    #[arg(long = "enable-var", value_name = "NAME")]
    pub enable_vars: Vec<String>,
    /// Disable every variable named NAME, applied after --var.
    #[arg(long = "disable-var", value_name = "NAME")]
    pub disable_vars: Vec<String>,
    /// Set the auth to none, or to inherit the parent folder's or collection's.
    #[arg(long, value_enum, value_name = "MODE")]
    pub auth: Option<FolderAuthArg>,
    /// Use a bearer token. Auth values land in the git-tracked file; pass `{{name}}` for
    /// a secret kept with `apinata env set --secret`.
    #[arg(long, value_name = "TOKEN")]
    pub bearer: Option<String>,
    /// Use basic auth; the password follows the first `:`.
    #[arg(long, value_name = "USER:PASS")]
    pub basic: Option<String>,
    /// Use digest auth; the password follows the first `:`.
    #[arg(long, value_name = "USER:PASS")]
    pub digest: Option<String>,
    /// Use an API key named NAME.
    #[arg(long, value_name = "NAME=VALUE")]
    pub api_key: Option<String>,
    /// Where the API key is sent; a header when absent.
    #[arg(long, value_enum, value_name = "WHERE", requires = "api_key")]
    pub api_key_in: Option<ApiKeyInArg>,
    /// Set any auth mode, OAuth 2.0 included, from JSON in the shape MCP's `read_folder`
    /// shows: `{"type": "oauth2", "flow": ..., ...}`.
    #[arg(long, value_name = "JSON")]
    pub auth_json: Option<String>,
    /// Set the auth from a JSON file as --auth-json does, `-` for standard input.
    #[arg(long, value_name = "FILE")]
    pub auth_json_file: Option<PathBuf>,
    /// Set the Markdown docs; blank text removes them.
    #[arg(long, conflicts_with = "docs_file")]
    pub docs: Option<String>,
    /// Set the docs from a file, `-` for standard input.
    #[arg(long, value_name = "FILE")]
    pub docs_file: Option<PathBuf>,
}

/// The auth modes `item folder --auth` sets without credentials.
#[derive(Clone, Copy, ValueEnum)]
pub enum FolderAuthArg {
    None,
    Inherit,
}

/// Where `item folder --api-key` sends the key.
#[derive(Clone, Copy, ValueEnum)]
pub enum ApiKeyInArg {
    Header,
    Query,
}

/// The kind of request `item new` makes.
#[derive(Clone, Copy, ValueEnum)]
pub enum ItemKindArg {
    Http,
    Graphql,
}

#[derive(clap::Args)]
#[command(override_usage = "apinata replace [OPTIONS] [COLLECTION] <PATTERN> <REPLACEMENT>")]
pub struct ReplaceArgs {
    /// Optionally the collection directory or a directory inside it, then the text to find
    /// (case-insensitively unless --case is given; secret variable values are never matched)
    /// and the text to put in its place (with --regex, `$1`, `${1}`, `$&` and `$$` expand).
    #[arg(value_name = "COLLECTION")]
    pub first: String,
    /// The pattern with a leading collection, the replacement without one.
    #[arg(value_name = "PATTERN")]
    pub second: String,
    /// The replacement when a collection was given first.
    #[arg(value_name = "REPLACEMENT")]
    pub third: Option<String>,
    /// The collection directory or a directory inside it; defaults to the working directory.
    /// The same as the leading COLLECTION.
    #[arg(long)]
    pub collection: Option<PathBuf>,
    /// Treat the pattern as an ECMAScript regular expression.
    #[arg(long)]
    pub regex: bool,
    /// Match letter case exactly.
    #[arg(long)]
    pub case: bool,
    /// List the hits and write nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// Print the hits (with --dry-run) or the report as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args)]
#[command(override_usage = "apinata search [OPTIONS] [COLLECTION] <QUERY>")]
pub struct SearchArgs {
    /// Optionally the collection directory or a directory inside it, then the text to find,
    /// case-insensitively (secret variable values are never searched).
    #[arg(num_args = 1..=2, value_names = ["COLLECTION", "QUERY"], required = true)]
    pub words: Vec<String>,
    /// The collection directory or a directory inside it; defaults to the working directory.
    /// The same as the leading COLLECTION.
    #[arg(long)]
    pub collection: Option<PathBuf>,
    /// Print the hits as JSON.
    #[arg(long)]
    pub json: bool,
    /// Show at most N hits (1 to 500) and report the total, so a search can read past the cap.
    #[arg(long, value_name = "N")]
    pub limit: Option<usize>,
    /// Skip the first N hits, to reach the next page.
    #[arg(long, value_name = "N")]
    pub offset: Option<usize>,
}

#[derive(clap::Args)]
pub struct CheckArgs {
    /// The collection directory or a directory inside it; defaults to the working directory.
    #[arg(value_name = "COLLECTION")]
    pub path: Option<PathBuf>,
    /// The same as the positional COLLECTION.
    #[arg(long)]
    pub collection: Option<PathBuf>,
    /// Check only this environment; by default every environment is checked.
    #[arg(long, value_name = "NAME")]
    pub env: Option<String>,
    /// Define a variable for the check, like `run --env-var` (`KEY=VALUE` or `KEY` from the environment).
    #[arg(long = "env-var", value_name = "KEY[=VALUE]", value_parser = parse_env_var)]
    pub env_var: Vec<EnvVar>,
    /// Define a global environment variable for the check.
    #[arg(long = "global-env-var", value_name = "KEY[=VALUE]", value_parser = parse_env_var)]
    pub global_env_var: Vec<EnvVar>,
    /// Do not read secret values from the platform keychain; a secret without --env-var then warns.
    #[arg(long)]
    pub no_keychain: bool,
    /// Print the report as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args)]
pub struct GitArgs {
    #[command(subcommand)]
    pub action: GitAction,
}

#[derive(Subcommand)]
pub enum GitAction {
    /// List the changed files of the collection, then the branch.
    Status {
        /// The collection directory or a directory inside it; defaults to the working directory.
        #[arg(value_name = "COLLECTION")]
        path: Option<PathBuf>,
        /// The same as the positional COLLECTION.
        #[arg(long)]
        collection: Option<PathBuf>,
        /// Print the state as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show the changed sections and a unified diff of one file against the last commit.
    Diff {
        /// The request file, relative to the working directory.
        file: PathBuf,
        /// Print the diff as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Clone a repository into a new folder, showing progress; Ctrl+C cancels and cleans up.
    Clone {
        /// The repository URL (https, http, ssh, file, scp form or a local path).
        url: String,
        /// The folder to create; defaults to the repository name in the working directory. The folder
        /// must not exist yet.
        dir: Option<PathBuf>,
    },
    /// List the remote branches of the collection's repository.
    Branches {
        /// The collection directory or a directory inside it; defaults to the working directory.
        #[arg(value_name = "COLLECTION")]
        path: Option<PathBuf>,
        /// The same as the positional COLLECTION.
        #[arg(long)]
        collection: Option<PathBuf>,
        /// Fetch every remote first, so branches pushed since the last fetch show up.
        #[arg(long)]
        fetch: bool,
        /// Print the branches as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Check out a remote branch, as `<remote>/<name>` or a unique `<name>`.
    Switch {
        /// The branch to switch to.
        branch: String,
        /// The collection directory or a directory inside it; defaults to the working directory.
        #[arg(long)]
        collection: Option<PathBuf>,
    },
    /// Fetch the upstream, then fast-forward the current branch; never merges or rebases.
    Pull {
        /// The collection directory or a directory inside it; defaults to the working directory.
        #[arg(long)]
        collection: Option<PathBuf>,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Push the current branch to its upstream; never forces.
    Push {
        /// The collection directory or a directory inside it; defaults to the working directory.
        #[arg(long)]
        collection: Option<PathBuf>,
        /// Publish the branch under its own name on REMOTE and track it.
        #[arg(long, value_name = "REMOTE")]
        set_upstream: Option<String>,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Commit collection changes locally, running the repository's hooks; nothing is pushed.
    Commit {
        /// The commit message; its first line is the subject.
        #[arg(short, long)]
        message: String,
        /// Stage the whole collection first.
        #[arg(long, conflicts_with = "paths")]
        all: bool,
        /// Files or folders to stage first, relative to the working directory. With neither these
        /// nor --all, what is already staged is committed.
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,
        /// The collection directory or a directory inside it; defaults to the working directory.
        #[arg(long)]
        collection: Option<PathBuf>,
        /// Print the new commit as JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(clap::Args)]
pub struct GenArgs {
    /// The request file.
    pub path: PathBuf,
    /// What to write: curl, httpie, python-requests, js-fetch, node-axios, go, powershell,
    /// php-curl, rust-reqwest, java-httpclient or csharp-httpclient.
    #[arg(long)]
    pub target: apinata_core::Target,
    /// The environment to use, by file name without `.yml`.
    #[arg(long)]
    pub env: Option<String>,
    /// Set or override an environment variable; repeatable. `KEY=VALUE`, or `KEY` to read the value
    /// from the process environment (empty if set but empty; kept secret).
    #[arg(long = "env-var", value_name = "KEY[=VALUE]", value_parser = parse_env_var)]
    pub env_var: Vec<EnvVar>,
}

#[derive(clap::Args)]
pub struct SelfUpdateArgs {
    /// Only report whether an update is available.
    #[arg(long)]
    pub check: bool,
    /// Install without asking.
    #[arg(long, conflicts_with = "check")]
    pub yes: bool,
}

#[derive(clap::Args)]
pub struct TrustArgs {
    #[command(subcommand)]
    pub action: TrustAction,
    /// The collection directory or a directory inside it; defaults to the working directory.
    #[arg(long, global = true)]
    pub collection: Option<PathBuf>,
    /// The trust file to edit instead of the per-user one; a test seam.
    #[arg(long, global = true, hide = true)]
    pub trust_file: Option<PathBuf>,
    /// The script trust file to edit instead of the per-user one; a test seam.
    #[arg(long, global = true, hide = true)]
    pub script_trust_file: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum TrustAction {
    /// Trust a host for the collection, taking it off the denied list.
    Add { host: String },
    /// Deny a host for the collection, taking it off the trusted list.
    Deny { host: String },
    /// Take a host off both of the collection's lists.
    Remove { host: String },
    /// Print the collection's trusted hosts, then its denied hosts.
    List,
    /// Print the collection's unapproved scripts and approve them after confirmation.
    Scripts {
        /// Approve without asking.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(clap::Args)]
pub struct NetworkCmdArgs {
    #[command(subcommand)]
    pub action: NetworkAction,
}

#[derive(Subcommand)]
pub enum NetworkAction {
    /// Send chosen host names to chosen addresses instead of their DNS answer.
    Hosts(HostsArgs),
}

#[derive(clap::Args)]
pub struct HostsArgs {
    #[command(subcommand)]
    pub action: HostsAction,
    /// The collection directory or a directory inside it; defaults to the working directory.
    #[arg(long, global = true)]
    pub collection: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum HostsAction {
    /// Print each host override as `host -> address`.
    List,
    /// Map a host name to one or more comma separated addresses (`ip`, `ip:port`, `[v6]:port`),
    /// replacing the host's row. A port counts only for a URL that names none.
    Set {
        host: String,
        address: String,
        /// Keep the row in the file but do not use it.
        #[arg(long)]
        disabled: bool,
    },
    /// Remove a host's row.
    Remove { host: String },
}

#[derive(clap::Args)]
pub struct McpArgs {
    /// The collection directory or a directory inside it; defaults to the working directory.
    #[arg(value_name = "COLLECTION")]
    pub path: Option<PathBuf>,
    /// The same as the positional COLLECTION.
    #[arg(long)]
    pub collection: Option<PathBuf>,
    /// The default environment, by file name without `.yml`.
    #[arg(long)]
    pub env: Option<String>,
    /// Set or override an environment variable; repeatable. `KEY=VALUE`, or `KEY` to read the value
    /// from the process environment (empty if set but empty; kept secret).
    #[arg(long = "env-var", value_name = "KEY[=VALUE]", value_parser = parse_env_var)]
    pub env_var: Vec<EnvVar>,
    /// The global environment to use, by file name without `.yml`.
    #[arg(long, value_name = "NAME")]
    pub global_env: Option<String>,
    /// Set or override a global environment variable; repeatable. `KEY=VALUE`, or `KEY` to read the
    /// value from the process environment (empty if set but empty; kept secret).
    #[arg(long = "global-env-var", value_name = "KEY[=VALUE]", value_parser = parse_env_var)]
    pub global_env_var: Vec<EnvVar>,
    /// A workspace folder (holding workspace.yml) whose environments `--global-env` can also
    /// name: `<name> (workspace)` picks the workspace one, a plain name the profile's first.
    /// Ignored without `--global-env`.
    #[arg(long, value_name = "DIR")]
    pub workspace: Option<PathBuf>,
    /// Accept invalid TLS certificates.
    #[arg(long)]
    pub insecure: bool,
    #[command(flatten)]
    pub network: NetworkArgs,
}

#[derive(clap::Args)]
pub struct MockArgs {
    /// The collection directory or a directory inside it.
    #[arg(value_name = "PATH")]
    pub path: Option<PathBuf>,
    /// The same as the positional PATH.
    #[arg(long)]
    pub collection: Option<PathBuf>,
    /// The environment to use, by file name without `.yml`.
    #[arg(long)]
    pub env: Option<String>,
    /// Set or override an environment variable; repeatable. `KEY=VALUE`, or `KEY` to read the value
    /// from the process environment (empty if set but empty; kept secret).
    #[arg(long = "env-var", value_name = "KEY[=VALUE]", value_parser = parse_env_var)]
    pub env_var: Vec<EnvVar>,
    /// The global environment to use, by file name without `.yml`.
    #[arg(long, value_name = "NAME")]
    pub global_env: Option<String>,
    /// Set or override a global environment variable; repeatable. `KEY=VALUE`, or `KEY` to read the
    /// value from the process environment (empty if set but empty; kept secret).
    #[arg(long = "global-env-var", value_name = "KEY[=VALUE]", value_parser = parse_env_var)]
    pub global_env_var: Vec<EnvVar>,
    /// A workspace folder (holding workspace.yml) whose environments `--global-env` can also
    /// name: `<name> (workspace)` picks the workspace one, a plain name the profile's first.
    /// Ignored without `--global-env`.
    #[arg(long, value_name = "DIR")]
    pub workspace: Option<PathBuf>,
    /// The port to listen on; 0 picks a free one.
    #[arg(long, default_value_t = 4010)]
    pub port: u16,
    /// The address to listen on.
    #[arg(long, default_value = "127.0.0.1", value_parser = clap::value_parser!(std::net::IpAddr))]
    pub bind: std::net::IpAddr,
    /// Allow --bind to name an address that is not loopback, exposing the examples to the network.
    #[arg(long)]
    pub allow_non_loopback: bool,
    /// Wait this many milliseconds before every reply.
    #[arg(long, default_value_t = 0, value_name = "MS")]
    pub delay: u64,
}

#[derive(clap::Args)]
pub struct NetworkArgs {
    /// Trust the CA certificates in this PEM file, in addition to the system's.
    #[arg(long, value_name = "FILE")]
    pub cacert: Option<PathBuf>,
    /// With --cacert, trust only its certificates.
    #[arg(long, requires = "cacert")]
    pub ignore_truststore: bool,
    /// Add client certificates from this JSON file (`{"enabled": true, "certs": [...]}`).
    #[arg(long, value_name = "FILE")]
    pub client_cert_config: Option<PathBuf>,
    /// Ignore every proxy: the collection's and the system's.
    #[arg(long)]
    pub noproxy: bool,
    /// Route requests that inherit the global proxy through this proxy auto-config file (URL or path).
    #[arg(long, value_name = "URL|PATH", value_parser = network::parse_pac_source, conflicts_with = "noproxy")]
    pub proxy_pac: Option<PacSource>,
    /// The HTTP version: 1.1 (HTTP/1.1 only, the default), auto (HTTP/2 over HTTPS when the server offers it) or 2-prior-knowledge (HTTP/2 without negotiation, also over plain http).
    #[arg(
        long,
        value_name = "VERSION",
        default_value = "1.1",
        value_parser = PossibleValuesParser::new(["1.1", "auto", "2-prior-knowledge"])
            .try_map(|s| s.parse::<HttpVersionPolicy>())
    )]
    pub http_version: HttpVersionPolicy,
}

#[derive(clap::Args)]
pub struct RunArgs {
    /// The collection directory, a folder inside it, or a request file.
    #[arg(value_name = "PATH")]
    pub path: Option<PathBuf>,
    /// The collection directory or a directory inside it; the same as PATH.
    #[arg(long)]
    pub collection: Option<PathBuf>,
    /// The environment to use, by file name without `.yml`.
    #[arg(long)]
    pub env: Option<String>,
    /// Set or override an environment variable; repeatable. `KEY=VALUE`, or `KEY` to read the value
    /// from the process environment (empty if set but empty; kept secret).
    #[arg(long = "env-var", value_name = "KEY[=VALUE]", value_parser = parse_env_var)]
    pub env_var: Vec<EnvVar>,
    /// Do not read secret values from the platform keychain; a secret then needs --env-var.
    #[arg(long)]
    pub no_keychain: bool,
    /// The global environment to use, by file name without `.yml`.
    #[arg(long, value_name = "NAME")]
    pub global_env: Option<String>,
    /// Set or override a global environment variable; repeatable. `KEY=VALUE`, or `KEY` to read the
    /// value from the process environment (empty if set but empty; kept secret).
    #[arg(long = "global-env-var", value_name = "KEY[=VALUE]", value_parser = parse_env_var)]
    pub global_env_var: Vec<EnvVar>,
    /// A workspace folder (holding workspace.yml) whose environments `--global-env` can also
    /// name: `<name> (workspace)` picks the workspace one, a plain name the profile's first.
    /// Ignored without `--global-env`.
    #[arg(long, value_name = "DIR")]
    pub workspace: Option<PathBuf>,
    /// The report format written to stdout. The JSON summary also counts `unverified` (passed
    /// with every check skipped) and `no_tests` (passed with no check), both subsets of `passed`.
    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,
    /// Also write the JSON report to this file. `-` writes it to stdout instead and moves the console report to stderr.
    #[arg(long, value_name = "PATH")]
    pub reporter_json: Option<PathBuf>,
    /// Also write a JUnit XML report to this file. `-` writes it to stdout instead and moves the console report to stderr.
    #[arg(long, value_name = "PATH")]
    pub reporter_junit: Option<PathBuf>,
    /// Also write a self-contained HTML report to this file. `-` writes it to stdout instead and moves the console report to stderr.
    #[arg(long, value_name = "PATH")]
    pub reporter_html: Option<PathBuf>,
    /// Stop after the first failed or errored request.
    #[arg(long)]
    pub bail: bool,
    /// Accept invalid TLS certificates.
    #[arg(long)]
    pub insecure: bool,
    /// Do not store or send cookies between requests in this run.
    #[arg(long)]
    pub disable_cookies: bool,
    /// Keep cookies between runs in this JSON file (plain text: keep it out of git). Cannot be combined with --disable-cookies.
    #[arg(long, value_name = "FILE", conflicts_with = "disable_cookies")]
    pub cookie_jar: Option<PathBuf>,
    /// Do not run collection, folder or request scripts; their tests are reported as skipped.
    #[arg(long)]
    pub no_scripts: bool,
    /// The most whole seconds one script phase may take.
    #[arg(long, value_name = "SECS", default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=86_400))]
    pub script_timeout: u64,
    /// Cap every request's timeout at this many whole seconds. The lower of the request's own timeout and this value wins, so a request with a shorter timeout of its own keeps it.
    #[arg(long, value_name = "SECS", value_parser = clap::value_parser!(u64).range(1..=86_400))]
    pub timeout: Option<u64>,
    /// End an event-stream after this many seconds; 0 means no limit.
    #[arg(long, value_name = "SECS", default_value_t = 30)]
    pub stream_max_time: u64,
    /// End an event-stream after about this many events; 0 means no limit.
    #[arg(long, value_name = "N", default_value_t = 10_000)]
    pub stream_max_events: u64,
    /// Cut a response body after this many bytes and say so on stderr; 0 means no limit.
    #[arg(long, value_name = "BYTES", default_value_t = 10 * 1024 * 1024)]
    pub max_body_bytes: u64,
    /// Fetch new OAuth 2.0 tokens instead of using cached ones.
    #[arg(long)]
    pub refetch_tokens: bool,
    /// Pause this many milliseconds between requests.
    #[arg(long, value_name = "MS", default_value_t = 0)]
    pub delay: u64,
    /// Send a request again, up to N more times, when the send fails in transit (timeout, refused connection, reset) or the answer is 502, 503 or 504. A failed assertion, test or script never retries, and scripts, variables and assertions run once, on the last attempt. Only GET, HEAD, OPTIONS, PUT, DELETE and TRACE retry unless --retry-unsafe is given. A run-wide --timeout is not retried.
    #[arg(long, value_name = "N", default_value_t = 0)]
    pub retries: u32,
    /// Pause this many milliseconds before each retry. A Retry-After header on a 502, 503 or 504 (whole seconds or an HTTP date) lengthens the pause when it asks for more; no pause passes 60 seconds.
    #[arg(long, value_name = "MS", default_value_t = 0)]
    pub retry_delay: u64,
    /// Also retry POST and PATCH, which a repeat could apply twice.
    #[arg(long)]
    pub retry_unsafe: bool,
    /// Fail a request whose final reply status is in range, even with no assertions, so the exit code is 1 and the summary names the status. The value is error (400 and above, the default when the flag stands alone), 4xx or 5xx, written with an equals sign: --fail-on-status=4xx. A status that a retry replaced is not judged.
    #[arg(
        long,
        value_name = "RANGE",
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "error"
    )]
    pub fail_on_status: Option<StatusRule>,
    /// Run only requests with one of these comma-separated tags.
    #[arg(long, value_name = "TAGS")]
    pub tags: Option<String>,
    /// Skip requests with one of these comma-separated tags.
    #[arg(long, value_name = "TAGS")]
    pub exclude_tags: Option<String>,
    /// Run the collection once per row of this CSV file.
    #[arg(long, value_name = "PATH", conflicts_with = "json_file_path")]
    pub csv_file_path: Option<PathBuf>,
    /// Run the collection once per object of this JSON array file.
    #[arg(long, value_name = "PATH")]
    pub json_file_path: Option<PathBuf>,
    /// Run the collection this many times; with a data file, at most this many rows.
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..))]
    pub iterations: Option<u32>,
    /// Print each network exchange to stderr.
    #[arg(long)]
    pub verbose: bool,
    /// Print each response (status line, headers, blank line, body) to stdout and the verdicts
    /// to stderr. JSON is indented on a terminal and left raw when piped. With --format json
    /// the report gains a `response` object per request instead and stdout stays the report.
    #[arg(short = 'i', long, conflicts_with = "body")]
    pub show_response: bool,
    /// Print only each response body to stdout, verbatim, and the verdicts to stderr; several
    /// requests print their bodies one after another. Cannot be combined with --format json.
    #[arg(long)]
    pub body: bool,
    /// Narrow each printed body (--body or -i) to what a JSONPath (JSON) or XPath (XML) matches,
    /// chosen by the response's content type. A path that can match only one value prints it;
    /// others print an array (JSON) or one node per line (XML); no match prints `[]` or nothing.
    /// Runs after secrets are redacted; -o files stay whole. Cannot be combined with --format json.
    #[arg(long, value_name = "EXPR")]
    pub filter: Option<String>,
    /// Write the response body bytes exactly as received (secrets are not scrubbed) to this
    /// file, the last request's if several run. An existing directory, or a path ending in `/`,
    /// gets one `NNN-<request>.body` per request instead. The file is complete even past
    /// --max-body-bytes (stdout stays capped); a failed transfer leaves no file.
    #[arg(short = 'o', long, value_name = "PATH")]
    pub output: Option<PathBuf>,
    /// Run once, then rerun with the same options whenever a collection file changes, until
    /// Ctrl+C. A run in flight always finishes, and edits made meanwhile cause one rerun.
    /// Ctrl+C exits with the last finished run's code (130 if none finished). Report files
    /// (--reporter-*, -o) are overwritten on every run. Cannot be combined with --format json.
    #[arg(long)]
    pub watch: bool,
    /// Save the response as an example named NAME in the request file, after the send. PATH
    /// must select exactly one request; with iterations, the last one's response is saved.
    /// Secret values the request knows become placeholders, cookies and credential headers are
    /// masked and a binary body holding a secret is left out.
    #[arg(long, value_name = "NAME", conflicts_with = "watch")]
    pub save_example: Option<String>,
    /// Also write every exchange the run sent to this file as one HAR 1.2 log, each request as
    /// it was sent with secret values, credential headers and derived credentials (OAuth 2.0
    /// tokens, signatures) replaced by `[redacted]`. A send whose binary body holds a secret is
    /// left out and named on stderr. With --watch the file is rewritten on every run.
    #[arg(long, value_name = "FILE")]
    pub har: Option<PathBuf>,
    /// With --save-example, keep the response exactly as received, secret values included.
    #[arg(long, requires = "save_example")]
    pub keep_secrets: bool,
    /// The collection root and request the example goes to, when not the one that runs.
    #[arg(skip)]
    pub example_target: Option<(PathBuf, apinata_core::ItemPath)>,
    #[command(flatten)]
    pub network: NetworkArgs,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Format {
    Text,
    Json,
}

/// One `--env-var` or `--global-env-var` argument. `from_env` marks a value read from the
/// process environment, which counts as a secret wherever it is used.
#[derive(Clone)]
pub struct EnvVar {
    pub key: String,
    pub value: String,
    pub from_env: bool,
}

/// Debug output names the key only, since the value may be a secret.
impl std::fmt::Debug for EnvVar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvVar")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

/// Split `KEY=VALUE` at the first `=`, as the reference CLI's `^([^=]+)=(.*)$` does; a bare `KEY`
/// (an Apinata extension) takes its value from the process environment.
fn parse_env_var(s: &str) -> Result<EnvVar, String> {
    parse_env_var_with(s, |k| std::env::var_os(k))
}

/// [`parse_env_var`] with the environment lookup injected. Errors name the variable, never a value.
fn parse_env_var_with(
    s: &str,
    lookup: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Result<EnvVar, String> {
    if let Some((key, value)) = s.split_once('=') {
        if key.is_empty() {
            return Err("expected KEY=VALUE or KEY".to_owned());
        }
        return Ok(EnvVar {
            key: key.to_owned(),
            value: value.to_owned(),
            from_env: false,
        });
    }
    if s.is_empty() {
        return Err("expected KEY=VALUE or KEY".to_owned());
    }
    match lookup(s) {
        None => Err(format!("environment variable {s} is not set")),
        Some(v) => v.into_string().map_or_else(
            |_| Err(format!("environment variable {s} is not valid UTF-8")),
            |value| {
                Ok(EnvVar {
                    key: s.to_owned(),
                    value,
                    from_env: true,
                })
            },
        ),
    }
}

fn main() -> ExitCode {
    // clap exits 2 on a usage error, which here means "a request was not sent", so map it to 3.
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) if e.kind() == ErrorKind::ValueValidation => {
            // clap's own message repeats the rejected value, which may be a secret, so name only
            // the argument and the reason.
            let arg = e
                .get(ContextKind::InvalidArg)
                .map_or_else(String::new, |a| a.to_string());
            let reason = std::error::Error::source(&e).map_or_else(String::new, |r| r.to_string());
            eprintln!("error: invalid value for '{arg}': {reason}");
            return ExitCode::from(EXIT_USAGE);
        }
        Err(e) => {
            let _ = e.print();
            return ExitCode::from(if e.use_stderr() { EXIT_USAGE } else { 0 });
        }
    };
    network_notice::set_quiet(cli.quiet);
    query_notice::set_quiet(cli.quiet);
    let mut args = match cli.command {
        Command::Run(args) => args,
        Command::Completions(args) => {
            let mut command = <Cli as clap::CommandFactory>::command();
            // Generated into memory first: clap_complete panics on a failed write.
            let mut script = Vec::new();
            clap_complete::generate(args.shell, &mut command, "apinata", &mut script);
            out::write_bytes(&script);
            return ExitCode::SUCCESS;
        }
        Command::Send(args) => return send::run(*args),
        Command::Mcp(args) => return mcp::serve(&args),
        Command::Mock(args) => return mock_cmd::run(&args),
        Command::Trust(args) => return trust::run(&args),
        Command::SelfUpdate(args) => return self_update::run(&args),
        Command::Import(args) => return import::run(&args),
        Command::Gen(args) => return gen_cmd::run(&args),
        Command::Graphql(args) => return graphql_cmd::run(&args),
        Command::Export(args) => return export::run(&args),
        Command::Convert(args) => return convert::run(&args),
        Command::Git(args) => return git_cmd::run(&args),
        Command::Search(args) => return search_cmd::run(&args),
        Command::Check(args) => return check_cmd::run(&args),
        Command::Replace(args) => return replace_cmd::run(&args),
        Command::Ls(args) => return ls_cmd::run(&args),
        Command::Env(args) => return env_cmd::run(&args),
        Command::Item(args) => return item_cmd::run(&args),
        Command::Network(args) => return hosts_cmd::run(&args),
    };
    // Fold `--collection` into the positional so the rest of the run sees one path.
    match collection_arg::require(
        args.path.as_deref(),
        args.collection.as_deref(),
        "`apinata run <PATH>` or `apinata run --collection <DIR>`",
    ) {
        Ok(path) => args.path = Some(path),
        Err(e) => {
            eprintln!("error: {e:#}");
            return ExitCode::from(EXIT_USAGE);
        }
    }
    if let Some(code) = validate(&args) {
        return ExitCode::from(code);
    }
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    if args.watch {
        return ExitCode::from(runtime.block_on(watch::run(&args)));
    }
    ExitCode::from(runtime.block_on(run_once(&args)))
}

impl RunArgs {
    /// The path to run, once `main` has folded `--collection` into it.
    pub fn target(&self) -> &std::path::Path {
        self.path.as_deref().expect("main resolves the run path")
    }
}

/// The usage errors that cost no request: the exit code to stop with, or None to go ahead.
fn validate(args: &RunArgs) -> Option<u8> {
    // Check every report directory now so a typo costs no requests.
    for path in [
        &args.reporter_json,
        &args.reporter_junit,
        &args.reporter_html,
    ]
    .into_iter()
    .flatten()
    .filter(|p| !is_stdout(p))
    {
        // A bare file name has an empty parent, which means the current directory.
        let dir = path.parent().filter(|d| !d.as_os_str().is_empty());
        if let Some(dir) = dir
            && !dir.is_dir()
        {
            eprintln!("error: output directory {} does not exist", dir.display());
            return Some(EXIT_USAGE);
        }
    }
    let claimed = stdout_claims(args);
    if claimed > 1 {
        let flags: Vec<&str> = [
            ("--reporter-json", &args.reporter_json),
            ("--reporter-junit", &args.reporter_junit),
            ("--reporter-html", &args.reporter_html),
        ]
        .into_iter()
        .filter(|(_, p)| p.as_deref().is_some_and(is_stdout))
        .map(|(f, _)| f)
        .collect();
        eprintln!(
            "error: only one reporter can write to stdout with `-`, but {} all do",
            flags.join(" and ")
        );
        return Some(EXIT_USAGE);
    }
    if claimed == 1 {
        let clash = if matches!(args.format, Format::Json) {
            Some("--format json")
        } else if args.body {
            Some("--body")
        } else if args.show_response {
            Some("--show-response")
        } else if args.watch {
            Some("--watch")
        } else {
            None
        };
        if let Some(flag) = clash {
            eprintln!(
                "error: a reporter writing to stdout with `-` cannot be combined with {flag}"
            );
            return Some(EXIT_USAGE);
        }
    }
    if args.body && matches!(args.format, Format::Json) {
        eprintln!("error: --body cannot be combined with --format json");
        return Some(EXIT_USAGE);
    }
    if let Some(expression) = &args.filter {
        // The filter reads printed bodies, so it needs a mode that prints them, and a bad
        // expression costs no request.
        if matches!(args.format, Format::Json) {
            eprintln!("error: --filter cannot be combined with --format json");
            return Some(EXIT_USAGE);
        }
        if !args.body && !args.show_response {
            eprintln!("error: --filter needs --body or -i, which print the response body");
            return Some(EXIT_USAGE);
        }
        if let Err(e) = apinata_core::validate_filter(expression) {
            match e {
                apinata_core::BodyFilterError::InvalidExpression { expression, reason } => {
                    eprintln!("error: invalid --filter '{expression}': {reason}");
                }
                other => eprintln!("error: invalid --filter '{expression}': {other}"),
            }
            return Some(EXIT_USAGE);
        }
    }
    // Concatenated JSON documents would not parse as one.
    if args.watch && matches!(args.format, Format::Json) {
        eprintln!("error: --watch cannot be combined with --format json");
        return Some(EXIT_USAGE);
    }
    None
}

/// A reporter path of `-` means stdout.
fn is_stdout(path: &std::path::Path) -> bool {
    path.as_os_str() == "-"
}

/// How many reporters ask for stdout.
fn stdout_claims(args: &RunArgs) -> usize {
    [
        &args.reporter_json,
        &args.reporter_junit,
        &args.reporter_html,
    ]
    .into_iter()
    .flatten()
    .filter(|p| is_stdout(p))
    .count()
}

/// Write one report to its file, or to stdout for `-` through the shared stdout rules (a
/// closed pipe exits 0, any other failure exits 3); false after printing a file error.
fn write_report(path: &std::path::Path, body: &str) -> bool {
    if is_stdout(path) {
        out::write_bytes(body.as_bytes());
        return true;
    }
    match std::fs::write(path, body) {
        Ok(()) => true,
        Err(e) => {
            eprintln!("error: cannot write report {}: {e}", path.display());
            false
        }
    }
}

/// One complete run: execute, print the report, write the report files; returns the exit code.
async fn run_once(args: &RunArgs) -> u8 {
    // The JSON report carries the response itself, so stdout gets nothing else in that case.
    let mode = match (args.show_response, args.body, args.format) {
        (_, true, _) => show::Mode::Body,
        (true, _, Format::Text) => show::Mode::Head,
        _ => show::Mode::Off,
    };
    let tty = std::io::IsTerminal::is_terminal(&std::io::stdout());
    let show = match show::Show::new(mode, tty, args.filter.clone(), args.output.as_deref()) {
        Ok(show) => show,
        Err(e) => {
            eprintln!("error: {e}");
            return EXIT_USAGE;
        }
    };
    let save_failed = show.failure_flag();
    let jar_failed = std::sync::atomic::AtomicBool::new(false);
    match run::run(args, show, &jar_failed).await {
        Ok(report) => {
            let json = report.to_json();
            match args.format {
                // With the response on stdout the verdicts move aside, so stdout stays pipeable.
                Format::Text if mode != show::Mode::Off || stdout_claims(args) > 0 => {
                    eprint!("{}", report.to_text())
                }
                Format::Text => out!("{}", report.to_text()),
                Format::Json => out!("{json}"),
            }
            // Every requested file is attempted even if an earlier one fails.
            let mut write_failed = save_failed.load(std::sync::atomic::Ordering::Relaxed)
                || jar_failed.load(std::sync::atomic::Ordering::Relaxed);
            if let Some(path) = &args.reporter_json {
                write_failed |= !write_report(path, &json);
            }
            if let Some(path) = &args.reporter_junit {
                let timestamp = chrono::Utc::now()
                    .format("%Y-%m-%dT%H:%M:%S%.3f")
                    .to_string();
                let hostname = gethostname::gethostname();
                let xml = report.to_junit(&timestamp, &hostname.to_string_lossy(), args.bail);
                write_failed |= !write_report(path, &xml);
            }
            if let Some(path) = &args.reporter_html {
                let generated = chrono::Utc::now()
                    .format("%Y-%m-%d %H:%M:%S UTC")
                    .to_string();
                write_failed |= !write_report(path, &report.to_html(&generated));
            }
            if write_failed {
                EXIT_USAGE
            } else {
                report.exit_code()
            }
        }
        Err(e) => {
            eprintln!("error: {e:#}");
            EXIT_USAGE
        }
    }
}

#[cfg(test)]
mod env_var_tests {
    use super::*;
    use std::ffi::OsString;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| OsString::from(v))
        }
    }

    #[test]
    fn key_equals_value_splits_at_the_first_equals() {
        let v = parse_env_var_with("A=b=c", env(&[])).unwrap();
        assert_eq!(
            (v.key.as_str(), v.value.as_str(), v.from_env),
            ("A", "b=c", false)
        );
    }

    #[test]
    fn a_bare_key_reads_the_environment_and_is_marked() {
        let v = parse_env_var_with("T", env(&[("T", "x")])).unwrap();
        assert_eq!(
            (v.key.as_str(), v.value.as_str(), v.from_env),
            ("T", "x", true)
        );
        let v = parse_env_var_with("T", env(&[("T", "")])).unwrap();
        assert_eq!((v.value.as_str(), v.from_env), ("", true));
    }

    #[test]
    fn unset_and_empty_key_fail_without_a_value() {
        assert_eq!(
            parse_env_var_with("T", env(&[])).unwrap_err(),
            "environment variable T is not set"
        );
        assert!(parse_env_var_with("=v", env(&[])).is_err());
        assert!(parse_env_var_with("", env(&[])).is_err());
    }
}
