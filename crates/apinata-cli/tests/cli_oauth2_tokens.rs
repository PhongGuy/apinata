//! `apinata oauth2 status` and `clear` over a token cache that `apinata run` filled from a local
//! mock provider. The cache is a temporary file under a fixed test key, never the keychain.

use crate::common;

use common::{ROOT, apinata, write_tree};
use std::path::Path;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "tok-91be-SECRET";
const REFRESH: &str = "rt-5d0c-SECRET";
/// 32 zero bytes as base64: the key debug builds read instead of the keychain.
const KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

fn auth(provider: &str, id: &str) -> String {
    format!(
        "    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{provider}/token\"\n    credentials:\n      clientId: app\n      clientSecret: cs\n    scope: read\n    tokenConfig:\n      id: {id}\n      placement:\n        header: Bearer\n"
    )
}

fn request(name: &str, url: &str, auth: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n  auth:\n{auth}"
    )
}

/// A provider whose token lives `expires_in` seconds, and a resource that accepts anything.
async fn provider(expires_in: u64) -> MockServer {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "access_token": TOKEN, "token_type": "Bearer", "expires_in": expires_in,
        "refresh_token": REFRESH, "scope": "read",
    });
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/r"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    server
}

/// A collection with one request `a.yml` that carries its own OAuth 2.0 auth.
fn own_auth(server: &MockServer) -> TempDir {
    let uri = server.uri();
    let text = request("a", &format!("{uri}/r"), &auth(&uri, "main"));
    write_tree(&[("opencollection.yml", ROOT), ("a.yml", &text)])
}

/// `apinata ARGS` with the token cache in `cache` under the test key.
fn cli(cache: &Path, args: &[&str]) -> std::process::Output {
    apinata()
        .args(args)
        .env("APINATA_TEST_TOKEN_CACHE", cache)
        .env("APINATA_TEST_TOKEN_KEY", KEY)
        .output()
        .expect("runs")
}

/// Fill the cache by running the collection once.
fn seed(dir: &Path, cache: &Path) {
    let out = cli(cache, &["run", dir.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{}", shown(&out));
}

fn shown(out: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn assert_no_token(out: &std::process::Output) {
    let all = shown(out);
    assert!(!all.contains(TOKEN), "{all}");
    assert!(!all.contains(REFRESH), "{all}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_shows_a_cached_token_without_its_value() {
    let server = provider(3600).await;
    let dir = own_auth(&server);
    let cache = TempDir::new().unwrap();
    seed(dir.path(), cache.path());
    let root = dir.path().to_str().unwrap();

    let out = cli(cache.path(), &["oauth2", "status", "--collection", root]);
    assert_eq!(out.status.code(), Some(0), "{}", shown(&out));
    assert_no_token(&out);
    let text = stdout(&out);
    assert_eq!(text.lines().count(), 1, "{text}");
    for part in [
        "a.yml",
        "client_credentials",
        "expires in",
        "read",
        "refresh token",
    ] {
        assert!(text.contains(part), "{part} missing from {text}");
    }
    assert!(!text.contains("expired"), "{text}");

    let out = cli(
        cache.path(),
        &[
            "oauth2",
            "status",
            "a.yml",
            "--collection",
            root,
            "--format",
            "json",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", shown(&out));
    assert_no_token(&out);
    let doc: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("json");
    let entry = &doc["tokens"][0];
    assert_eq!(entry["path"], "a.yml");
    assert_eq!(entry["level"], "request");
    assert_eq!(entry["grant_type"], "client_credentials");
    assert_eq!(entry["expired"], false);
    assert_eq!(entry["has_refresh_token"], true);
    assert_eq!(entry["scope"], "read");
    assert!(entry["expires_at"].as_u64().is_some(), "{doc}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_token_past_its_expiry_reads_as_expired() {
    // Ten seconds is inside the thirty second margin, so the token is stale at once.
    let server = provider(10).await;
    let dir = own_auth(&server);
    let cache = TempDir::new().unwrap();
    seed(dir.path(), cache.path());
    let root = dir.path().to_str().unwrap();

    let out = cli(cache.path(), &["oauth2", "status", "--collection", root]);
    assert_eq!(out.status.code(), Some(0), "{}", shown(&out));
    assert!(stdout(&out).contains("expired"), "{}", shown(&out));
    let out = cli(
        cache.path(),
        &["oauth2", "status", "--collection", root, "--format", "json"],
    );
    let doc: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("json");
    assert_eq!(doc["tokens"][0]["expired"], true, "{doc}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clear_removes_the_token_and_status_then_shows_nothing() {
    let server = provider(3600).await;
    let dir = own_auth(&server);
    let cache = TempDir::new().unwrap();
    seed(dir.path(), cache.path());
    let root = dir.path().to_str().unwrap();

    let out = cli(
        cache.path(),
        &["oauth2", "clear", "a.yml", "--collection", root],
    );
    assert_eq!(out.status.code(), Some(0), "{}", shown(&out));
    assert_no_token(&out);
    assert!(stdout(&out).contains("cleared"), "{}", shown(&out));
    assert!(stdout(&out).contains("a.yml"), "{}", shown(&out));

    let out = cli(cache.path(), &["oauth2", "status", "--collection", root]);
    assert_eq!(out.status.code(), Some(0), "{}", shown(&out));
    assert_eq!(stdout(&out), "", "{}", shown(&out));
    let out = cli(
        cache.path(),
        &["oauth2", "status", "--collection", root, "--format", "json"],
    );
    let doc: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("json");
    assert_eq!(doc["tokens"], serde_json::json!([]), "{doc}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clearing_nothing_is_not_an_error() {
    let server = provider(3600).await;
    let dir = own_auth(&server);
    let cache = TempDir::new().unwrap();
    let root = dir.path().to_str().unwrap();
    for args in [
        &["oauth2", "clear", "--collection", root][..],
        &["oauth2", "clear", "a.yml", "--collection", root][..],
    ] {
        let out = cli(cache.path(), args);
        assert_eq!(out.status.code(), Some(0), "{args:?}: {}", shown(&out));
    }
    let out = cli(
        cache.path(),
        &["oauth2", "clear", "--collection", root, "--format", "json"],
    );
    let doc: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("json");
    assert_eq!(doc["cleared"], serde_json::json!([]), "{doc}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_inherited_token_is_listed_under_the_level_that_defines_it() {
    let server = provider(3600).await;
    let uri = server.uri();
    let root_file = format!("{ROOT}request:\n  auth:\n{}", auth(&uri, "top"));
    let inherit = format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{uri}/r\"\n  auth: inherit\n"
    );
    let dir = write_tree(&[("opencollection.yml", &root_file), ("f/r.yml", &inherit)]);
    let cache = TempDir::new().unwrap();
    seed(dir.path(), cache.path());
    let root = dir.path().to_str().unwrap();

    for args in [
        &["oauth2", "status", "--collection", root, "--format", "json"][..],
        &[
            "oauth2",
            "status",
            "f/r.yml",
            "--collection",
            root,
            "--format",
            "json",
        ][..],
    ] {
        let out = cli(cache.path(), args);
        assert_eq!(out.status.code(), Some(0), "{args:?}: {}", shown(&out));
        let doc: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("json");
        let tokens = doc["tokens"].as_array().expect("tokens");
        assert_eq!(tokens.len(), 1, "{doc}");
        assert_eq!(tokens[0]["level"], "collection", "{doc}");
        assert_eq!(tokens[0]["path"], "", "{doc}");
    }

    let out = cli(cache.path(), &["oauth2", "clear", "--collection", root]);
    assert_eq!(out.status.code(), Some(0), "{}", shown(&out));
    let out = cli(cache.path(), &["oauth2", "status", "--collection", root]);
    assert_eq!(stdout(&out), "", "{}", shown(&out));
}

#[test]
fn a_request_without_oauth2_is_a_usage_error() {
    let text =
        "info:\n  name: p\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: https://x.test\n";
    let dir = write_tree(&[("opencollection.yml", ROOT), ("p.yml", text)]);
    let cache = TempDir::new().unwrap();
    let root = dir.path().to_str().unwrap();
    for action in ["status", "clear"] {
        let out = cli(
            cache.path(),
            &["oauth2", action, "p.yml", "--collection", root],
        );
        assert_eq!(out.status.code(), Some(3), "{}", shown(&out));
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.contains("request p.yml: its auth is not OAuth 2.0"),
            "{action}: {err}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_token_fetched_with_global_variables_is_found_with_global_env() {
    let server = provider(3600).await;
    let uri = server.uri();
    // The token URL and client id live only in the global environment.
    let global = format!(
        "name: shared\nvariables:\n  - name: issuer\n    value: {uri}\n  - name: client\n    value: app\n"
    );
    let profile = write_tree(&[("global-environments/shared.yml", &global)]);
    let settings = auth("{{issuer}}", "main").replace("clientId: app", "clientId: \"{{client}}\"");
    let text = request("a", &format!("{uri}/r"), &settings);
    let dir = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &text)]);
    let cache = TempDir::new().unwrap();
    let root = dir.path().to_str().unwrap();
    let with_profile = |args: &[&str]| {
        apinata()
            .args(args)
            .env("APINATA_PROFILE_DIR", profile.path())
            .env("APINATA_TEST_TOKEN_CACHE", cache.path())
            .env("APINATA_TEST_TOKEN_KEY", KEY)
            .output()
            .expect("runs")
    };
    let out = with_profile(&["run", root, "--global-env", "shared"]);
    assert_eq!(out.status.code(), Some(0), "{}", shown(&out));

    let out = with_profile(&[
        "oauth2",
        "status",
        "--collection",
        root,
        "--global-env",
        "shared",
        "--format",
        "json",
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", shown(&out));
    assert_no_token(&out);
    let doc: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("json");
    assert_eq!(doc["tokens"][0]["path"], "a.yml", "{doc}");

    let out = with_profile(&[
        "oauth2",
        "clear",
        "a.yml",
        "--collection",
        root,
        "--global-env",
        "shared",
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", shown(&out));
    assert!(stdout(&out).contains("cleared"), "{}", shown(&out));
}
