//! Single sign-on to a team server: the start URL, the loopback callback, the redeem and the
//! session it keeps, and resuming once the token has expired.

use crate::common::potluck::*;
use apinata_core::team::{
    self, Account, Binding, Role, ServerUrl, Standing, TeamApi, TeamError, TeamStore, token_key,
};
use apinata_core::{CancelHandle, MemoryStore, SecretStore};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::TempDir;
use url::Url;
use wiremock::{MockServer, ResponseTemplate};

const LONG: Duration = Duration::from_secs(10);
const REDEEM: &str = "/api/v1/oidc/redeem";

fn file(dir: &TempDir) -> PathBuf {
    dir.path().join("team.json")
}

fn url(s: &MockServer) -> ServerUrl {
    ServerUrl::parse(&s.uri()).unwrap()
}

async fn fresh() -> (MockServer, TempDir, MemoryStore) {
    (
        MockServer::start().await,
        TempDir::new().unwrap(),
        MemoryStore::new(),
    )
}

/// The redeem answer: the `POST /session` shape with an expiring token.
fn redeemed(id: i64, expires_at: &str) -> ResponseTemplate {
    let mut meta = token_meta(id);
    meta["expires_at"] = json!(expires_at);
    ResponseTemplate::new(201)
        .set_body_json(json!({"token": tok(), "token_meta": meta, "user": user("ada", false)}))
}

/// The start URL's query as pairs.
fn query(start: &str) -> Vec<(String, String)> {
    Url::parse(start)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect()
}

fn param(start: &str, key: &str) -> String {
    query(start)
        .into_iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
        .unwrap_or_else(|| panic!("{key} missing from {start}"))
}

fn callback_addr(start: &str) -> SocketAddr {
    let port: u16 = param(start, "port").parse().unwrap();
    assert_eq!(param(start, "loopback"), "v4");
    SocketAddr::from(([127, 0, 0, 1], port))
}

/// Plays the browser landing on the app's loopback with `query`; returns the page.
async fn land(addr: SocketAddr, query: String) -> String {
    tokio::task::spawn_blocking(move || {
        let mut s = TcpStream::connect(addr).expect("connect");
        write!(
            s,
            "GET /callback?{query} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).expect("response");
        out
    })
    .await
    .unwrap()
}

/// Whether a connect to `addr` is refused within 2 s, meaning the listener closed.
fn port_closes(addr: SocketAddr) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        match TcpStream::connect_timeout(&addr, Duration::from_millis(200)) {
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => return true,
            _ => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    false
}

/// Runs a sign-in whose browser comes back with `back` (given the app's state), returning the
/// outcome and the loopback address.
async fn sign_in_with(
    s: &MockServer,
    secrets: &MemoryStore,
    team_file: &Path,
    back: impl FnOnce(&str) -> String,
) -> (Result<Account, TeamError>, SocketAddr) {
    let api = client(s);
    let pending = team::begin_sso(&api).await.unwrap();
    let start = pending.url().to_owned();
    let addr = callback_addr(&start);
    let q = back(&param(&start, "state"));
    let cancel = CancelHandle::new();
    let (got, _) = tokio::join!(
        pending.finish(&api, secrets, team_file, LONG, &cancel),
        land(addr, q)
    );
    (got, addr)
}

async fn redeem_bodies(s: &MockServer) -> Vec<Value> {
    s.received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == REDEEM)
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect()
}

fn nothing_kept(s: &MockServer, secrets: &MemoryStore, team_file: &Path) {
    assert_eq!(secrets.get(&token_key(&url(s), "ada")).unwrap(), None);
    assert!(TeamStore::load(team_file).unwrap().accounts().is_empty());
}

#[tokio::test]
async fn the_start_url_is_built_on_the_entered_address_with_pkce_and_state() {
    let s = MockServer::start().await;
    let api = client(&s);
    let pending = team::begin_sso(&api).await.unwrap();
    let start = pending.url().to_owned();
    assert!(
        start.starts_with(&format!("{}/api/v1/oidc/start?", s.uri())),
        "{start}"
    );
    let keys: Vec<String> = query(&start).into_iter().map(|(k, _)| k).collect();
    assert_eq!(
        keys,
        ["port", "state", "challenge", "challenge_method", "loopback"]
    );
    let b64url = |v: &str| {
        v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    };
    let (state, challenge) = (param(&start, "state"), param(&start, "challenge"));
    assert!(
        (16..=64).contains(&state.len()) && b64url(&state),
        "{state}"
    );
    assert!(challenge.len() == 43 && b64url(&challenge), "{challenge}");
    assert_eq!(param(&start, "challenge_method"), "S256");
    // The port is held before the URL exists.
    let addr = callback_addr(&start);
    assert!(TcpStream::connect(addr).is_ok());
    let again = team::begin_sso(&api).await.unwrap();
    assert_ne!(param(again.url(), "state"), state);
    assert_ne!(param(again.url(), "challenge"), challenge);
}

#[tokio::test]
async fn a_returned_code_is_redeemed_and_the_session_kept_with_its_expiry() {
    let (s, dir, secrets) = fresh().await;
    let expires = "2099-01-01T00:00:00Z";
    answer(&s, "POST", REDEEM, redeemed(9, expires)).await;
    let api = client(&s);
    let pending = team::begin_sso(&api).await.unwrap();
    let start = pending.url().to_owned();
    let (state, challenge) = (param(&start, "state"), param(&start, "challenge"));
    let addr = callback_addr(&start);
    let (cancel, team_file) = (CancelHandle::new(), file(&dir));
    let (got, page) = tokio::join!(
        pending.finish(&api, &secrets, &team_file, LONG, &cancel),
        land(addr, format!("code=handoff-123&state={state}"))
    );
    let account = got.unwrap();
    assert!(page.starts_with("HTTP/1.1 200"), "{page}");
    assert!(!page.contains("handoff-123"), "{page}");
    assert_eq!(
        (account.username.as_str(), account.token_id.as_str()),
        ("ada", "9")
    );
    assert_eq!(account.expires_at.as_deref(), Some(expires));
    assert_eq!(
        secrets.get(&token_key(&url(&s), "ada")).unwrap(),
        Some(tok())
    );
    assert_eq!(TeamStore::load(&file(&dir)).unwrap().accounts(), [account]);

    let bodies = redeem_bodies(&s).await;
    assert_eq!(bodies.len(), 1);
    let body = &bodies[0];
    assert_eq!(body["code"], "handoff-123");
    assert_eq!(body["token_name"], "Apinata");
    let verifier = body["verifier"].as_str().unwrap();
    assert_eq!(
        URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
        challenge
    );
    assert!(port_closes(addr), "the listener is still open");
}

#[tokio::test]
async fn sso_replaces_a_password_session_and_revokes_the_old_token() {
    let (s, dir, secrets) = fresh().await;
    answer(&s, "GET", "/api/v1/server", info(true)).await;
    answer(&s, "POST", "/api/v1/session", session(7, false)).await;
    answer(&s, "DELETE", "/api/v1/tokens/7", ResponseTemplate::new(204)).await;
    answer(&s, "POST", REDEEM, redeemed(9, "2099-01-01T00:00:00Z")).await;
    team::sign_in(
        &client(&s),
        &secrets,
        &file(&dir),
        "ada",
        "hunter2",
        &apinata_core::CancelFlag::new(),
    )
    .await
    .unwrap();
    let (got, _) = sign_in_with(&s, &secrets, &file(&dir), |state| {
        format!("code=c&state={state}")
    })
    .await;
    assert_eq!(got.unwrap().token_id, "9");
    assert_eq!(hits(&s, "DELETE", "/api/v1/tokens/7").await, 1);
    let store = TeamStore::load(&file(&dir)).unwrap();
    assert_eq!(store.accounts().len(), 1);
    assert_eq!(store.accounts()[0].token_id, "9");
}

#[tokio::test]
async fn a_wrong_state_never_redeems_and_keeps_nothing() {
    let (s, dir, secrets) = fresh().await;
    answer(&s, "POST", REDEEM, redeemed(9, "2099-01-01T00:00:00Z")).await;
    for back in ["code=c&state=someone-elses-state-value", "code=c"] {
        let (got, addr) = sign_in_with(&s, &secrets, &file(&dir), |_| back.to_owned()).await;
        assert_eq!(got.unwrap_err(), TeamError::SsoStateMismatch, "{back}");
        assert!(port_closes(addr), "the listener is still open");
    }
    assert_eq!(hits(&s, "POST", REDEEM).await, 0);
    nothing_kept(&s, &secrets, &file(&dir));
}

#[tokio::test]
async fn each_error_from_the_browser_ends_in_its_own_error_and_keeps_nothing() {
    let (s, dir, secrets) = fresh().await;
    answer(&s, "POST", REDEEM, redeemed(9, "2099-01-01T00:00:00Z")).await;
    for (error, want) in [
        ("oidc_refused", TeamError::SsoRefused),
        ("account_not_linked", TeamError::AccountNotLinked),
        ("oidc_expired", TeamError::SsoExpired),
        ("something_new", TeamError::SsoRefused),
    ] {
        let (got, addr) = sign_in_with(&s, &secrets, &file(&dir), |state| {
            format!("error={error}&state={state}")
        })
        .await;
        assert_eq!(got.unwrap_err(), want, "{error}");
        assert!(port_closes(addr), "the listener is still open");
    }
    assert_eq!(hits(&s, "POST", REDEEM).await, 0);
    nothing_kept(&s, &secrets, &file(&dir));
}

#[tokio::test]
async fn an_invalid_code_is_redeemed_once_and_keeps_nothing() {
    let (s, dir, secrets) = fresh().await;
    let invalid = ResponseTemplate::new(400).set_body_json(error(
        "oidc_code_invalid",
        &format!("bad code near {}", tok()),
        None,
    ));
    answer(&s, "POST", REDEEM, invalid).await;
    let (got, _) = sign_in_with(&s, &secrets, &file(&dir), |state| {
        format!("code=used&state={state}")
    })
    .await;
    let e = got.unwrap_err();
    assert_eq!(e, TeamError::SsoCodeInvalid);
    assert!(!format!("{e:?} {e}").contains(&tok()));
    assert_eq!(hits(&s, "POST", REDEEM).await, 1);
    nothing_kept(&s, &secrets, &file(&dir));
}

#[tokio::test]
async fn a_server_without_sso_answers_redeem_as_off() {
    let (s, dir, secrets) = fresh().await;
    let missing = ResponseTemplate::new(404).set_body_json(error("not_found", "No route.", None));
    answer(&s, "POST", REDEEM, missing).await;
    let (got, _) = sign_in_with(&s, &secrets, &file(&dir), |state| {
        format!("code=c&state={state}")
    })
    .await;
    assert_eq!(got.unwrap_err(), TeamError::SsoOff);
    nothing_kept(&s, &secrets, &file(&dir));
}

#[tokio::test]
async fn no_browser_return_in_time_is_a_timeout_that_closes_the_port() {
    let (s, dir, secrets) = fresh().await;
    let api = client(&s);
    let pending = team::begin_sso(&api).await.unwrap();
    let addr = callback_addr(pending.url());
    let e = pending
        .finish(
            &api,
            &secrets,
            &file(&dir),
            Duration::from_millis(200),
            &CancelHandle::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(e, TeamError::SsoTimeout);
    assert!(port_closes(addr), "the listener is still open");
    nothing_kept(&s, &secrets, &file(&dir));
}

#[tokio::test]
async fn a_cancel_ends_the_wait_as_a_timeout_and_closes_the_port() {
    let (s, dir, secrets) = fresh().await;
    let api = client(&s);
    let pending = team::begin_sso(&api).await.unwrap();
    let addr = callback_addr(pending.url());
    let cancel = CancelHandle::new();
    let stop = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        stop.cancel();
    });
    let e = pending
        .finish(&api, &secrets, &file(&dir), LONG, &cancel)
        .await
        .unwrap_err();
    assert_eq!(e, TeamError::SsoTimeout);
    assert!(port_closes(addr), "the listener is still open");
    nothing_kept(&s, &secrets, &file(&dir));
}

#[tokio::test]
async fn debug_output_never_shows_the_state_challenge_or_token() {
    let (s, dir, secrets) = fresh().await;
    answer(&s, "POST", REDEEM, redeemed(9, "2099-01-01T00:00:00Z")).await;
    let api = client(&s);
    let pending = team::begin_sso(&api).await.unwrap();
    let start = pending.url().to_owned();
    let shown = format!("{pending:?}");
    for secret in [param(&start, "state"), param(&start, "challenge")] {
        assert!(!shown.contains(&secret), "{shown}");
    }
    let (got, _) = sign_in_with(&s, &secrets, &file(&dir), |state| {
        format!("code=c&state={state}")
    })
    .await;
    let shown = format!("{:?}", got.unwrap());
    assert!(!shown.contains(&tok()), "{shown}");
}

#[tokio::test]
async fn the_server_info_names_the_sso_provider_only_when_it_is_set() {
    for (oidc, want) in [
        (json!(null), None),
        (json!(false), None),
        (json!({"name": "Acme ID"}), Some("Acme ID".to_owned())),
        (
            json!({"name": "Ac\u{202e}me\u{0007} ID"}),
            Some("Acme ID".to_owned()),
        ),
        (json!({"name": "x".repeat(100)}), Some("x".repeat(64))),
    ] {
        let s = MockServer::start().await;
        let body = json!({"version": "0.2.0", "api": 1,
                          "auth": {"password": true, "oidc": oidc.clone()}});
        answer(
            &s,
            "GET",
            "/api/v1/server",
            ResponseTemplate::new(200).set_body_json(body),
        )
        .await;
        let got = client(&s).server_info().await.unwrap();
        assert_eq!(got.sso.map(|p| p.name), want, "{oidc}");
    }
    let s = MockServer::start().await;
    let missing = json!({"version": "0.1.0", "api": 1, "auth": {"password": true}});
    answer(
        &s,
        "GET",
        "/api/v1/server",
        ResponseTemplate::new(200).set_body_json(missing),
    )
    .await;
    assert!(client(&s).server_info().await.unwrap().sso.is_none());
}

/// An account that signed in with single sign-on, kept with its token and a binding.
fn kept_sso(team_file: &Path, secrets: &MemoryStore, server: &str, expires_at: &str) -> Binding {
    let server = ServerUrl::parse(server).unwrap();
    let mut account = Account::new(&server, "ada", "Ada Lovelace", "9", false);
    account.expires_at = Some(expires_at.to_owned());
    let b = Binding::new(
        PathBuf::from("/work/api"),
        &account,
        "core",
        "api",
        Role::Write,
    );
    secrets.set(&token_key(&server, "ada"), &tok()).unwrap();
    let kept = b.clone();
    TeamStore::update(team_file, |t| {
        t.put_account(account);
        t.put_binding(kept);
    })
    .unwrap();
    b
}

#[tokio::test]
async fn an_expired_sso_account_resumes_signed_out_without_asking_the_server() {
    let (s, dir, secrets) = fresh().await;
    answer(&s, "GET", "/api/v1/me", me(false, vec![])).await;
    let b = kept_sso(&file(&dir), &secrets, &s.uri(), "2020-01-01T00:00:00Z");
    let account = TeamStore::load(&file(&dir)).unwrap().accounts()[0].clone();
    assert!(account.expired());
    let r = team::resume(&client(&s), &secrets, &file(&dir), &b)
        .await
        .unwrap();
    assert_eq!(r.standing, Standing::SignedOut);
    assert_eq!(r.auth.token(), "");
    assert_eq!(hits(&s, "GET", "/api/v1/me").await, 0);
}

#[tokio::test]
async fn an_unexpired_sso_account_resumes_through_the_server() {
    let (s, dir, secrets) = fresh().await;
    let listed = workspace(&s.uri(), "core", "api", "apinata", "write");
    answer(&s, "GET", "/api/v1/me", me(false, vec![listed])).await;
    let b = kept_sso(&file(&dir), &secrets, &s.uri(), "2099-01-01T00:00:00Z");
    assert!(!TeamStore::load(&file(&dir)).unwrap().accounts()[0].expired());
    let r = team::resume(&client(&s), &secrets, &file(&dir), &b)
        .await
        .unwrap();
    assert_eq!(r.standing, Standing::Ready);
    assert_eq!(hits(&s, "GET", "/api/v1/me").await, 1);
}

#[tokio::test]
async fn the_default_redeem_is_sso_off() {
    struct NoSso(ServerUrl);
    impl TeamApi for NoSso {
        fn server(&self) -> &ServerUrl {
            &self.0
        }
        fn server_info(&self) -> team::TeamFuture<'_, team::ServerInfo> {
            unimplemented!()
        }
        fn sign_in<'a>(&'a self, _: &'a str, _: &'a str) -> team::TeamFuture<'a, team::SignIn> {
            unimplemented!()
        }
        fn me<'a>(&'a self, _: &'a team::TeamToken) -> team::TeamFuture<'a, team::Me> {
            unimplemented!()
        }
        fn teams<'a>(&'a self, _: &'a team::TeamToken) -> team::TeamFuture<'a, Vec<team::Team>> {
            unimplemented!()
        }
        fn create_workspace<'a>(
            &'a self,
            _: &'a team::TeamToken,
            _: &'a team::NewWorkspace,
        ) -> team::TeamFuture<'a, team::Workspace> {
            unimplemented!()
        }
        fn revoke<'a>(&'a self, _: &'a team::TeamToken, _: &'a str) -> team::TeamFuture<'a, ()> {
            unimplemented!()
        }
        fn server_update<'a>(
            &'a self,
            _: &'a team::TeamToken,
        ) -> team::TeamFuture<'a, team::ServerUpdate> {
            unimplemented!()
        }
        fn check_server_update<'a>(
            &'a self,
            _: &'a team::TeamToken,
        ) -> team::TeamFuture<'a, team::ServerUpdate> {
            unimplemented!()
        }
    }
    let api = NoSso(ServerUrl::parse("http://127.0.0.1:9").unwrap());
    assert_eq!(
        api.redeem_sso("c", "v").await.unwrap_err(),
        TeamError::SsoOff
    );
}
