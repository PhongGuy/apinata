//! A stub team server on the app's tokio runtime and a signed-in team file.

use super::potluck::tok;
use super::{Started, collection, frame, started_on};
use apinata_app::persist::Paths;
use apinata_core::team::{Account, ServerUrl, TeamStore, token_key};
use apinata_core::{MemoryStore, SecretStore};
use gpui_kit::TestAppContext;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use wiremock::{Mock, MockServer};

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
