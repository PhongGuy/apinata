//! The request-independent variable layers of a collection, snapshotted once per run.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use serde_json::Value;

use crate::aws_profile::AwsSharedFiles;
use crate::collection::{Collection, Folder};
use crate::document::{Document, read_regular};
use crate::dotenv::parse_dotenv;
use crate::dynamic::Generator;
use crate::env_overlay::{EnvOverlay, OverlayLayer};
use crate::environment::{EnvValue, ResolvedEnv, resolve_environment};
use crate::error::{ContextError, UndefinedHint};
use crate::global_env::GlobalEnvironments;
use crate::interpolate::{Tracker, Vars, interpolate_lenient};
use crate::iteration_data::IterationData;
use crate::js;
use crate::network::{
    CertificateKind, ClientCertificate, CollectionNetwork, CollectionProxy, read_network,
};
use crate::oauth2::TokenCache;
use crate::post_response::{PostResponseVar, post_response_vars_in};
use crate::request::RequestFile;
use crate::runtime_vars::RuntimeVars;
use crate::secret_names::{MIN_CHARS, render};
use crate::secrets::{SecretError, SecretKey, SecretStore};
use crate::variable_set::{VarScope, VariableSet};

/// Choices that shape a [`VarContext`]: the environment, command-line overrides and secret store.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct ContextOptions {
    env: Option<String>,
    /// Overrides in call order; the flag marks a sensitive one.
    env_vars: Vec<(String, String, bool)>,
    global_env: Option<(Arc<GlobalEnvironments>, String)>,
    /// Global overrides in call order; the flag marks a sensitive one.
    global_env_vars: Vec<(String, String, bool)>,
    process_env: Option<Vec<(String, String)>>,
    secrets: Option<Arc<dyn SecretStore>>,
    strict_secrets: bool,
    /// Seed and clock for `{{$name}}` values; None draws them fresh on every resolve.
    dynamic_values: Option<(u64, SystemTime)>,
    runtime_vars: Option<RuntimeVars>,
    env_overlay: Option<EnvOverlay>,
    client_certificates: Vec<ClientCertificate>,
    oauth2_tokens: Option<TokenCache>,
    aws_shared_files: Option<AwsSharedFiles>,
}

impl ContextOptions {
    /// No environment and no overrides.
    pub fn new() -> ContextOptions {
        ContextOptions::default()
    }

    /// Select the environment whose file stem is `name` (`--env`).
    pub fn env(mut self, name: impl Into<String>) -> ContextOptions {
        self.env = Some(name.into());
        self
    }

    /// Override or supply an environment variable (`--env-var`); later calls win.
    pub fn env_var(mut self, name: impl Into<String>, value: impl Into<String>) -> ContextOptions {
        self.env_vars.push((name.into(), value.into(), false));
        self
    }

    /// Like [`ContextOptions::env_var`], but the value is a secret supplied by a human, so
    /// [`ResolvedRequest::sensitive_values`](crate::ResolvedRequest::sensitive_values) reports it.
    pub fn sensitive_env_var(
        mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> ContextOptions {
        self.env_vars.push((name.into(), value.into(), true));
        self
    }

    /// Select global environment `name` from `globals` (`--global-env`).
    pub fn global_env(
        mut self,
        globals: Arc<GlobalEnvironments>,
        name: impl Into<String>,
    ) -> ContextOptions {
        self.global_env = Some((globals, name.into()));
        self
    }

    /// Override or supply a global variable (`--global-env-var`); later calls win.
    pub fn global_env_var(
        mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> ContextOptions {
        self.global_env_vars
            .push((name.into(), value.into(), false));
        self
    }

    /// Like [`ContextOptions::global_env_var`], but the value is a secret supplied by a human.
    pub fn sensitive_global_env_var(
        mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> ContextOptions {
        self.global_env_vars.push((name.into(), value.into(), true));
        self
    }

    /// Read secret values of the selected environment and global environment from `store`;
    /// overrides still win.
    pub fn secrets(mut self, store: Arc<dyn SecretStore>) -> ContextOptions {
        self.secrets = Some(store);
        self
    }

    /// Fail [`VarContext::new`] with [`ContextError::SecretUnavailable`] (or
    /// [`ContextError::GlobalSecretUnavailable`]) when the secret store
    /// errors, instead of recording a warning and leaving the secret unset.
    pub fn strict_secrets(mut self) -> ContextOptions {
        self.strict_secrets = true;
        self
    }

    /// Use `vars` as the process environment instead of a snapshot of the real one.
    pub fn process_env<I, K, V>(mut self, vars: I) -> ContextOptions
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        self.process_env = Some(
            vars.into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        );
        self
    }

    /// Makes `{{$name}}` values repeatable: every resolve seeds its generator with `seed`
    /// and reads `now` as the clock. For tests.
    pub fn dynamic_values(mut self, seed: u64, now: SystemTime) -> ContextOptions {
        self.dynamic_values = Some((seed, now));
        self
    }

    /// Share `vars` as the context's runtime scope instead of starting an empty one.
    pub fn runtime_vars(mut self, vars: RuntimeVars) -> ContextOptions {
        self.runtime_vars = Some(vars);
        self
    }

    /// Share `overlay` as the context's script edits to environment and collection variables
    /// instead of starting an empty one. The context reads and writes the values of its own
    /// environment whatever is selected later, until that environment is cleared or renamed.
    pub fn env_overlay(mut self, overlay: EnvOverlay) -> ContextOptions {
        self.env_overlay = Some(overlay);
        self
    }

    /// Offer `certificates` after the collection's own (`--client-cert-config`).
    pub fn client_certificates(mut self, certificates: Vec<ClientCertificate>) -> ContextOptions {
        self.client_certificates = certificates;
        self
    }

    /// Keep OAuth 2.0 tokens in `cache`; without it the context gets its own in-memory cache.
    pub fn oauth2_tokens(mut self, cache: TokenCache) -> ContextOptions {
        self.oauth2_tokens = Some(cache);
        self
    }

    /// Read AWS profiles from `files`; without it, [`AwsSharedFiles::from_env`] is used, and
    /// only for SigV4 auth that names a profile.
    pub fn aws_shared_files(mut self, files: AwsSharedFiles) -> ContextOptions {
        self.aws_shared_files = Some(files);
        self
    }
}

/// Every variable layer that does not depend on the request being resolved.
///
/// Built for one collection: a request from another collection resolves without folder layers.
#[derive(Clone)]
#[non_exhaustive]
pub struct VarContext {
    root: PathBuf,
    /// The root as the filesystem spells it, for requests loaded through another spelling.
    canonical_root: Option<PathBuf>,
    /// The selected global environment with its overrides and stored secrets, lowest of all.
    global_layer: Vars,
    /// Global names whose final value came from the secret store or a sensitive override.
    global_sensitive: BTreeSet<String>,
    collection: Vars,
    environment: Option<String>,
    env_layer: Vars,
    /// Enabled environment secrets with no stored value, which scripts read as `""`.
    env_unstored: BTreeSet<String>,
    /// Each environment's defined variable names, secrets with or without a value; no values.
    env_names: BTreeMap<String, BTreeSet<String>>,
    /// Environment-level names whose final value came from the secret store or a sensitive override.
    env_sensitive: BTreeSet<String>,
    /// The collection's `request` block, for header and auth defaults.
    collection_request: Option<yaml_serde::Value>,
    /// Each folder's layers, keyed by its path relative to the collection root.
    folders: HashMap<PathBuf, FolderLayer>,
    warnings: Vec<String>,
    secret_failures: Vec<SecretReadFailure>,
    /// The `process` namespace: `{"env": ...}`.
    process: Value,
    /// The keys `<root>/.env` sets, in file order.
    dotenv_keys: Vec<String>,
    /// See [`ContextOptions::dynamic_values`].
    dynamic_values: Option<(u64, SystemTime)>,
    /// Shared with every clone, so values captured through one are seen by all.
    runtime_vars: RuntimeVars,
    /// Script edits over the collection and environment layers; clones share it.
    env_overlay: EnvOverlay,
    /// The collection's name, which scripts read.
    collection_name: String,
    /// The collection's proxy and certificates, the options' certificates appended.
    network: CollectionNetwork,
    /// Whether later script phases run outermost first (`flow: sequential`), not innermost.
    sequential_flow: bool,
    /// OAuth 2.0 tokens; clones share it.
    tokens: TokenCache,
    /// See [`ContextOptions::aws_shared_files`].
    aws_shared_files: Option<AwsSharedFiles>,
    /// The current data row of an iterated run, shared by clones so they stay cheap.
    iteration_row: Option<Arc<Vec<(String, String)>>>,
    /// Prompt answers by label, the top layer as `?label`; shared by clones like the row.
    prompt_values: Option<Arc<Vec<(String, String)>>>,
    /// The timeout for requests that inherit theirs; `None` means no limit.
    default_timeout: Option<Duration>,
}

impl VarContext {
    /// Snapshot the collection's variables with the selected environment and overrides applied.
    ///
    /// Fails with [`ContextError::EnvironmentFailedToLoad`] when the selected environment's file
    /// is broken, [`ContextError::EnvironmentNotFound`] when there is none, and with
    /// [`ContextError::DotEnv`] when `<root>/.env` cannot be read. With
    /// [`ContextOptions::strict_secrets`], the first secret (by name) the store fails to read
    /// is [`ContextError::SecretUnavailable`]. A selected global environment fails the same
    /// ways with the `Global` variants.
    pub fn new(
        collection: &Collection,
        mut options: ContextOptions,
    ) -> Result<VarContext, ContextError> {
        let (process, dotenv_keys) =
            process_namespace(collection.root(), options.process_env.take())?;
        let mut warnings = Vec::new();
        let mut secret_failures = Vec::new();
        let store = options.secrets.as_deref();
        let strict = options.strict_secrets;

        // The global layer: overrides beat the file, and an unstored secret is simply absent.
        let mut global = ResolvedEnv::default();
        if let Some((globals, name)) = &options.global_env {
            global = resolve_environment(globals.environments(), name)
                .ok_or_else(|| missing_global_environment(globals, name))?;
        }
        warnings.append(&mut global.warnings);
        let global = fill_layer(
            global,
            options.global_env_vars,
            (store, strict),
            |owner, name| match &options.global_env {
                Some((globals, _)) => globals.secret_key(owner, name),
                // Without a selected global environment there are no secrets to look up.
                None => collection.secret_key(owner, name),
            },
            true,
            (&mut warnings, &mut secret_failures),
        )
        .map_err(
            |(environment, name, message)| ContextError::GlobalSecretUnavailable {
                environment,
                name,
                message,
            },
        )?;

        // Overrides sit at the environment level and beat the secret store.
        let mut env = ResolvedEnv::default();
        if let Some(name) = &options.env {
            env = resolve_environment(collection.environments(), name)
                .ok_or_else(|| missing_environment(collection, name))?;
            env.vars.insert(
                "__name__".to_owned(),
                EnvValue::Plain(Value::String(name.clone())),
            );
        }
        warnings.append(&mut env.warnings);
        let env = fill_layer(
            env,
            options.env_vars,
            (store, strict),
            |owner, name| collection.secret_key(owner, name),
            false,
            (&mut warnings, &mut secret_failures),
        )
        .map_err(
            |(environment, name, message)| ContextError::SecretUnavailable {
                environment,
                name,
                message,
            },
        )?;

        let root = collection.root().to_path_buf();
        let mut folders = HashMap::new();
        for folder in collection.tree().folders() {
            collect_folders(folder, &root, &mut folders);
        }
        let mut network = read_network(collection.root_document());
        network.certificates.extend(options.client_certificates);
        let flow = collection
            .root_document()
            .get(&["extensions", "bruno", "scripts", "flow"]);
        let env_names = collection
            .environments()
            .iter()
            .filter_map(|e| {
                let flat = resolve_environment(collection.environments(), e.name())?;
                Some((e.name().to_owned(), flat.vars.into_keys().collect()))
            })
            .collect();
        let env_overlay = (options.env_overlay.unwrap_or_default()).pinned(options.env.as_deref());
        Ok(VarContext {
            network,
            sequential_flow: matches!(flow, Some(yaml_serde::Value::String(f)) if f == "sequential"),
            collection: layer(collection.root_document(), &["request", "variables"]),
            collection_request: collection.root_document().get(&["request"]).cloned(),
            canonical_root: fs::canonicalize(&root).ok(),
            root,
            environment: options.env,
            global_layer: global.vars,
            global_sensitive: global.sensitive,
            env_layer: env.vars,
            env_unstored: env.unstored,
            env_names,
            env_sensitive: env.sensitive,
            folders,
            warnings,
            secret_failures,
            process,
            dotenv_keys,
            dynamic_values: options.dynamic_values,
            runtime_vars: options.runtime_vars.unwrap_or_default(),
            env_overlay,
            collection_name: collection.name().to_owned(),
            tokens: options.oauth2_tokens.unwrap_or_else(TokenCache::in_memory),
            aws_shared_files: options.aws_shared_files,
            iteration_row: None,
            prompt_values: None,
            default_timeout: None,
        })
    }

    /// The context a run gives iteration `index` of `total`: row `index` of `data` (none
    /// without data or past its end) plus `iteration` and `iterationCount`, unless the row
    /// has columns of those names. A run's single pass is iteration 0 of 1.
    pub fn for_iteration(
        &self,
        data: Option<&IterationData>,
        index: usize,
        total: usize,
    ) -> VarContext {
        let row = data.and_then(|d| d.row(index)).unwrap_or(&[]);
        let mut vars = row.to_vec();
        for (name, value) in [("iteration", index), ("iterationCount", total)] {
            if !row.iter().any(|(k, _)| k == name) {
                vars.push((name.to_owned(), value.to_string()));
            }
        }
        self.with_iteration_row(&vars)
    }

    /// A clone carrying `row` as the iteration layer, above the request's own variables and
    /// below OAuth 2.0 tokens, the runtime scope and `process`.
    pub(crate) fn with_iteration_row(&self, row: &[(String, String)]) -> VarContext {
        let mut ctx = self.clone();
        ctx.iteration_row = Some(Arc::new(row.to_vec()));
        ctx
    }

    /// A clone answering prompt variables: each `(label, value)` fills `{{?label}}`, above
    /// every other layer. Replaces any earlier answers; answers are never sensitive.
    pub fn with_prompt_values<I, K, V>(&self, values: I) -> VarContext
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let mut ctx = self.clone();
        let values: Vec<(String, String)> = values
            .into_iter()
            .map(|(label, value)| (label.into(), value.into()))
            .collect();
        ctx.prompt_values = Some(Arc::new(values));
        ctx
    }

    /// A clone that reads `values` as if `<root>/.env` held them, over the file and the
    /// process environment alike: values the caller will write to `.env` but has not yet.
    /// They are secret like every `.env` value.
    pub fn with_dotenv_overrides<I, K, V>(&self, values: I) -> VarContext
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let mut ctx = self.clone();
        let values = values.into_iter().map(|(k, v)| (k.into(), v.into()));
        (ctx.process, ctx.dotenv_keys) =
            with_dotenv_overrides(ctx.process, ctx.dotenv_keys, values.collect());
        ctx
    }

    /// A clone whose requests that inherit their timeout use `timeout`; `None` means no limit.
    pub fn with_default_timeout(&self, timeout: Option<Duration>) -> VarContext {
        let mut ctx = self.clone();
        ctx.default_timeout = timeout;
        ctx
    }

    /// The timeout given to requests that inherit theirs.
    pub(crate) fn default_timeout(&self) -> Option<Duration> {
        self.default_timeout
    }

    /// Whether this context holds an answer for the prompt `label`.
    pub(crate) fn has_prompt_value(&self, label: &str) -> bool {
        self.prompt_vars().iter().any(|(name, _)| name == label)
    }

    /// The prompt layer's (label, answer) pairs; empty when nothing was answered.
    fn prompt_vars(&self) -> &[(String, String)] {
        self.prompt_values.as_deref().map_or(&[], Vec::as_slice)
    }

    /// The iteration layer's (column, value) pairs; empty outside an iterated run.
    fn iteration_vars(&self) -> &[(String, String)] {
        self.iteration_row.as_deref().map_or(&[], Vec::as_slice)
    }

    /// The OAuth 2.0 token cache this context reads and fills.
    pub(crate) fn oauth2_tokens(&self) -> &TokenCache {
        &self.tokens
    }

    /// Where AWS profiles are read from; the environment's files unless the options chose.
    pub(crate) fn aws_shared_files(&self) -> AwsSharedFiles {
        self.aws_shared_files
            .clone()
            .unwrap_or_else(AwsSharedFiles::from_env)
    }

    /// The collection root as the token cache keys it: canonical when known.
    pub(crate) fn key_root(&self) -> PathBuf {
        self.canonical_root
            .clone()
            .unwrap_or_else(|| self.root.clone())
    }

    /// A generator for one resolve: seeded as the options asked, else OS-seeded.
    pub(crate) fn generator(&self) -> Generator {
        match self.dynamic_values {
            Some((seed, now)) => Generator::seeded(seed, now),
            None => Generator::system(),
        }
    }

    /// The selected environment's name, if any.
    pub fn environment(&self) -> Option<&str> {
        self.environment.as_deref()
    }

    /// The runtime scope, above every file and environment layer; clones share it.
    pub fn runtime_vars(&self) -> &RuntimeVars {
        &self.runtime_vars
    }

    /// A tracker for the sensitive `names` that also records values scripts tainted.
    pub(crate) fn tracker(&self, names: BTreeSet<String>) -> Tracker {
        Tracker::new(names).tainted(self.runtime_vars.tainted())
    }

    /// Script edits to the environment, global and collection variables, which sit just above the
    /// layer they edit and never reach disk; clones share them.
    pub fn env_overlay(&self) -> &EnvOverlay {
        &self.env_overlay
    }

    /// The collection's `info.name`, else its directory name.
    pub(crate) fn collection_name(&self) -> &str {
        &self.collection_name
    }

    /// The collection layer as scripts see it, overlay applied; `None` is `undefined`.
    pub(crate) fn collection_view(&self) -> BTreeMap<String, Option<Value>> {
        self.env_overlay
            .view(OverlayLayer::Collection, &self.collection)
    }

    /// The global layer as scripts see it, overlay applied; `None` is `undefined`.
    pub(crate) fn global_view(&self) -> BTreeMap<String, Option<Value>> {
        self.env_overlay
            .view(OverlayLayer::Global, &self.global_layer)
    }

    /// The environment layer as scripts see it, overlay applied; `None` is `undefined`.
    pub(crate) fn env_view(&self) -> BTreeMap<String, Option<Value>> {
        self.env_overlay
            .view(OverlayLayer::Environment, &self.env_layer)
    }

    /// The environment layer as scripts see it: [`Self::env_view`] with each unstored secret
    /// read as `""`, placed under the overlay so a script's own set or delete still wins.
    pub(crate) fn script_env_view(&self) -> BTreeMap<String, Option<Value>> {
        let mut with_empties = self.env_layer.clone();
        for name in &self.env_unstored {
            with_empties.insert(name.clone(), Value::String(String::new()));
        }
        self.env_overlay
            .view(OverlayLayer::Environment, &with_empties)
    }

    /// Whether the global `name` holds a secret or a sensitive override.
    pub(crate) fn global_name_sensitive(&self, name: &str) -> bool {
        self.global_sensitive.contains(name)
    }

    /// Whether the environment-level `name` holds a secret or a sensitive override.
    pub(crate) fn env_name_sensitive(&self, name: &str) -> bool {
        self.env_sensitive.contains(name)
    }

    /// The folder variables `req` sees, outer folders first and inner ones overwriting.
    pub(crate) fn folder_vars(&self, req: &RequestFile) -> Vars {
        let mut vars = Vars::new();
        for folder in self.ancestors(req.path()) {
            vars.extend(folder.vars.clone());
        }
        vars
    }

    /// The request's own `runtime.variables`.
    pub(crate) fn request_vars(req: &RequestFile) -> Vars {
        layer(req.document(), &["runtime", "variables"])
    }

    /// The `process.env` map.
    pub(crate) fn process_env(&self) -> Option<&serde_json::Map<String, Value>> {
        self.process.get("env").and_then(Value::as_object)
    }

    /// Problems found while building the context, such as a broken `extends` chain.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// The secrets the store failed to read in lenient mode, each also present in
    /// [`Self::warnings`] as text. Empty under [`ContextOptions::strict_secrets`], which fails.
    pub fn secret_read_failures(&self) -> &[SecretReadFailure] {
        &self.secret_failures
    }

    /// The lookup map for `req`, from one read of the runtime scope; see [`Self::lookup_for`].
    pub(crate) fn vars_for(&self, req: &RequestFile) -> Vars {
        self.vars_with(req, &self.runtime_vars.snapshot())
    }

    /// `req`'s enabled post-response variables: the collection's, then each folder's outer to
    /// inner, then its own. A later one with a name already listed replaces it in place.
    pub(crate) fn post_response_vars_for(&self, req: &RequestFile) -> Vec<PostResponseVar> {
        let actions = |request: Option<&yaml_serde::Value>| {
            post_response_vars_in(request.and_then(|r| r.get("actions")))
        };
        let defaults = self.defaults_for(req);
        let levels = std::iter::once(actions(defaults.collection))
            .chain(defaults.folders.into_iter().map(|f| actions(Some(f))))
            .chain(std::iter::once(req.post_response_vars()));
        let mut merged: Vec<PostResponseVar> = Vec::new();
        for var in levels.flatten().filter(PostResponseVar::enabled) {
            match merged.iter_mut().find(|m| m.name() == var.name()) {
                Some(slot) => *slot = var,
                None => merged.push(var),
            }
        }
        merged
    }

    /// The lookup map and its sensitive names, both from the same read of the runtime scope,
    /// so a concurrent capture cannot leave a secret in the map that the names no longer cover.
    pub(crate) fn lookup_for(&self, req: &RequestFile) -> (Vars, BTreeSet<String>) {
        self.lookup_with(req, &self.runtime_vars.snapshot())
    }

    /// [`Self::lookup_for`] over an already taken runtime snapshot.
    pub(crate) fn lookup_with(
        &self,
        req: &RequestFile,
        captured: &BTreeMap<String, Option<Value>>,
    ) -> (Vars, BTreeSet<String>) {
        (
            self.vars_with(req, captured),
            self.sensitive_names_with(req, captured),
        )
    }

    /// Every variable `req` can see, with its scope and rendered value, for editors.
    pub fn variables(&self, req: &RequestFile) -> VariableSet {
        VariableSet::new(self.clone(), req.clone())
    }

    /// The layers that define top-level `key`, highest first, in the order [`Self::vars_with`]
    /// lets a later layer overwrite an earlier one. A `None` value is an `undefined` removal.
    pub(crate) fn layers_of(
        &self,
        req: &RequestFile,
        key: &str,
        captured: &BTreeMap<String, Option<Value>>,
    ) -> Vec<(VarScope, Option<Value>)> {
        let mut out = Vec::new();
        if let Some(v) = captured.get(key) {
            out.push((VarScope::Runtime, v.clone()));
        }
        if let Some(v) = layer(req.document(), &["runtime", "variables"]).get(key) {
            out.push((VarScope::Request, Some(v.clone())));
        }
        for folder in self.ancestors(req.path()).iter().rev() {
            if let Some(v) = folder.vars.get(key) {
                out.push((VarScope::Folder, Some(v.clone())));
            }
        }
        if let Some(v) = self.env_view().get(key) {
            out.push((VarScope::Environment, v.clone()));
        }
        if let Some(v) = self.collection_view().get(key) {
            out.push((VarScope::Collection, v.clone()));
        }
        if let Some(Some(v)) = self.global_view().get(key) {
            out.push((VarScope::GlobalEnvironment, Some(v.clone())));
        }
        out
    }

    /// Which layer supplies top-level `key` in `req`'s lookup map: the walk of
    /// [`Self::vars_with`] read from the top down, so the first layer holding it wins.
    pub(crate) fn scope_of(
        &self,
        req: &RequestFile,
        key: &str,
        captured: &BTreeMap<String, Option<Value>>,
    ) -> VarScope {
        if key == "process" {
            VarScope::ProcessEnv
        } else if captured.contains_key(key) {
            VarScope::Runtime
        } else if key.starts_with("$oauth2.") && self.oauth2_layer().iter().any(|(n, _)| n == key) {
            VarScope::OAuth2
        } else if self.iteration_vars().iter().any(|(n, _)| n == key) {
            VarScope::IterationData
        } else {
            let layers = self.layers_of(req, key, captured);
            layers.first().map_or(VarScope::Collection, |(s, _)| *s)
        }
    }

    /// The `$oauth2.<id>.<field>` variables of every cached token for this collection and
    /// environment, the newest entry winning per credentials id.
    fn oauth2_layer(&self) -> Vec<(String, Value)> {
        let mut newest: BTreeMap<String, (u64, serde_json::Map<String, Value>)> = BTreeMap::new();
        for entry in self
            .tokens
            .entries_for(&self.key_root(), self.environment())
        {
            let id = entry.key.credentials_id.clone();
            if newest
                .get(&id)
                .is_none_or(|(at, _)| entry.obtained_at_ms > *at)
            {
                newest.insert(id, (entry.obtained_at_ms, entry.token));
            }
        }
        let mut layer = Vec::new();
        for (id, (obtained_at_ms, token)) in newest {
            for (field, value) in token {
                layer.push((format!("$oauth2.{id}.{field}"), value));
            }
            layer.push((format!("$oauth2.{id}.created_at"), obtained_at_ms.into()));
        }
        layer
    }

    /// The cached `$oauth2` variable `name` and whether its value is sensitive.
    pub(crate) fn oauth2_var(&self, name: &str) -> Option<(Value, bool)> {
        let (_, value) = self.oauth2_layer().into_iter().find(|(n, _)| n == name)?;
        Some((value, !is_public_oauth2_field(name)))
    }

    /// The keys `<root>/.env` sets, in file order.
    pub(crate) fn dotenv_keys(&self) -> &[String] {
        &self.dotenv_keys
    }

    /// Every global and environment secret as (name, value rendered at the root), whether or
    /// not a folder, request, iteration or captured variable shadows it; a value that cannot
    /// render is kept as written.
    pub(crate) fn layer_secrets(&self) -> Vec<(String, String)> {
        let phantom = RequestFile::from_document(Document::draft(
            self.root.join("_.yml"),
            yaml_serde::Mapping::new(),
        ));
        let vars = self.vars_for(&phantom);
        let as_rendered =
            |value: &Value| render(value, &vars).unwrap_or_else(|| js::to_text(value));
        let mut out = Vec::new();
        let layers = [
            (
                &self.global_sensitive,
                &self.global_layer,
                self.global_view(),
            ),
            (&self.env_sensitive, &self.env_layer, self.env_view()),
        ];
        for (names, layer, view) in &layers {
            for name in *names {
                let found = layer
                    .get(name)
                    .into_iter()
                    .chain(view.get(name).and_then(Option::as_ref));
                out.extend(found.map(|value| (name.clone(), as_rendered(value))));
            }
        }
        out
    }

    /// Every value this context holds as secret, of at least four characters, deduplicated and
    /// longest first: global and environment secrets (shadowed ones too), `.env` values, cached
    /// OAuth 2.0 token fields that are not public, and values scripts tainted.
    pub fn secret_values(&self) -> Vec<String> {
        let mut values: Vec<String> = self
            .layer_secrets()
            .into_iter()
            .map(|(_, text)| text)
            .collect();
        for (key, value) in self.process_env().into_iter().flatten() {
            if self.dotenv_keys.iter().any(|k| k == key) {
                values.push(js::to_text(value));
            }
        }
        for (name, value) in self.oauth2_layer() {
            if !is_public_oauth2_field(&name) {
                values.push(js::to_text(&value));
            }
        }
        values.extend(self.runtime_vars.tainted());
        values.retain(|v| v.chars().count() >= MIN_CHARS);
        values.sort_by(|a, b| {
            b.chars()
                .count()
                .cmp(&a.chars().count())
                .then_with(|| a.cmp(b))
        });
        values.dedup();
        values
    }

    /// The lookup map for `req`: the global layer, collection and its overlay, environment and its overlay,
    /// folders outer to inner, then the request's own runtime variables, the iteration row,
    /// cached OAuth 2.0 token fields, the shared runtime scope and the prompt answers as `?label`,
    /// each later layer overwriting earlier names; an overlay or runtime `undefined` removes the name. The `process`
    /// namespace goes in last and replaces any variable of that name.
    pub(crate) fn vars_with(
        &self,
        req: &RequestFile,
        captured: &BTreeMap<String, Option<Value>>,
    ) -> Vars {
        let mut vars = self.authored_vars(req);
        // Inserted in column order, so of a repeated column name the last one wins.
        for (name, value) in self.iteration_vars() {
            vars.insert(name.clone(), Value::String(value.clone()));
        }
        for (name, value) in self.oauth2_layer() {
            vars.insert(name, value);
        }
        apply_captured(&mut vars, captured);
        for (label, value) in self.prompt_vars() {
            vars.insert(format!("?{label}"), Value::String(value.clone()));
        }
        vars.insert("process".to_owned(), self.process.clone());
        vars
    }

    /// The merged variables a prompt scan walks for `req`: the authored layers of
    /// [`Self::vars_with`] plus the runtime snapshot, without the iteration row, OAuth 2.0
    /// fields or `process`.
    pub(crate) fn prompt_scan_vars(&self, req: &RequestFile) -> Vars {
        let mut vars = self.authored_vars(req);
        apply_captured(&mut vars, &self.runtime_vars.snapshot());
        vars
    }

    /// The global, collection, environment, folder and request variable layers merged in that
    /// order; an overlay `undefined` removes the name.
    fn authored_vars(&self, req: &RequestFile) -> Vars {
        let mut vars = Vars::new();
        for view in [self.global_view(), self.collection_view(), self.env_view()] {
            for (name, value) in view {
                match value {
                    Some(value) => vars.insert(name, value),
                    None => vars.remove(&name),
                };
            }
        }
        for folder in self.ancestors(req.path()) {
            vars.extend(folder.vars.clone());
        }
        vars.extend(layer(req.document(), &["runtime", "variables"]));
        vars
    }

    /// The variables a collection-level text (the network settings) can see: global,
    /// collection and environment layers with their overlays, and `process`; plus the names
    /// among them whose values are sensitive. No folder, request, row, token or runtime layer
    /// takes part, so the answer follows the files and the selected environment alone.
    /// A send resolves these fields with its own request's layers, so a folder or request
    /// variable that shadows a name here can make a send's value differ from this one; the
    /// fingerprint then misses a change to that shadowing variable (see the i471 follow-ups).
    pub(crate) fn network_scope(&self) -> (Vars, BTreeSet<String>) {
        let mut vars = Vars::new();
        for view in [self.global_view(), self.collection_view(), self.env_view()] {
            for (name, value) in view {
                match value {
                    Some(value) => vars.insert(name, value),
                    None => vars.remove(&name),
                };
            }
        }
        vars.insert("process".to_owned(), self.process.clone());
        let (collection, env) = (self.collection_view(), self.env_view());
        let mut names: BTreeSet<String> = self
            .global_sensitive
            .iter()
            .filter(|name| !collection.contains_key(*name) && !env.contains_key(*name))
            .cloned()
            .collect();
        names.extend(self.env_sensitive.iter().cloned());
        names.insert("process".to_owned());
        (vars, names)
    }

    /// The top-level names in `req`'s lookup map whose values are sensitive: global and
    /// environment-level secrets and sensitive overrides no higher layer redefines, and `process`.
    pub(crate) fn sensitive_names_with(
        &self,
        req: &RequestFile,
        captured: &BTreeMap<String, Option<Value>>,
    ) -> BTreeSet<String> {
        // A global secret stays sensitive only while no collection or environment name hides it.
        let (collection, env) = (self.collection_view(), self.env_view());
        let mut names: BTreeSet<String> = self
            .global_sensitive
            .iter()
            .filter(|name| !collection.contains_key(*name) && !env.contains_key(*name))
            .cloned()
            .collect();
        names.extend(self.env_sensitive.iter().cloned());
        for folder in self.ancestors(req.path()) {
            names.retain(|name| !folder.vars.contains_key(name));
        }
        let runtime = layer(req.document(), &["runtime", "variables"]);
        names.retain(|name| !runtime.contains_key(name));
        let row = self.iteration_vars();
        names.retain(|name| !row.iter().any(|(n, _)| n == name));
        names.extend(
            self.oauth2_layer()
                .into_iter()
                .map(|(name, _)| name)
                .filter(|name| !is_public_oauth2_field(name)),
        );
        names.retain(|name| !captured.contains_key(name));
        // A captured value a person marked sensitive stays secret, though a capture otherwise
        // replaces the environment's secret of that name.
        names.extend(
            captured
                .keys()
                .filter(|name| self.runtime_vars.is_sensitive(name))
                .cloned(),
        );
        let prompts = self.prompt_vars();
        names.retain(|name| {
            !name
                .strip_prefix('?')
                .is_some_and(|l| prompts.iter().any(|(label, _)| label == l))
        });
        names.insert("process".to_owned());
        names
    }

    /// Names of the environments other than the selected one that define `name`, sorted.
    pub(crate) fn other_envs_defining(&self, name: &str) -> Vec<&str> {
        self.env_names
            .iter()
            .filter(|(env, names)| {
                self.environment.as_deref() != Some(env.as_str()) && names.contains(name)
            })
            .map(|(env, _)| env.as_str())
            .collect()
    }

    /// Enabled secrets of the selected environment with no value from the store or overrides.
    pub(crate) fn unstored_env_secrets(&self) -> &BTreeSet<String> {
        &self.env_unstored
    }

    /// The hint for the unstored secret `name` of `environment`: the store's error when it
    /// failed to read the value, else plain "no value".
    pub(crate) fn secret_hint(&self, environment: &str, name: &str) -> UndefinedHint {
        let failure = self
            .secret_failures
            .iter()
            .find(|f| !f.global && f.name == name);
        match failure {
            Some(f) => UndefinedHint::SecretUnreadable {
                environment: environment.to_owned(),
                reason: f.error.to_string(),
            },
            None => UndefinedHint::SecretWithoutValue {
                environment: environment.to_owned(),
            },
        }
    }

    /// Why `name` is undefined for `req` and how to supply it, from names only; `None` when the
    /// name cannot be shown safely or no environment explains it.
    pub fn undefined_hint(&self, req: &RequestFile, name: &str) -> Option<UndefinedHint> {
        if self.name_inside_sensitive_value(req, name) {
            return None;
        }
        if let Some(environment) = &self.environment
            && self.env_unstored.contains(name)
        {
            return Some(self.secret_hint(environment, name));
        }
        let defining = self.other_envs_defining(name);
        (!defining.is_empty()).then(|| UndefinedHint::DefinedElsewhere {
            environments: defining.into_iter().map(str::to_owned).collect(),
        })
    }

    /// Whether naming the undefined or cyclic variable `name` could reveal a sensitive value:
    /// `{{name}}` is inside, or `name` contains, a sensitive value this request substitutes, or
    /// `{{name}}` is not literally written in the request, its `request` defaults or a plain variable.
    pub fn name_inside_sensitive_value(&self, req: &RequestFile, name: &str) -> bool {
        let needle = format!("{{{{{name}}}}}");
        let (vars, sensitive) = self.lookup_for(req);
        let mut texts = Vec::new();
        let defaults = self.defaults_for(req);
        for doc in defaults.collection.into_iter().chain(defaults.folders) {
            yaml_strings(doc, &mut texts);
        }
        for value in req.document().mapping().values() {
            yaml_strings(value, &mut texts);
        }
        // The sweep also covers what resolution expands beyond these texts: the network config,
        // and assertion `in` and `between` items, split on commas wherever a text holds one.
        let mut sweep = texts.clone();
        sweep.extend(self.network_texts());
        let pieces: Vec<&str> = sweep
            .iter()
            .filter(|t| t.contains(','))
            .flat_map(|t| t.split(','))
            .collect();
        sweep.extend(pieces);
        let swept = self.substituted_sensitive(&sweep, &vars, &sensitive);
        if swept
            .iter()
            .any(|value| value.contains(&needle) || (!value.is_empty() && name.contains(value)))
        {
            return true;
        }
        // A secret whose expansion cycles records nothing in the sweep, so its raw text is
        // scanned too; `process` is left out, being the host's env rather than the collection's.
        let mut raw = Vec::new();
        for value in sensitive
            .iter()
            .filter(|key| *key != "process")
            .filter_map(|key| vars.get(key))
        {
            json_strings(value, &mut raw);
        }
        if raw.iter().any(|text| text.contains(&needle)) {
            return true;
        }
        // Fail safe: a name is shown only when someone literally wrote it in plain text, since
        // substituted values can join into a placeholder whose name is a secret or a piece of one.
        let mut authored = texts;
        for (_, value) in vars.iter().filter(|(key, _)| !sensitive.contains(*key)) {
            json_strings(value, &mut authored);
        }
        !authored.iter().any(|text| text.contains(&needle))
    }

    /// Every string in the collection's proxy and client certificates that resolution may expand.
    fn network_texts(&self) -> Vec<&str> {
        let mut out = Vec::new();
        if let CollectionProxy::Custom(config) = &self.network.proxy {
            out.extend([
                &config.protocol,
                &config.hostname,
                &config.port,
                &config.bypass,
            ]);
            if let Some(auth) = &config.auth {
                out.extend([&auth.username, &auth.password]);
            }
        }
        for row in &self.network.host_overrides {
            out.extend([&row.host, &row.address]);
        }
        for cert in &self.network.certificates {
            out.extend([&cert.domain, &cert.passphrase]);
            match &cert.kind {
                CertificateKind::Pem {
                    certificate,
                    private_key,
                } => out.extend([certificate, private_key]),
                CertificateKind::Pkcs12 { file } => out.push(file),
            }
        }
        out.into_iter().map(String::as_str).collect()
    }

    /// The sensitive values that expanding `texts` substitutes, found by a lenient sweep, so
    /// secrets and host env values the request never reads play no part.
    fn substituted_sensitive(
        &self,
        texts: &[&str],
        vars: &Vars,
        sensitive: &BTreeSet<String>,
    ) -> Vec<String> {
        let tracker = self.tracker(sensitive.clone());
        let generator = RefCell::new(self.generator());
        for text in texts {
            interpolate_lenient(text, vars, &tracker, &generator);
        }
        tracker.into_values()
    }

    /// The `request` blocks that supply header and auth defaults to `req`.
    pub(crate) fn defaults_for(&self, req: &RequestFile) -> Defaults<'_> {
        Defaults {
            collection: self.collection_request.as_ref(),
            folders: self
                .ancestors(req.path())
                .into_iter()
                .filter_map(|folder| folder.request.as_ref())
                .collect(),
        }
    }

    /// The blocks holding `req`'s `scripts`, outermost first, each with the level that names it:
    /// the collection's `request`, each folder's `request`, then the request's `runtime`.
    pub(crate) fn script_blocks<'a>(
        &'a self,
        req: &'a RequestFile,
    ) -> Vec<(String, Option<&'a yaml_serde::Value>)> {
        std::iter::once(("collection".to_owned(), self.collection_request.as_ref()))
            .chain(
                self.ancestors(req.path())
                    .into_iter()
                    .map(|f| (format!("folder {}", f.label), f.request.as_ref())),
            )
            .chain(std::iter::once((
                "request".to_owned(),
                req.document().get(&["runtime"]),
            )))
            .collect()
    }

    /// Whether the post-response and tests phases run outermost first.
    pub(crate) fn sequential_flow(&self) -> bool {
        self.sequential_flow
    }

    /// The collection's network block, with the options' certificates appended.
    pub(crate) fn network(&self) -> &CollectionNetwork {
        &self.network
    }

    /// The collection root, against which relative file paths in bodies resolve.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// The folder layers above `path`, outermost first; none when it lies outside the root.
    fn ancestors(&self, path: &Path) -> Vec<&FolderLayer> {
        let Some(dir) = path.parent() else {
            return Vec::new();
        };
        // Spellings may differ (`x/..`, symlinks), so fall back to comparing canonical paths.
        let rel = match dir.strip_prefix(&self.root) {
            Ok(rel) => rel.to_path_buf(),
            Err(_) => {
                let canonical = self.canonical_root.as_ref().zip(fs::canonicalize(dir).ok());
                match canonical
                    .and_then(|(root, dir)| dir.strip_prefix(root).ok().map(Path::to_path_buf))
                {
                    Some(rel) => rel,
                    None => return Vec::new(),
                }
            }
        };
        let mut key = PathBuf::new();
        let mut found = Vec::new();
        for part in rel.components() {
            key.push(part);
            if let Some(folder) = self.folders.get(&key) {
                found.push(folder);
            }
        }
        found
    }
}

/// The `request` blocks above a request: collection first, then folders outer to inner.
pub(crate) struct Defaults<'a> {
    pub(crate) collection: Option<&'a yaml_serde::Value>,
    pub(crate) folders: Vec<&'a yaml_serde::Value>,
}

/// What one folder contributes: its variables, its raw `request` block, and its
/// `/`-separated path relative to the root, which names it in script errors.
#[derive(Clone)]
struct FolderLayer {
    vars: Vars,
    request: Option<yaml_serde::Value>,
    label: String,
}

/// Records `folder` and its descendants' layers under their root-relative paths.
fn collect_folders(folder: &Folder, root: &Path, out: &mut HashMap<PathBuf, FolderLayer>) {
    if let (Some(doc), Ok(rel)) = (folder.document(), folder.path().strip_prefix(root)) {
        let layer = FolderLayer {
            vars: layer(doc, &["request", "variables"]),
            request: doc.get(&["request"]).cloned(),
            label: rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/"),
        };
        out.insert(rel.to_path_buf(), layer);
    }
    for sub in folder.folders() {
        collect_folders(sub, root, out);
    }
}

/// Reads a `variables` list into a map, skipping disabled rows, empty names and unusable values.
fn layer(doc: &Document, keys: &[&str]) -> Vars {
    let mut vars = Vars::new();
    let Some(yaml_serde::Value::Sequence(rows)) = doc.get(keys) else {
        return vars;
    };
    for row in rows {
        if js::is_disabled(row) {
            continue;
        }
        let Some(name) = row.get("name").and_then(js::yaml_to_string) else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        if let Some(value) = js::variable_value(row.get("value")) {
            vars.insert(name, value);
        }
    }
    vars
}

// Overrides and the process environment carry secrets, so Debug shows variable names only.
impl fmt::Debug for ContextOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self
            .env_vars
            .iter()
            .map(|(name, _, _)| name.as_str())
            .collect();
        let global_names: Vec<&str> = self
            .global_env_vars
            .iter()
            .map(|(name, _, _)| name.as_str())
            .collect();
        f.debug_struct("ContextOptions")
            .field("env", &self.env)
            .field("env_vars", &names)
            .field(
                "global_env",
                &self.global_env.as_ref().map(|(_, name)| name),
            )
            .field("global_env_vars", &global_names)
            .field("process_env_len", &self.process_env.as_ref().map(Vec::len))
            .field("oauth2_tokens", &self.oauth2_tokens)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for VarContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names = |vars: &Vars| vars.keys().cloned().collect::<Vec<_>>();
        let process_len = self
            .process
            .get("env")
            .and_then(Value::as_object)
            .map_or(0, serde_json::Map::len);
        f.debug_struct("VarContext")
            .field("root", &self.root)
            .field("environment", &self.environment)
            .field("global_layer", &names(&self.global_layer))
            .field("global_sensitive", &self.global_sensitive)
            .field("collection", &names(&self.collection))
            .field("env_layer", &names(&self.env_layer))
            .field("env_sensitive", &self.env_sensitive)
            .field("folders", &self.folders.keys().collect::<Vec<_>>())
            .field("process_env_len", &process_len)
            .field("warnings", &self.warnings)
            .field("tokens", &self.tokens)
            .finish_non_exhaustive()
    }
}

/// Builds `{"env": ...}` from the given pairs or the real environment, with `<root>/.env` on top,
/// and lists the keys the `.env` file set.
fn process_namespace(
    root: &Path,
    given: Option<Vec<(String, String)>>,
) -> Result<(Value, Vec<String>), ContextError> {
    let pairs = given.unwrap_or_else(|| {
        std::env::vars_os()
            .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
            .collect()
    });
    let mut env: Vars = pairs
        .into_iter()
        .map(|(k, v)| (k, Value::String(v)))
        .collect();
    let path = root.join(".env");
    let dot_env = |message: String| ContextError::DotEnv {
        path: path.clone(),
        message,
    };
    let mut dotenv_keys = Vec::new();
    match read_regular(&path) {
        Ok(bytes) => {
            let text = String::from_utf8(bytes)
                .map_err(|_| dot_env("file is not valid UTF-8".to_owned()))?;
            for (k, v) in parse_dotenv(&text) {
                dotenv_keys.push(k.clone());
                env.insert(k, Value::String(v));
            }
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(dot_env(e.to_string())),
    }
    let mut process = Vars::new();
    process.insert("env".to_owned(), Value::Object(env));
    Ok((Value::Object(process), dotenv_keys))
}

/// `process` and `keys` with `overrides` set in `process.env` and counted as `.env` keys.
fn with_dotenv_overrides(
    mut process: Value,
    mut keys: Vec<String>,
    overrides: Vec<(String, String)>,
) -> (Value, Vec<String>) {
    if let Some(env) = process.get_mut("env").and_then(Value::as_object_mut) {
        for (key, value) in overrides {
            if !keys.contains(&key) {
                keys.push(key.clone());
            }
            env.insert(key, Value::String(value));
        }
    }
    (process, keys)
}

/// Why `name` is not among the loaded environments: its file failed to load, or there is none.
fn missing_environment(collection: &Collection, name: &str) -> ContextError {
    let file = collection
        .root()
        .join("environments")
        .join(format!("{name}.yml"));
    match collection.load_errors().iter().find(|e| e.path() == file) {
        Some(e) => ContextError::EnvironmentFailedToLoad {
            name: name.to_owned(),
            message: e.message().to_owned(),
        },
        None => ContextError::EnvironmentNotFound {
            name: name.to_owned(),
            available: collection
                .environments()
                .iter()
                .map(|e| e.name().to_owned())
                .collect(),
        },
    }
}

/// Why `name` is not among the loaded global environments: its file failed to load, or there is none.
fn missing_global_environment(globals: &GlobalEnvironments, name: &str) -> ContextError {
    let file = globals.dir().join(format!("{name}.yml"));
    match globals.load_errors().iter().find(|e| e.path() == file) {
        Some(e) => ContextError::GlobalEnvironmentFailedToLoad {
            name: name.to_owned(),
            message: e.message().to_owned(),
        },
        None => ContextError::GlobalEnvironmentNotFound {
            name: name.to_owned(),
            available: globals
                .environments()
                .iter()
                .map(|e| e.name().to_owned())
                .collect(),
        },
    }
}

/// A secret the store could not read while building a context; the variable stays unset.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SecretReadFailure {
    /// The environment that declares the secret.
    pub environment: String,
    /// The secret variable's name.
    pub name: String,
    /// Whether the environment is the selected global one.
    pub global: bool,
    /// What the store reported.
    pub error: SecretError,
}

impl fmt::Display for SecretReadFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = if self.global {
            "global environment"
        } else {
            "environment"
        };
        write!(
            f,
            "secret `{}` of {kind} `{}` could not be read: {}",
            self.name, self.environment, self.error
        )
    }
}

/// An environment-level layer after its overrides and the secret store.
struct FilledLayer {
    vars: Vars,
    /// Enabled secrets with no stored value, left out of `vars`.
    unstored: BTreeSet<String>,
    /// Names whose final value came from the store or a sensitive override, and the unstored
    /// secrets, whose later value (a script's write) is a secret too.
    sensitive: BTreeSet<String>,
}

/// Applies `overrides` over `env`, then reads each remaining secret from the store under
/// `key(owner, name)`. A store failure is returned as (owner, name, message) when strict, else
/// recorded as a [`SecretReadFailure`] and its warning text.
fn fill_layer(
    env: ResolvedEnv,
    overrides: Vec<(String, String, bool)>,
    (store, strict): (Option<&dyn SecretStore>, bool),
    key: impl Fn(&str, &str) -> SecretKey,
    global: bool,
    (warnings, failures): (&mut Vec<String>, &mut Vec<SecretReadFailure>),
) -> Result<FilledLayer, (String, String, String)> {
    let ResolvedEnv {
        mut vars,
        secret_owners,
        ..
    } = env;
    let mut sensitive = BTreeSet::new();
    for (name, value, flagged) in overrides {
        if flagged {
            sensitive.insert(name.clone());
        } else {
            sensitive.remove(&name);
        }
        vars.insert(name, EnvValue::Plain(Value::String(value)));
    }
    // Whatever is still a secret after the overrides takes its value from the store.
    if let Some(store) = store {
        for (name, value) in &mut vars {
            if *value != EnvValue::Secret {
                continue;
            }
            let Some(owner) = secret_owners.get(name) else {
                continue;
            };
            match store.get(&key(owner, name)) {
                Ok(Some(text)) => {
                    *value = EnvValue::Plain(Value::String(text));
                    sensitive.insert(name.clone());
                }
                Ok(None) => {}
                Err(error) if strict => {
                    return Err((owner.clone(), name.clone(), error.to_string()));
                }
                Err(error) => {
                    let failure = SecretReadFailure {
                        environment: owner.clone(),
                        name: name.clone(),
                        global,
                        error,
                    };
                    warnings.push(failure.to_string());
                    failures.push(failure);
                }
            }
        }
    }
    let mut unstored = BTreeSet::new();
    let vars = vars
        .into_iter()
        .filter_map(|(name, value)| match value {
            EnvValue::Plain(value) => Some((name, value)),
            EnvValue::Secret => {
                sensitive.insert(name.clone());
                unstored.insert(name);
                None
            }
        })
        .collect();
    Ok(FilledLayer {
        vars,
        unstored,
        sensitive,
    })
}

/// Lays the runtime snapshot over `vars`; a captured `undefined` removes the name.
fn apply_captured(vars: &mut Vars, captured: &BTreeMap<String, Option<Value>>) {
    for (name, value) in captured {
        match value {
            Some(value) => vars.insert(name.clone(), value.clone()),
            None => vars.remove(name),
        };
    }
}

/// Collects every string inside `value`, nested arrays and objects included.
pub(crate) fn json_strings<'a>(value: &'a Value, out: &mut Vec<&'a str>) {
    match value {
        Value::String(text) => out.push(text),
        Value::Array(items) => items.iter().for_each(|item| json_strings(item, out)),
        Value::Object(map) => map.values().for_each(|item| json_strings(item, out)),
        _ => {}
    }
}

/// Collects every string value inside the YAML `value`, nested sequences and mappings included.
pub(crate) fn yaml_strings<'a>(value: &'a yaml_serde::Value, out: &mut Vec<&'a str>) {
    match value {
        yaml_serde::Value::String(text) => out.push(text),
        yaml_serde::Value::Sequence(items) => items.iter().for_each(|item| yaml_strings(item, out)),
        yaml_serde::Value::Mapping(map) => map.values().for_each(|item| yaml_strings(item, out)),
        _ => {}
    }
}

/// The token fields that are safe to show; every other `$oauth2` name is sensitive.
fn is_public_oauth2_field(name: &str) -> bool {
    ["token_type", "expires_in", "scope", "created_at"]
        .iter()
        .any(|field| name.ends_with(&format!(".{field}")))
}

#[cfg(test)]
mod secret_values_tests {
    use super::*;

    #[test]
    fn values_a_script_tainted_are_listed_above_the_floor() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("opencollection.yml"),
            "opencollection: 1.0.0\ninfo:\n  name: c\n",
        )
        .unwrap();
        let c = Collection::open(dir.path()).unwrap();
        let runtime = RuntimeVars::default();
        runtime.taint(&["tainted-by-script".to_owned(), "abc".to_owned()]);
        let ctx = VarContext::new(&c, ContextOptions::new().runtime_vars(runtime)).unwrap();
        let values = ctx.secret_values();
        assert!(
            values.contains(&"tainted-by-script".to_owned()),
            "{values:?}"
        );
        assert!(!values.contains(&"abc".to_owned()));
    }

    /// A one-request collection and a context whose runtime scope the test controls.
    fn marked_fixture(runtime: &RuntimeVars) -> (tempfile::TempDir, Collection, VarContext) {
        let dir = tempfile::TempDir::new().unwrap();
        let files = [
            (
                "opencollection.yml",
                "opencollection: 1.0.0\ninfo:\n  name: c\n",
            ),
            (
                "r.yml",
                "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: https://x.test\n",
            ),
        ];
        for (rel, text) in files {
            std::fs::write(dir.path().join(rel), text).unwrap();
        }
        let c = Collection::open(dir.path()).unwrap();
        let options = ContextOptions::new()
            .sensitive_env_var("token", "env-secret")
            .runtime_vars(runtime.clone());
        let ctx = VarContext::new(&c, options).unwrap();
        (dir, c, ctx)
    }

    fn request_of(c: &Collection) -> &RequestFile {
        &c.tree().requests()[0]
    }

    #[test]
    fn a_marked_captured_name_is_secret_without_a_value() {
        let runtime = RuntimeVars::default();
        runtime.set_text("session", "plain-looking");
        runtime.mark_sensitive("session");
        let (_dir, c, ctx) = marked_fixture(&runtime);
        let info = ctx.variables(request_of(&c)).get("session").unwrap();
        assert!(info.is_secret());
        assert_eq!(info.value(), None);
    }

    #[test]
    fn an_unmarked_captured_name_keeps_its_value() {
        let runtime = RuntimeVars::default();
        runtime.set_text("session", "plain-looking");
        let (_dir, c, ctx) = marked_fixture(&runtime);
        let info = ctx.variables(request_of(&c)).get("session").unwrap();
        assert!(!info.is_secret());
        assert_eq!(info.value(), Some("plain-looking"));
    }

    #[test]
    fn a_value_containing_a_tainted_one_is_secret() {
        let runtime = RuntimeVars::default();
        runtime.set_text("copy", "Bearer tainted-by-script!");
        runtime.set_text("other", "unrelated");
        runtime.taint(&["tainted-by-script".to_owned()]);
        let (_dir, c, ctx) = marked_fixture(&runtime);
        let set = ctx.variables(request_of(&c));
        let copy = set.get("copy").unwrap();
        assert!(copy.is_secret());
        assert_eq!(copy.value(), None);
        assert_eq!(set.get("other").unwrap().value(), Some("unrelated"));
    }

    #[test]
    fn a_marked_capture_shadowing_an_env_secret_stays_secret() {
        let runtime = RuntimeVars::default();
        runtime.set_text("token", "recaptured");
        let (_dir, c, ctx) = marked_fixture(&runtime);
        let set = ctx.variables(request_of(&c));
        // Unmarked, the capture replaces the environment secret and shows its value.
        assert_eq!(set.get("token").unwrap().value(), Some("recaptured"));
        runtime.mark_sensitive("token");
        let marked = set.get("token").unwrap();
        assert!(marked.is_secret());
        assert_eq!(marked.value(), None);
    }

    #[test]
    fn the_redaction_set_includes_a_marked_captured_name() {
        let runtime = RuntimeVars::default();
        runtime.set_text("session", "v");
        runtime.set_text("open", "v");
        runtime.mark_sensitive("session");
        let (_dir, c, ctx) = marked_fixture(&runtime);
        let (_, names) = ctx.lookup_for(request_of(&c));
        assert!(names.contains("session"), "{names:?}");
        assert!(!names.contains("open"), "{names:?}");
    }
}
