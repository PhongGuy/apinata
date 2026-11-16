//! Resuming a bound folder: the role, identity, token and events address its loop starts with.

use crate::common::dead_port;
use crate::common::potluck::*;
use apinata_core::team::{
    self, Account, Binding, HttpTeamClient, Identity, Role, ServerUrl, Standing, TeamError,
    TeamStore, token_key,
};
use apinata_core::{MemoryStore, SecretStore};
use serde_json::json;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use wiremock::{MockServer, ResponseTemplate};

fn file(dir: &TempDir) -> PathBuf {
    dir.path().join("team.json")
}

fn ada(server: &str) -> Account {
    Account::new(
        &ServerUrl::parse(server).unwrap(),
        "ada",
        "Ada Lovelace",
        "7",
        false,
    )
}

/// `/work/api` bound to `core/api` as `role`, on `account`'s server, with `branch` known.
fn binding(account: &Account, role: Role, branch: &str) -> Binding {
    let mut b = Binding::new(PathBuf::from("/work/api"), account, "core", "api", role);
    b.branch = branch.to_owned();
    b
}

/// Keeps `account` and its token, and `b`, as an earlier session would have.
fn kept(team_file: &Path, secrets: &MemoryStore, account: Account, b: &Binding) {
    let url = ServerUrl::parse(&account.server).unwrap();
    secrets.set(&token_key(&url, "ada"), &tok()).unwrap();
    let b = b.clone();
    TeamStore::update(team_file, |t| {
        t.put_account(account);
        t.put_binding(b);
    })
    .unwrap();
}

async fn fresh() -> (MockServer, TempDir, MemoryStore) {
    (
        MockServer::start().await,
        TempDir::new().unwrap(),
        MemoryStore::new(),
    )
}

fn noreply() -> Identity {
    Identity::new("Ada Lovelace", "ada@users.noreply.127.0.0.1")
}

#[tokio::test]
async fn a_listed_workspace_resumes_with_the_server_role_and_refreshes_the_binding() {
    let (s, dir, secrets) = fresh().await;
    let base = s.uri();
    let listed = workspace(&base, "core", "api", "apinata", "read");
    answer(&s, "GET", "/api/v1/me", me(false, vec![listed])).await;
    let b = binding(&ada(&base), Role::Write, "");
    kept(&file(&dir), &secrets, ada(&base), &b);
    let r = team::resume(&client(&s), &secrets, &file(&dir), &b)
        .await
        .unwrap();
    assert_eq!(r.standing, Standing::Ready);
    assert_eq!(r.role, Role::Read);
    assert_eq!(r.identity, noreply());
    assert_eq!(r.auth.token(), tok());
    let events = format!("{base}/api/v1/workspaces/core/api/events");
    assert_eq!(r.events_url.as_deref(), Some(events.as_str()));
    assert_eq!(r.branch, "main");
    let store = TeamStore::load(&file(&dir)).unwrap();
    let bound = store.binding_at(Path::new("/work/api")).unwrap();
    assert_eq!((bound.role, bound.branch.as_str()), (Role::Read, "main"));
    let account = store.account(&ServerUrl::parse(&base).unwrap()).unwrap();
    assert_eq!(account.identity, Some(noreply()));
}

#[tokio::test]
async fn a_refused_token_resumes_signed_out_without_token_or_events() {
    let (s, dir, secrets) = fresh().await;
    let base = s.uri();
    let refused =
        ResponseTemplate::new(401).set_body_json(error("unauthenticated", "Revoked.", None));
    answer(&s, "GET", "/api/v1/me", refused).await;
    let b = binding(&ada(&base), Role::Write, "main");
    kept(&file(&dir), &secrets, ada(&base), &b);
    let r = team::resume(&client(&s), &secrets, &file(&dir), &b)
        .await
        .unwrap();
    assert_eq!((r.standing, r.role), (Standing::SignedOut, Role::Write));
    assert_eq!(r.auth.token(), "");
    assert_eq!(r.events_url, None);
    let key = token_key(&ServerUrl::parse(&base).unwrap(), "ada");
    assert_eq!(secrets.get(&key).unwrap(), None);
}

#[tokio::test]
async fn no_account_resumes_signed_out_under_the_noreply_identity_without_asking() {
    let (s, dir, secrets) = fresh().await;
    let b = binding(&ada(&s.uri()), Role::Write, "main");
    TeamStore::update(&file(&dir), |t| t.put_binding(b.clone())).unwrap();
    let r = team::resume(&client(&s), &secrets, &file(&dir), &b)
        .await
        .unwrap();
    assert_eq!(r.standing, Standing::SignedOut);
    assert_eq!(
        r.identity,
        Identity::new("ada", "ada@users.noreply.127.0.0.1")
    );
    assert_eq!(hits(&s, "GET", "/api/v1/me").await, 0);
}

#[tokio::test]
async fn a_folder_bound_by_another_user_resumes_signed_out_and_keeps_its_binding() {
    let (s, dir, secrets) = fresh().await;
    let base = s.uri();
    let listed = workspace(&base, "core", "api", "apinata", "admin");
    answer(&s, "GET", "/api/v1/me", me(false, vec![listed])).await;
    let grace = Account::new(
        &ServerUrl::parse(&base).unwrap(),
        "grace",
        "Grace",
        "9",
        false,
    );
    let b = binding(&grace, Role::Read, "main");
    kept(&file(&dir), &secrets, ada(&base), &b);
    let r = team::resume(&client(&s), &secrets, &file(&dir), &b)
        .await
        .unwrap();
    assert_eq!((r.standing, r.role), (Standing::SignedOut, Role::Read));
    assert_eq!(r.auth.token(), "");
    assert_eq!(hits(&s, "GET", "/api/v1/me").await, 0);
    let store = TeamStore::load(&file(&dir)).unwrap();
    assert_eq!(
        store.binding_at(Path::new("/work/api")).unwrap().role,
        Role::Read
    );
}

#[tokio::test]
async fn a_server_answering_as_another_user_resumes_signed_out_under_the_bound_identity() {
    let (s, dir, secrets) = fresh().await;
    let base = s.uri();
    let listed = workspace(&base, "core", "api", "apinata", "admin");
    // The account was swapped for grace's while the resume asked, so `/me` speaks for her.
    let grace = json!({
        "user": {"username": "grace", "display_name": "Grace", "email": null,
                 "server_admin": false, "disabled": false, "created_at": "2026-11-15T10:00:00Z"},
        "identity": {"name": "Grace", "email": "grace@users.noreply.127.0.0.1"},
        "workspaces": [listed],
    });
    answer(
        &s,
        "GET",
        "/api/v1/me",
        ResponseTemplate::new(200).set_body_json(grace),
    )
    .await;
    let b = binding(&ada(&base), Role::Write, "");
    kept(&file(&dir), &secrets, ada(&base), &b);
    let r = team::resume(&client(&s), &secrets, &file(&dir), &b)
        .await
        .unwrap();
    assert_eq!((r.standing, r.role), (Standing::SignedOut, Role::Write));
    assert_eq!(r.identity.name(), "Ada Lovelace");
    assert_eq!(r.auth.token(), "");
    assert_eq!(r.events_url, None);
    let store = TeamStore::load(&file(&dir)).unwrap();
    let bound = store.binding_at(Path::new("/work/api")).unwrap();
    assert_eq!((bound.role, bound.branch.as_str()), (Role::Write, ""));
}

#[tokio::test]
async fn a_workspace_missing_from_me_is_gone() {
    let (s, dir, secrets) = fresh().await;
    let base = s.uri();
    let other = workspace(&base, "core", "web", "apinata", "write");
    answer(&s, "GET", "/api/v1/me", me(false, vec![other])).await;
    let b = binding(&ada(&base), Role::Write, "main");
    kept(&file(&dir), &secrets, ada(&base), &b);
    let r = team::resume(&client(&s), &secrets, &file(&dir), &b)
        .await
        .unwrap();
    assert_eq!((r.standing, r.role), (Standing::Gone, Role::Write));
    assert_eq!(r.events_url, None);
}

#[tokio::test]
async fn a_foreign_events_address_is_refused() {
    let (s, dir, secrets) = fresh().await;
    let base = s.uri();
    let mut listed = workspace(&base, "core", "api", "apinata", "write");
    listed["events_url"] = json!("http://evil.example/api/v1/workspaces/core/api/events");
    answer(&s, "GET", "/api/v1/me", me(false, vec![listed])).await;
    let b = binding(&ada(&base), Role::Write, "main");
    kept(&file(&dir), &secrets, ada(&base), &b);
    let r = team::resume(&client(&s), &secrets, &file(&dir), &b).await;
    assert!(matches!(r, Err(TeamError::Protocol(_))), "{r:?}");
}

#[tokio::test]
async fn an_unreachable_server_resumes_offline_with_the_cached_identity_and_token() {
    let (dir, secrets) = (TempDir::new().unwrap(), MemoryStore::new());
    let server = format!("http://127.0.0.1:{}", dead_port());
    let mut account = ada(&server);
    account.identity = Some(Identity::new("Ada Cached", "ada@cached.example"));
    let b = binding(&account, Role::Write, "main");
    kept(&file(&dir), &secrets, account, &b);
    let api = HttpTeamClient::new(ServerUrl::parse(&server).unwrap()).unwrap();
    let r = team::resume(&api, &secrets, &file(&dir), &b).await.unwrap();
    assert_eq!((r.standing, r.role), (Standing::Offline, Role::Write));
    assert_eq!(
        r.identity,
        Identity::new("Ada Cached", "ada@cached.example")
    );
    assert_eq!(r.auth.token(), tok());
    let events = format!("{server}/api/v1/workspaces/core/api/events");
    assert_eq!(r.events_url.as_deref(), Some(events.as_str()));
}

#[tokio::test]
async fn offline_with_no_cache_and_no_branch_falls_back_and_follows_no_events() {
    let (dir, secrets) = (TempDir::new().unwrap(), MemoryStore::new());
    let server = format!("http://127.0.0.1:{}", dead_port());
    let b = binding(&ada(&server), Role::Read, "");
    kept(&file(&dir), &secrets, ada(&server), &b);
    let api = HttpTeamClient::new(ServerUrl::parse(&server).unwrap()).unwrap();
    let r = team::resume(&api, &secrets, &file(&dir), &b).await.unwrap();
    assert_eq!(r.standing, Standing::Offline);
    assert_eq!(r.identity, noreply());
    assert_eq!(r.events_url, None);
}

#[test]
fn a_team_file_from_before_the_cache_still_loads() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        file(&dir),
        r#"{"accounts":[{"server":"http://127.0.0.1:1","username":"ada","display_name":"Ada",
            "token_id":"t","server_admin":false}],
           "bindings":[{"root":"/w","server":"http://127.0.0.1:1","team":"core",
            "workspace":"api","username":"ada","role":"write"}]}"#,
    )
    .unwrap();
    let store = TeamStore::load(&file(&dir)).unwrap();
    assert_eq!(store.accounts()[0].identity, None);
    assert_eq!(store.bindings()[0].branch, "");
}
