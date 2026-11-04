//! The MCP server: one collection, reloaded from disk before every tool call.

use std::collections::HashSet;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use apinata_core::import::{FetchError, SourceFetch, default_import_name, fetch_source};
use apinata_core::mock::{self as core_mock, MockHandle, MockOptions};
use apinata_core::{
    ApprovalInbox, AuthorizeOptions, CancelHandle, Collection, CookieJar, EnvOverlay, ExecError,
    ExecOptions, ExecutionMode, Executor, ItemPath, OAuth2Error, Outcome, RunError, RunEvent,
    RunOptions, RuntimeVars, ScriptPhase, ScriptTextKind, SecretStore, TokenCache,
    assertion_sensitive_values, plan_script_texts, request_secrets, script_texts, send_one,
};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::handler::server::wrapper::Json;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, ContentBlock, GetPromptRequestParams,
    GetPromptResponse, ListPromptsResult, ListResourcesResult, PaginatedRequestParams,
    ProgressNotificationParam, ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult,
    ResourceContents, ServerCapabilities, ServerConfig, SubscribeRequestParams,
    UnsubscribeRequestParams,
};
use rmcp::service::{NotificationContext, RequestContext};
use rmcp::{ErrorData, RoleServer, ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::apply::{ConfigPatch, RequestPatch};
use crate::check::{self, CheckArgs};
use crate::cookies::{self, ClearArgs, Cleared, CookieList};
use crate::dto::{ConfigDoc, EnvironmentDoc, RequestDoc};
use crate::envs::{self, DeletedEnv, EnvNameResult, RenameEnvArgs, UpdateEnvArgs};
use crate::examples::{
    self, LastSend, LastSends, SaveExampleArgs, SavedExample, ScrubArgs, ScrubbedUserinfo,
};
use crate::generate::{self, GenerateArgs, GeneratedCode};
use crate::git::{self, GitDiffDoc, GitStatusDoc};
use crate::import_collection::{
    self, FETCH_LABEL, ImportCollectionArgs, ImportedCollections, NoSecrets,
};
use crate::imports::{self, ImportRequestArgs, ImportedRequestResult};
use crate::items::{self, ItemList, ListItemsArgs};
use crate::mock::{self, MockHits, MockHitsArgs, MockStartArgs, MockStarted, MockStopped};
use crate::oauth2::{self, OAuth2TokenArgs, OAuth2TokenDoc, TokenAction};
use crate::params::Parameters;
use crate::policy::{HostTrust, TrustedHosts};
use crate::prompts;
use crate::reads::{self, NameArgs, PathArgs};
use crate::redact::Redactor;
use crate::replace::{self, ReplaceArgs, ReplaceDoc};
use crate::resources::{self, ReadError};
use crate::run::{self, RunArgs, RunResult};
use crate::scripts;
use crate::search::{self, SearchArgs, SearchDoc};
use crate::send::{self, SendArgs, SendResult};
use crate::tree::{
    self, CreateFolderArgs, CreateRequestArgs, Deleted, ItemPathResult, MoveArgs, NameItemArgs,
    RestoredItem, RestoredPrune,
};
use crate::writes;
use crate::{McpError, McpOptions};
use apinata_core::{KEYCHAIN_TIMEOUT, LazyKeychain, TimeoutStore};

/// What a client sees when a tool panics; the payload may quote a secret, so it is fixed.
const PANICKED: &str = "internal error: the tool failed unexpectedly";

/// Tools that are not read-only (they send, run or change session state) yet write no collection
/// file, so a call to one never changes the resource list.
const NO_FILE_WRITES: &[&str] = &[
    "send_request",
    "run_folder",
    "mock_start",
    "mock_stop",
    "clear_cookies",
    "oauth2_token",
    "set_runtime_variable",
    "clear_runtime_variables",
];

/// Tools that write collection files and so announce a resource change after every call. Only
/// the policy test reads it: any other non-read-only tool notifies by default.
#[cfg(test)]
const WRITES_FILES: &[&str] = &[
    "replace",
    "update_request",
    "update_folder",
    "create_request",
    "import_request",
    "import_collection",
    "create_folder",
    "clone_request",
    "clone_folder",
    "rename_item",
    "move_item",
    "delete_item",
    "restore_item",
    "restore_pruned",
    "create_environment",
    "duplicate_environment",
    "rename_environment",
    "delete_environment",
    "update_environment",
    "save_example",
    "scrub_example_userinfo",
];

/// The most response body the executor reads, as the app does.
const BODY_CAP: u64 = 10 * 1024 * 1024;

/// Serves the collection at [`McpOptions::root`] to one MCP client.
#[derive(Clone)]
pub struct ApinataServer {
    collection: Arc<Mutex<Collection>>,
    options: Arc<McpOptions>,
    store: Arc<dyn SecretStore>,
    trust: HostTrust,
    /// Where refused hosts are filed for the user to approve; `None` without a data directory.
    inbox: Option<ApprovalInbox>,
    /// One client for every send, so cookies persist across calls; it asks `trust` per hop.
    exec: Arc<Executor>,
    /// The options `exec` was built from, with the shared jar, so a run can build its own
    /// executor over the same jar and trust.
    exec_options: ExecOptions,
    /// Every sensitive value seen this session; each tool result is scrubbed of all of them.
    seen: Arc<std::sync::Mutex<Redactor>>,
    /// Variables captured after responses; they last as long as the server.
    runtime: RuntimeVars,
    /// Script edits to environment and collection variables; they last as long as `runtime`.
    overlay: EnvOverlay,
    /// OAuth 2.0 tokens, shared by every send this session.
    tokens: TokenCache,
    /// The one loopback mock this session may run; dropping the last clone stops it.
    mock: Arc<Mutex<Option<MockHandle>>>,
    /// Each request's last completed send, unredacted, for `save_example` to scrub and save.
    last_sends: LastSends,
    /// The resource URIs the client subscribed to; shared by every clone of the server.
    subscriptions: Arc<std::sync::Mutex<HashSet<String>>>,
    /// What `resources/list` last showed the client, shared by the tool path and the watcher so
    /// one change is announced once.
    listed: Arc<std::sync::Mutex<Vec<String>>>,
    /// Mutating tool calls in flight; the watcher snapshots only when none is.
    writes: crate::gate::WriteGate,
    /// The file watcher feeding resource notifications; started once, on the first `initialized`.
    watcher: Arc<std::sync::Mutex<Option<crate::watch::WatcherHandle>>>,
    tool_router: ToolRouter<Self>,
}

/// What `get_collection` reports: identity, environments and files that failed to load.
#[derive(Serialize, JsonSchema)]
pub(crate) struct CollectionInfo {
    name: String,
    root: String,
    environments: Vec<String>,
    default_environment: Option<String>,
    load_errors: Vec<String>,
}

/// What `list_runtime_variables` reports.
#[derive(Serialize, JsonSchema)]
pub(crate) struct RuntimeVarList {
    variables: Vec<RuntimeVarDoc>,
}

/// `set_runtime_variable`'s arguments.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SetRuntimeArgs {
    /// The variable's name, as `{{name}}` refers to it.
    name: String,
    /// The value as text.
    value: String,
}

/// What `set_runtime_variable` reports; the value is never echoed.
#[derive(Serialize, JsonSchema)]
pub(crate) struct RuntimeVarSet {
    name: String,
    /// Whether the name already held a value.
    replaced: bool,
}

/// `restore_pruned`'s arguments.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RestorePrunedArgs {
    /// The prune to put back, by the id a refusal lists; the newest prune when absent.
    #[serde(default)]
    batch: Option<String>,
}

/// `clear_runtime_variables`'s arguments.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClearRuntimeArgs {
    /// The names to clear; every runtime variable when absent.
    names: Option<Vec<String>>,
}

/// What `clear_runtime_variables` reports.
#[derive(Serialize, JsonSchema)]
pub(crate) struct RuntimeVarsCleared {
    /// How many variables were removed.
    cleared: usize,
}

/// One runtime variable; `value` is `None` for undefined.
#[derive(Serialize, JsonSchema)]
pub(crate) struct RuntimeVarDoc {
    name: String,
    value: Option<String>,
}

impl ApinataServer {
    /// Open the collection at `options.root()`, keeping secret values in the platform keychain,
    /// opened on first use and bounded per call; fails with [`McpError::Open`] when it cannot.
    pub fn new(options: McpOptions) -> Result<ApinataServer, McpError> {
        let keychain = TimeoutStore::new(Arc::new(LazyKeychain::default()), KEYCHAIN_TIMEOUT);
        ApinataServer::with_store(options, Arc::new(keychain))
    }

    /// As [`ApinataServer::new`], keeping secret values in `store` instead of the platform
    /// keychain.
    pub fn with_store(
        options: McpOptions,
        store: Arc<dyn SecretStore>,
    ) -> Result<ApinataServer, McpError> {
        let collection = Collection::open(options.root())?;
        let trust = HostTrust::new(options.root(), options.trust_path());
        let mut exec_options = ExecOptions::new()
            .insecure(options.is_insecure())
            .http_version(options.http_version_policy())
            .max_body_bytes(Some(BODY_CAP))
            .host_policy(trust.policy())
            .confine_files(collection.root())
            .cookie_jar(Arc::new(CookieJar::default()));
        if let Some(settings) = options.network_settings() {
            exec_options = exec_options.network(settings.clone());
        }
        let exec =
            Executor::new(exec_options.clone()).map_err(|e| McpError::Client(e.to_string()))?;
        let inbox = options.approval_path().map(ApprovalInbox::new);
        let mut seen = Redactor::default();
        seen.add_values(options.env_vars().iter().map(|(_, value)| value));
        let tokens = match options.token_cache_path() {
            Some(path) => TokenCache::open(path, Arc::clone(&store)),
            None => TokenCache::in_memory(),
        };
        Ok(ApinataServer {
            collection: Arc::new(Mutex::new(collection)),
            options: Arc::new(options),
            store,
            trust,
            inbox,
            exec: Arc::new(exec),
            exec_options,
            seen: Arc::new(std::sync::Mutex::new(seen)),
            runtime: RuntimeVars::new(),
            overlay: EnvOverlay::new(),
            tokens,
            mock: Arc::new(Mutex::new(None)),
            last_sends: LastSends::default(),
            subscriptions: Arc::new(std::sync::Mutex::new(HashSet::new())),
            listed: Arc::new(std::sync::Mutex::new(Vec::new())),
            writes: crate::gate::WriteGate::default(),
            watcher: Arc::new(std::sync::Mutex::new(None)),
            tool_router: Self::tool_router(),
        })
    }

    /// Accept `uri` only when it names an item the collection holds as it is now.
    async fn check_resource_uri(&self, uri: &str) -> Result<(), ErrorData> {
        let uri = uri.to_owned();
        let known = self
            .with_collection(move |c| {
                Ok(resources::parse(&uri, c.name()).is_some_and(|t| resources::exists(c, &t)))
            })
            .await
            .map_err(|e| self.scrub_error(ErrorData::internal_error(e, None)))?;
        if known {
            Ok(())
        } else {
            Err(ErrorData::invalid_params(
                "not an existing resource of this collection",
                None,
            ))
        }
    }

    /// A clone with its own empty watcher slot, for the notifier task: the task is owned by the
    /// handle in the real slot, so sharing that slot would make a reference cycle.
    pub(crate) fn without_watcher(&self) -> Self {
        let mut clone = self.clone();
        clone.watcher = Arc::new(std::sync::Mutex::new(None));
        clone
    }

    /// The gate counting mutating tool calls in flight.
    pub(crate) fn write_gate(&self) -> crate::gate::WriteGate {
        self.writes.clone()
    }

    /// Remember `listed` as what the client last saw; true when it differs from before.
    pub(crate) fn record_listing(&self, listed: Vec<String>) -> bool {
        let mut last = self.listed.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::replace(&mut *last, listed.clone()) != listed
    }

    /// The resource URIs the client is subscribed to.
    pub(crate) fn subscribed(&self) -> HashSet<String> {
        self.subscriptions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Add `redactor`'s values to those every later tool result is scrubbed of.
    pub(crate) fn remember(&self, redactor: &Redactor) {
        self.seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .merge(redactor);
    }

    /// Add `values` to those every later result is scrubbed of.
    pub(crate) fn learn(&self, values: Vec<String>) {
        let mut learned = Redactor::default();
        learned.add_values(values);
        self.remember(&learned);
    }

    /// A copy of the session redactor, so scrubbing does not hold the lock.
    /// It includes what an in-flight script has read, which the session learns only on return.
    pub(crate) fn seen(&self) -> Redactor {
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner()).clone();
        seen.add_values(self.runtime.tainted());
        seen
    }

    // Structured content is scrubbed as JSON and the text block rebuilt from it, as rmcp's
    // `CallToolResult::structured` builds it: scrubbing the text on its own would miss values
    // whose quotes it escapes.
    fn scrub(&self, response: CallToolResponse) -> CallToolResponse {
        let CallToolResponse::Complete(mut result) = response else {
            return response;
        };
        let seen = self.seen();
        if let Some(value) = result.structured_content.as_mut() {
            seen.scrub_json(value);
            result.content = vec![ContentBlock::text(value.to_string())];
        } else {
            for block in &mut result.content {
                if let ContentBlock::Text(text) = block {
                    text.text = seen.scrub(&text.text);
                }
            }
        }
        CallToolResponse::Complete(result)
    }

    /// `error` with every value seen this session scrubbed from its message and data.
    pub(crate) fn scrub_error(&self, mut error: ErrorData) -> ErrorData {
        let seen = self.seen();
        error.message = seen.scrub(&error.message).into();
        if let Some(data) = error.data.as_mut() {
            seen.scrub_json(data);
        }
        error
    }

    /// Reload the collection from disk, then run `f` on it. Core calls block on file IO, so
    /// both run on the blocking pool while this call holds the mutex for their whole duration.
    pub(crate) async fn with_collection<T, F>(&self, f: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(&mut Collection) -> Result<T, String> + Send + 'static,
    {
        let mut guard = Arc::clone(&self.collection).lock_owned().await;
        tokio::task::spawn_blocking(move || {
            guard.reload();
            let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| f(&mut guard)));
            outcome.unwrap_or_else(|_| {
                // A panic can leave drafts that a reload keeps; reopening drops them.
                if let Ok(fresh) = Collection::open(guard.root()) {
                    *guard = fresh;
                }
                Err(PANICKED.to_owned())
            })
        })
        .await
        .map_err(|_| PANICKED.to_owned())?
    }
}

impl ApinataServer {
    // One token action, then the fresh status, as the app's token panel does. The reply is built
    // from the status alone; errors go through the send's messages and host refusals.
    async fn token_action(
        &self,
        args: OAuth2TokenArgs,
        ct: CancellationToken,
    ) -> Result<OAuth2TokenDoc, String> {
        let (options, store) = (Arc::clone(&self.options), Arc::clone(&self.store));
        let (runtime, tokens) = (self.runtime.clone(), self.tokens.clone());
        let overlay = self.overlay.clone();
        let (raw, action) = (args.path.clone(), args.action);
        let (ctx, owned) = self
            .with_collection(move |c| {
                let env = args.environment.as_deref();
                let ctx = send::context(
                    c, env, &args.vars, &options, store, runtime, tokens, overlay,
                )?;
                Ok((ctx, oauth2::Owned::of(c, &args.path)?))
            })
            .await?;
        let target = owned.target();
        if !apinata_core::is_oauth2(target, &ctx) {
            return Err(OAuth2Error::NotOAuth2.to_string());
        }
        // Registered first, so a refusal quoting a secret is scrubbed.
        if let Some(file) = owned.file() {
            self.learn(assertion_sensitive_values(file, &ctx));
        }
        self.tokens.load().await;
        let cancel = CancelHandle::new();
        let watcher = {
            let (ct, cancel) = (ct.clone(), cancel.clone());
            tokio::spawn(async move {
                ct.cancelled().await;
                cancel.cancel();
            })
        };
        let acted = match action {
            TokenAction::Status => Ok(None),
            TokenAction::Fetch => {
                self.trust.refresh();
                let how = AuthorizeOptions::new();
                let fetched = self.exec.fetch_token(target, &ctx, &how, &cancel).await;
                fetched.map(|_| None)
            }
            TokenAction::Refresh => {
                self.trust.refresh();
                let refreshed = self.exec.refresh_token(target, &ctx, &cancel).await;
                refreshed.map(|_| None)
            }
            TokenAction::Clear => apinata_core::clear_oauth2_token(target, &ctx)
                .await
                .map(Some),
        };
        watcher.abort();
        if ct.is_cancelled() {
            return Err(OAuth2Error::Cancelled.to_string());
        }
        let removed = acted.map_err(|e| match e {
            OAuth2Error::Exec(e) => {
                let label = owned.label(&raw);
                self.trust
                    .refusal(&e, label, self.inbox.as_ref(), &self.seen())
            }
            OAuth2Error::Cancelled => OAuth2Error::Cancelled.to_string(),
            e => oauth2::message(&e, &owned, &ctx),
        })?;
        let status = apinata_core::oauth2_status(target, &ctx)
            .map_err(|e| oauth2::message(&e, &owned, &ctx))?;
        Ok(oauth2::doc(
            action,
            status,
            removed,
            &self.tokens.persistence(),
        ))
    }
}

#[tool_router]
impl ApinataServer {
    #[tool(
        description = "Describe the served collection: its name, root directory, environments, \
                       the default environment and any files that failed to load.",
        annotations(read_only_hint = true)
    )]
    async fn get_collection(&self) -> Result<Json<CollectionInfo>, String> {
        let default_environment = self.options.default_env().map(str::to_owned);
        self.with_collection(move |collection| {
            Ok(Json(CollectionInfo {
                name: collection.name().to_owned(),
                root: collection.root().display().to_string(),
                environments: collection
                    .environments()
                    .iter()
                    .map(|env| env.name().to_owned())
                    .collect(),
                default_environment,
                load_errors: collection
                    .load_errors()
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
            }))
        })
        .await
    }

    #[tool(
        description = "List the collection's folders, requests and other items as a tree, in the \
                       order the app shows them. A file that fails to load is listed with kind \
                       `broken` and an `error` saying why; a folder whose `folder.yml` fails \
                       carries an `error` too. Give `path` to list one folder. On a large \
                       collection page instead of listing everything: call with `depth` 1 \
                       first (direct items only; each folder then has a `child_count` and no \
                       `items`), then drill into a folder with `path`. `limit` (at most 500) and \
                       `offset` page a folder's direct items; the reply then has `total` and \
                       `has_more`, and while `has_more` is true call again with `offset` \
                       raised by the page size.",
        annotations(read_only_hint = true)
    )]
    async fn list_items(
        &self,
        Parameters(args): Parameters<ListItemsArgs>,
    ) -> Result<Json<ItemList>, String> {
        self.with_collection(move |collection| items::list(collection, &args).map(Json))
            .await
    }

    #[tool(
        description = "Search the text of the collection's requests (names, URLs, path params, \
                       headers, bodies, scripts and docs). Matching ignores letter case unless \
                       `case` is true, and `query` is literal text unless `regex` is true, \
                       when it is an ECMAScript regular expression (an invalid one is an \
                       error). Returns at most 500 hits in tree order, with `capped` set when \
                       more matched. To read past 500, give `limit` and `offset`: the reply \
                       then has `total` and `has_more` instead of `capped`, and while \
                       `has_more` is true call again with `offset` raised by the page size. Variables are matched as written, and secret \
                       variable values are never searched.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn search(
        &self,
        Parameters(args): Parameters<SearchArgs>,
    ) -> Result<Json<SearchDoc>, String> {
        self.with_collection(move |c| search::search(c, &args).map(Json))
            .await
    }

    #[tool(
        description = "Check the collection without sending anything: undefined variables, \
                       missing files and certificates, script syntax errors, unparsable \
                       items and duplicate sequence numbers. `env` limits the check to one \
                       environment (an unknown one is an error). Returns the same JSON as \
                       `apinata check --json`. A secret that has a stored value is not reported; secret values are never returned.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn check_collection(
        &self,
        Parameters(args): Parameters<CheckArgs>,
    ) -> Result<Json<serde_json::Map<String, serde_json::Value>>, String> {
        let store = Arc::clone(&self.store);
        self.with_collection(move |c| check::check(c, &args, store).map(Json))
            .await
    }

    #[tool(
        description = "Find and replace text across the collection's requests (names, URLs, \
                       path params, headers, bodies, scripts and docs). dry_run defaults to \
                       true: nothing is written until dry_run is false, and a dry run returns \
                       the hits a real run would change. Matching ignores letter case unless \
                       `case` is true, and `pattern` is literal text unless `regex` is true, \
                       when it is an ECMAScript regular expression whose `$1` groups work in \
                       `replacement` (an invalid one is an error). Secret variable values are \
                       never matched. Edits go through the same writer as the editor, so \
                       unchanged bytes stay identical. Changed scripts get new hashes and ask \
                       the user to trust them again. Requests with unsaved edits in the app \
                       are not visible: only the files are read, and the app skips those.",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    /// Notifies a list change even on a dry run: over-notifying is intended and harmless.
    async fn replace(
        &self,
        Parameters(args): Parameters<ReplaceArgs>,
    ) -> Result<Json<ReplaceDoc>, String> {
        self.with_collection(move |c| replace::replace(c, &args).map(Json))
            .await
    }

    #[tool(
        description = "List the hosts this collection may send to without approval (trusted) \
                       and the hosts the user refused (denied), as the Apinata app keeps them.",
        annotations(read_only_hint = true)
    )]
    async fn list_trusted_hosts(&self) -> Json<TrustedHosts> {
        self.trust.refresh();
        Json(self.trust.lists())
    }

    #[tool(
        description = "Start a mock server on 127.0.0.1 that answers with the collection's \
                       saved response examples, and return its URL and routes. Only one mock \
                       runs at a time; stop it with `mock_stop`.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn mock_start(
        &self,
        Parameters(args): Parameters<MockStartArgs>,
    ) -> Result<Json<MockStarted>, String> {
        // Held through `start` so two racing calls cannot both bind.
        let mut slot = self.mock.lock().await;
        if let Some(running) = slot.as_ref() {
            return Err(format!(
                "a mock server is already running at http://{}; call mock_stop first",
                running.addr()
            ));
        }
        let (options, store) = (Arc::clone(&self.options), Arc::clone(&self.store));
        let env = args.env.clone();
        // Fresh runtime state: script-set variables never reach mock routes.
        let ctx = self
            .with_collection(move |c| {
                send::context(
                    c,
                    env.as_deref(),
                    &Default::default(),
                    &options,
                    store,
                    RuntimeVars::new(),
                    TokenCache::in_memory(),
                    EnvOverlay::new(),
                )
                .map_err(|e| match e.strip_suffix("; nothing was sent") {
                    Some(stripped) => stripped.to_owned(),
                    None => e,
                })
            })
            .await?;
        let mut mock_options = MockOptions::new().port(args.port.unwrap_or(0));
        if let Some(ms) = args.delay_ms {
            mock_options = mock_options.delay_ms(ms);
        }
        // Route paths and warnings quote resolved URLs, so register every secret before reporting.
        let mut secrets = Redactor::default();
        secrets.add_values(ctx.secret_values());
        self.remember(&secrets);
        let guard = self.collection.lock().await;
        let handle = core_mock::start(&guard, &ctx, mock_options)
            .await
            .map_err(|e| e.to_string())?;
        drop(guard);
        let started = MockStarted {
            url: format!("http://{}", handle.addr()),
            routes: handle.routes().iter().map(Into::into).collect(),
            warnings: handle.warnings().to_vec(),
        };
        *slot = Some(handle);
        Ok(Json(started))
    }

    #[tool(
        description = "List the requests the running mock server has answered, oldest first. \
                       Pass the previous result's `next` as `since` to see only newer ones.",
        annotations(read_only_hint = true)
    )]
    async fn mock_hits(
        &self,
        Parameters(args): Parameters<MockHitsArgs>,
    ) -> Result<Json<MockHits>, String> {
        // The lock is held only long enough to copy the log.
        let log = self
            .mock
            .lock()
            .await
            .as_ref()
            .map(MockHandle::hits)
            .ok_or_else(|| "no mock server is running".to_owned())?;
        Ok(Json(mock::hits_since(log, args.since.unwrap_or(0))))
    }

    #[tool(
        description = "Stop the running mock server and release its port.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn mock_stop(&self) -> Result<Json<MockStopped>, String> {
        // Taking the handle out means a concurrent `mock_hits` sees nothing running.
        let handle = self
            .mock
            .lock()
            .await
            .take()
            .ok_or_else(|| "no mock server is running".to_owned())?;
        let stopped = MockStopped {
            url: format!("http://{}", handle.addr()),
            hits: handle.hits().len(),
        };
        handle.stop().await;
        Ok(Json(stopped))
    }

    #[tool(
        description = "List the cookies this session's sends have stored: name, domain, path, \
                       expiry (unix seconds) and flags. Values are never shown.",
        annotations(read_only_hint = true)
    )]
    async fn list_cookies(&self) -> Json<CookieList> {
        Json(cookies::list(self.exec.cookies()))
    }

    #[tool(
        description = "List the runtime variables captured from responses this session, by \
                       name; `value` is null for one holding undefined.",
        annotations(read_only_hint = true)
    )]
    async fn list_runtime_variables(&self) -> Json<RuntimeVarList> {
        let variables = self
            .runtime
            .entries()
            .iter()
            .map(|v| RuntimeVarDoc {
                name: v.name().to_owned(),
                value: v.value().map(str::to_owned),
            })
            .collect();
        Json(RuntimeVarList { variables })
    }

    #[tool(
        description = "Set a runtime variable for this session, as a response capture would, so \
                       the next send_request or run_folder resolves `{{name}}` to `value`. It \
                       changes no file and no secret, and lasts until cleared or the server \
                       stops. The result names the variable and never repeats the value.",
        annotations(
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn set_runtime_variable(
        &self,
        Parameters(args): Parameters<SetRuntimeArgs>,
    ) -> Result<Json<RuntimeVarSet>, ErrorData> {
        // A name outside the shared variable-name rule is a caller mistake, not a tool failure.
        if let Some(problem) = apinata_core::variable_name_problem(&args.name) {
            return Err(self.scrub_error(ErrorData::invalid_params(problem, None)));
        }
        let replaced = self.runtime.entries().iter().any(|v| v.name() == args.name);
        self.runtime.set_text(args.name.clone(), args.value);
        Ok(Json(RuntimeVarSet {
            name: args.name,
            replaced,
        }))
    }

    #[tool(
        description = "Remove the named runtime variables, or every one when `names` is absent, \
                       so lower layers show through again. Returns how many were removed.",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn clear_runtime_variables(
        &self,
        Parameters(args): Parameters<ClearRuntimeArgs>,
    ) -> Json<RuntimeVarsCleared> {
        let cleared = match args.names {
            Some(names) => names.iter().filter(|n| self.runtime.unset(n)).count(),
            None => {
                let all = self.runtime.len();
                self.runtime.clear();
                all
            }
        };
        Json(RuntimeVarsCleared { cleared })
    }

    #[tool(
        description = "Remove the cookies stored for `domain`, or every cookie when `domain` is \
                       absent. Returns how many were removed.",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn clear_cookies(&self, Parameters(args): Parameters<ClearArgs>) -> Json<Cleared> {
        Json(cookies::clear(self.exec.cookies(), args.domain.as_deref()))
    }

    #[tool(
        description = "Read one HTTP or GraphQL request file: method, URL, params, headers, \
                       body, auth, variables, assertions, settings, docs, tags and seq. Values \
                       are as written, variables unresolved. `query_mismatch` is present when \
                       the URL's query and the enabled query params disagree, naming the \
                       params; the URL's query is what a send uses. `examples` lists the saved \
                       response examples (name, description, status), when there are any.",
        annotations(read_only_hint = true)
    )]
    async fn read_request(
        &self,
        Parameters(args): Parameters<PathArgs>,
    ) -> Result<Json<RequestDoc>, String> {
        self.with_collection(move |collection| reads::request(collection, &args.path).map(Json))
            .await
    }

    #[tool(
        description = "Write one request as code, resolved against an environment and `vars` \
                       as send_request would be, without sending it. `target` is one of the \
                       ids `apinata gen` takes, such as `curl` or `python-requests`; an \
                       unknown one lists them. Secret values appear as `<secret>`. A missing \
                       variable is an error. An OAuth 2.0 token is used only if already cached; fetch one with \
                       oauth2_token. \
                       `warnings` lists what the snippet cannot reproduce, such as scripts \
                       that are not run; it is empty otherwise.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn generate_code(
        &self,
        Parameters(args): Parameters<GenerateArgs>,
        ct: CancellationToken,
    ) -> Result<Json<GeneratedCode>, String> {
        let target = generate::target(&args.target)?;
        let (options, store) = (Arc::clone(&self.options), Arc::clone(&self.store));
        let (runtime, tokens) = (self.runtime.clone(), self.tokens.clone());
        let overlay = self.overlay.clone();
        let (ctx, file) = self
            .with_collection(move |c| {
                let env = args.environment.as_deref();
                let ctx = send::context(
                    c, env, &args.vars, &options, store, runtime, tokens, overlay,
                )?;
                Ok((ctx, send::request_file(c, &args.path)?))
            })
            .await?;
        // Registered first, so an error quoting a secret is scrubbed.
        let mut learned = Redactor::default();
        learned.add_values(assertion_sensitive_values(&file, &ctx));
        self.remember(&learned);
        let resolved = match generate::resolve_once(&file, &ctx) {
            // A token may sit in the on-disk cache but not in memory: load it from the cache
            // only, so generating code never fetches one, then resolve again.
            Err(e) if generate::needs_cached_token(&e) => {
                let how = AuthorizeOptions::new().cache_only(true);
                let cancel = CancelHandle::new();
                let watcher = {
                    let (ct, cancel) = (ct.clone(), cancel.clone());
                    tokio::spawn(async move {
                        ct.cancelled().await;
                        cancel.cancel();
                    })
                };
                let prepared = self.exec.prepare_auth(&file, &ctx, &how, &cancel).await;
                watcher.abort();
                prepared.map_err(|e| send::oauth2_message(&e, &file, &ctx))?;
                generate::resolve_once(&file, &ctx)
            }
            result => result,
        }
        .map_err(|e| send::resolve_message(&e, &file, &ctx))?;
        let mut found = Redactor::default();
        let code = generate::render(&resolved, target, &mut found);
        self.remember(&found);
        // The session's own redactor scrubs what earlier tools learned as well.
        let code = self.seen().scrub(&code);
        Ok(Json(GeneratedCode {
            target: target.id().to_owned(),
            code,
            warnings: resolved
                .warnings()
                .iter()
                .map(ToString::to_string)
                .collect(),
        }))
    }

    #[tool(
        description = "Show, fetch, refresh or clear the OAuth 2.0 token of a request (`path` \
                       such as `users/get.yml`), a folder (`users`) or the collection (empty \
                       `path`), as the app's token panel does. `status` reads the cache; \
                       `fetch` gets a new token (client credentials or password; a browser \
                       sign-in must be done in the Apinata app); `refresh` uses the cached \
                       refresh token; `clear` forgets the token. The reply describes the token \
                       (times in unix seconds, scope, whether a refresh token is held) and \
                       never returns a token, secret or password. Only local hosts and hosts \
                       the user trusts are reached; others need the user's approval in the \
                       Apinata app.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = true
        )
    )]
    async fn oauth2_token(
        &self,
        Parameters(args): Parameters<OAuth2TokenArgs>,
        ct: CancellationToken,
    ) -> Result<Json<OAuth2TokenDoc>, String> {
        self.token_action(args, ct)
            .await
            .map(Json)
            .map_err(|e| self.seen().scrub(&e))
    }

    #[tool(
        description = "Read the headers, auth, variables and docs a folder shares with its \
                       requests. An empty `path` reads the collection's own.",
        annotations(read_only_hint = true)
    )]
    async fn read_folder(
        &self,
        Parameters(args): Parameters<PathArgs>,
    ) -> Result<Json<ConfigDoc>, String> {
        self.with_collection(move |collection| reads::folder(collection, &args.path).map(Json))
            .await
    }

    #[tool(
        description = "Read one environment's variables. A secret variable shows its name and \
                       that it is secret, never its value.",
        annotations(read_only_hint = true)
    )]
    async fn read_environment(
        &self,
        Parameters(args): Parameters<NameArgs>,
    ) -> Result<Json<EnvironmentDoc>, String> {
        self.with_collection(move |collection| reads::environment(collection, &args.name).map(Json))
            .await
    }

    #[tool(
        description = "List the collection's files changed since the last commit (modified, \
                       added, deleted, untracked, renamed, conflicted), with the branch. Reads \
                       git only; never writes or fetches.",
        annotations(read_only_hint = true)
    )]
    async fn git_status(&self) -> Result<Json<GitStatusDoc>, String> {
        let options = self.options.git_options().clone();
        self.with_collection(move |collection| git::status(collection.root(), &options).map(Json))
            .await
    }

    #[tool(
        description = "Show one collection file's changed request sections and a line diff \
                       against the last commit. Only .yml and .yaml files inside the collection.",
        annotations(read_only_hint = true)
    )]
    async fn git_diff(
        &self,
        Parameters(args): Parameters<PathArgs>,
    ) -> Result<Json<GitDiffDoc>, String> {
        let options = self.options.git_options().clone();
        self.with_collection(move |collection| {
            git::diff(collection.root(), &args.path, &options).map(Json)
        })
        .await
    }

    #[tool(
        description = "Update one HTTP or GraphQL request file. Send `path` and only the \
                       sections to replace, each in read_request's shape; absent sections stay \
                       as they are. A graphql request takes only a graphql body (a `none` body is \
                       refused): clear it with an empty `query`. Returns the request as saved, \
                       whose `query_mismatch` reports a URL and params disagreement left in the \
                       file. Settle one with `sync_query`: `url` keeps the URL's query and \
                       rewrites the query params to match, `params` rebuilds the URL's query from \
                       the enabled query params.",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn update_request(
        &self,
        Parameters(patch): Parameters<RequestPatch>,
    ) -> Result<Json<RequestDoc>, String> {
        self.with_collection(move |collection| writes::update_request(collection, &patch).map(Json))
            .await
    }

    #[tool(
        description = "Update the headers, auth, variables or docs a folder shares with its \
                       requests. Send `path` (empty for the collection itself) and only the \
                       sections to replace, each in read_folder's shape; absent sections stay \
                       as they are. Returns the folder's sections as saved.",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn update_folder(
        &self,
        Parameters(patch): Parameters<ConfigPatch>,
    ) -> Result<Json<ConfigDoc>, String> {
        self.with_collection(move |collection| writes::update_folder(collection, &patch).map(Json))
            .await
    }
    #[tool(
        description = "Create a request named `name` in the folder `parent` (empty for the \
                       root), of `kind` `http` or `graphql` (default: the collection's \
                       request-type preset, else http): a GET, or a POST for graphql. The \
                       URL is `url`, else the collection's preset URL, else empty. The file \
                       name follows the name. Returns its path.",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn create_request(
        &self,
        Parameters(args): Parameters<CreateRequestArgs>,
    ) -> Result<Json<ItemPathResult>, String> {
        self.with_collection(move |c| tree::create_request(c, &args).map(Json))
            .await
    }

    #[tool(
        description = "Create a request named `name` in the folder `parent` (empty for the \
                       root) from `text`, a command in a supported format: `format` is `auto` \
                       (detect) or `curl`. Only parses: nothing is sent and no file the \
                       command names is read. A secret the command carries (for example \
                       --aws-sigv4 with --user) is stored as a secret variable of \
                       `environment` if given, else of the collection's only environment; a \
                       named environment that does not exist stores nothing, and so do \
                       several environments with none named, or none to choose; the note \
                       says why. The file only ever \
                       holds a placeholder. Returns the request's path and a note for each \
                       part that did not map.",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn import_request(
        &self,
        Parameters(args): Parameters<ImportRequestArgs>,
    ) -> Result<Json<ImportedRequestResult>, String> {
        let store = Arc::clone(&self.store);
        self.with_collection(move |c| imports::import_request(c, &args, &*store).map(Json))
            .await
    }

    #[tool(
        description = "Import a collection or API description (Postman, OpenAPI or Swagger, \
                       Insomnia, Hoppscotch, HAR) from `url` (http or https) or from `text`; \
                       give exactly one. Its requests and folders are added to this collection \
                       as a new folder named after the imported collection, inside `into` \
                       (default `imports`, relative to the root, no `..`, no symlinks). \
                       Environments and the imported root's headers, variables, auth and \
                       scripts are not imported and are listed as skipped. Fetching a URL on \
                       a non-local host needs the user's approval in the Apinata app, the \
                       same as send_request; approving a host there also lets send_request \
                       reach it. Returns the folders written, their counts and a note for \
                       each part that did not map. With `update: true` and an OpenAPI \
                       description, a folder is refreshed instead: the collection root, or the \
                       folder `into` names exactly (an earlier import of the description \
                       recorded its baseline there; it is never created): unedited spec-owned fields are refreshed, new \
                       operations are added, and operations that left the spec are reported \
                       as orphaned. `prune: true` also moves unedited orphans to the system \
                       trash (refused, changing nothing, when no trash is available; the \
                       result's \
                       `pruned_destination` is \"trash\" once files were moved); `dry_run: \
                       true` only reports. A collection with no recorded baseline is left \
                       unchanged and a note says so. The result then lists added, changed, \
                       kept, edited, orphaned and pruned paths; `edited_fields` names the \
                       spec-owned fields found edited per request (such as `method`).",
        annotations(destructive_hint = false, open_world_hint = true)
    )]
    async fn import_collection(
        &self,
        Parameters(args): Parameters<ImportCollectionArgs>,
    ) -> Result<Json<ImportedCollections>, String> {
        let plan = import_collection::plan(&args)?;
        let (input, default_name) = match (&args.url, &args.text) {
            (Some(url), _) => {
                // Read the trust file first so a host approved a moment ago passes.
                self.trust.refresh();
                let policy = self.trust.policy();
                // The link goes out the way sends do: same proxy, CA and insecure flag.
                let mut fetch = SourceFetch::default();
                if let Some(settings) = self.options.network_settings() {
                    fetch = fetch.network(settings.clone());
                }
                fetch.insecure = self.options.is_insecure();
                let fetch = fetch.host_policy(move |host| policy(host));
                let bytes = fetch_source(url, &fetch).await.map_err(|e| match e {
                    FetchError::HostNotAllowed { host, hop } => {
                        self.trust
                            .refuse_fetch(&host, hop, FETCH_LABEL, self.inbox.as_ref())
                    }
                    other => format!("cannot fetch the link: {other}"),
                })?;
                (bytes, default_import_name(url))
            }
            (None, text) => (text.clone().unwrap_or_default().into_bytes(), None),
        };
        let bundles = import_collection::parse(&plan, &input, default_name)?;
        let (bin, log) = (self.options.trash_bin(), self.options.trash_record());
        self.with_collection(move |c| {
            if plan.is_update() {
                import_collection::update(c, &plan, &bundles, bin.as_ref(), log)
            } else {
                import_collection::place(c, &plan, &bundles, &NoSecrets)
            }
            .map(Json)
        })
        .await
    }

    #[tool(
        description = "Create a folder named `name` in the folder `parent` (empty for the \
                       root). Returns its path.",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn create_folder(
        &self,
        Parameters(args): Parameters<CreateFolderArgs>,
    ) -> Result<Json<ItemPathResult>, String> {
        self.with_collection(move |c| tree::create_folder(c, &args).map(Json))
            .await
    }

    #[tool(
        description = "Copy the request, or an item of another type, at `path` beside it, named \
                       `name`. Returns the copy's path.",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn clone_request(
        &self,
        Parameters(args): Parameters<NameItemArgs>,
    ) -> Result<Json<ItemPathResult>, String> {
        self.with_collection(move |c| tree::clone_request(c, &args).map(Json))
            .await
    }

    #[tool(
        description = "Copy the folder at `path` with everything in it beside it, named \
                       `name`. Returns the copy's path.",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn clone_folder(
        &self,
        Parameters(args): Parameters<NameItemArgs>,
    ) -> Result<Json<ItemPathResult>, String> {
        self.with_collection(move |c| tree::clone_folder(c, &args).map(Json))
            .await
    }

    #[tool(
        description = "Rename the request or folder at `path` to `name`, renaming its file or \
                       directory to match. Returns the new path.",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn rename_item(
        &self,
        Parameters(args): Parameters<NameItemArgs>,
    ) -> Result<Json<ItemPathResult>, String> {
        self.with_collection(move |c| tree::rename_item(c, &args).map(Json))
            .await
    }

    #[tool(
        description = "Send one request and return its response: status, headers (credentials \
                       redacted), body text, assertion verdicts and the redirect timeline, where each hop's \
                       `cookies` lists the Set-Cookie names it carried and what the jar did \
                       (`stored`, `removed` or a rejection `reason`), never their values; \
                       `script_cookies` lists the same, with the script's `phase`, for cookies \
                       the request's scripts set or removed; a failed send's error ends with one \
                       `set-cookie` line per such cookie. \
                       An event-stream ends at `stream_max_seconds` (default 30) or \
                       `stream_max_events` (default 10000) and reports `stream_end`. \
                       `timeout_secs` (1 to 86400) caps the request's timeout; the lower of it \
                       and the request's own applies. \
                       `body_filter` (a JSONPath for JSON, an XPath for XML) returns only the \
                       matched text, so a large list can be read in parts; `total_bytes` stays \
                       the whole body. A response cut at capture may not parse, which \
                       `body.filter_error` reports. \
                       `warnings` names a URL and params query mismatch, if any; the URL's \
                       query is sent. \
                       `warnings` also reports when credentials in the URL were not sent because \
                       the request has path params. \
                       The server's cookie jar is held in memory only: it lasts for the server \
                       process and is never loaded from or saved to disk. \
                       Only local hosts and hosts the user trusts are sent to.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = true
        )
    )]
    async fn send_request(
        &self,
        Parameters(args): Parameters<SendArgs>,
        ct: CancellationToken,
    ) -> Result<Json<SendResult>, String> {
        // Any earlier response is now stale, so only a send that gets one can be saved.
        if let Some(path) = ItemPath::new(&args.path) {
            (self.last_sends.lock().unwrap_or_else(|e| e.into_inner())).remove(&path);
        }
        let (options, store) = (Arc::clone(&self.options), Arc::clone(&self.store));
        let (runtime, tokens) = (self.runtime.clone(), self.tokens.clone());
        let overlay = self.overlay.clone();
        let max_body = args.max_body_bytes.unwrap_or(send::DEFAULT_BODY_BYTES);
        // The engine is picked from the response, so the check is that the expression is valid
        // in either dialect; a typo then costs no request.
        let filter = args.body_filter.clone();
        if let Some(expression) = &filter {
            apinata_core::validate_filter(expression).map_err(|e| match e {
                apinata_core::BodyFilterError::InvalidExpression { expression, reason } => {
                    format!("invalid body_filter '{expression}': {reason}; nothing was sent")
                }
                other => format!("invalid body_filter '{expression}': {other}; nothing was sent"),
            })?;
        }
        // A capped send gets its own executor over the shared jar, like a cookieless run.
        let exec = match send::timeout_cap(args.timeout_secs)? {
            Some(cap) => Arc::new(
                Executor::new(self.exec_options.clone().timeout_cap(Some(cap)))
                    .map_err(|e| e.to_string())?,
            ),
            None => Arc::clone(&self.exec),
        };
        let limits = send::stream_limits(args.stream_max_seconds, args.stream_max_events);
        let (item, skip) = (args.path.clone(), args.skip_scripts);
        let (ctx, file) = self
            .with_collection(move |c| {
                let env = args.environment.as_deref();
                let ctx = send::context(
                    c, env, &args.vars, &options, store, runtime, tokens, overlay,
                )?;
                Ok((ctx, send::request_file(c, &args.path)?))
            })
            .await?;
        self.trust.refresh();
        // Registered before anything is decided, so a refusal quoting a secret is scrubbed.
        let mut early = Redactor::default();
        early.add_values(assertion_sensitive_values(&file, &ctx));
        self.remember(&early);
        // Scripts run only as approved by exact text; anything else refuses before a byte is sent.
        let hashes: Vec<String> = script_texts(&file, &ctx)
            .into_iter()
            .map(|t| t.hash)
            .collect();
        let scripts = scripts::gate(
            &hashes,
            skip,
            self.options.root(),
            self.options.script_trust_path(),
            self.inbox.as_ref(),
            &item,
        )?;
        let path = ItemPath::new(&item).ok_or_else(|| format!("`{item}` is not an item path"))?;
        // The guard registers each resolved request's secrets and refuses a fronted `Host`,
        // including one a script set.
        let opts = RunOptions::new()
            .stream(limits)
            .execution_mode(ExecutionMode::Standalone)
            .scripts(scripts)
            .guard({
                let seen = Arc::clone(&self.seen);
                Arc::new(move |req| run::host_guard(req, &seen))
            });
        // A client cancel fires core's handle, which stops a token fetch, a script or the send.
        let cancel = CancelHandle::new();
        let watcher = {
            let (ct, cancel) = (ct.clone(), cancel.clone());
            tokio::spawn(async move {
                ct.cancelled().await;
                cancel.cancel();
            })
        };
        let result = send_one(path.clone(), &file, &ctx, &exec, &opts, &cancel).await;
        watcher.abort();
        // Remembered even when cancelled: a script may already have copied a secret into the
        // server's runtime variables, which later tool results show.
        let mut read = Redactor::default();
        read.add_values(result.script_sensitive_values());
        read.add_values(result.derived_credentials());
        self.remember(&read);
        if ct.is_cancelled() || result.outcome == Outcome::Cancelled {
            return Err("cancelled; the request may not have been sent".to_owned());
        }
        // A host refused to a script's own send is asked for like a refused send's host.
        let seen = self.seen();
        let asked: Vec<String> = result
            .script_refusals()
            .iter()
            .map(|e| script_refusal(&self.trust.refusal(e, &item, self.inbox.as_ref(), &seen)))
            .collect();
        if let Some(response) = &result.response {
            // Kept as received with the secrets known now, so a later save_example can scrub it.
            let mut derived = result.derived_credentials().to_vec();
            derived.extend_from_slice(result.script_sensitive_values());
            let secrets = request_secrets(&file, &ctx, &derived).map_err(|e| e.to_string());
            let last = LastSend {
                response: response.clone(),
                secrets,
            };
            (self.last_sends.lock().unwrap_or_else(|e| e.into_inner())).insert(path, last);
            let mut sent = send::send_result(
                &file,
                &ctx,
                &result,
                response,
                max_body,
                filter.as_deref(),
                &seen,
            );
            sent.warnings.extend(asked);
            return Ok(Json(sent));
        }
        // A failed send still says what its scripts did to the jar, as the CLI's lines word it.
        let cookies: Vec<String> = (result.set_cookie_lines().iter())
            .map(|line| seen.scrub(line))
            .collect();
        let message = match result.error {
            Some(RunError::Resolve(e)) => send::resolve_message(&e, &file, &ctx),
            Some(RunError::Exec(e) | RunError::OAuth2(OAuth2Error::Exec(e))) => {
                self.trust.refusal(&e, &item, self.inbox.as_ref(), &seen)
            }
            Some(RunError::OAuth2(e)) => send::oauth2_message(&e, &file, &ctx),
            Some(RunError::Refused { message }) => message,
            Some(e @ RunError::Script { .. }) => format!("{e}; nothing was sent"),
            Some(other) => format!("{other}; nothing was sent"),
            None => "the request was not sent".to_owned(),
        };
        Err(std::iter::once(message)
            .chain(cookies)
            .chain(asked)
            .collect::<Vec<_>>()
            .join("\n"))
    }

    #[tool(
        description = "Save the response the last send_request of `path` got this session as a \
                       named example in the request file, where mock_start serves it and \
                       read_request lists it. Every secret value the request knew is replaced \
                       by its placeholder, cookies and credential headers are masked, and a \
                       binary body holding a secret is left out. Errors when the request was \
                       not sent this session or `name` is blank or taken.",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn save_example(
        &self,
        Parameters(args): Parameters<SaveExampleArgs>,
    ) -> Result<Json<SavedExample>, String> {
        let (path, example, description, withheld) = examples::prepare(&self.last_sends, &args)?;
        self.with_collection(move |c| {
            let (index, name) = examples::save(c, &path, &args.name, &description, &example)?;
            Ok(Json(SavedExample {
                path: args.path,
                name,
                index,
                body_withheld: withheld,
            }))
        })
        .await
    }

    #[tool(
        description = "Remove the username and password from the URLs that saved examples \
                       recorded, in every request file. The requests' own URLs are never \
                       changed. Lists the examples it would change unless `write` is true; \
                       with `write`, only files holding such an example are saved. Reports the \
                       scrubbed URLs, never the removed credentials.",
        annotations(
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn scrub_example_userinfo(
        &self,
        Parameters(args): Parameters<ScrubArgs>,
    ) -> Result<Json<ScrubbedUserinfo>, String> {
        self.with_collection(move |c| examples::scrub(c, args.write).map(Json))
            .await
    }

    #[tool(
        description = "Run every request under the folder `path` (a request file runs alone; \
                       empty runs the whole collection) in the app's order, and return each \
                       outcome, status and assertion verdicts, without bodies. `bail` stops at \
                       the first failure; `tags` and `exclude_tags` filter by request tag and \
                       `delay_ms` pauses between requests. `retries` (0 to 10) sends a request again when the send \
                       fails in transit or the answer is 502, 503 or 504, pausing `retry_delay_ms` (a longer \
                       Retry-After wins); POST and PATCH retry only with `retry_unsafe`, and a retried item \
                       reports `attempts`; a run cancelled during the pause before a retry also \
                       reports `last_attempt_error`, the failure of the attempt before. `fail_on_status` (`error` for \
                       400 and above, `4xx` or `5xx`) fails a request whose final reply is in that range even with \
                       no assertions, and the item reports `status_failure`. Reports progress per request; a \
                       cancel stops the run. The summary counts `unverified` (passed with every check skipped) \
                       and `no_tests` (passed with no check at all), both subsets of `passed`. Event-streams end at `stream_max_seconds` \
                       (default 30) or `stream_max_events` (default 10000). `timeout_secs` (1 to 86400) caps every \
                       request's timeout, the lower of it and the request's own applying. Only local hosts and hosts the user trusts are \
                       sent to. `iterations` (1 to 100) repeats the run and `data` (inline rows, at most \
                       100, never a file) runs it once per row with each key as a variable; the \
                       result then groups requests per iteration with that iteration's data, and \
                       `results` is empty. `disable_cookies` neither stores nor sends cookies \
                       for that run only. A result carries `query_mismatch` when its request's \
                       URL and params disagree; the URL's query is sent. TLS verification \
                       cannot be turned off here: the host policy decides trust, so there is no \
                       `insecure` option.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = true
        )
    )]
    async fn run_folder(
        &self,
        Parameters(args): Parameters<RunArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<RunResult>, String> {
        let (options, store) = (Arc::clone(&self.options), Arc::clone(&self.store));
        let (runtime, tokens) = (self.runtime.clone(), self.tokens.clone());
        let overlay = self.overlay.clone();
        let (data, count) = args.iteration_plan()?;
        let (retries, retry_delay, retry_unsafe) = args.retry_plan()?;
        let status_rule = args.status_rule()?;
        // A cookieless run gets its own executor over the shared jar with both switches off, so
        // nothing shared is mutated and concurrent sends keep their cookies.
        let cap = send::timeout_cap(args.timeout_secs)?;
        let exec = if args.disable_cookies || cap.is_some() {
            let mut options = self.exec_options.clone().timeout_cap(cap);
            if args.disable_cookies {
                options = options.store_cookies(false).send_cookies(false);
            }
            Arc::new(Executor::new(options).map_err(|e| e.to_string())?)
        } else {
            Arc::clone(&self.exec)
        };
        let (bail, skip) = (args.bail, args.skip_scripts);
        let (path, delay) = (args.path.clone(), Duration::from_millis(args.delay_ms));
        let filter = args.tag_filter();
        let limits = send::stream_limits(args.stream_max_seconds, args.stream_max_events);
        let plan_filter = filter.clone();
        let (ctx, plan) = self
            .with_collection(move |c| {
                let no_vars = Default::default();
                let env = args.environment.as_deref();
                let ctx =
                    send::context(c, env, &no_vars, &options, store, runtime, tokens, overlay)?;
                Ok((ctx, run::plan(c, &args.path, &plan_filter)?))
            })
            .await?;
        let requests: Vec<_> = plan
            .items()
            .cloned()
            .zip(plan.requests().cloned())
            .collect();
        self.trust.refresh();
        let mut early = Redactor::default();
        for (_, file) in &requests {
            early.add_values(assertion_sensitive_values(file, &ctx));
        }
        self.remember(&early);
        // One gate for the whole run, decided before any token is fetched or byte sent.
        let hashes: Vec<String> = plan_script_texts(&plan, &ctx)
            .into_iter()
            .map(|(_, t)| t.hash)
            .collect();
        let scripts = scripts::gate(
            &hashes,
            skip,
            self.options.root(),
            self.options.script_trust_path(),
            self.inbox.as_ref(),
            &path,
        )?;
        // A pre-request script may set auth or headers, so its request is judged at send time.
        let scripted: HashSet<ItemPath> = if skip {
            HashSet::new()
        } else {
            requests
                .iter()
                .filter(|(_, file)| {
                    script_texts(file, &ctx).iter().any(|t| {
                        matches!(
                            t.kind,
                            ScriptTextKind::Script {
                                phase: ScriptPhase::PreRequest,
                                ..
                            }
                        )
                    })
                })
                .map(|(item, _)| item.clone())
                .collect()
        };

        // A client cancel fires core's handle, which stops a token fetch or the send in flight
        // and skips the rest.
        let cancel = CancelHandle::new();
        let watcher = {
            let (ct, cancel) = (context.ct.clone(), cancel.clone());
            tokio::spawn(async move {
                ct.cancelled().await;
                cancel.cancel();
            })
        };
        // Every token is in the cache before the first send, so a refused token host stops
        // the run with nothing sent.
        let how = AuthorizeOptions::new();
        for (item, file) in requests.iter().filter(|(item, _)| !scripted.contains(item)) {
            let refused = match exec.prepare_auth(file, &ctx, &how, &cancel).await {
                Ok(()) => continue,
                Err(OAuth2Error::Cancelled) => "cancelled; nothing was sent".to_owned(),
                Err(OAuth2Error::Exec(e)) => {
                    self.trust
                        .refusal(&e, item.as_str(), self.inbox.as_ref(), &self.seen())
                }
                Err(e) => format!(
                    "`{}`: {}",
                    item.as_str(),
                    send::oauth2_message(&e, file, &ctx)
                ),
            };
            watcher.abort();
            return Err(refused);
        }

        let (redactor, prepared) = self
            .with_collection(move |c| Ok(run::prepare(c, &path, &filter, ctx, &scripted)))
            .await
            .inspect_err(|_| watcher.abort())?;
        // Registered before the outcome is looked at, so a refusal quoting a secret is scrubbed.
        self.remember(&redactor);
        let prepared = prepared.inspect_err(|_| watcher.abort())?;

        // The runner reports from a synchronous callback; a channel carries each finished
        // request to this task, which sends it on as a progress notification.
        let (tx, mut finished) = tokio::sync::mpsc::unbounded_channel();
        let on_event = Box::new(move |event: RunEvent| {
            if let RunEvent::RequestFinished { index, result } = event {
                let _ = tx.send((index, result));
            }
        });
        let mut run_options = RunOptions::new();
        if let Some(data) = data.clone() {
            run_options = run_options.iterations(data);
        }
        if let Some(n) = count {
            run_options = run_options.iteration_count(n);
        }
        let running = tokio::spawn(apinata_core::run(
            prepared.plan.clone(),
            prepared.ctx.clone(),
            exec,
            run_options
                .stream(limits)
                .bail(bail)
                .scripts(scripts)
                .delay(delay)
                .retries(retries)
                .retry_delay(retry_delay)
                .retry_unsafe(retry_unsafe)
                .fail_on_status(status_rule)
                .guard({
                    let seen = Arc::clone(&self.seen);
                    Arc::new(move |req| run::host_guard(req, &seen))
                }),
            cancel,
            on_event,
        ));
        let token = context.meta.get_progress_token();
        let total = prepared.plan.requests().count();
        while let Some((index, result)) = finished.recv().await {
            // What a script read is remembered before anything about the result is reported.
            let mut read = Redactor::default();
            read.add_values(result.script_sensitive_values());
            read.add_values(result.derived_credentials());
            self.remember(&read);
            if let Some(token) = &token {
                let message = self.seen().scrub(&format!(
                    "{}: {}",
                    result.path.as_str(),
                    run::outcome(result.outcome)
                ));
                let progress = ProgressNotificationParam::new(token.clone(), (index + 1) as f64)
                    .with_total(total as f64)
                    .with_message(message);
                let _ = context.peer.notify_progress(progress).await;
            }
        }
        watcher.abort();
        let report = running.await.map_err(|_| PANICKED.to_owned())?;
        let mut read = Redactor::default();
        for result in &report.results {
            read.add_values(result.script_sensitive_values());
            read.add_values(result.derived_credentials());
        }
        self.remember(&read);
        let seen = self.seen();
        let exec_error =
            |e: &ExecError, item: &str| self.trust.refusal(e, item, self.inbox.as_ref(), &seen);
        // A host refused to a script's own send is filed for approval; the run reports no text.
        // A cancelled request files nothing, as a cancelled send_request does not.
        let finished = report
            .results
            .iter()
            .filter(|r| r.outcome != Outcome::Cancelled);
        for result in finished {
            for refused in result.script_refusals() {
                exec_error(refused, result.path.as_str());
            }
        }
        let items: Vec<_> = report
            .results
            .iter()
            .map(|r| (r.iteration, run::item(r, &prepared, &exec_error)))
            .collect();
        let (results, iterations) = if data.is_some() || count.is_some() {
            (Vec::new(), Some(run::group(items, data.as_ref(), &seen)))
        } else {
            (items.into_iter().map(|(_, i)| i).collect(), None)
        };
        Ok(Json(RunResult {
            summary: run::summary(&report.summary),
            results,
            iterations,
            load_errors: report.load_errors.iter().map(ToString::to_string).collect(),
        }))
    }

    #[tool(
        description = "Move the item at `path` into the folder `inside` (empty for the root), \
                       or above the sibling `before`, or below the sibling `after`; give exactly \
                       one. Siblings are renumbered. Returns the new path.",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn move_item(
        &self,
        Parameters(args): Parameters<MoveArgs>,
    ) -> Result<Json<ItemPathResult>, String> {
        self.with_collection(move |c| tree::move_item(c, &args).map(Json))
            .await
    }

    #[tool(
        description = "Move the item at `path` to the trash; a folder goes with everything in \
                       it. Its siblings are renumbered. Refused, changing nothing, when no trash \
                       is available.",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn delete_item(
        &self,
        Parameters(args): Parameters<PathArgs>,
    ) -> Result<Json<Deleted>, String> {
        let (bin, log) = (self.options.trash_bin(), self.options.trash_record());
        self.with_collection(move |c| {
            tree::delete_item(c, &args.path, bin.as_ref(), log.as_ref()).map(Json)
        })
        .await
    }

    #[tool(
        description = "Put back the item the newest `delete_item` (or `apinata item rm`) in this \
                       collection moved to the trash, in its place among its siblings, which \
                       end up numbered 1..n. Refused, changing nothing, when something is at \
                       its path now. Only items this collection deleted are restored, newest \
                       first.",
        annotations(
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn restore_item(&self) -> Result<Json<RestoredItem>, String> {
        let (bin, log) = (self.options.trash_bin(), self.options.trash_record());
        self.with_collection(move |c| tree::restore_item(c, bin.as_ref(), log.as_ref()).map(Json))
            .await
    }

    #[tool(
        description = "Put back every request one `import_collection` update with `prune` (or \
                       `apinata import --update --prune`) moved to the trash, newest first, \
                       and number their folders 1..n: the newest prune, or the one `batch` \
                       names, past any other delete in the record. Refused, changing nothing, \
                       when `batch` is unknown, when the newest delete was a single \
                       `delete_item` and `batch` is absent (use `restore_item` for that delete; \
                       the refusal lists the recorded batch ids), or when any item cannot go \
                       back, listing each. If an item turns out to be gone from the trash or \
                       the trash fails, the ones before it stay restored and a warning says \
                       how many remain.",
        annotations(
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn restore_pruned(
        &self,
        Parameters(args): Parameters<RestorePrunedArgs>,
    ) -> Result<Json<RestoredPrune>, String> {
        let (bin, log) = (self.options.trash_bin(), self.options.trash_record());
        self.with_collection(move |c| {
            tree::restore_pruned(c, bin.as_ref(), log.as_ref(), args.batch.as_deref()).map(Json)
        })
        .await
    }

    #[tool(
        description = "Create an empty environment named `name`. The name is made safe for a \
                       file name and suffixed if taken. Returns the name used.",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn create_environment(
        &self,
        Parameters(args): Parameters<NameArgs>,
    ) -> Result<Json<EnvNameResult>, String> {
        let store = Arc::clone(&self.store);
        self.with_collection(move |c| envs::create(c, &args.name, store.as_ref()).map(Json))
            .await
    }

    #[tool(
        description = "Copy the environment `name` (variables, colour, extends and stored \
                       secret values) to a new environment named `<name> copy`, suffixed if \
                       taken. Returns the name used and any steps that could not be finished.",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn duplicate_environment(
        &self,
        Parameters(args): Parameters<NameArgs>,
    ) -> Result<Json<EnvNameResult>, String> {
        let store = Arc::clone(&self.store);
        self.with_collection(move |c| envs::duplicate(c, &args.name, store.as_ref()).map(Json))
            .await
    }

    #[tool(
        description = "Rename the environment `from` to `to`, moving its secret values and \
                       updating environments that extend it. Returns the new name and any steps \
                       that could not be finished.",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn rename_environment(
        &self,
        Parameters(args): Parameters<RenameEnvArgs>,
    ) -> Result<Json<EnvNameResult>, String> {
        let store = Arc::clone(&self.store);
        self.with_collection(move |c| envs::rename(c, &args, store.as_ref()).map(Json))
            .await
    }

    #[tool(
        description = "Delete the environment `name`. An environment holding secret variables \
                       is refused; the user deletes it in the Apinata app. One that other \
                       environments extend is refused too, naming them; change their `extends` \
                       first.",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn delete_environment(
        &self,
        Parameters(args): Parameters<NameArgs>,
    ) -> Result<Json<DeletedEnv>, String> {
        let store = Arc::clone(&self.store);
        self.with_collection(move |c| envs::delete(c, &args.name, store.as_ref()).map(Json))
            .await
    }

    #[tool(
        description = "Edit environment `name`: `remove` drops plain variables, then `set` adds \
                       or changes variables (absent fields keep their value). Secret values and \
                       types are never set, shown or removed here; `secret: true` moves a plain \
                       variable's value into the keychain, and is refused when that name already \
                       has a stored value. Two phases: the plain edits are saved \
                       first, then each variable is made secret, so a failure in the second \
                       phase leaves the plain edits saved and the error lists the variables \
                       still plain. Returns the environment without secret values.",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn update_environment(
        &self,
        Parameters(args): Parameters<UpdateEnvArgs>,
    ) -> Result<Json<EnvironmentDoc>, String> {
        let store = Arc::clone(&self.store);
        self.with_collection(move |c| envs::update(c, &args, store.as_ref()).map(Json))
            .await
    }
}

#[tool_handler]
impl ServerHandler for ApinataServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_resources_list_changed()
                .enable_resources_subscribe()
                .enable_prompts()
                .build(),
        )
    }

    // The watcher starts once the client has said it is ready, since it needs the peer to push to.
    async fn on_initialized(&self, context: NotificationContext<RoleServer>) {
        let started = self
            .watcher
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some();
        if started {
            return;
        }
        if let Some(handle) = crate::watch::start(self.without_watcher(), context.peer).await {
            let mut slot = self.watcher.lock().unwrap_or_else(|e| e.into_inner());
            // A repeated `initialized` may have raced here; the first watcher stays.
            if slot.is_none() {
                *slot = Some(handle);
            }
        }
    }

    // Resources are scrubbed with the session redactor, as tool results are, after it has learned
    // the collection's secrets: a value pasted into a request file must not reach the client.
    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let (options, store) = (Arc::clone(&self.options), Arc::clone(&self.store));
        let (mut listed, secrets) = self
            .with_collection(move |c| {
                Ok((
                    resources::list(c),
                    resources::secret_values(c, &options, &store),
                ))
            })
            .await
            .map_err(|e| self.scrub_error(ErrorData::internal_error(e, None)))?;
        self.learn(secrets);
        let seen = self.seen();
        for resource in &mut listed {
            resource.name = seen.scrub(&resource.name);
            resource.title = resource.title.as_deref().map(|t| seen.scrub(t));
        }
        Ok(ListResourcesResult::with_all_items(listed))
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, ErrorData> {
        Ok(ListPromptsResult::with_all_items(prompts::list()))
    }

    // The prompt text is scrubbed like a tool result, after the session has learned the secrets.
    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, ErrorData> {
        let (options, store) = (Arc::clone(&self.options), Arc::clone(&self.store));
        let (outcome, secrets) = self
            .with_collection(move |c| {
                let outcome = prompts::get(c, &request.name, request.arguments.as_ref());
                Ok((outcome, resources::secret_values(c, &options, &store)))
            })
            .await
            .map_err(|e| self.scrub_error(ErrorData::internal_error(e, None)))?;
        self.learn(secrets);
        match outcome {
            Ok(mut result) => {
                let seen = self.seen();
                for message in &mut result.messages {
                    if let ContentBlock::Text(text) = &mut message.content {
                        text.text = seen.scrub(&text.text);
                    }
                }
                Ok(result.into())
            }
            Err(message) => Err(self.scrub_error(ErrorData::invalid_params(message, None))),
        }
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let (options, store) = (Arc::clone(&self.options), Arc::clone(&self.store));
        let uri = request.uri.clone();
        let (outcome, secrets) = self
            .with_collection(move |c| {
                let outcome = match resources::parse(&uri, c.name()) {
                    Some(target) => resources::read(c, &target),
                    None => Err(ReadError::NotFound),
                };
                Ok((outcome, resources::secret_values(c, &options, &store)))
            })
            .await
            .map_err(|e| self.scrub_error(ErrorData::internal_error(e, None)))?;
        self.learn(secrets);
        match outcome {
            Ok(mut document) => {
                self.seen().scrub_json(&mut document);
                let contents = ResourceContents::text(document.to_string(), request.uri)
                    .with_mime_type("application/json");
                Ok(ReadResourceResult::new(vec![contents]).into())
            }
            Err(ReadError::NotFound) => Err(ErrorData::resource_not_found(
                "no such resource in this collection",
                None,
            )),
            Err(ReadError::Failed(e)) => Err(self.scrub_error(ErrorData::internal_error(e, None))),
        }
    }

    // Only URIs that name a resource of this collection can be subscribed to.
    #[allow(deprecated)]
    async fn subscribe(
        &self,
        request: SubscribeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        self.check_resource_uri(&request.uri).await?;
        self.subscriptions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(request.uri);
        Ok(())
    }

    #[allow(deprecated)]
    async fn unsubscribe(
        &self,
        request: UnsubscribeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        self.check_resource_uri(&request.uri).await?;
        self.subscriptions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&request.uri);
        Ok(())
    }

    // Every tool call passes through here, so no result or error reaches the client unscrubbed.
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        // A tool that writes collection files may have changed the listing, even when it then
        // failed, so it is compared with what the client last saw.
        let read_only = self
            .get_tool(&request.name)
            .and_then(|tool| tool.annotations)
            .and_then(|a| a.read_only_hint)
            .unwrap_or(false);
        let mutates = announces_list_change(&request.name, read_only);
        let peer = context.peer.clone();
        // Held until the listing is recorded and announced, even if this call is cancelled.
        let _write = mutates.then(|| self.writes.enter());
        let call = ToolCallContext::new(self, request, context);
        let outcome = self.tool_router.call(call).await;
        if mutates {
            // Even a failed tool may have written files. The listing is read and recorded in one
            // collection pass, so a stale watcher snapshot cannot regress it, and only the side
            // that records a difference announces it.
            let this = self.clone();
            let changed = self
                .with_collection(move |c| Ok(this.record_listing(resources::signature(c))))
                .await
                .unwrap_or(true);
            if changed {
                // The client may have gone away; the tool's own result is still returned.
                let _ = peer.notify_resource_list_changed().await;
            }
        }
        match outcome {
            Ok(response) => Ok(self.scrub(response)),
            Err(error) => Err(self.scrub_error(error)),
        }
    }
}

/// Whether a call to the tool may have changed the resource list, so the client is told to list
/// again: any tool that is not read-only and not known to write no collection file.
fn announces_list_change(name: &str, read_only: bool) -> bool {
    !read_only && !NO_FILE_WRITES.contains(&name)
}

/// Marks a host refusal as one of a script's own requests, since the request itself was sent.
fn script_refusal(message: &str) -> String {
    format!("a script's own request was not sent: {message}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: shop\n";

    /// A server whose secrets live in memory, so no test touches the machine's keychain.
    fn open_server(options: McpOptions) -> Result<ApinataServer, McpError> {
        ApinataServer::with_store(options, Arc::new(apinata_core::MemoryStore::new()))
    }

    fn collection() -> tempfile::TempDir {
        let dir = tempfile::TempDir::new().expect("tempdir");
        fs::write(dir.path().join("opencollection.yml"), ROOT).expect("write");
        dir
    }

    // The notifier task must not keep the watcher slot alive: once the session ends and the
    // last outside clone is gone, the slot (and the OS watcher in it) has to be freed.
    #[tokio::test]
    async fn ending_the_session_frees_the_watcher_slot() {
        use rmcp::ServiceExt;
        use std::time::Duration;

        let dir = collection();
        let server = open_server(McpOptions::new(dir.path())).expect("opens");
        let slot = Arc::downgrade(&server.watcher);
        let (server_io, client_io) = tokio::io::duplex(64 * 1024);
        let serving = tokio::spawn(async move {
            let running = server.serve(server_io).await.expect("serves");
            let _ = running.waiting().await;
        });
        let client = ().serve(client_io).await.expect("client connects");

        // The watcher starts after the client's `initialized`, so wait for the slot to fill.
        let started = async {
            loop {
                if let Some(slot) = slot.upgrade()
                    && slot.lock().unwrap_or_else(|e| e.into_inner()).is_some()
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(10), started)
            .await
            .expect("the watcher starts");

        client.cancel().await.expect("client stops");
        serving.await.expect("server task ends");
        let freed = async {
            while slot.upgrade().is_some() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(10), freed)
            .await
            .expect("the watcher slot is freed");
    }

    #[test]
    fn protocol_errors_are_scrubbed_in_message_and_data() {
        let dir = collection();
        let options = McpOptions::new(dir.path()).env_var("key", "cli-Secret-5");
        let server = ApinataServer::new(options).expect("opens");
        let error = ErrorData::invalid_params(
            "bad value cli-Secret-5",
            Some(serde_json::json!({"echo": "cli-Secret-5"})),
        );
        let scrubbed = server.scrub_error(error);
        assert_eq!(scrubbed.message, "bad value <secret>");
        assert_eq!(scrubbed.data, Some(serde_json::json!({"echo": "<secret>"})));
    }

    #[test]
    fn remembered_values_are_scrubbed_from_later_errors() {
        let dir = collection();
        let server = open_server(McpOptions::new(dir.path())).expect("opens");
        let mut sent = Redactor::default();
        sent.add_value("req-Token-77");
        server.remember(&sent);
        let scrubbed = server.scrub_error(ErrorData::invalid_params("saw req-Token-77", None));
        assert_eq!(scrubbed.message, "saw <secret>");
    }

    #[test]
    fn new_refuses_a_directory_without_opencollection_yml() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let err = open_server(McpOptions::new(dir.path()))
            .err()
            .expect("refused");
        assert!(matches!(err, McpError::Open(_)), "{err}");
    }

    /// A new tool must choose: read-only, or listed as writing files or not.
    #[test]
    fn every_tool_is_read_only_or_listed_for_the_notification_policy() {
        let dir = collection();
        let server = open_server(McpOptions::new(dir.path())).expect("opens");
        let tools = server.tool_router.list_all();
        let mut unsorted = Vec::new();
        for tool in &tools {
            let read_only = tool.annotations.as_ref().and_then(|a| a.read_only_hint) == Some(true);
            let name = tool.name.as_ref();
            let (quiet, writes) = (NO_FILE_WRITES.contains(&name), WRITES_FILES.contains(&name));
            assert!(!(quiet && writes), "{name} is on both lists");
            assert!(
                !(read_only && (quiet || writes)),
                "{name} is read-only and listed"
            );
            if !read_only && !quiet && !writes {
                unsorted.push(name.to_owned());
            }
        }
        assert!(
            unsorted.is_empty(),
            "list these in NO_FILE_WRITES or WRITES_FILES: {unsorted:?}"
        );
        for listed in NO_FILE_WRITES.iter().chain(WRITES_FILES) {
            assert!(
                tools.iter().any(|t| t.name == *listed),
                "{listed} is not a tool"
            );
        }
    }

    /// The deterministic guarantee behind the policy; the integration test can only miss by
    /// passing early.
    #[test]
    fn only_file_writing_tools_announce_a_list_change() {
        for name in NO_FILE_WRITES {
            assert!(!announces_list_change(name, false), "{name} must be quiet");
        }
        for name in WRITES_FILES {
            assert!(announces_list_change(name, false), "{name} must announce");
        }
        assert!(!announces_list_change("get_collection", true));
        assert!(!announces_list_change("replace", true));
    }

    #[tokio::test]
    async fn get_collection_reports_name_environments_and_default() {
        let dir = collection();
        fs::create_dir(dir.path().join("environments")).expect("mkdir");
        fs::write(
            dir.path().join("environments/dev.yml"),
            "name: dev\nvariables: []\n",
        )
        .expect("write");
        let server = open_server(McpOptions::new(dir.path()).env("dev")).expect("opens");
        let Json(info) = server.get_collection().await.expect("ok");
        assert_eq!(info.name, "shop");
        assert_eq!(info.root, dir.path().display().to_string());
        assert_eq!(info.environments, ["dev"]);
        assert_eq!(info.default_environment.as_deref(), Some("dev"));
        assert!(info.load_errors.is_empty());
    }

    #[tokio::test]
    async fn every_call_sees_the_disk_as_it_is_now() {
        let dir = collection();
        let server = open_server(McpOptions::new(dir.path())).expect("opens");
        let Json(before) = server.get_collection().await.expect("ok");
        assert!(before.environments.is_empty());
        // Another process (the app, git, an editor) changes the collection between calls.
        fs::write(
            dir.path().join("opencollection.yml"),
            ROOT.replace("shop", "store"),
        )
        .expect("write");
        fs::create_dir(dir.path().join("environments")).expect("mkdir");
        fs::write(dir.path().join("environments/prod.yml"), "name: prod\n").expect("write");
        fs::write(dir.path().join("broken.yml"), "info: [unclosed\n").expect("write");
        let Json(after) = server.get_collection().await.expect("ok");
        assert_eq!(after.name, "store");
        assert_eq!(after.environments, ["prod"]);
        assert_eq!(after.load_errors.len(), 1, "{:?}", after.load_errors);
        assert!(
            after.load_errors[0].contains("broken.yml"),
            "{:?}",
            after.load_errors
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn calls_are_serialised_on_the_collection() {
        use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
        let dir = collection();
        let server = open_server(McpOptions::new(dir.path())).expect("opens");
        let inside = Arc::new(AtomicUsize::new(0));
        let mut calls = tokio::task::JoinSet::new();
        for _ in 0..8 {
            let (server, inside) = (server.clone(), Arc::clone(&inside));
            calls.spawn(async move {
                server
                    .with_collection(move |_| {
                        // A second closure running now would find the counter above zero.
                        let others = inside.fetch_add(1, SeqCst);
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        inside.fetch_sub(1, SeqCst);
                        Ok(others)
                    })
                    .await
            });
        }
        while let Some(joined) = calls.join_next().await {
            assert_eq!(joined.expect("joins").expect("ok"), 0);
        }
    }

    #[tokio::test]
    async fn a_panicking_tool_reports_a_fixed_error_and_frees_the_collection() {
        let dir = collection();
        let server = open_server(McpOptions::new(dir.path())).expect("opens");
        let err = server
            .with_collection::<(), _>(|_| panic!("sentinel-s3cr3t"))
            .await
            .expect_err("panic surfaces as an error");
        assert!(!err.contains("sentinel-s3cr3t"), "{err}");
        assert!(err.starts_with("internal error"), "{err}");
        let Json(info) = server.get_collection().await.expect("next call runs");
        assert_eq!(info.name, "shop");
    }

    #[tokio::test]
    async fn a_panic_after_an_edit_leaves_no_draft_behind() {
        let dir = collection();
        let request = "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: http://localhost/a\n";
        fs::write(dir.path().join("r.yml"), request).expect("write");
        let server = open_server(McpOptions::new(dir.path())).expect("opens");
        let path = apinata_core::ItemPath::new("r.yml").expect("path");
        let edited = path.clone();
        let err = server
            .with_collection::<(), _>(move |c| {
                c.request_mut(&edited).expect("request").set_method("POST");
                panic!("after the edit")
            })
            .await
            .expect_err("panic surfaces as an error");
        assert!(err.starts_with("internal error"), "{err}");
        let (dirty, method) = server
            .with_collection(move |c| {
                let r = c.request(&path).expect("request");
                Ok((r.is_dirty(), r.method().to_owned()))
            })
            .await
            .expect("next call runs");
        assert_eq!((dirty, method.as_str()), (false, "GET"));
    }
}
