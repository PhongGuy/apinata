//! GraphQL requests through MCP: read, update, kind checks and create.

use crate::common;

use common::{ROOT, TestServer, request, research};
use serde_json::json;
use std::fs;

fn corpus(rel: &str) -> String {
    fs::read_to_string(research("2026-10-13-graphql-parity").join(rel)).expect("corpus")
}

#[tokio::test]
async fn read_request_shows_the_graphql_body() {
    let full = corpus("normalized/synthetic/graphql-full.yml");
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("g.yml", &full)]).await;
    let doc = server.ok("read_request", json!({"path": "g.yml"})).await;
    assert_eq!(doc["method"], "POST");
    assert_eq!(doc["body"]["type"], "graphql");
    assert!(
        doc["body"]["query"]
            .as_str()
            .unwrap()
            .starts_with("query Search(")
    );
    assert!(
        doc["body"]["variables"]
            .as_str()
            .unwrap()
            .contains("\"limit\": 10")
    );
    assert_eq!(doc["headers"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn update_request_writes_the_reference_bytes() {
    let small = corpus("normalized/synthetic/graphql.yml");
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("g.yml", &small)]).await;
    server
        .ok(
            "update_request",
            json!({"path": "g.yml", "body": {"type": "graphql", "query": "{ users { id } }", "variables": "{\"limit\": 10}"}}),
        )
        .await;
    assert_eq!(
        fs::read_to_string(server.root().join("g.yml")).unwrap(),
        corpus("expected/gql-variables-single.yml")
    );
}

#[tokio::test]
async fn mismatched_bodies_are_refused_and_nothing_is_written() {
    let small = corpus("normalized/synthetic/graphql.yml");
    let http = request("h", 1, "GET", "http://localhost/h");
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("g.yml", &small),
        ("h.yml", &http),
    ])
    .await;
    let err = server
        .err(
            "update_request",
            json!({"path": "g.yml", "body": {"type": "json", "text": "{}"}}),
        )
        .await;
    assert!(
        err.contains("a graphql request takes only a graphql body"),
        "{err}"
    );
    let err = server
        .err(
            "update_request",
            json!({"path": "h.yml", "body": {"type": "graphql", "query": "{ a }", "variables": ""}}),
        )
        .await;
    assert!(
        err.contains("an http request cannot take a graphql body"),
        "{err}"
    );
    assert_eq!(
        fs::read_to_string(server.root().join("g.yml")).unwrap(),
        small
    );
    assert_eq!(
        fs::read_to_string(server.root().join("h.yml")).unwrap(),
        http
    );
}

#[tokio::test]
async fn create_request_takes_a_kind() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let made = server
        .ok(
            "create_request",
            json!({"parent": "", "name": "New GraphQL", "kind": "graphql"}),
        )
        .await;
    assert_eq!(made, json!({"path": "New GraphQL.yml"}));
    assert_eq!(
        fs::read_to_string(server.root().join("New GraphQL.yml")).unwrap(),
        fs::read_to_string(research(
            "2026-10-17-new-request-timeout-inherit/gql-new.yml"
        ))
        .unwrap()
    );
    let err = server
        .err(
            "create_request",
            json!({"parent": "", "name": "X", "kind": "grpc"}),
        )
        .await;
    assert!(err.contains("`kind`: unknown variant"), "{err}");
    assert!(!err.contains("grpc"), "{err}");
}

#[tokio::test]
async fn update_request_description_states_the_graphql_body_rule() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let tools = server.tools().await;
    let tool = tools
        .iter()
        .find(|t| t.name == "update_request")
        .expect("listed");
    let description = tool.description.as_deref().unwrap_or_default();
    assert!(
        description.contains("A graphql request takes only a graphql body")
            && description.contains("empty `query`"),
        "{description}"
    );
}

const PRESET_ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: shop\nextensions:\n  bruno:\n    presets:\n      request:\n        type: graphql\n        url: https://g.test/graphql\n";

#[tokio::test]
async fn create_request_applies_the_collection_presets() {
    let server = TestServer::start(&[("opencollection.yml", PRESET_ROOT)]).await;
    server
        .ok("create_request", json!({"parent": "", "name": "Preset"}))
        .await;
    let doc = server
        .ok("read_request", json!({"path": "Preset.yml"}))
        .await;
    assert_eq!(doc["method"], "POST");
    assert_eq!(doc["url"], "https://g.test/graphql");
    assert_eq!(doc["body"]["type"], "graphql");

    // What the caller sets wins over the presets.
    server
        .ok(
            "create_request",
            json!({"parent": "", "name": "Own", "kind": "http", "url": "https://o.test/x"}),
        )
        .await;
    let doc = server.ok("read_request", json!({"path": "Own.yml"})).await;
    assert_eq!(doc["method"], "GET");
    assert_eq!(doc["url"], "https://o.test/x");

    // An explicit kind alone still gets the preset URL.
    server
        .ok(
            "create_request",
            json!({"parent": "", "name": "Kind", "kind": "http"}),
        )
        .await;
    let doc = server.ok("read_request", json!({"path": "Kind.yml"})).await;
    assert_eq!(doc["method"], "GET");
    assert_eq!(doc["url"], "https://g.test/graphql");
}

#[tokio::test]
async fn create_request_without_presets_is_an_empty_get() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    server
        .ok("create_request", json!({"parent": "", "name": "Plain"}))
        .await;
    let doc = server
        .ok("read_request", json!({"path": "Plain.yml"}))
        .await;
    assert_eq!(doc["method"], "GET");
    assert_eq!(doc["url"], "");
}
