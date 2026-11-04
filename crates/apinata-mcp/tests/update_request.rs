//! `update_request`: partial updates, refusals that leave no draft behind, and every request case
//! of the edit parity corpus replayed as an agent would send it.

use crate::common;

use common::{ROOT, TestServer, patch, request, research};
use serde_json::{Value, json};
use std::fs;

const GRPC: &str = "info:\n  name: Q\n  type: grpc\n  seq: 2\ngrpc:\n  url: http://localhost/q\n";

#[tokio::test]
async fn replaces_only_the_sections_given_and_returns_the_saved_doc() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        (
            "users/get.yml",
            &request("Get", 1, "GET", "http://localhost/users"),
        ),
    ])
    .await;
    let doc = server
        .ok(
            "update_request",
            json!({"path": "users/get.yml", "method": "POST",
            "headers": [{"name": "Accept", "value": "application/json", "enabled": true}]}),
        )
        .await;
    assert_eq!(doc["method"], "POST");
    assert_eq!(doc["url"], "http://localhost/users");
    assert_eq!(doc["headers"][0]["name"], "Accept");
    let disk = fs::read_to_string(server.root().join("users/get.yml")).expect("read");
    assert!(
        disk.contains("method: POST") && disk.contains("name: Accept"),
        "{disk}"
    );
    assert_eq!(
        server
            .ok("read_request", json!({"path": "users/get.yml"}))
            .await,
        doc
    );
}

#[tokio::test]
async fn params_sent_with_a_url_win_over_its_query() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request("r", 1, "GET", "http://localhost/a")),
    ])
    .await;
    let doc = server
        .ok(
            "update_request",
            json!({"path": "r.yml", "url": "http://localhost/b?a=1",
            "params": [{"name": "b", "value": "2", "kind": "query", "enabled": true}]}),
        )
        .await;
    assert_eq!(doc["url"], "http://localhost/b?b=2");
    assert_eq!(doc["params"].as_array().expect("params").len(), 1);
}

#[tokio::test]
async fn refuses_opaque_and_missing_items_by_path() {
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("q.yml", GRPC)]).await;
    let err = server
        .err(
            "update_request",
            json!({"path": "q.yml", "url": "http://x"}),
        )
        .await;
    assert!(err.contains("q.yml") && err.contains("grpc"), "{err}");
    let err = server
        .err(
            "update_request",
            json!({"path": "nope.yml", "method": "PUT"}),
        )
        .await;
    assert!(err.contains("nope.yml"), "{err}");
    assert_eq!(
        fs::read_to_string(server.root().join("q.yml")).expect("read"),
        GRPC
    );
}

#[tokio::test]
async fn a_refused_section_leaves_no_half_applied_draft() {
    let original = request("Get", 1, "GET", "http://localhost/a");
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", &original)]).await;
    // The URL applies first; the header name with a line break is then refused.
    let err = server
        .err(
            "update_request",
            json!({"path": "r.yml", "url": "http://localhost/changed",
            "headers": [{"name": "X\nY", "value": "1", "enabled": true}]}),
        )
        .await;
    assert!(!err.is_empty());
    assert_eq!(
        fs::read_to_string(server.root().join("r.yml")).expect("read"),
        original
    );
    let doc = server.ok("read_request", json!({"path": "r.yml"})).await;
    assert_eq!(doc["url"], "http://localhost/a", "the draft was dropped");
}

#[tokio::test]
async fn is_annotated_as_a_closed_world_non_destructive_write() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let tools = server.tools().await;
    let tool = tools
        .iter()
        .find(|t| t.name == "update_request")
        .expect("listed");
    let hints = tool.annotations.clone().expect("annotated");
    assert_eq!(hints.read_only_hint, None);
    assert_eq!(hints.destructive_hint, Some(false));
    assert_eq!(hints.open_world_hint, Some(false));
}

#[tokio::test]
async fn every_request_case_of_the_edit_corpus_matches_bruno() {
    let corpus = research("2026-10-09-edit-parity");
    let cases: Value =
        serde_json::from_str(&fs::read_to_string(corpus.join("cases.json")).expect("cases"))
            .expect("json");
    let mut failures = Vec::new();
    let mut replayed = 0;
    for case in cases.as_array().expect("array") {
        let id = case["id"].as_str().expect("id");
        // Opaque items are list, move, rename and delete only (spec "Document shapes").
        if id.starts_with("opaque-") {
            continue;
        }
        let fixture = case["fixture"].as_str().expect("fixture");
        let source = fs::read_to_string(corpus.join("normalized").join(fixture)).expect("fixture");
        let expected = fs::read_to_string(corpus.join("expected").join(format!("{id}.yml")))
            .expect("expected");
        // Bruno's output, read as a document, is the target the agent edits towards.
        let server = TestServer::start(&[
            ("opencollection.yml", ROOT),
            ("case.yml", &source),
            ("target.yml", &expected),
        ])
        .await;
        let current = server.ok("read_request", json!({"path": "case.yml"})).await;
        let target = server
            .ok("read_request", json!({"path": "target.yml"}))
            .await;
        let result = server
            .call("update_request", patch("case.yml", &current, &target))
            .await;
        let actual = fs::read_to_string(server.root().join("case.yml")).expect("read back");
        if result.is_error == Some(true) || actual != expected {
            failures.push(format!(
                "{id}: {}\n  expected {expected:?}\n  actual   {actual:?}",
                common::text(&result)
            ));
        }
        replayed += 1;
    }
    assert_eq!(replayed, 56, "every case but opaque-seq");
    assert!(
        failures.is_empty(),
        "{} differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[tokio::test]
async fn a_request_sent_back_as_read_leaves_the_file_byte_identical() {
    // No `http.params` list: the read shows params derived from the URL, as the app does.
    let original = request("r", 1, "GET", "http://localhost/u/:id?a=1&b");
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", &original)]).await;
    let mut echo = server.ok("read_request", json!({"path": "r.yml"})).await;
    assert_eq!(
        echo["params"].as_array().expect("params").len(),
        3,
        "{echo}"
    );
    echo["path"] = json!("r.yml");
    server.ok("update_request", echo).await;
    assert_eq!(
        fs::read_to_string(server.root().join("r.yml")).expect("read"),
        original
    );
}

#[tokio::test]
async fn editing_a_derived_param_writes_the_list_and_the_url() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &request("r", 1, "GET", "http://localhost/a?a=1&b=2"),
        ),
    ])
    .await;
    let doc = server
        .ok(
            "update_request",
            json!({"path": "r.yml", "params": [
                {"name": "a", "value": "1", "kind": "query", "enabled": true},
                {"name": "b", "value": "3", "kind": "query", "enabled": true}
            ]}),
        )
        .await;
    assert_eq!(doc["url"], "http://localhost/a?a=1&b=3");
    let disk = fs::read_to_string(server.root().join("r.yml")).expect("read");
    assert!(
        disk.contains("params:") && disk.contains("value: \"3\""),
        "{disk}"
    );
}

#[tokio::test]
async fn a_custom_method_is_stored_uppercased_and_a_bad_one_changes_nothing() {
    let original = request("Get", 1, "GET", "http://localhost/a");
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", &original)]).await;
    let doc = server
        .ok(
            "update_request",
            json!({"path": "r.yml", "method": "propfind"}),
        )
        .await;
    assert_eq!(doc["method"], "PROPFIND");
    assert!(
        fs::read_to_string(server.root().join("r.yml"))
            .expect("read")
            .contains("method: PROPFIND")
    );
    let before = fs::read_to_string(server.root().join("r.yml")).expect("read");
    for bad in ["GE T", ""] {
        let err = server
            .err("update_request", json!({"path": "r.yml", "method": bad}))
            .await;
        assert!(err.contains("method"), "{err}");
    }
    assert_eq!(
        fs::read_to_string(server.root().join("r.yml")).expect("read"),
        before
    );
}

/// A request whose URL sends `z=6` while its params block holds `z: 3`.
const MISMATCHED: &str = "info:\n  name: m\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://localhost/m?z=6\"\n  params:\n    - name: z\n      value: \"3\"\n      type: query\n";

#[tokio::test]
async fn a_mismatch_left_by_a_patch_of_another_section_is_reported_as_saved() {
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("m.yml", MISMATCHED)]).await;
    let doc = server
        .ok("update_request", json!({"path": "m.yml", "docs": "Notes."}))
        .await;
    assert_eq!(doc["docs"], "Notes.");
    assert_eq!(
        doc["query_mismatch"],
        json!({"names": ["z"], "note": "the URL's query is sent"})
    );
}

#[tokio::test]
async fn a_patch_that_sets_the_url_settles_the_mismatch() {
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("m.yml", MISMATCHED)]).await;
    let doc = server
        .ok(
            "update_request",
            json!({"path": "m.yml", "url": "http://localhost/m?z=7"}),
        )
        .await;
    assert!(doc.get("query_mismatch").is_none(), "{doc}");
}

#[tokio::test]
async fn an_echoed_read_with_a_query_mismatch_still_applies() {
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("m.yml", MISMATCHED)]).await;
    let mut echo = server.ok("read_request", json!({"path": "m.yml"})).await;
    echo["path"] = json!("m.yml");
    let doc = server.ok("update_request", echo).await;
    assert_eq!(
        doc["query_mismatch"],
        json!({"names": ["z"], "note": "the URL's query is sent"})
    );
    assert_eq!(
        fs::read_to_string(server.root().join("m.yml")).expect("read"),
        MISMATCHED
    );
}

#[tokio::test]
async fn the_input_schema_leaves_out_query_mismatch() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let tools = server.tools().await;
    let tool = tools
        .iter()
        .find(|t| t.name == "update_request")
        .expect("listed");
    let props = tool.input_schema["properties"].as_object().expect("props");
    assert!(props.contains_key("url"), "{props:?}");
    assert!(!props.contains_key("query_mismatch"), "{props:?}");
}

#[tokio::test]
async fn sync_query_url_keeps_the_urls_query_and_rewrites_the_params() {
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("m.yml", MISMATCHED)]).await;
    let doc = server
        .ok(
            "update_request",
            json!({"path": "m.yml", "sync_query": "url"}),
        )
        .await;
    assert!(doc.get("query_mismatch").is_none(), "{doc}");
    assert_eq!(doc["url"], "http://localhost/m?z=6");
    assert_eq!(doc["params"][0]["name"], "z");
    assert_eq!(doc["params"][0]["value"], "6");
    let read = server.ok("read_request", json!({"path": "m.yml"})).await;
    assert!(read.get("query_mismatch").is_none(), "{read}");
}

#[tokio::test]
async fn sync_query_params_rebuilds_the_urls_query_from_the_params() {
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("m.yml", MISMATCHED)]).await;
    let doc = server
        .ok(
            "update_request",
            json!({"path": "m.yml", "sync_query": "params"}),
        )
        .await;
    assert!(doc.get("query_mismatch").is_none(), "{doc}");
    assert_eq!(doc["url"], "http://localhost/m?z=3");
    let read = server.ok("read_request", json!({"path": "m.yml"})).await;
    assert!(read.get("query_mismatch").is_none(), "{read}");
}

#[tokio::test]
async fn sync_query_params_escapes_a_delimiter_in_a_value_as_the_app_does() {
    let file = "info:\n  name: m\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://localhost/m?v=x\"\n  params:\n    - name: v\n      value: \"a&b\"\n      type: query\n";
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("m.yml", file)]).await;
    let doc = server
        .ok(
            "update_request",
            json!({"path": "m.yml", "sync_query": "params"}),
        )
        .await;
    assert_eq!(doc["url"], "http://localhost/m?v=a%26b");
    assert_eq!(doc["params"][0]["value"], "a&b");
    let read = server.ok("read_request", json!({"path": "m.yml"})).await;
    assert!(read.get("query_mismatch").is_none(), "{read}");
}

#[tokio::test]
async fn an_echoed_read_with_sync_query_settles_the_mismatch() {
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("m.yml", MISMATCHED)]).await;
    let mut echo = server.ok("read_request", json!({"path": "m.yml"})).await;
    echo["path"] = json!("m.yml");
    echo["sync_query"] = json!("params");
    let doc = server.ok("update_request", echo).await;
    assert!(doc.get("query_mismatch").is_none(), "{doc}");
    assert_eq!(doc["url"], "http://localhost/m?z=3");
}

#[tokio::test]
async fn sync_query_on_an_agreeing_request_leaves_its_bytes() {
    // The URL and the table carry the same pairs in different orders, so either side rewritten
    // would reorder the other; agreeing, neither is touched.
    let agreeing = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://localhost/a?b=2&a=1\"\n  params:\n    - name: a\n      value: \"1\"\n      type: query\n    - name: b\n      value: \"2\"\n      type: query\n";
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("a.yml", agreeing)]).await;
    for side in ["url", "params"] {
        server
            .ok(
                "update_request",
                json!({"path": "a.yml", "sync_query": side}),
            )
            .await;
        assert_eq!(
            fs::read_to_string(server.root().join("a.yml")).expect("read"),
            agreeing
        );
    }
}

#[tokio::test]
async fn an_unknown_sync_query_side_is_refused() {
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("m.yml", MISMATCHED)]).await;
    let err = server
        .err(
            "update_request",
            json!({"path": "m.yml", "sync_query": "both"}),
        )
        .await;
    assert!(err.contains("`sync_query`: unknown variant"), "{err}");
    assert!(!err.contains("both"), "{err}");
    assert_eq!(
        fs::read_to_string(server.root().join("m.yml")).expect("read"),
        MISMATCHED
    );
}

#[tokio::test]
async fn the_input_schema_offers_sync_query_as_url_or_params() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let tools = server.tools().await;
    let tool = tools
        .iter()
        .find(|t| t.name == "update_request")
        .expect("listed");
    let schema = Value::Object((*tool.input_schema).clone()).to_string();
    let props = tool.input_schema["properties"].as_object().expect("props");
    assert!(props.contains_key("sync_query"), "{props:?}");
    assert!(schema.contains(r#"["url","params"]"#), "{schema}");
    let description = tool.description.as_deref().unwrap_or_default();
    assert!(description.contains("sync_query"), "{description}");
}
