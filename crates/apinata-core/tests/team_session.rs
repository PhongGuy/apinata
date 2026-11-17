//! The team file, the keychain token and the sign-in, session and sign-out flows.

use crate::common::dead_port;
use crate::common::potluck::*;
use apinata_core::team::{
    self, Account, Binding, HttpTeamClient, Role, ServerUrl, TeamError, TeamStore, token_key,
};
use apinata_core::{CancelFlag, MemoryStore, SecretError, SecretKey, SecretStore};
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use wiremock::{MockServer, ResponseTemplate};

/// Fails every call, like a locked keychain.
struct BrokenStore;

impl SecretStore for BrokenStore {
    fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
        Err(SecretError::Unavailable {
            message: "locked".into(),
        })
    }
    fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
        Err(SecretError::Unavailable {
            message: "locked".into(),
        })
    }
    fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
        Err(SecretError::Unavailable {
            message: "locked".into(),
        })
    }
}

fn file(dir: &TempDir) -> PathBuf {
    dir.path().join("nested/team.json")
}

fn url(s: &MockServer) -> ServerUrl {
    ServerUrl::parse(&s.uri()).unwrap()
}

fn unauthenticated() -> ResponseTemplate {
    ResponseTemplate::new(401).set_body_json(error("unauthenticated", "Sign in again.", None))
}

async fn joined(s: &MockServer, secrets: &MemoryStore, team_file: &Path) -> Account {
    answer(s, "GET", "/api/v1/server", info(true)).await;
    answer(s, "POST", "/api/v1/session", session(7, true)).await;
    team::sign_in(
        &client(s),
        secrets,
        team_file,
        "ada",
        "hunter2",
        &CancelFlag::new(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn sign_in_keeps_the_token_in_the_keychain_and_the_account_in_the_file() {
    let (s, dir, secrets) = (
        MockServer::start().await,
        TempDir::new().unwrap(),
        MemoryStore::new(),
    );
    let account = joined(&s, &secrets, &file(&dir)).await;
    assert_eq!(
        (
            account.username.as_str(),
            account.token_id.as_str(),
            account.server_admin
        ),
        ("ada", "7", true)
    );
    let key = token_key(&url(&s), "ada");
    assert_eq!(
        key,
        SecretKey::new("", "team-server", format!("{}#ada", s.uri()))
    );
    assert_eq!(secrets.get(&key).unwrap(), Some(tok()));
    assert_eq!(TeamStore::load(&file(&dir)).unwrap().accounts(), [account]);
    let text = std::fs::read_to_string(file(&dir)).unwrap();
    assert!(
        !text.contains("plk_") && !text.contains("hunter2"),
        "{text}"
    );
}

#[tokio::test]
async fn a_server_without_password_sign_in_is_never_sent_the_password() {
    let (s, dir, secrets) = (
        MockServer::start().await,
        TempDir::new().unwrap(),
        MemoryStore::new(),
    );
    answer(&s, "GET", "/api/v1/server", info(false)).await;
    answer(&s, "POST", "/api/v1/session", session(7, false)).await;
    let e = team::sign_in(
        &client(&s),
        &secrets,
        &file(&dir),
        "ada",
        "hunter2",
        &CancelFlag::new(),
    )
    .await
    .unwrap_err();
    assert_eq!(e, TeamError::PasswordSignInOff);
    assert_eq!(hits(&s, "POST", "/api/v1/session").await, 0);
    assert!(!file(&dir).exists());
}

#[tokio::test]
async fn a_locked_keychain_revokes_the_new_token_and_saves_nothing() {
    let (s, dir) = (MockServer::start().await, TempDir::new().unwrap());
    answer(&s, "GET", "/api/v1/server", info(true)).await;
    answer(&s, "POST", "/api/v1/session", session(7, false)).await;
    answer(&s, "DELETE", "/api/v1/tokens/7", ResponseTemplate::new(204)).await;
    let e = team::sign_in(
        &client(&s),
        &BrokenStore,
        &file(&dir),
        "ada",
        "hunter2",
        &CancelFlag::new(),
    )
    .await
    .unwrap_err();
    assert!(matches!(e, TeamError::Secrets(_)), "{e:?}");
    assert_eq!(hits(&s, "DELETE", "/api/v1/tokens/7").await, 1);
    assert!(!file(&dir).exists());
}

#[tokio::test]
async fn joining_again_revokes_the_previous_token_once_the_new_one_is_kept() {
    let (s, dir, secrets) = (
        MockServer::start().await,
        TempDir::new().unwrap(),
        MemoryStore::new(),
    );
    joined(&s, &secrets, &file(&dir)).await;
    s.reset().await;
    answer(&s, "GET", "/api/v1/server", info(true)).await;
    answer(&s, "POST", "/api/v1/session", session(8, true)).await;
    answer(&s, "DELETE", "/api/v1/tokens/7", ResponseTemplate::new(204)).await;
    let again = team::sign_in(
        &client(&s),
        &secrets,
        &file(&dir),
        "ada",
        "hunter2",
        &CancelFlag::new(),
    )
    .await
    .unwrap();
    assert_eq!(again.token_id, "8");
    assert_eq!(hits(&s, "DELETE", "/api/v1/tokens/7").await, 1);
    let store = TeamStore::load(&file(&dir)).unwrap();
    assert_eq!(store.accounts().len(), 1);
    assert_eq!(store.account(&url(&s)).unwrap().token_id, "8");
}

#[tokio::test]
async fn rejoining_revokes_the_previous_token_with_the_new_one() {
    let (s, dir, secrets) = (
        MockServer::start().await,
        TempDir::new().unwrap(),
        MemoryStore::new(),
    );
    joined(&s, &secrets, &file(&dir)).await;
    // The old token is gone from the keychain, as after a quit without one.
    secrets.delete(&token_key(&url(&s), "ada")).unwrap();
    s.reset().await;
    let fresh = format!("plk_{}", "B".repeat(43));
    let body = serde_json::json!({"token": fresh, "token_meta": token_meta(8), "user": user("ada", false)});
    answer(&s, "GET", "/api/v1/server", info(true)).await;
    answer(
        &s,
        "POST",
        "/api/v1/session",
        ResponseTemplate::new(201).set_body_json(body),
    )
    .await;
    wiremock::Mock::given(wiremock::matchers::method("DELETE"))
        .and(wiremock::matchers::path("/api/v1/tokens/7"))
        .and(wiremock::matchers::header(
            "authorization",
            format!("Bearer {fresh}").as_str(),
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&s)
        .await;
    team::sign_in(
        &client(&s),
        &secrets,
        &file(&dir),
        "ada",
        "hunter2",
        &CancelFlag::new(),
    )
    .await
    .unwrap();
    s.verify().await;
}

#[tokio::test]
async fn a_mistyped_password_on_rejoining_keeps_the_working_session() {
    let (s, dir, secrets) = (
        MockServer::start().await,
        TempDir::new().unwrap(),
        MemoryStore::new(),
    );
    joined(&s, &secrets, &file(&dir)).await;
    s.reset().await;
    answer(&s, "GET", "/api/v1/server", info(true)).await;
    answer(&s, "POST", "/api/v1/session", unauthenticated()).await;
    let e = team::sign_in(
        &client(&s),
        &secrets,
        &file(&dir),
        "ada",
        "typo",
        &CancelFlag::new(),
    )
    .await
    .unwrap_err();
    assert_eq!(e, TeamError::Unauthenticated);
    assert_eq!(hits(&s, "DELETE", "/api/v1/tokens/7").await, 0);
    assert_eq!(
        secrets.get(&token_key(&url(&s), "ada")).unwrap(),
        Some(tok())
    );
    assert_eq!(
        TeamStore::load(&file(&dir))
            .unwrap()
            .account(&url(&s))
            .unwrap()
            .token_id,
        "7"
    );
}

#[tokio::test]
async fn a_sign_in_cancelled_while_the_server_answers_revokes_its_token_and_keeps_nothing() {
    let (s, dir, secrets) = (
        MockServer::start().await,
        TempDir::new().unwrap(),
        MemoryStore::new(),
    );
    joined(&s, &secrets, &file(&dir)).await;
    let key = token_key(&url(&s), "ada");
    let kept = format!("plk_{}", "C".repeat(43));
    secrets.set(&key, &kept).unwrap();
    let before = std::fs::read_to_string(file(&dir)).unwrap();
    s.reset().await;
    // The user cancels while the server is still issuing the new token.
    let cancel = CancelFlag::new();
    let flag = cancel.clone();
    answer(&s, "GET", "/api/v1/server", info(true)).await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/api/v1/session"))
        .respond_with(move |_: &wiremock::Request| {
            flag.cancel();
            session(8, false)
        })
        .mount(&s)
        .await;
    answer(&s, "DELETE", "/api/v1/tokens/8", ResponseTemplate::new(204)).await;
    let e = team::sign_in(
        &client(&s),
        &secrets,
        &file(&dir),
        "ada",
        "hunter2",
        &cancel,
    )
    .await
    .unwrap_err();
    assert_eq!(e, TeamError::Cancelled);
    assert_eq!(hits(&s, "DELETE", "/api/v1/tokens/8").await, 1);
    assert_eq!(hits(&s, "DELETE", "/api/v1/tokens/7").await, 0);
    assert_eq!(secrets.get(&key).unwrap(), Some(kept));
    assert_eq!(std::fs::read_to_string(file(&dir)).unwrap(), before);
}

#[tokio::test]
async fn a_sign_in_cancelled_before_it_starts_never_sends_the_password() {
    let (s, dir, secrets) = (
        MockServer::start().await,
        TempDir::new().unwrap(),
        MemoryStore::new(),
    );
    answer(&s, "GET", "/api/v1/server", info(true)).await;
    answer(&s, "POST", "/api/v1/session", session(7, false)).await;
    let cancel = CancelFlag::new();
    cancel.cancel();
    let e = team::sign_in(
        &client(&s),
        &secrets,
        &file(&dir),
        "ada",
        "hunter2",
        &cancel,
    )
    .await
    .unwrap_err();
    assert_eq!(e, TeamError::Cancelled);
    assert_eq!(hits(&s, "POST", "/api/v1/session").await, 0);
    assert!(!file(&dir).exists());
}

#[tokio::test]
async fn joining_as_another_user_forgets_the_first_users_keychain_entry() {
    let (s, dir, secrets) = (
        MockServer::start().await,
        TempDir::new().unwrap(),
        MemoryStore::new(),
    );
    joined(&s, &secrets, &file(&dir)).await;
    s.reset().await;
    let bob = serde_json::json!({"token": tok(), "token_meta": token_meta(9), "user": user("bob", false)});
    answer(&s, "GET", "/api/v1/server", info(true)).await;
    answer(
        &s,
        "POST",
        "/api/v1/session",
        ResponseTemplate::new(201).set_body_json(bob),
    )
    .await;
    answer(&s, "DELETE", "/api/v1/tokens/7", ResponseTemplate::new(204)).await;
    team::sign_in(
        &client(&s),
        &secrets,
        &file(&dir),
        "bob",
        "pw",
        &CancelFlag::new(),
    )
    .await
    .unwrap();
    assert_eq!(secrets.get(&token_key(&url(&s), "ada")).unwrap(), None);
    assert_eq!(
        secrets.get(&token_key(&url(&s), "bob")).unwrap(),
        Some(tok())
    );
    assert_eq!(
        TeamStore::load(&file(&dir))
            .unwrap()
            .account(&url(&s))
            .unwrap()
            .username,
        "bob"
    );
}

#[tokio::test]
async fn sign_out_revokes_then_forgets_the_token_and_the_account_but_keeps_bindings() {
    let (s, dir, secrets) = (
        MockServer::start().await,
        TempDir::new().unwrap(),
        MemoryStore::new(),
    );
    let account = joined(&s, &secrets, &file(&dir)).await;
    TeamStore::update(&file(&dir), |t| {
        t.put_binding(Binding::new(
            PathBuf::from("/w/api"),
            &account,
            "core",
            "api",
            Role::Write,
        ))
    })
    .unwrap();
    answer(&s, "DELETE", "/api/v1/tokens/7", ResponseTemplate::new(204)).await;
    team::sign_out(&client(&s), &secrets, &file(&dir))
        .await
        .unwrap();
    assert_eq!(hits(&s, "DELETE", "/api/v1/tokens/7").await, 1);
    assert_eq!(secrets.get(&token_key(&url(&s), "ada")).unwrap(), None);
    let reloaded = TeamStore::load(&file(&dir)).unwrap();
    assert!(reloaded.accounts().is_empty());
    assert!(reloaded.binding_at(Path::new("/w/api")).is_some());
}

#[tokio::test]
async fn sign_out_after_the_server_already_revoked_still_cleans_up() {
    let (s, dir, secrets) = (
        MockServer::start().await,
        TempDir::new().unwrap(),
        MemoryStore::new(),
    );
    joined(&s, &secrets, &file(&dir)).await;
    answer(&s, "DELETE", "/api/v1/tokens/7", unauthenticated()).await;
    team::sign_out(&client(&s), &secrets, &file(&dir))
        .await
        .unwrap();
    assert!(TeamStore::load(&file(&dir)).unwrap().accounts().is_empty());
    assert_eq!(secrets.get(&token_key(&url(&s), "ada")).unwrap(), None);
}

#[tokio::test]
async fn sign_out_without_a_token_forgets_the_account_without_a_request() {
    let (s, dir, secrets) = (
        MockServer::start().await,
        TempDir::new().unwrap(),
        MemoryStore::new(),
    );
    joined(&s, &secrets, &file(&dir)).await;
    secrets.delete(&token_key(&url(&s), "ada")).unwrap();
    team::sign_out(&client(&s), &secrets, &file(&dir))
        .await
        .unwrap();
    assert_eq!(hits(&s, "DELETE", "/api/v1/tokens/7").await, 0);
    assert!(TeamStore::load(&file(&dir)).unwrap().accounts().is_empty());
}

#[tokio::test]
async fn sign_out_with_a_locked_keychain_changes_nothing() {
    let (s, dir) = (MockServer::start().await, TempDir::new().unwrap());
    TeamStore::update(&file(&dir), |t| {
        t.put_account(Account::new(&url(&s), "ada", "Ada Lovelace", "7", false))
    })
    .unwrap();
    let e = team::sign_out(&client(&s), &BrokenStore, &file(&dir))
        .await
        .unwrap_err();
    assert!(matches!(e, TeamError::Secrets(_)), "{e:?}");
    assert_eq!(hits(&s, "DELETE", "/api/v1/tokens/7").await, 0);
    assert_eq!(TeamStore::load(&file(&dir)).unwrap().accounts().len(), 1);
}

#[tokio::test]
async fn sign_out_while_offline_keeps_everything_so_it_can_be_retried() {
    let (dir, secrets) = (TempDir::new().unwrap(), MemoryStore::new());
    let server = ServerUrl::parse(&format!("http://127.0.0.1:{}", dead_port())).unwrap();
    TeamStore::update(&file(&dir), |t| {
        t.put_account(Account::new(&server, "ada", "Ada Lovelace", "7", false))
    })
    .unwrap();
    secrets.set(&token_key(&server, "ada"), &tok()).unwrap();
    let api = HttpTeamClient::new(server.clone()).unwrap();
    let e = team::sign_out(&api, &secrets, &file(&dir))
        .await
        .unwrap_err();
    assert!(matches!(e, TeamError::Offline(_)), "{e:?}");
    assert_eq!(
        secrets.get(&token_key(&server, "ada")).unwrap(),
        Some(tok())
    );
    assert_eq!(TeamStore::load(&file(&dir)).unwrap().accounts().len(), 1);
}

#[tokio::test]
async fn a_revoked_session_deletes_the_token_and_keeps_the_account() {
    let (s, dir, secrets) = (
        MockServer::start().await,
        TempDir::new().unwrap(),
        MemoryStore::new(),
    );
    joined(&s, &secrets, &file(&dir)).await;
    answer(&s, "GET", "/api/v1/me", unauthenticated()).await;
    let e = team::current(&client(&s), &secrets, &file(&dir))
        .await
        .unwrap_err();
    assert_eq!(e, TeamError::Unauthenticated);
    assert_eq!(secrets.get(&token_key(&url(&s), "ada")).unwrap(), None);
    assert_eq!(TeamStore::load(&file(&dir)).unwrap().accounts().len(), 1);
    s.reset().await;
    assert_eq!(
        team::current(&client(&s), &secrets, &file(&dir))
            .await
            .unwrap_err(),
        TeamError::Unauthenticated
    );
    assert_eq!(hits(&s, "GET", "/api/v1/me").await, 0);
}

#[tokio::test]
async fn a_session_refreshes_the_admin_flag_from_the_server() {
    let (s, dir, secrets) = (
        MockServer::start().await,
        TempDir::new().unwrap(),
        MemoryStore::new(),
    );
    joined(&s, &secrets, &file(&dir)).await;
    answer(&s, "GET", "/api/v1/me", me(false, vec![])).await;
    team::current(&client(&s), &secrets, &file(&dir))
        .await
        .unwrap();
    assert!(
        !TeamStore::load(&file(&dir))
            .unwrap()
            .account(&url(&s))
            .unwrap()
            .server_admin
    );
}

#[test]
fn refreshing_an_account_keeps_its_place_in_the_file() {
    let dir = TempDir::new().unwrap();
    let (b, a) = (
        ServerUrl::parse("http://127.0.0.2:9").unwrap(),
        ServerUrl::parse("http://127.0.0.1:9").unwrap(),
    );
    TeamStore::update(&file(&dir), |t| {
        t.put_account(Account::new(&b, "ada", "Ada", "1", false));
        t.put_account(Account::new(&a, "ada", "Ada", "2", false));
        t.put_account(Account::new(&b, "ada", "Ada Lovelace", "1", true));
    })
    .unwrap();
    let store = TeamStore::load(&file(&dir)).unwrap();
    let order: Vec<_> = (store.accounts().iter())
        .map(|x| x.server.clone())
        .collect();
    assert_eq!(order, [b.as_str(), a.as_str()]);
    assert!(store.accounts()[0].server_admin);
}

#[test]
fn the_kept_token_is_read_back_and_anything_else_reads_as_none() {
    let server = ServerUrl::parse("http://127.0.0.1:9").unwrap();
    let secrets = MemoryStore::new();
    assert!(
        team::kept_token(&server, &secrets, "ada")
            .unwrap()
            .is_none()
    );
    secrets
        .set(&token_key(&server, "ada"), "not a token")
        .unwrap();
    assert!(
        team::kept_token(&server, &secrets, "ada")
            .unwrap()
            .is_none()
    );
    secrets.set(&token_key(&server, "ada"), &tok()).unwrap();
    let kept = team::kept_token(&server, &secrets, "ada").unwrap().unwrap();
    assert_eq!(kept.expose(), tok());
    let locked = team::kept_token(&server, &BrokenStore, "ada").unwrap_err();
    assert!(matches!(locked, TeamError::Secrets(_)), "{locked:?}");
}

#[test]
fn bindings_are_keyed_by_root_and_found_by_workspace() {
    let dir = TempDir::new().unwrap();
    let server = ServerUrl::parse("http://127.0.0.1:9").unwrap();
    let account = Account::new(&server, "ada", "Ada Lovelace", "7", false);
    TeamStore::update(&file(&dir), |t| {
        t.put_binding(Binding::new(
            PathBuf::from("/w/api"),
            &account,
            "core",
            "api",
            Role::Read,
        ));
        t.put_binding(Binding::new(
            PathBuf::from("/w/site"),
            &account,
            "web",
            "site",
            Role::Write,
        ));
        t.put_binding(Binding::new(
            PathBuf::from("/w/api"),
            &account,
            "core",
            "api",
            Role::Admin,
        ));
    })
    .unwrap();
    let reloaded = TeamStore::load(&file(&dir)).unwrap();
    assert_eq!(reloaded.bindings().len(), 2);
    assert_eq!(
        reloaded.binding_at(Path::new("/w/api")).unwrap().role,
        Role::Admin
    );
    assert_eq!(
        reloaded.binding_for(&server, "web", "site").unwrap().root,
        PathBuf::from("/w/site")
    );
    TeamStore::update(&file(&dir), |t| t.remove_binding(Path::new("/w/api"))).unwrap();
    assert!(
        TeamStore::load(&file(&dir))
            .unwrap()
            .binding_at(Path::new("/w/api"))
            .is_none()
    );
}

#[test]
fn concurrent_updates_never_lose_a_write() {
    let dir = TempDir::new().unwrap();
    let path = file(&dir);
    let account = Account::new(
        &ServerUrl::parse("http://127.0.0.1:9").unwrap(),
        "ada",
        "Ada",
        "7",
        false,
    );
    std::thread::scope(|scope| {
        for n in 0..8 {
            let (path, account) = (&path, &account);
            scope.spawn(move || {
                for k in 0..5 {
                    let root = PathBuf::from(format!("/w/{n}-{k}"));
                    TeamStore::update(path, |t| {
                        t.put_binding(Binding::new(root, account, "core", "api", Role::Read))
                    })
                    .unwrap();
                }
            });
        }
    });
    assert_eq!(TeamStore::load(&path).unwrap().bindings().len(), 40);
}

#[test]
fn a_corrupt_team_file_is_a_store_error_and_is_not_overwritten() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(file(&dir).parent().unwrap()).unwrap();
    std::fs::write(file(&dir), "{not json").unwrap();
    assert!(matches!(
        TeamStore::load(&file(&dir)),
        Err(TeamError::Store(_))
    ));
    assert!(matches!(
        TeamStore::update(&file(&dir), |_| ()),
        Err(TeamError::Store(_))
    ));
    assert_eq!(std::fs::read_to_string(file(&dir)).unwrap(), "{not json");
}

/// A session answer for `name` with a token unlike `tok()`, so the keychain shows which one it holds.
#[cfg(unix)]
fn other_session(name: &str, id: i64) -> (String, ResponseTemplate) {
    let fresh = format!("plk_{}", "B".repeat(43));
    let body = serde_json::json!({"token": fresh, "token_meta": token_meta(id), "user": user(name, false)});
    (fresh, ResponseTemplate::new(201).set_body_json(body))
}

/// Rejoins as `name` while the team file's folder refuses writes, and checks ada's session survives.
#[cfg(unix)]
async fn rejoin_with_an_unwritable_team_file(name: &str) {
    let (s, dir, secrets) = (
        MockServer::start().await,
        TempDir::new().unwrap(),
        MemoryStore::new(),
    );
    joined(&s, &secrets, &file(&dir)).await;
    s.reset().await;
    let (fresh, response) = other_session(name, 8);
    answer(&s, "GET", "/api/v1/server", info(true)).await;
    answer(&s, "POST", "/api/v1/session", response).await;
    answer(&s, "DELETE", "/api/v1/tokens/8", ResponseTemplate::new(204)).await;
    let parent = file(&dir).parent().unwrap().to_path_buf();
    let guard = crate::common::ModeGuard::set(&parent, 0o555);
    let e = team::sign_in(
        &client(&s),
        &secrets,
        &file(&dir),
        name,
        "pw",
        &CancelFlag::new(),
    )
    .await
    .unwrap_err();
    drop(guard);
    assert!(matches!(e, TeamError::Store(_)), "{e:?}");
    assert_eq!(hits(&s, "DELETE", "/api/v1/tokens/8").await, 1);
    assert_eq!(hits(&s, "DELETE", "/api/v1/tokens/7").await, 0);
    assert_eq!(
        secrets.get(&token_key(&url(&s), "ada")).unwrap(),
        Some(tok())
    );
    if name != "ada" {
        assert_eq!(secrets.get(&token_key(&url(&s), name)).unwrap(), None);
    }
    assert_ne!(
        secrets.get(&token_key(&url(&s), name)).unwrap(),
        Some(fresh)
    );
    assert_eq!(
        TeamStore::load(&file(&dir))
            .unwrap()
            .account(&url(&s))
            .unwrap()
            .token_id,
        "7"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_failed_save_on_rejoining_restores_the_previous_token() {
    rejoin_with_an_unwritable_team_file("ada").await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_failed_save_on_joining_as_another_user_leaves_no_dead_token() {
    rejoin_with_an_unwritable_team_file("bob").await;
}

/// A server whose `/me` refuses the session after `during` has run, as a sign-in that lands
/// while the check is in flight would leave things.
struct RacedMe {
    server: ServerUrl,
    during: Box<dyn Fn() + Send + Sync>,
}

impl team::TeamApi for RacedMe {
    fn server(&self) -> &ServerUrl {
        &self.server
    }
    fn server_info(&self) -> team::TeamFuture<'_, team::ServerInfo> {
        unreachable!()
    }
    fn sign_in<'a>(&'a self, _: &'a str, _: &'a str) -> team::TeamFuture<'a, team::SignIn> {
        unreachable!()
    }
    fn me<'a>(&'a self, _: &'a team::TeamToken) -> team::TeamFuture<'a, team::Me> {
        (self.during)();
        Box::pin(async { Err(TeamError::Unauthenticated) })
    }
    fn teams<'a>(&'a self, _: &'a team::TeamToken) -> team::TeamFuture<'a, Vec<team::Team>> {
        unreachable!()
    }
    fn create_workspace<'a>(
        &'a self,
        _: &'a team::TeamToken,
        _: &'a team::NewWorkspace,
    ) -> team::TeamFuture<'a, team::Workspace> {
        unreachable!()
    }
    fn revoke<'a>(&'a self, _: &'a team::TeamToken, _: &'a str) -> team::TeamFuture<'a, ()> {
        unreachable!()
    }
    fn server_update<'a>(
        &'a self,
        _: &'a team::TeamToken,
    ) -> team::TeamFuture<'a, team::ServerUpdate> {
        unreachable!()
    }
    fn check_server_update<'a>(
        &'a self,
        _: &'a team::TeamToken,
    ) -> team::TeamFuture<'a, team::ServerUpdate> {
        unreachable!()
    }
}

// Ada signed in on a stub server with token id `7` and the shared test token.
fn raced(dir: &TempDir, secrets: &MemoryStore) -> ServerUrl {
    let server = ServerUrl::parse("http://127.0.0.1:9").unwrap();
    TeamStore::update(&file(dir), |t| {
        t.put_account(Account::new(&server, "ada", "Ada Lovelace", "7", false))
    })
    .unwrap();
    secrets.set(&token_key(&server, "ada"), &tok()).unwrap();
    server
}

#[tokio::test]
async fn a_token_kept_while_the_check_was_refused_survives() {
    let (dir, secrets) = (
        TempDir::new().unwrap(),
        std::sync::Arc::new(MemoryStore::new()),
    );
    let server = raced(&dir, &secrets);
    let fresh = format!("plk_{}", "B".repeat(43));
    let (store, key, text) = (secrets.clone(), token_key(&server, "ada"), fresh.clone());
    let api = RacedMe {
        server,
        during: Box::new(move || store.set(&key, &text).unwrap()),
    };
    let e = team::current(&api, &*secrets, &file(&dir))
        .await
        .unwrap_err();
    assert_eq!(e, TeamError::Unauthenticated);
    assert_eq!(
        secrets.get(&token_key(&api.server, "ada")).unwrap(),
        Some(fresh)
    );
}

#[tokio::test]
async fn a_session_replaced_while_the_check_was_refused_keeps_its_token() {
    let (dir, secrets) = (TempDir::new().unwrap(), MemoryStore::new());
    let server = raced(&dir, &secrets);
    let (team_file, at) = (file(&dir), server.clone());
    let api = RacedMe {
        server,
        during: Box::new(move || {
            TeamStore::update(&team_file, |t| {
                t.put_account(Account::new(&at, "ada", "Ada Lovelace", "8", false))
            })
            .unwrap()
        }),
    };
    let e = team::current(&api, &secrets, &file(&dir))
        .await
        .unwrap_err();
    assert_eq!(e, TeamError::Unauthenticated);
    assert_eq!(
        secrets.get(&token_key(&api.server, "ada")).unwrap(),
        Some(tok())
    );
}
