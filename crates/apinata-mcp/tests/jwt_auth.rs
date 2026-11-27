//! JWT auth through MCP: `update_request` writes it, `read_request` shows its fields, and no
//! result carries the secret, the key text or a signed token.

use crate::common;

use common::{ROOT, TestServer, request};
use serde_json::{Value, json};
use std::fs;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const KEY: &str = include_str!("../../apinata-core/tests/fixtures/jwt/ec256-sec1.pem");
const SECRET: &str = "jwt-mcp-secret-77";
const DEV: &str = "name: dev\nvariables:\n  - name: jwt_secret\n    secret: true\n  - name: jwt_key\n    secret: true\n";

/// An HS256 auth signed with the `jwt_secret` variable, in MCP's shape.
fn hs256() -> Value {
    json!({"type": "jwt", "algorithm": "HS256", "secret": "{{jwt_secret}}",
        "secret_base64": false, "private_key": null, "headers": "{\"kid\":\"k1\"}",
        "payload": "{\"sub\":\"me\"}", "expires_in": "60", "placement": "header",
        "header_prefix": "Bearer", "query_param_key": "token"})
}

/// An ES256 auth whose inline key is the `jwt_key` variable, sent in the query.
fn es256() -> Value {
    json!({"type": "jwt", "algorithm": "ES256", "secret": "", "secret_base64": false,
        "private_key": {"type": "text", "value": "{{jwt_key}}"}, "headers": "",
        "payload": "{\"sub\":\"me\"}", "expires_in": "", "placement": "query",
        "header_prefix": null, "query_param_key": "jwt"})
}

#[tokio::test]
async fn update_request_writes_jwt_auth_and_read_request_shows_it() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request("r", 1, "GET", "http://127.0.0.1:9/")),
    ])
    .await;
    for auth in [hs256(), es256()] {
        let doc = server
            .ok("update_request", json!({"path": "r.yml", "auth": auth}))
            .await;
        assert_eq!(doc["auth"], auth);
        let read = server.ok("read_request", json!({"path": "r.yml"})).await;
        assert_eq!(read["auth"], auth);
    }
    let file = fs::read_to_string(server.root().join("r.yml")).expect("read");
    assert!(
        file.contains("  auth:\n    type: jwt\n    algorithm: ES256\n    secret: \"\"\n    secretBase64: false\n    privateKey:\n      type: text\n      value: \"{{jwt_key}}\"\n    headers: \"\"\n    payload: '{\"sub\":\"me\"}'\n    placement: query\n    queryParamKey: jwt\n"),
        "{file}"
    );
}

#[tokio::test]
async fn a_bad_jwt_auth_is_refused_by_field_and_writes_nothing() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request("r", 1, "GET", "http://127.0.0.1:9/")),
    ])
    .await;
    let before = fs::read_to_string(server.root().join("r.yml")).expect("read");
    let mut bad = hs256();
    bad["secret"] = json!(SECRET);
    bad["secret_base64"] = json!("yes");
    let message = server
        .err("update_request", json!({"path": "r.yml", "auth": bad}))
        .await;
    assert!(message.contains("secret_base64"), "{message}");
    assert!(!message.contains(SECRET), "{message}");
    let mut missing = hs256();
    missing.as_object_mut().expect("object").remove("payload");
    let message = server
        .err("update_request", json!({"path": "r.yml", "auth": missing}))
        .await;
    assert!(message.contains("payload"), "{message}");
    let after = fs::read_to_string(server.root().join("r.yml")).expect("read");
    assert_eq!(before, after);
}

/// Answers with the Authorization header and the URL, so a leak would come straight back.
struct Echo;
impl Respond for Echo {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let auth = req
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        ResponseTemplate::new(200).set_body_string(format!("{auth} | {}", req.url))
    }
}

/// The token each received request carried, in the header or the `jwt` query parameter.
async fn tokens(s: &MockServer) -> Vec<String> {
    let mut out = Vec::new();
    for r in s.received_requests().await.expect("recording") {
        if let Some(v) = r.headers.get("authorization").and_then(|v| v.to_str().ok()) {
            out.push(v.trim_start_matches("Bearer ").to_owned());
        }
        out.extend(
            r.url
                .query_pairs()
                .filter(|(k, _)| k == "jwt")
                .map(|(_, v)| v.into_owned()),
        );
    }
    out
}

#[tokio::test]
async fn no_result_holds_the_secret_the_key_text_or_a_token() {
    let s = MockServer::start().await;
    Mock::given(any()).respond_with(Echo).mount(&s).await;
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("h.yml", &request("h", 1, "GET", &format!("{}/h", s.uri()))),
        ("q.yml", &request("q", 2, "GET", &format!("{}/q", s.uri()))),
    ])
    .await;
    server.set_secret("dev", "jwt_secret", SECRET);
    server.set_secret("dev", "jwt_key", KEY);
    let mut seen = Vec::new();
    for (path, auth) in [("h.yml", hs256()), ("q.yml", es256())] {
        let written = server
            .ok("update_request", json!({"path": path, "auth": auth}))
            .await;
        seen.push(written.to_string());
        let sent = server
            .call("send_request", json!({"path": path, "environment": "dev"}))
            .await;
        assert_ne!(sent.is_error, Some(true), "{:?}", sent.content);
        seen.push(serde_json::to_value(&sent).expect("serialises").to_string());
        let read = server.ok("read_request", json!({"path": path})).await;
        seen.push(read.to_string());
    }
    let tokens = tokens(&s).await;
    assert_eq!(tokens.len(), 2, "{tokens:?}");
    let key_lines = KEY
        .lines()
        .filter(|l| !l.starts_with("-----") && l.len() >= 16);
    let leaks: Vec<String> = tokens
        .iter()
        .flat_map(|t| {
            let mut parts = vec![t.clone()];
            parts.extend(t.split('.').filter(|p| p.len() >= 16).map(str::to_owned));
            parts
        })
        .chain([SECRET.to_owned()])
        .chain(key_lines.map(str::to_owned))
        .collect();
    for text in &seen {
        for leak in &leaks {
            assert!(!text.contains(leak.as_str()), "{leak} in {text}");
        }
    }
}

#[tokio::test]
async fn a_jwt_request_to_a_non_local_host_needs_approval_first() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        (
            "r.yml",
            &request("r", 1, "GET", "https://api.example.com/v1"),
        ),
    ])
    .await;
    server.set_secret("dev", "jwt_secret", SECRET);
    server
        .ok("update_request", json!({"path": "r.yml", "auth": hs256()}))
        .await;
    let message = server
        .err(
            "send_request",
            json!({"path": "r.yml", "environment": "dev"}),
        )
        .await;
    assert!(message.contains("`api.example.com`"), "{message}");
    assert!(!message.contains(SECRET), "{message}");
    let pending = server.inbox().pending().expect("inbox");
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0].host, "api.example.com");
    let filed = fs::read_to_string(&pending[0].file).expect("read");
    assert!(!filed.contains(SECRET), "{filed}");
}
