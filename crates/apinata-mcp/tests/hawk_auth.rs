//! Hawk auth through MCP: `update_request` writes it and `read_request` shows its fields.

use crate::common;

use common::{ROOT, TestServer, request};
use serde_json::{Value, json};
use std::fs;

fn hawk() -> Value {
    json!({"type": "hawk", "auth_id": "id-1", "auth_key": "{{hawk_key}}",
        "algorithm": "sha256", "ext": "x", "app": "", "dlg": "", "nonce": "",
        "timestamp": "", "include_payload_hash": true})
}

#[tokio::test]
async fn update_request_writes_hawk_auth_and_read_request_shows_it() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request("r", 1, "GET", "http://127.0.0.1:9/")),
    ])
    .await;
    let doc = server
        .ok("update_request", json!({"path": "r.yml", "auth": hawk()}))
        .await;
    assert_eq!(doc["auth"], hawk());
    let read = server.ok("read_request", json!({"path": "r.yml"})).await;
    assert_eq!(read["auth"], hawk());
    let file = fs::read_to_string(server.root().join("r.yml")).expect("read");
    assert!(
        file.contains("  auth:\n    type: hawk\n    authId: id-1\n    authKey: \"{{hawk_key}}\"\n    algorithm: sha256\n    ext: x\n"),
        "{file}"
    );
}

#[tokio::test]
async fn a_hawk_auth_missing_a_field_is_refused_by_name() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request("r", 1, "GET", "http://127.0.0.1:9/")),
    ])
    .await;
    let mut missing = hawk();
    missing.as_object_mut().expect("object").remove("dlg");
    let message = server
        .err("update_request", json!({"path": "r.yml", "auth": missing}))
        .await;
    assert!(message.contains("dlg"), "{message}");
}
