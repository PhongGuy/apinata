//! `TestServer`: an `ApinataServer` on a temporary collection, driven by an in-process rmcp
//! client over a duplex pipe, so tests see exactly what an agent would.
#![allow(dead_code)] // each test binary uses a different subset

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use apinata_core::{
    ApprovalInbox, MemoryStore, SecretKey, SecretStore, Trash, TrashError, TrashHandle, TrustStore,
};
use apinata_mcp::{ApinataServer, McpOptions};
use rmcp::model::{
    CallToolRequest, CallToolRequestParams, CallToolResult, ClientRequest, GetPromptRequestParams,
    GetPromptResult, ProgressNotificationParam, Prompt, ReadResourceRequestParams,
    ReadResourceResult, Resource, ResourceContents, Tool,
};
use rmcp::service::{NotificationContext, PeerRequestOptions, RequestHandle, RunningService};
use rmcp::{ClientHandler, RoleClient, ServiceExt};
use serde_json::Value;
use tempfile::TempDir;

/// A collection root file naming the collection `shop`.
pub const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: shop\n";

/// A trash that moves files into its own temporary directory, so no test reaches the user's.
#[derive(Debug)]
pub struct FakeTrash {
    dir: TempDir,
    usable: bool,
    next: Mutex<u32>,
    /// Every id the server asked to restore, in order.
    restored: Mutex<Vec<OsString>>,
}

impl FakeTrash {
    pub fn new(usable: bool) -> FakeTrash {
        FakeTrash {
            dir: TempDir::new().expect("tempdir"),
            usable,
            next: Mutex::new(0),
            restored: Mutex::new(Vec::new()),
        }
    }

    /// Where the `n`th trashed item (from 1) sits, so a test can empty it from the bin.
    pub fn stash(&self, n: u32) -> PathBuf {
        self.dir.path().join(format!("item{n}"))
    }

    /// The ids restore was called with, in order.
    pub fn restored(&self) -> Vec<String> {
        let ids = self.restored.lock().expect("restored");
        ids.iter()
            .map(|id| id.to_string_lossy().into_owned())
            .collect()
    }

    /// How many items it holds.
    pub fn taken(&self) -> usize {
        fs::read_dir(self.dir.path()).expect("read").count()
    }
}

impl Trash for FakeTrash {
    fn available(&self) -> bool {
        self.usable
    }

    fn trash(&self, path: &Path) -> Result<TrashHandle, TrashError> {
        if !self.usable {
            return Err(TrashError::Unavailable("no trash".into()));
        }
        let mut next = self.next.lock().expect("next");
        *next += 1;
        let id = OsString::from(format!("item{next}"));
        fs::rename(path, self.dir.path().join(&id))
            .map_err(|e| TrashError::Failed(e.to_string()))?;
        Ok(TrashHandle::new(path.to_path_buf(), id))
    }

    fn restore(&self, handle: &TrashHandle) -> Result<(), TrashError> {
        self.restored
            .lock()
            .expect("restored")
            .push(handle.id().clone());
        let stash = self.dir.path().join(handle.id());
        if !stash.exists() {
            return Err(TrashError::Missing);
        }
        fs::rename(stash, handle.original()).map_err(|e| TrashError::Failed(e.to_string()))
    }
}

/// A server on a temporary collection and a connected client.
pub struct TestServer {
    dir: TempDir,
    /// The trust file and approval inbox, outside the collection as in production.
    state: TempDir,
    client: RunningService<RoleClient, Recorder>,
    recorder: Recorder,
    store: Arc<MemoryStore>,
}

/// A client that records the progress notifications the server sends.
#[derive(Clone, Default)]
pub struct Recorder {
    progress: Arc<Mutex<Vec<ProgressNotificationParam>>>,
    /// How many `notifications/resources/list_changed` arrived.
    list_changed: Arc<Mutex<usize>>,
    /// Woken on every list-changed notification, so a test waits instead of sleeping.
    arrived: Arc<tokio::sync::Notify>,
    /// The URI of every `notifications/resources/updated`, in arrival order.
    updated: Arc<Mutex<Vec<String>>>,
    /// Woken on every resource-updated notification.
    updated_arrived: Arc<tokio::sync::Notify>,
}

impl ClientHandler for Recorder {
    async fn on_progress(
        &self,
        params: ProgressNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        self.progress.lock().expect("progress").push(params);
    }

    async fn on_resource_updated(
        &self,
        params: rmcp::model::ResourceUpdatedNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        self.updated.lock().expect("updated").push(params.uri);
        self.updated_arrived.notify_waiters();
    }

    async fn on_resource_list_changed(&self, _context: NotificationContext<RoleClient>) {
        *self.list_changed.lock().expect("list_changed") += 1;
        self.arrived.notify_waiters();
    }
}

impl TestServer {
    /// Write `files` (relative path, contents) under a new collection and serve it.
    pub async fn start(files: &[(&str, &str)]) -> TestServer {
        TestServer::start_with(files, |options| options).await
    }

    /// As `start`, with launch options adjusted by `configure`.
    pub async fn start_with(
        files: &[(&str, &str)],
        configure: impl FnOnce(McpOptions) -> McpOptions,
    ) -> TestServer {
        TestServer::launch(files, true, configure).await
    }

    /// As `start`, but with no approval directory, so a refused host cannot be filed.
    pub async fn start_without_inbox(files: &[(&str, &str)]) -> TestServer {
        TestServer::launch(files, false, |options| options).await
    }

    async fn launch(
        files: &[(&str, &str)],
        with_inbox: bool,
        configure: impl FnOnce(McpOptions) -> McpOptions,
    ) -> TestServer {
        let dir = TempDir::new().expect("tempdir");
        for (rel, text) in files {
            write_file(dir.path(), rel, text);
        }
        // Secrets go to memory, never the machine's keychain.
        let store = Arc::new(MemoryStore::new());
        let state = TempDir::new().expect("tempdir");
        // Pruning moves files to a trash; the default is a scratch one, never the user's.
        let options = McpOptions::new(dir.path())
            .trash(Arc::new(FakeTrash::new(true)))
            .trash_log(state.path().join("trash-log"))
            .trust_file(state.path().join("trusted-hosts.json"))
            .script_trust_file(state.path().join("script-trust.json"));
        // Never fall back to the per-user default: a test must not file into the real inbox.
        let options = if with_inbox {
            options.approval_dir(state.path().join("approvals"))
        } else {
            options.without_approval_dir()
        };
        let server =
            ApinataServer::with_store(configure(options), store.clone()).expect("server opens");
        // The server's handshake waits for the client's, so it must run on its own task.
        let (server_io, client_io) = tokio::io::duplex(64 * 1024);
        tokio::spawn(async move {
            if let Ok(running) = server.serve(server_io).await {
                let _ = running.waiting().await;
            }
        });
        let recorder = Recorder::default();
        let client = recorder
            .clone()
            .serve(client_io)
            .await
            .expect("client connects");
        TestServer {
            dir,
            state,
            client,
            recorder,
            store,
        }
    }

    /// As `start`, from owned pairs.
    pub async fn start_owned(files: &[(String, String)]) -> TestServer {
        let refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(p, t)| (p.as_str(), t.as_str()))
            .collect();
        TestServer::start(&refs).await
    }

    /// The collection root.
    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    /// The directory the server keeps its record of trashed items in.
    pub fn trash_log_dir(&self) -> PathBuf {
        self.state.path().join("trash-log")
    }

    /// The trust file the server reads.
    pub fn trust_file(&self) -> PathBuf {
        self.state.path().join("trusted-hosts.json")
    }

    /// Trust `host` for this collection, as the app would.
    pub fn trust(&self, host: &str) {
        let mut store = TrustStore::load(self.trust_file()).expect("trust file");
        store.trust(self.root(), host).expect("trust");
        store.save().expect("save");
    }

    /// Deny `host` for this collection, as the app would.
    pub fn deny(&self, host: &str) {
        let mut store = TrustStore::load(self.trust_file()).expect("trust file");
        store.deny(self.root(), host).expect("deny");
        store.save().expect("save");
    }

    /// The script trust file the server reads.
    pub fn script_trust_file(&self) -> PathBuf {
        self.state.path().join("script-trust.json")
    }

    /// Allow scripts for the collection and approve each text, as the app's dialog does.
    pub fn approve_scripts(&self, texts: &[&str]) {
        let mut trust = apinata_core::ScriptTrust::load(self.script_trust_file()).expect("load");
        trust.set_decision(self.root(), Some(apinata_core::ScriptDecision::Allowed));
        trust.approve(
            self.root(),
            texts.iter().map(|t| apinata_core::script_hash(t)),
        );
        trust.save().expect("save");
    }

    /// Block scripts for the collection, as the app's prompt does.
    pub fn block_scripts(&self) {
        let mut trust = apinata_core::ScriptTrust::load(self.script_trust_file()).expect("load");
        trust.set_decision(self.root(), Some(apinata_core::ScriptDecision::Blocked));
        trust.save().expect("save");
    }

    /// The approval inbox the server files refused hosts in.
    pub fn inbox(&self) -> ApprovalInbox {
        ApprovalInbox::new(self.state.path().join("approvals"))
    }

    /// Every progress notification received so far, in arrival order.
    pub fn progress(&self) -> Vec<ProgressNotificationParam> {
        self.recorder.progress.lock().expect("progress").clone()
    }

    /// How many resource list-changed notifications have arrived, after a round trip that
    /// lets any already sent by the server be delivered first.
    pub async fn list_changed(&self) -> usize {
        self.client.list_all_tools().await.expect("tools/list");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        *self.recorder.list_changed.lock().expect("list_changed")
    }

    /// Wait until `count` list-changed notifications have arrived in all; panics after 10 s.
    pub async fn wait_list_changed(&self, count: usize) {
        let wait = async {
            loop {
                let arrived = self.recorder.arrived.notified();
                tokio::pin!(arrived);
                arrived.as_mut().enable();
                if *self.recorder.list_changed.lock().expect("list_changed") >= count {
                    return;
                }
                arrived.await;
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(10), wait)
            .await
            .unwrap_or_else(|_| panic!("fewer than {count} list_changed notifications arrived"));
    }

    /// Every `resources/updated` URI received so far, in arrival order.
    pub fn updated(&self) -> Vec<String> {
        self.recorder.updated.lock().expect("updated").clone()
    }

    /// Wait until `uri` has been announced updated `count` times in all; panics after 10 s.
    pub async fn wait_updated(&self, uri: &str, count: usize) {
        let wait = async {
            loop {
                let arrived = self.recorder.updated_arrived.notified();
                tokio::pin!(arrived);
                arrived.as_mut().enable();
                if self.updated().iter().filter(|u| *u == uri).count() >= count {
                    return;
                }
                arrived.await;
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(10), wait)
            .await
            .unwrap_or_else(|_| panic!("no resources/updated for {uri}"));
    }

    /// The notifications delivered so far, once everything the server sent before this point
    /// has been handed to the client: several round trips with the runtime yielding between.
    pub async fn settled_list_changed(&self) -> usize {
        for _ in 0..5 {
            self.client.list_all_tools().await.expect("tools/list");
            tokio::task::yield_now().await;
        }
        *self.recorder.list_changed.lock().expect("list_changed")
    }

    /// The capabilities the server announced at the handshake.
    pub fn server_capabilities(&self) -> rmcp::model::ServerCapabilities {
        self.client
            .peer_info()
            .expect("handshake done")
            .capabilities
            .clone()
    }

    /// Subscribe to `uri`; `Err` is the JSON-RPC error's text.
    #[allow(deprecated)]
    pub async fn subscribe(&self, uri: &str) -> Result<(), String> {
        self.client
            .subscribe(rmcp::model::SubscribeRequestParams::new(uri))
            .await
            .map_err(|e| e.to_string())
    }

    /// Unsubscribe from `uri`; `Err` is the JSON-RPC error's text.
    #[allow(deprecated)]
    pub async fn unsubscribe(&self, uri: &str) -> Result<(), String> {
        self.client
            .unsubscribe(rmcp::model::UnsubscribeRequestParams::new(uri))
            .await
            .map_err(|e| e.to_string())
    }

    /// Every resource the server lists.
    pub async fn resources(&self) -> Vec<Resource> {
        self.client
            .list_all_resources()
            .await
            .expect("resources/list")
    }

    /// Read the resource at `uri`; `Err` is the JSON-RPC error's text.
    pub async fn try_read_resource(&self, uri: &str) -> Result<ReadResourceResult, String> {
        self.client
            .read_resource(ReadResourceRequestParams::new(uri))
            .await
            .map_err(|e| e.to_string())
    }

    /// The text of the one content block of the resource at `uri`.
    pub async fn resource_text(&self, uri: &str) -> String {
        let result = self
            .try_read_resource(uri)
            .await
            .unwrap_or_else(|e| panic!("{uri}: {e}"));
        assert_eq!(result.contents.len(), 1, "{uri}");
        match &result.contents[0] {
            ResourceContents::TextResourceContents { text, .. } => text.clone(),
            _ => panic!("{uri}: not text"),
        }
    }

    /// The prompts the server lists.
    pub async fn prompts(&self) -> Vec<Prompt> {
        self.client.list_all_prompts().await.expect("prompts/list")
    }

    /// Fetch prompt `name` with string `args`; `Err` is the JSON-RPC error's text.
    pub async fn try_prompt(&self, name: &str, args: Value) -> Result<GetPromptResult, String> {
        let mut params = GetPromptRequestParams::new(name);
        if let Value::Object(args) = args {
            params.arguments = Some(args);
        }
        self.client
            .get_prompt(params)
            .await
            .map_err(|e| e.to_string())
    }

    /// The text of every message of prompt `name`, joined by newlines.
    pub async fn prompt_text(&self, name: &str, args: Value) -> String {
        let result = self
            .try_prompt(name, args)
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        result
            .messages
            .iter()
            .filter_map(|m| m.content.as_text().map(|t| t.text.clone()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Start a call to `tool` without waiting for its result, so a test can cancel it.
    pub async fn start_call(&self, tool: &str, args: Value) -> RequestHandle<RoleClient> {
        let Value::Object(args) = args else {
            panic!("tool arguments must be a JSON object")
        };
        let params = CallToolRequestParams::new(tool.to_owned()).with_arguments(args);
        self.client
            .send_request_with_option(
                ClientRequest::CallToolRequest(CallToolRequest::new(params)),
                PeerRequestOptions::no_options(),
            )
            .await
            .expect("request sent")
    }

    /// Write one file under the collection, creating its directories.
    pub fn write(&self, rel: &str, text: &str) {
        write_file(self.dir.path(), rel, text);
    }

    /// Save `text` the way a simple editor does: truncate the file, then write it, so the
    /// watcher may see the item vanish and return. Tests using it are regression guards on the
    /// real filesystem; the unit tests in `watch.rs` are the deterministic coverage.
    pub fn write_in_place(&self, rel: &str, text: &str) {
        use std::io::Write;
        let path = self.dir.path().join(rel);
        let mut file = fs::File::create(&path).expect("truncate");
        file.write_all(text.as_bytes()).expect("write");
    }

    /// The secret stored for `variable` of environment `env`, keyed as core keys it.
    pub fn secret(&self, env: &str, variable: &str) -> Option<String> {
        self.store
            .get(&self.secret_key(env, variable))
            .expect("memory store")
    }

    /// Store `value` as the secret of `variable` in environment `env`.
    pub fn set_secret(&self, env: &str, variable: &str, value: &str) {
        self.store
            .set(&self.secret_key(env, variable), value)
            .expect("memory store");
    }

    fn secret_key(&self, env: &str, variable: &str) -> SecretKey {
        let root = fs::canonicalize(self.dir.path()).expect("canonical root");
        SecretKey::new(root, env, variable)
    }

    /// The tools the server lists.
    pub async fn tools(&self) -> Vec<Tool> {
        self.client.list_all_tools().await.expect("tools/list")
    }

    /// Call `tool` with `args`, a JSON object; `Err` is a JSON-RPC error, not an `isError` result.
    pub async fn try_call(&self, tool: &str, args: Value) -> Result<CallToolResult, String> {
        let Value::Object(args) = args else {
            panic!("tool arguments must be a JSON object")
        };
        self.client
            .call_tool(CallToolRequestParams::new(tool.to_owned()).with_arguments(args))
            .await
            .map_err(|e| e.to_string())
    }

    /// Call `tool` with no `arguments` key at all, as a client may for an all-optional tool.
    pub async fn call_bare(&self, tool: &str) -> CallToolResult {
        self.client
            .call_tool(CallToolRequestParams::new(tool.to_owned()))
            .await
            .unwrap_or_else(|e| panic!("{tool}: {e}"))
    }

    /// Call `tool` with `args`, requiring a tool result of either kind.
    pub async fn call(&self, tool: &str, args: Value) -> CallToolResult {
        self.try_call(tool, args)
            .await
            .unwrap_or_else(|e| panic!("{tool}: {e}"))
    }

    /// Call `tool`, require success, and return its structured content.
    pub async fn ok(&self, tool: &str, args: Value) -> Value {
        let result = self.call(tool, args).await;
        assert_ne!(
            result.is_error,
            Some(true),
            "{tool} failed: {}",
            text(&result)
        );
        result.structured_content.expect("structured content")
    }

    /// Call `tool`, require an `isError` result, and return its text.
    pub async fn err(&self, tool: &str, args: Value) -> String {
        let result = self.call(tool, args).await;
        assert_eq!(
            result.is_error,
            Some(true),
            "{tool} succeeded: {:?}",
            result.structured_content
        );
        text(&result)
    }
}

/// A result's text content, joined by newlines.
pub fn text(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn write_file(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    // Write beside the target and rename, so the watcher never reads a half-written file and
    // sees an item vanish and return.
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text).expect("write");
    fs::rename(&tmp, &path).expect("rename");
}

/// A request file: `info` with `name` and `seq`, then `http` with `method` and `url`.
pub fn request(name: &str, seq: u32, method: &str, url: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: {method}\n  url: \"{url}\"\n"
    )
}

/// A directory under the core crate's `tests/fixtures/reference`.
pub fn research(entry: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../apinata-core/tests/fixtures/reference")
        .join(entry)
}

/// The sections of `target` that differ from `current`, plus `path`: what an agent sends to make
/// the item at `path` read as `target`.
pub fn patch(path: &str, current: &Value, target: &Value) -> Value {
    let mut out = serde_json::Map::new();
    out.insert("path".into(), Value::from(path));
    for (key, value) in target.as_object().expect("a document") {
        if current.get(key) != Some(value) {
            out.insert(key.clone(), value.clone());
        }
    }
    Value::Object(out)
}

/// Every file under `dir`, as (path relative to `dir`, contents).
pub fn files_under(dir: &Path) -> Vec<(String, String)> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, String)>) {
        for entry in fs::read_dir(dir).expect("read dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                walk(&path, base, out);
            } else {
                let rel = path.strip_prefix(base).expect("under base");
                let text = fs::read_to_string(&path).expect("read");
                out.push((rel.to_str().expect("utf-8").to_owned(), text));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out
}

/// Collection files for the mock tests: one plain route, one behind the `dev` environment's `base`.
pub const MOCK_ITEMS: &[(&str, &str)] = &[
    ("opencollection.yml", ROOT),
    (
        "environments/dev.yml",
        "name: dev\nvariables:\n  - name: base\n    value: http://api.test/v1\n",
    ),
    (
        "ping.yml",
        concat!(
            "info:\n  name: Ping\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"/ping\"\n",
            "examples:\n  - name: ok\n    response:\n      status: 201\n"
        ),
    ),
    (
        "items.yml",
        concat!(
            "info:\n  name: Items\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: \"{{base}}/items\"\n",
            "examples:\n  - name: list\n    response:\n      status: 202\n"
        ),
    ),
];

/// A raw HTTP/1.1 GET to 127.0.0.1:`port`; returns (status, body). `Connection: close` ends the read.
pub async fn http_get(port: u16, path: &str) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.expect("write");
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.expect("read");
    let raw = String::from_utf8_lossy(&raw).into_owned();
    let status = raw
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .expect("status line");
    let body = raw
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_owned())
        .unwrap_or_default();
    (status, body)
}

/// The port in a mock URL such as `http://127.0.0.1:4010`.
pub fn port_of(url: &str) -> u16 {
    url.rsplit(':')
        .next()
        .and_then(|p| p.parse().ok())
        .expect("url with port")
}

#[path = "../../../apinata-core/tests/common/sse_server.rs"]
pub mod sse_server;

/// Drops a path's permissions for one test and puts the original mode back on drop, so a
/// failing assertion never leaves a temp dir the cleanup cannot remove. Bind it with
/// `let _g =` after the `TempDir` so it drops first.
#[cfg(unix)]
pub struct ModeGuard {
    path: std::path::PathBuf,
    original: u32,
}

#[cfg(unix)]
impl ModeGuard {
    /// Records the current mode, then applies `mode`. The guard exists before the chmod, so a
    /// panic right after still restores.
    pub fn set(path: impl AsRef<std::path::Path>, mode: u32) -> Self {
        use std::os::unix::fs::PermissionsExt as _;
        let path = path.as_ref().to_path_buf();
        let original = std::fs::metadata(&path).expect("stat").permissions().mode();
        let guard = Self { path, original };
        std::fs::set_permissions(&guard.path, std::fs::Permissions::from_mode(mode))
            .expect("chmod");
        guard
    }

    /// Puts the original mode back now; the drop does it again harmlessly.
    pub fn restore(&self) {
        use std::os::unix::fs::PermissionsExt as _;
        let _ =
            std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(self.original));
    }
}

#[cfg(unix)]
impl Drop for ModeGuard {
    fn drop(&mut self) {
        self.restore();
    }
}
