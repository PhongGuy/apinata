//! Launch options for the MCP server, as given on the `apinata mcp` command line.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use apinata_core::{
    ApprovalInbox, ClientCertificate, GitOptions, GlobalEnvironments, HttpVersionPolicy,
    NetworkSettings, ScriptTrust, SystemTrash, Trash, TrashLog, TrustStore,
};

/// The collection to serve and the defaults every tool call starts from.
#[derive(Clone)]
#[non_exhaustive]
pub struct McpOptions {
    root: PathBuf,
    env: Option<String>,
    env_vars: Vec<(String, String)>,
    global_env: Option<(Arc<GlobalEnvironments>, String)>,
    global_env_vars: Vec<(String, String)>,
    insecure: bool,
    network: Option<NetworkSettings>,
    certificates: Vec<ClientCertificate>,
    trust_file: Option<PathBuf>,
    script_trust_file: Option<PathBuf>,
    approval_dir: Option<PathBuf>,
    token_cache: Option<PathBuf>,
    http_version: HttpVersionPolicy,
    git: GitOptions,
    trash: Arc<dyn Trash>,
    trash_log: Option<PathBuf>,
}

impl McpOptions {
    /// Serve the collection at `root` with no environment, no overrides and verified TLS.
    pub fn new(root: impl Into<PathBuf>) -> McpOptions {
        McpOptions {
            root: root.into(),
            env: None,
            env_vars: Vec::new(),
            global_env: None,
            global_env_vars: Vec::new(),
            insecure: false,
            network: None,
            certificates: Vec::new(),
            trust_file: TrustStore::default_path(),
            script_trust_file: ScriptTrust::default_path(),
            // Unit tests of this crate must never file into the real per-user inbox.
            approval_dir: if cfg!(test) {
                None
            } else {
                ApprovalInbox::default_dir()
            },
            token_cache: None,
            http_version: HttpVersionPolicy::default(),
            git: GitOptions::new(),
            trash: Arc::new(SystemTrash::new()),
            // As with the inbox, unit tests never write the real record of deletes.
            trash_log: if cfg!(test) {
                None
            } else {
                TrashLog::default_dir()
            },
        }
    }

    /// Select the default environment by file stem (`--env`).
    pub fn env(mut self, name: impl Into<String>) -> McpOptions {
        self.env = Some(name.into());
        self
    }

    /// Append a variable override (`--env-var`); later entries win.
    pub fn env_var(mut self, name: impl Into<String>, value: impl Into<String>) -> McpOptions {
        self.env_vars.push((name.into(), value.into()));
        self
    }

    /// Select global environment `name` from `globals` for every send (`--global-env`).
    pub fn global_env(
        mut self,
        globals: Arc<GlobalEnvironments>,
        name: impl Into<String>,
    ) -> McpOptions {
        self.global_env = Some((globals, name.into()));
        self
    }

    /// Append a global variable override (`--global-env-var`); later entries win.
    pub fn global_env_var(
        mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> McpOptions {
        self.global_env_vars.push((name.into(), value.into()));
        self
    }

    /// Skip TLS certificate verification (`--insecure`).
    pub fn insecure(mut self, insecure: bool) -> McpOptions {
        self.insecure = insecure;
        self
    }

    /// Apply these network settings (global proxy layer, CA); without them the server connects
    /// directly, ignoring the system proxy.
    pub fn network(mut self, settings: NetworkSettings) -> McpOptions {
        self.network = Some(settings);
        self
    }

    /// Offer these client certificates after the collection's own (`--client-cert-config`).
    pub fn client_certificates(mut self, certificates: Vec<ClientCertificate>) -> McpOptions {
        self.certificates = certificates;
        self
    }

    /// Read trusted and denied hosts from `path` instead of the per-user default.
    pub fn trust_file(mut self, path: impl Into<PathBuf>) -> McpOptions {
        self.trust_file = Some(path.into());
        self
    }

    /// Read script decisions and approvals from `path` instead of the per-user default.
    pub fn script_trust_file(mut self, path: impl Into<PathBuf>) -> McpOptions {
        self.script_trust_file = Some(path.into());
        self
    }

    /// File approval requests in `dir` instead of the per-user default.
    pub fn approval_dir(mut self, dir: impl Into<PathBuf>) -> McpOptions {
        self.approval_dir = Some(dir.into());
        self
    }

    /// Move pruned files to `trash` instead of the system trash; tests pass a fake.
    pub fn trash(mut self, trash: Arc<dyn Trash>) -> McpOptions {
        self.trash = trash;
        self
    }

    /// Record deletes in `dir`, so `restore_item` can undo them, instead of the per-user
    /// default.
    pub fn trash_log(mut self, dir: impl Into<PathBuf>) -> McpOptions {
        self.trash_log = Some(dir.into());
        self
    }

    /// Record no deletes, so `restore_item` is unavailable.
    pub fn without_trash_log(mut self) -> McpOptions {
        self.trash_log = None;
        self
    }

    /// File no approval requests at all, so a refused host is reported with the command that
    /// trusts it instead.
    pub fn without_approval_dir(mut self) -> McpOptions {
        self.approval_dir = None;
        self
    }

    /// Keep OAuth 2.0 tokens in the encrypted file at `path`, its key in the server's secret
    /// store; without it tokens last as long as the server.
    pub fn token_cache_file(mut self, path: impl Into<PathBuf>) -> McpOptions {
        self.token_cache = Some(path.into());
        self
    }

    /// The HTTP version policy every send uses (`--http-version`); HTTP/1.1 only by default.
    pub fn http_version(mut self, policy: HttpVersionPolicy) -> McpOptions {
        self.http_version = policy;
        self
    }

    /// The HTTP version policy every send uses.
    pub fn http_version_policy(&self) -> HttpVersionPolicy {
        self.http_version
    }

    /// Run git with `options` instead of the defaults; a test seam, like `trust_file`.
    pub fn git(mut self, options: GitOptions) -> McpOptions {
        self.git = options;
        self
    }

    /// How the git tools run git.
    pub(crate) fn git_options(&self) -> &GitOptions {
        &self.git
    }

    /// The token cache file, or `None` when tokens are kept in memory.
    pub fn token_cache_path(&self) -> Option<&Path> {
        self.token_cache.as_deref()
    }

    /// The trust file, or `None` when the platform has no config directory.
    pub fn trust_path(&self) -> Option<&Path> {
        self.trust_file.as_deref()
    }

    /// The script trust file, or `None` when the platform has no config directory.
    pub fn script_trust_path(&self) -> Option<&Path> {
        self.script_trust_file.as_deref()
    }

    /// The approval inbox directory, or `None` when the platform has no data directory.
    pub fn approval_path(&self) -> Option<&Path> {
        self.approval_dir.as_deref()
    }

    /// Where pruned files go.
    pub(crate) fn trash_bin(&self) -> Arc<dyn Trash> {
        Arc::clone(&self.trash)
    }

    /// The record of deletes, when there is a directory for it.
    pub(crate) fn trash_record(&self) -> Option<TrashLog> {
        self.trash_log.as_ref().map(TrashLog::new)
    }

    /// The collection root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The default environment's name, if one was selected.
    pub fn default_env(&self) -> Option<&str> {
        self.env.as_deref()
    }

    /// The variable overrides in the order they were given.
    pub fn env_vars(&self) -> &[(String, String)] {
        &self.env_vars
    }

    /// The global environments and the selected name, if one was selected.
    pub fn default_global_env(&self) -> Option<(&Arc<GlobalEnvironments>, &str)> {
        self.global_env
            .as_ref()
            .map(|(globals, name)| (globals, name.as_str()))
    }

    /// The global variable overrides in the order they were given.
    pub fn global_env_vars(&self) -> &[(String, String)] {
        &self.global_env_vars
    }

    /// The network settings, if any were given.
    pub fn network_settings(&self) -> Option<&NetworkSettings> {
        self.network.as_ref()
    }

    /// The launch-time client certificates.
    pub fn certificates(&self) -> &[ClientCertificate] {
        &self.certificates
    }

    /// Whether TLS certificate verification is skipped.
    pub fn is_insecure(&self) -> bool {
        self.insecure
    }
}

// `--env-var` and `--global-env-var` values are often secrets, so Debug lists names only.
impl fmt::Debug for McpOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names = |vars: &[(String, String)]| -> Vec<String> {
            vars.iter().map(|(name, _)| name.clone()).collect()
        };
        let global_env = self.global_env.as_ref().map(|(_, name)| name);
        f.debug_struct("McpOptions")
            .field("root", &self.root)
            .field("env", &self.env)
            .field("env_vars", &names(&self.env_vars))
            .field("global_env", &global_env)
            .field("global_env_vars", &names(&self.global_env_vars))
            .field("insecure", &self.insecure)
            .field("network", &self.network)
            .field("certificates", &self.certificates.len())
            .field("trust_file", &self.trust_file)
            .field("script_trust_file", &self.script_trust_file)
            .field("approval_dir", &self.approval_dir)
            .field("trash_log", &self.trash_log)
            .field("token_cache", &self.token_cache)
            .field("http_version", &self.http_version)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::McpOptions;
    use apinata_core::{ApprovalInbox, ScriptTrust, TrustStore};
    use std::path::Path;

    #[test]
    fn defaults_to_no_environment_no_overrides_and_verified_tls() {
        let options = McpOptions::new("/srv/shop");
        assert_eq!(options.root(), Path::new("/srv/shop"));
        assert_eq!(options.default_env(), None);
        assert!(options.env_vars().is_empty());
        assert!(!options.is_insecure());
        assert_eq!(options.trust_path(), TrustStore::default_path().as_deref());
        assert_eq!(
            options.script_trust_path(),
            ScriptTrust::default_path().as_deref()
        );
    }

    #[test]
    fn a_test_built_options_never_resolves_to_the_real_approval_inbox() {
        let options = McpOptions::new("/srv/shop");
        assert_eq!(options.approval_path(), None);
        assert_ne!(
            options.approval_path(),
            ApprovalInbox::default_dir().as_deref()
        );
    }

    #[test]
    fn a_test_built_options_never_records_deletes_in_the_real_data_dir() {
        assert_eq!(McpOptions::new("/srv/shop").trash_record(), None);
        let options = McpOptions::new("/srv/shop").trash_log("/tmp/t/trash-log");
        assert_eq!(
            options.trash_record(),
            Some(apinata_core::TrashLog::new("/tmp/t/trash-log"))
        );
    }

    #[test]
    fn trust_file_and_approval_dir_can_be_redirected() {
        let options = McpOptions::new("/srv/shop")
            .trust_file("/tmp/t/trusted-hosts.json")
            .script_trust_file("/tmp/t/script-trust.json")
            .approval_dir("/tmp/t/approvals");
        assert_eq!(
            options.trust_path(),
            Some(Path::new("/tmp/t/trusted-hosts.json"))
        );
        assert_eq!(
            options.script_trust_path(),
            Some(Path::new("/tmp/t/script-trust.json"))
        );
        assert_eq!(options.approval_path(), Some(Path::new("/tmp/t/approvals")));
    }

    #[test]
    fn builder_keeps_every_option_and_env_vars_in_order() {
        let options = McpOptions::new("/srv/shop")
            .env("dev")
            .env_var("token", "abc")
            .env_var("user", "me")
            .insecure(true);
        assert_eq!(options.default_env(), Some("dev"));
        assert_eq!(
            options.env_vars(),
            [
                ("token".to_owned(), "abc".to_owned()),
                ("user".to_owned(), "me".to_owned())
            ]
        );
        assert!(options.is_insecure());
    }

    #[test]
    fn tokens_stay_in_memory_unless_a_cache_file_is_given() {
        assert_eq!(McpOptions::new("/srv/shop").token_cache_path(), None);
        let options = McpOptions::new("/srv/shop").token_cache_file("/tmp/t/oauth2-tokens.json");
        assert_eq!(
            options.token_cache_path(),
            Some(Path::new("/tmp/t/oauth2-tokens.json"))
        );
    }

    #[test]
    fn debug_names_env_vars_but_hides_their_values() {
        let options = McpOptions::new("/srv/shop").env_var("token", "sentinel-s3cr3t");
        let shown = format!("{options:?}");
        assert!(shown.contains("token"), "{shown}");
        assert!(!shown.contains("sentinel-s3cr3t"), "{shown}");
    }

    #[test]
    fn debug_names_global_overrides_but_hides_their_values() {
        let options = McpOptions::new("/srv/shop").global_env_var("gtoken", "sentinel-g10bal");
        let shown = format!("{options:?}");
        assert!(shown.contains("gtoken"), "{shown}");
        assert!(!shown.contains("sentinel-g10bal"), "{shown}");
        assert_eq!(
            options.global_env_vars(),
            [("gtoken".to_owned(), "sentinel-g10bal".to_owned())]
        );
    }
}
