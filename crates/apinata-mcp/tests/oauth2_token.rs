//! `oauth2_token`: status, fetch, refresh and clear for a request's, folder's or the
//! collection's OAuth 2.0 auth, without ever showing a token or secret value.

use crate::common;

use common::{ROOT, TestServer};
use rmcp::ServiceError;
use rmcp::model::{CancelledNotification, CancelledNotificationParam};
use serde_json::{Value, json};
use std::time::Duration;
use wiremock::matchers::{any, body_string_contains, method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const TOKEN: &str = "tok-5d2a-SECRET";
const REFRESH: &str = "rt-83c0-SECRET";
const CLIENT_SECRET: &str = "cs-19fe-SECRET";

fn auth(token_url: &str) -> String {
    format!(
        "    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{token_url}\"\n    credentials:\n      clientId: app\n      clientSecret: {CLIENT_SECRET}\n    scope: read\n    tokenConfig:\n      id: main\n      placement:\n        header: Bearer\n"
    )
}

fn get(url: &str, auth: &str) -> String {
    format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n  auth:\n{auth}"
    )
}

/// A GET of `url` that inherits its auth.
fn inherit(url: &str) -> String {
    format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n  auth: inherit\n"
    )
}

/// Echoes the request's Authorization header, as a careless API might.
struct Echo;
impl Respond for Echo {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let auth = request
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        ResponseTemplate::new(200).set_body_string(auth.to_owned())
    }
}

/// A provider whose client-credentials grant issues TOKEN and REFRESH `expect` times, and
/// whose `/r` echoes the Authorization header.
async fn provider(expect: u64) -> MockServer {
    let server = MockServer::start().await;
    let body = json!({
        "access_token": TOKEN,
        "token_type": "Bearer",
        "expires_in": 3600,
        "refresh_token": REFRESH,
        "scope": "read write",
    });
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=client_credentials"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .expect(expect)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/r"))
        .respond_with(Echo)
        .mount(&server)
        .await;
    server
}

async fn one_request(p: &MockServer) -> TestServer {
    let uri = p.uri();
    let r = get(&format!("{uri}/r"), &auth(&format!("{uri}/token")));
    TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", &r)]).await
}

fn assert_no_values(shown: &str) {
    for value in [TOKEN, REFRESH, CLIENT_SECRET] {
        assert!(!shown.contains(value), "{value} shown: {shown}");
    }
}

async fn token(server: &TestServer, path: &str, action: &str) -> Value {
    server
        .ok("oauth2_token", json!({"path": path, "action": action}))
        .await
}

#[tokio::test]
async fn status_before_any_fetch_has_no_token() {
    let p = provider(0).await;
    let server = one_request(&p).await;
    let status = token(&server, "r.yml", "status").await;
    assert_eq!(status["action"], "status");
    assert_eq!(status["token"], Value::Null, "{status}");
}

#[tokio::test]
async fn fetch_reports_the_status_and_no_value() {
    let p = provider(1).await;
    let server = one_request(&p).await;
    let result = server
        .call("oauth2_token", json!({"path": "r.yml", "action": "fetch"}))
        .await;
    let shown = serde_json::to_string(&result).unwrap();
    assert_no_values(&shown);
    let fetched = result.structured_content.expect("structured");
    let t = &fetched["token"];
    assert!(t["expires_at"].as_u64().unwrap() > t["obtained_at"].as_u64().unwrap());
    assert_eq!(t["expired"], false);
    assert_eq!(t["has_refresh_token"], true);
    assert_eq!(t["scope"], "read write");
    assert!(fetched["memory_only"].is_string(), "{fetched}");
    let status = token(&server, "r.yml", "status").await;
    assert_eq!(status["token"], fetched["token"]);
    assert_no_values(&status.to_string());
}

#[tokio::test]
async fn a_fetched_token_is_used_by_the_next_send() {
    let p = provider(1).await;
    let server = one_request(&p).await;
    token(&server, "r.yml", "fetch").await;
    let sent = server
        .ok("send_request", json!({"path": "r.yml"}))
        .await
        .to_string();
    assert!(sent.contains("Bearer"), "the echo came back: {sent}");
    assert_no_values(&sent);
}

#[tokio::test]
async fn refresh_posts_the_cached_refresh_token() {
    let p = provider(1).await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"access_token": "tok-2", "expires_in": 60})),
        )
        .expect(1)
        .mount(&p)
        .await;
    let server = one_request(&p).await;
    let none = server
        .err(
            "oauth2_token",
            json!({"path": "r.yml", "action": "refresh"}),
        )
        .await;
    assert!(none.contains("no token yet"), "{none}");
    token(&server, "r.yml", "fetch").await;
    let result = server
        .call(
            "oauth2_token",
            json!({"path": "r.yml", "action": "refresh"}),
        )
        .await;
    let shown = serde_json::to_string(&result).unwrap();
    assert_no_values(&shown);
    let t = &result.structured_content.expect("structured")["token"];
    let lifetime = t["expires_at"].as_u64().unwrap() - t["obtained_at"].as_u64().unwrap();
    assert!(lifetime <= 60, "the refreshed token's expiry: {t}");
    let received = p.received_requests().await.unwrap();
    let refresh = received
        .iter()
        .find(|r| String::from_utf8_lossy(&r.body).contains("grant_type=refresh_token"))
        .expect("a refresh was posted");
    assert!(String::from_utf8_lossy(&refresh.body).contains(REFRESH));
}

#[tokio::test]
async fn clear_forgets_the_token() {
    let p = provider(1).await;
    let server = one_request(&p).await;
    token(&server, "r.yml", "fetch").await;
    let cleared = token(&server, "r.yml", "clear").await;
    assert!(cleared["removed"].as_u64().unwrap() >= 1, "{cleared}");
    assert_eq!(cleared["token"], Value::Null);
    assert_eq!(
        token(&server, "r.yml", "status").await["token"],
        Value::Null
    );
}

#[tokio::test]
async fn a_browser_grant_points_at_the_app_and_sends_nothing() {
    let p = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&p)
        .await;
    let uri = p.uri();
    let code = format!(
        "    type: oauth2\n    flow: authorization_code\n    authorizationUrl: \"{uri}/authorize\"\n    accessTokenUrl: \"{uri}/token\"\n    credentials:\n      clientId: app\n      clientSecret: {CLIENT_SECRET}\n    tokenConfig:\n      id: main\n"
    );
    let r = get(&format!("{uri}/r"), &code);
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", &r)]).await;
    let message = server
        .err("oauth2_token", json!({"path": "r.yml", "action": "fetch"}))
        .await;
    assert!(message.contains("Apinata app"), "{message}");
    assert_no_values(&message);
}

#[tokio::test]
async fn an_untrusted_token_host_files_an_approval_and_sends_nothing() {
    let api = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&api)
        .await;
    let r = get(
        &format!("{}/r", api.uri()),
        &auth("https://auth.example.test/token"),
    );
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", &r)]).await;
    let message = server
        .err("oauth2_token", json!({"path": "r.yml", "action": "fetch"}))
        .await;
    assert!(
        message.contains("retry after the user approves"),
        "{message}"
    );
    assert_no_values(&message);
    let pending = server.inbox().pending().expect("inbox");
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(
        (pending[0].host.as_str(), pending[0].request.as_str()),
        ("auth.example.test", "r.yml")
    );
}

#[tokio::test]
async fn a_folder_token_is_used_by_a_request_inheriting_it() {
    let p = provider(1).await;
    let uri = p.uri();
    let folder = format!(
        "info:\n  name: f\n  type: folder\nrequest:\n  auth:\n{}",
        auth(&format!("{uri}/token"))
    );
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder),
        ("f/r.yml", &inherit(&format!("{uri}/r"))),
    ])
    .await;
    assert!(token(&server, "f", "fetch").await["token"].is_object());
    assert!(token(&server, "f/r.yml", "status").await["token"].is_object());
    let sent = server
        .ok("send_request", json!({"path": "f/r.yml"}))
        .await
        .to_string();
    assert!(sent.contains("Bearer"), "{sent}");
    assert_no_values(&sent);
}

#[tokio::test]
async fn a_refresh_to_an_untrusted_host_files_an_approval_and_keeps_the_token() {
    let p = provider(1).await;
    let uri = p.uri();
    let folder = format!(
        "info:\n  name: f\n  type: folder\nrequest:\n  auth:\n{}    refreshTokenUrl: \"https://auth.example.test/refresh\"\n",
        auth(&format!("{uri}/token"))
    );
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder),
        ("f/r.yml", &inherit(&format!("{uri}/r"))),
    ])
    .await;
    let fetched = token(&server, "f", "fetch").await;
    assert!(fetched["token"].is_object(), "{fetched}");
    let message = server
        .err("oauth2_token", json!({"path": "f", "action": "refresh"}))
        .await;
    assert!(
        message.contains("retry after the user approves"),
        "{message}"
    );
    assert_no_values(&message);
    let pending = server.inbox().pending().expect("inbox");
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(
        (pending[0].host.as_str(), pending[0].request.as_str()),
        ("auth.example.test", "f")
    );
    assert_eq!(p.received_requests().await.unwrap().len(), 1);
    let status = token(&server, "f", "status").await;
    assert_eq!(status["token"], fetched["token"]);
}

#[tokio::test]
async fn the_empty_path_is_the_collection_auth() {
    let p = provider(1).await;
    let uri = p.uri();
    let root = format!("{ROOT}request:\n  auth:\n{}", auth(&format!("{uri}/token")));
    let server = TestServer::start(&[
        ("opencollection.yml", &root),
        ("r.yml", &inherit(&format!("{uri}/r"))),
    ])
    .await;
    assert_eq!(token(&server, "", "status").await["token"], Value::Null);
    assert!(token(&server, "", "fetch").await["token"].is_object());
    assert!(token(&server, "r.yml", "status").await["token"].is_object());
}

#[tokio::test]
async fn another_auth_is_not_oauth2() {
    let r = get(
        "http://127.0.0.1:9/r",
        "    type: bearer\n    token: plain\n",
    );
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", &r)]).await;
    for action in ["status", "fetch", "refresh", "clear"] {
        let message = server
            .err("oauth2_token", json!({"path": "r.yml", "action": action}))
            .await;
        assert!(message.contains("not OAuth 2.0"), "{action}: {message}");
    }
}

#[tokio::test]
async fn a_path_that_is_no_item_is_an_error() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let message = server
        .err(
            "oauth2_token",
            json!({"path": "nope.yml", "action": "status"}),
        )
        .await;
    assert!(message.contains("nope.yml"), "{message}");
}

#[tokio::test]
async fn the_tool_is_listed_as_open_world_and_not_read_only() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let tools = server.tools().await;
    let tool = tools
        .iter()
        .find(|t| t.name == "oauth2_token")
        .expect("listed");
    let a = tool.annotations.as_ref().expect("annotations");
    assert_eq!(
        (a.read_only_hint, a.destructive_hint, a.open_world_hint),
        (Some(false), Some(false), Some(true))
    );
}

#[tokio::test]
async fn a_cancel_during_the_token_fetch_stores_nothing() {
    let p = MockServer::start().await;
    let body = json!({"access_token": TOKEN, "token_type": "Bearer", "expires_in": 3600});
    let slow = ResponseTemplate::new(200)
        .set_body_json(body)
        .set_delay(Duration::from_secs(1));
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(slow)
        .mount(&p)
        .await;
    let server = one_request(&p).await;
    let handle = server
        .start_call("oauth2_token", json!({"path": "r.yml", "action": "fetch"}))
        .await;
    for _ in 0..100 {
        if !p.received_requests().await.unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        p.received_requests().await.unwrap().len(),
        1,
        "fetch in flight"
    );
    // Cancel by hand rather than through `handle.cancel`, which drops the reply, so the test
    // sees what a client that keeps listening gets back; then wait past the provider's delay.
    let cancel = CancelledNotification::new(CancelledNotificationParam::new(
        Some(handle.id.clone()),
        None,
    ));
    handle
        .peer
        .send_notification(cancel.into())
        .await
        .expect("cancel sent");
    let reply = tokio::time::timeout(Duration::from_secs(5), handle.await_response())
        .await
        .expect("a reply");
    assert!(
        matches!(reply, Err(ServiceError::Cancelled { reason: None })),
        "{reply:?}"
    );
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(
        token(&server, "r.yml", "status").await["token"],
        Value::Null
    );
}

/// A token endpoint that must never be reached.
async fn unreachable_provider() -> MockServer {
    let p = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&p)
        .await;
    p
}

/// An oauth2 auth whose client secret is the secret `secretVar`.
fn secret_auth(token_url: &str) -> String {
    format!(
        "    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{token_url}\"\n    credentials:\n      clientId: app\n      clientSecret: \"{{{{secretVar}}}}\"\n    tokenConfig:\n      id: main\n"
    )
}

const SECRET_DEV: &str = "name: dev\nvariables:\n  - name: secretVar\n    secret: true\n";
const SECRET_VALUE: &str = "pre-{{hiddenName}}";
const UNDEFINED: &str = "a variable written inside a secret value is not defined; nothing was sent";

/// A secret that names `loopy`, which the environment defines as itself.
const LOOPY_DEV: &str = "name: dev\nvariables:\n  - name: secretVar\n    secret: true\n  - name: loopy\n    value: \"{{loopy}}\"\n";
const LOOPY_VALUE: &str = "pre-{{loopy}}";
const CYCLIC: &str = "a variable written inside a secret value refers to itself; nothing was sent";

/// A fetch with `secretVar` set to `value` fails with exactly `expected`, which names neither the
/// variable written inside the secret (`hidden`) nor the secret value, and sends nothing.
async fn assert_withheld(
    server: &TestServer,
    target: &str,
    p: &MockServer,
    (value, hidden, expected): (&str, &str, &str),
) {
    server.set_secret("dev", "secretVar", value);
    let message = server
        .err(
            "oauth2_token",
            json!({"path": target, "action": "fetch", "environment": "dev"}),
        )
        .await;
    assert_eq!(message, expected);
    assert!(!message.contains(hidden) && !message.contains(value));
    assert!(p.received_requests().await.unwrap().is_empty());
}

async fn assert_hidden_name_withheld(server: &TestServer, target: &str, p: &MockServer) {
    assert_withheld(server, target, p, (SECRET_VALUE, "hiddenName", UNDEFINED)).await;
}

async fn folder_server(p: &MockServer, dev: &str) -> TestServer {
    let folder = format!(
        "info:\n  name: f\n  type: folder\nrequest:\n  auth:\n{}",
        secret_auth(&format!("{}/token", p.uri()))
    );
    TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", dev),
        ("f/folder.yml", &folder),
    ])
    .await
}

async fn collection_server(p: &MockServer, dev: &str) -> TestServer {
    let root = format!(
        "{ROOT}request:\n  auth:\n{}",
        secret_auth(&format!("{}/token", p.uri()))
    );
    TestServer::start(&[("opencollection.yml", &root), ("environments/dev.yml", dev)]).await
}

#[tokio::test]
async fn a_folder_fetch_withholds_a_name_written_inside_a_secret() {
    let p = unreachable_provider().await;
    let server = folder_server(&p, SECRET_DEV).await;
    assert_hidden_name_withheld(&server, "f", &p).await;
}

#[tokio::test]
async fn a_collection_fetch_withholds_a_name_written_inside_a_secret() {
    let p = unreachable_provider().await;
    let server = collection_server(&p, SECRET_DEV).await;
    assert_hidden_name_withheld(&server, "", &p).await;
}

#[tokio::test]
async fn a_folder_fetch_withholds_a_cyclic_name_written_inside_a_secret() {
    let p = unreachable_provider().await;
    let server = folder_server(&p, LOOPY_DEV).await;
    assert_withheld(&server, "f", &p, (LOOPY_VALUE, "loopy", CYCLIC)).await;
}

#[tokio::test]
async fn a_collection_fetch_withholds_a_cyclic_name_written_inside_a_secret() {
    let p = unreachable_provider().await;
    let server = collection_server(&p, LOOPY_DEV).await;
    assert_withheld(&server, "", &p, (LOOPY_VALUE, "loopy", CYCLIC)).await;
}
