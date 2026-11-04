//! OAuth 2.0 under MCP: a token host outside the trusted list files an approval and sends
//! nothing, and no token or client secret reaches a tool result.

use crate::common;

use common::{ROOT, TestServer};
use serde_json::json;
use std::time::Duration;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const TOKEN: &str = "tok-4f1c-SECRET";
const CLIENT_SECRET: &str = "cs-77aa-SECRET";

fn auth(token_url: &str) -> String {
    format!(
        "    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{token_url}\"\n    credentials:\n      clientId: app\n      clientSecret: {CLIENT_SECRET}\n    scope: read\n    tokenConfig:\n      id: main\n      placement:\n        header: Bearer\n"
    )
}

/// A GET of `url`; `headers` holds `  headers:` lines placed before the auth block.
fn get(url: &str, auth: &str, headers: &str) -> String {
    format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n{headers}  auth:\n{auth}"
    )
}

/// Echoes the request's Authorization and X-Copy headers, as a careless API might.
struct Echo;
impl Respond for Echo {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let value = |name: &str| {
            request
                .headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_owned()
        };
        ResponseTemplate::new(200).set_body_string(format!(
            "{} {}",
            value("authorization"),
            value("x-copy")
        ))
    }
}

async fn provider() -> MockServer {
    let server = MockServer::start().await;
    let body = json!({"access_token": TOKEN, "token_type": "Bearer", "expires_in": 3600});
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/r"))
        .respond_with(Echo)
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn an_untrusted_token_host_files_an_approval_and_sends_nothing() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&api)
        .await;
    let r = get(
        &format!("{}/r", api.uri()),
        &auth("https://auth.example.test/token"),
        "",
    );
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", &r)]).await;
    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(message.contains("`auth.example.test`"), "{message}");
    assert!(
        message.contains("retry after the user approves"),
        "{message}"
    );
    let pending = server.inbox().pending().expect("inbox");
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(
        (pending[0].host.as_str(), pending[0].request.as_str()),
        ("auth.example.test", "r.yml")
    );
    let refused = server.err("run_folder", json!({"path": ""})).await;
    assert!(refused.contains("auth.example.test"), "{refused}");
    assert_eq!(server.inbox().pending().expect("inbox").len(), 1);
}

#[tokio::test]
async fn tokens_and_client_secrets_never_reach_a_result() {
    let p = provider().await;
    let uri = p.uri();
    let copy = "  headers:\n    - name: X-Copy\n      value: \"{{$oauth2.main.access_token}}\"\n";
    let r = get(&format!("{uri}/r"), &auth(&format!("{uri}/token")), copy);
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", &r)]).await;
    let sent = server
        .ok("send_request", json!({"path": "r.yml"}))
        .await
        .to_string();
    let ran = server
        .ok("run_folder", json!({"path": ""}))
        .await
        .to_string();
    for shown in [&sent, &ran] {
        assert!(!shown.contains(TOKEN), "{shown}");
        assert!(!shown.contains(CLIENT_SECRET), "{shown}");
    }
    assert!(sent.contains("Bearer"), "the echo came back: {sent}");
}

#[tokio::test]
async fn a_provider_error_reaches_the_agent_without_the_client_secret() {
    let p = MockServer::start().await;
    let refused = json!({"error": "invalid_client", "error_description": format!("client {CLIENT_SECRET} unknown")});
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(401).set_body_json(refused))
        .mount(&p)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&p)
        .await;
    let r = get(
        &format!("{}/r", p.uri()),
        &auth(&format!("{}/token", p.uri())),
        "",
    );
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", &r)]).await;
    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(message.contains("invalid_client"), "{message}");
    assert!(!message.contains(CLIENT_SECRET), "{message}");
}

#[tokio::test]
async fn a_token_cache_file_keeps_tokens_encrypted() {
    let p = provider().await;
    let uri = p.uri();
    let r = get(&format!("{uri}/r"), &auth(&format!("{uri}/token")), "");
    let state = tempfile::TempDir::new().unwrap();
    let file = state.path().join("oauth2-tokens.json");
    let cache = file.clone();
    let server = TestServer::start_with(&[("opencollection.yml", ROOT), ("r.yml", &r)], move |o| {
        o.token_cache_file(cache)
    })
    .await;
    server.ok("send_request", json!({"path": "r.yml"})).await;
    let bytes = std::fs::read(&file).expect("the cache file");
    assert!(!String::from_utf8_lossy(&bytes).contains(TOKEN));
}

const REFRESH: &str = "rt-91be-SECRET";

/// `auth` with refreshing on and the refresh token sent to `refresh_url`.
fn refreshing(token_url: &str, refresh_url: &str) -> String {
    format!(
        "{}    refreshTokenUrl: \"{refresh_url}\"\n    settings:\n      autoFetchToken: true\n      autoRefreshToken: true\n",
        auth(token_url)
    )
}

#[tokio::test]
async fn an_untrusted_refresh_host_files_an_approval_and_keeps_the_cached_token() {
    let p = MockServer::start().await;
    // The first token is already expired, so the next send must refresh it.
    let expired = json!({"access_token": TOKEN, "token_type": "Bearer", "expires_in": 0, "refresh_token": REFRESH});
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=client_credentials"))
        .respond_with(ResponseTemplate::new(200).set_body_json(expired))
        .mount(&p)
        .await;
    let fresh = json!({"access_token": "tok-2", "token_type": "Bearer", "expires_in": 3600});
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fresh))
        .mount(&p)
        .await;
    Mock::given(method("GET"))
        .and(path("/r"))
        .respond_with(Echo)
        .mount(&p)
        .await;
    let uri = p.uri();
    let request = |refresh_url: &str| {
        get(
            &format!("{uri}/r"),
            &refreshing(&format!("{uri}/token"), refresh_url),
            "",
        )
    };
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request("https://auth.example.test/refresh")),
    ])
    .await;
    server.ok("send_request", json!({"path": "r.yml"})).await;

    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(message.contains("`auth.example.test`"), "{message}");
    assert!(
        !message.contains(REFRESH) && !message.contains(TOKEN),
        "{message}"
    );
    let pending = server.inbox().pending().expect("inbox");
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0].host, "auth.example.test");
    let gets = |p: &[Request]| p.iter().filter(|r| r.method.as_str() == "GET").count();
    assert_eq!(
        gets(&p.received_requests().await.unwrap()),
        1,
        "the refused send went out"
    );

    // With a local refresh URL the kept refresh token is used, so the refusal left the cache alone.
    server.write("r.yml", &request(&format!("{uri}/token")));
    let sent = server
        .ok("send_request", json!({"path": "r.yml"}))
        .await
        .to_string();
    assert!(sent.contains("Bearer"), "{sent}");
    let received = p.received_requests().await.unwrap();
    let refreshed = received
        .iter()
        .filter(|r| String::from_utf8_lossy(&r.body).contains("grant_type=refresh_token"))
        .count();
    assert_eq!(refreshed, 1, "{received:?}");
    assert_eq!(gets(&received), 2);
}

#[tokio::test]
async fn a_cancel_during_the_token_fetch_sends_nothing() {
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
    let r = get(
        &format!("{}/r", p.uri()),
        &auth(&format!("{}/token", p.uri())),
        "",
    );
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", &r)]).await;
    let handle = server
        .start_call("send_request", json!({"path": "r.yml"}))
        .await;
    // Cancel once the token request is in flight, then wait past the provider's delay.
    for _ in 0..100 {
        if !p.received_requests().await.unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    handle.cancel(None).await.expect("cancel sent");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let received = p.received_requests().await.unwrap();
    assert!(
        received.iter().all(|r| r.method.as_str() == "POST"),
        "{received:?}"
    );
}
