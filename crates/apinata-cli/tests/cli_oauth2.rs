//! `apinata run` with OAuth 2.0 against a local mock provider: headless grants, the token cache
//! shared by later runs, `--refetch-tokens`, and grants that need the app. The cache is a
//! temporary file under a fixed test key, never the machine's keychain.

use crate::common;

use common::{ROOT, apinata, write_tree};
use std::path::Path;
use tempfile::TempDir;
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "tok-4f1c-SECRET";
const CLIENT_SECRET: &str = "cs-77aa-SECRET";
/// 32 zero bytes as base64: the key debug builds read instead of the keychain.
const KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

fn auth(provider: &str, flow: &str, extra: &str) -> String {
    format!(
        "    type: oauth2\n    flow: {flow}\n    accessTokenUrl: \"{provider}/token\"\n    authorizationUrl: \"{provider}/authorize\"\n    credentials:\n      clientId: app\n      clientSecret: {CLIENT_SECRET}\n    scope: read\n    tokenConfig:\n      id: main\n      placement:\n        header: Bearer\n{extra}"
    )
}

fn get(name: &str, url: &str, auth: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n  auth:\n{auth}"
    )
}

/// `apinata run <dir> <extra>` with the token cache in `cache` under the test key.
fn run(dir: &Path, cache: &Path, extra: &[&str]) -> std::process::Output {
    apinata()
        .arg("run")
        .arg(dir)
        .args(extra)
        .env("APINATA_TEST_TOKEN_CACHE", cache)
        .env("APINATA_TEST_TOKEN_KEY", KEY)
        .output()
        .expect("runs")
}

/// A provider issuing TOKEN `tokens` times and a resource that wants it `sends` times.
async fn provider(tokens: u64, sends: u64) -> MockServer {
    let server = MockServer::start().await;
    let body =
        serde_json::json!({"access_token": TOKEN, "token_type": "Bearer", "expires_in": 3600});
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .expect(tokens)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/r"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(ResponseTemplate::new(200))
        .expect(sends)
        .mount(&server)
        .await;
    server
}

fn collection(server: &MockServer, flow: &str, extra: &str) -> TempDir {
    let text = get(
        "a",
        &format!("{}/r", server.uri()),
        &auth(&server.uri(), flow, extra),
    );
    write_tree(&[("opencollection.yml", ROOT), ("a.yml", &text)])
}

fn shown(out: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_run_reuses_the_token_the_first_one_cached() {
    let server = provider(1, 2).await;
    let dir = collection(&server, "client_credentials", "");
    let cache = TempDir::new().unwrap();
    for _ in 0..2 {
        let out = run(dir.path(), cache.path(), &[]);
        assert_eq!(out.status.code(), Some(0), "{}", shown(&out));
    }
    let file = std::fs::read(cache.path().join("oauth2-tokens.json")).expect("the cache file");
    assert!(!String::from_utf8_lossy(&file).contains(TOKEN));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refetch_tokens_ignores_the_cached_token() {
    let server = provider(2, 2).await;
    let dir = collection(&server, "client_credentials", "");
    let cache = TempDir::new().unwrap();
    assert_eq!(run(dir.path(), cache.path(), &[]).status.code(), Some(0));
    let out = run(dir.path(), cache.path(), &["--refetch-tokens"]);
    assert_eq!(out.status.code(), Some(0), "{}", shown(&out));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_password_grant_runs_headless() {
    let server = MockServer::start().await;
    let body = serde_json::json!({"access_token": TOKEN, "token_type": "Bearer"});
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=password"))
        .and(body_string_contains("username=ann"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let owner = "    resourceOwner:\n      username: ann\n      password: pw\n";
    let dir = collection(&server, "resource_owner_password_credentials", owner);
    let cache = TempDir::new().unwrap();
    let out = run(dir.path(), cache.path(), &[]);
    assert_eq!(out.status.code(), Some(0), "{}", shown(&out));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_browser_grant_without_a_token_points_at_the_app() {
    let server = provider(0, 0).await;
    let dir = collection(&server, "authorization_code", "");
    let cache = TempDir::new().unwrap();
    let out = run(dir.path(), cache.path(), &[]);
    assert_eq!(out.status.code(), Some(2), "{}", shown(&out));
    assert!(
        shown(&out).contains("OAuth 2.0 token \"main\": this grant needs a browser sign-in; use Get token in the Apinata app, then run again"),
        "{}",
        shown(&out)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_fetch_sends_nothing_and_shows_no_secret() {
    let server = MockServer::start().await;
    let refused = serde_json::json!({"error": "invalid_client", "error_description": format!("bad secret {CLIENT_SECRET}")});
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(401).set_body_json(refused))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let dir = collection(&server, "client_credentials", "");
    let cache = TempDir::new().unwrap();
    let out = run(dir.path(), cache.path(), &[]);
    assert_eq!(out.status.code(), Some(2), "{}", shown(&out));
    assert!(shown(&out).contains("invalid_client"), "{}", shown(&out));
    assert!(!shown(&out).contains(CLIENT_SECRET), "{}", shown(&out));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_json_report_of_a_failed_fetch_carries_no_client_secret() {
    let server = MockServer::start().await;
    let refused = serde_json::json!({"error": "invalid_client", "error_description": format!("bad secret {CLIENT_SECRET}")});
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(401).set_body_json(refused))
        .mount(&server)
        .await;
    let dir = collection(&server, "client_credentials", "");
    let cache = TempDir::new().unwrap();
    let out = run(dir.path(), cache.path(), &["--format", "json"]);
    assert_eq!(out.status.code(), Some(2), "{}", shown(&out));
    assert!(shown(&out).contains("invalid_client"), "{}", shown(&out));
    assert!(!shown(&out).contains(CLIENT_SECRET), "{}", shown(&out));
    // The CLI writes no report files; the cache directory must not hold the secret either.
    for entry in std::fs::read_dir(cache.path()).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains(CLIENT_SECRET));
    }
}
