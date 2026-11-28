//! ASAP auth through MCP: `update_request` writes it and `read_request` shows its fields.

use crate::common;

use common::{ROOT, TestServer, request};
use serde_json::{Value, json};
use std::fs;

fn asap() -> Value {
    json!({"type": "asap", "algorithm": "RS256", "issuer": "svc/a", "audience": "svc/b",
        "key_id": "svc/a/key1", "subject": "", "expires_in": "600",
        "additional_claims": "", "private_key": "{{asap_key}}"})
}

#[tokio::test]
async fn update_request_writes_asap_auth_and_read_request_shows_it() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request("r", 1, "GET", "http://127.0.0.1:9/")),
    ])
    .await;
    let doc = server
        .ok("update_request", json!({"path": "r.yml", "auth": asap()}))
        .await;
    assert_eq!(doc["auth"], asap());
    let read = server.ok("read_request", json!({"path": "r.yml"})).await;
    assert_eq!(read["auth"], asap());
    let file = fs::read_to_string(server.root().join("r.yml")).expect("read");
    assert!(
        file.contains("  auth:\n    type: asap\n    algorithm: RS256\n    issuer: svc/a\n    audience: svc/b\n    keyId: svc/a/key1\n"),
        "{file}"
    );
}

#[tokio::test]
async fn an_asap_auth_missing_a_field_is_refused_by_name() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request("r", 1, "GET", "http://127.0.0.1:9/")),
    ])
    .await;
    let mut missing = asap();
    missing.as_object_mut().expect("object").remove("key_id");
    let message = server
        .err("update_request", json!({"path": "r.yml", "auth": missing}))
        .await;
    assert!(message.contains("key_id"), "{message}");
    assert!(!message.contains("asap_key"), "{message}");
}
