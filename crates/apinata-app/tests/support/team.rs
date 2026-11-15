//! A stub team server on the app's tokio runtime and a signed-in team file.

use super::potluck::tok;
use super::{Started, collection, frame, started_on};
use crate::git_support::Sandbox;
use apinata_app::persist::Paths;
use apinata_app::watch::set::notify_factory;
use apinata_core::team::{Account, Binding, Role, ServerUrl, TeamStore, token_key};
use apinata_core::{MemoryStore, SecretStore};
use gpui_kit::TestAppContext;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A dedicated stub server; call `http_ready` first.
pub fn potluck_server(cx: &mut TestAppContext) -> MockServer {
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    handle.block_on(MockServer::builder().start())
}

pub fn mount(cx: &mut TestAppContext, server: &MockServer, mock: Mock) {
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    handle.block_on(mock.mount(server));
}

pub fn hits_on(cx: &mut TestAppContext, server: &MockServer, verb: &str, at: &str) -> usize {
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    handle.block_on(super::potluck::hits(server, verb, at))
}

pub fn server_url(server: &MockServer) -> ServerUrl {
    ServerUrl::parse(&server.uri()).unwrap()
}

pub fn team_file(s: &Started) -> PathBuf {
    s.files.path().join("team.json")
}

/// Writes an account for `ada` (token id `tok-7`) and her token, as a finished join would.
pub fn signed_in(paths: &Paths, secrets: &MemoryStore, server: &MockServer, admin: bool) {
    let url = server_url(server);
    TeamStore::update(&paths.team, |t| {
        t.put_account(Account::new(&url, "ada", "Ada Lovelace", "tok-7", admin))
    })
    .unwrap();
    secrets.set(&token_key(&url, "ada"), &tok()).unwrap();
}

/// The started app over its own secret store; `prepare` runs on the temporary paths first.
/// Call `http_ready` before this.
pub fn team_app(
    cx: &mut TestAppContext,
    prepare: impl FnOnce(&Paths, &MemoryStore),
) -> (TempDir, Started, Arc<MemoryStore>) {
    let col = collection();
    let secrets = Arc::new(MemoryStore::new());
    let store: Arc<dyn SecretStore> = secrets.clone();
    let s = started_on(cx, &col, |paths| prepare(paths, &secrets), store);
    frame(cx, s.win);
    (col, s, secrets)
}

/// `col` as `main` of a new bare origin at `<sandbox>/origin.git`, plus a teammate clone at
/// `<sandbox>/mate`; returns the origin.
pub fn shared_origin(sb: &Sandbox, col: &Path) -> PathBuf {
    let bare = sb.path().join("origin.git");
    sb.git(
        sb.path(),
        &["init", "-q", "--bare", "-b", "main", "origin.git"],
    );
    sb.git(col, &["init", "-q", "-b", "main"]);
    sb.commit_all(col, "base");
    sb.git(col, &["remote", "add", "origin", bare.to_str().unwrap()]);
    sb.git(col, &["push", "-q", "-u", "origin", "main"]);
    sb.git(sb.path(), &["clone", "-q", bare.to_str().unwrap(), "mate"]);
    bare
}

/// Whether `rel` on the origin's `main` holds `text`.
pub fn origin_has(sb: &Sandbox, bare: &Path, rel: &str, text: &str) -> bool {
    let out = std::process::Command::new("git")
        .arg("--git-dir")
        .arg(bare)
        .args(["show", &format!("main:{rel}")])
        .envs(sb.env())
        .output()
        .unwrap();
    out.status.success() && String::from_utf8_lossy(&out.stdout).contains(text)
}

/// Binds `root` to `core/api` on `server` for ada as `role`, branch `main`.
pub fn bind(paths: &Paths, root: &Path, server: &str, role: Role) {
    let account = Account::new(
        &ServerUrl::parse(server).unwrap(),
        "ada",
        "Ada Lovelace",
        "tok-7",
        false,
    );
    let mut b = Binding::new(root.to_path_buf(), &account, "core", "api", role);
    b.branch = "main".into();
    TeamStore::update(&paths.team, |t| t.put_binding(b)).unwrap();
}

/// `/me` listing `core/api` with `role`, and an events stream that delivers one keep-alive.
pub fn serve_workspace(cx: &mut TestAppContext, server: &MockServer, role: &str) {
    let base = server.uri();
    let listed = super::potluck::workspace(&base, "core", "api", "apinata", role);
    mount(
        cx,
        server,
        Mock::given(method("GET"))
            .and(path("/api/v1/me"))
            .respond_with(super::potluck::me(false, vec![listed])),
    );
    mount(
        cx,
        server,
        Mock::given(method("GET"))
            .and(path("/api/v1/workspaces/core/api/events"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(": ok\n\n"),
            ),
    );
}

/// The started app over `col` with `prepare` run on its paths first, then git from `sb` and
/// the real watcher turned on. Call `http_ready` before this.
pub fn team_started(
    cx: &mut TestAppContext,
    sb: &Sandbox,
    col: &TempDir,
    prepare: impl FnOnce(&Paths, &MemoryStore),
) -> (Started, Arc<MemoryStore>) {
    let secrets = Arc::new(MemoryStore::new());
    let store: Arc<dyn SecretStore> = secrets.clone();
    let s = super::started_on(cx, col, |paths| prepare(paths, &secrets), store);
    s.ws.update(cx, |w, cx| {
        w.set_git(Some(sb.options()), cx);
        w.set_watch_factory(Some(notify_factory()), cx);
    });
    (s, secrets)
}

/// Team mode on `s` as `role` with copies under `<files>/conflicts/b1`, settled after its start pull.
pub fn start_team_as(
    cx: &mut TestAppContext,
    sb: &Sandbox,
    s: &Started,
    role: Role,
    debounce: std::time::Duration,
) {
    use apinata_core::team::{Identity, TeamAuth, TeamConfig, TeamState};
    let config = TeamConfig::new(
        s.root.clone(),
        copies_of(s),
        TeamAuth::new("http://127.0.0.1:9", "plk_test"),
        Identity::new("Ada", "ada@example.com"),
        role,
    )
    .base_options(sb.options())
    .debounce_unclamped(debounce);
    let root = s.root.clone();
    s.ws.update(cx, |w, cx| w.start_team(&root, config, None, cx));
    super::wait_for(cx, "the start pull", |cx| {
        s.ws.read_with(cx, |w, _| {
            w.team_status(&root).is_some_and(|t| {
                matches!(
                    t.state,
                    TeamState::Idle | TeamState::ReadOnly { paused: false }
                )
            })
        })
    });
}

pub fn copies_of(s: &Started) -> PathBuf {
    s.files.path().join("conflicts").join("b1")
}
