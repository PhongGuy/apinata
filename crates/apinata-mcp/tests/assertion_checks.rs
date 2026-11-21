//! Assertions over MCP: writes refuse an operator the file format does not know, JavaScript
//! sides are skipped without asking for script approval, and each verdict names what it checked.

use crate::common;

use common::{ROOT, TestServer, request};
use serde_json::json;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A request to `url` carrying one assertion written as `operator` and `value`.
fn asserted(url: &str, expression: &str, operator: &str, value: &str) -> String {
    format!(
        "{}runtime:\n  assertions:\n    - expression: \"{expression}\"\n      operator: {operator}\n      value: \"{value}\"\n",
        request("r", 1, "GET", url)
    )
}

async fn json_ok() -> MockServer {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"name": "ann"})))
        .mount(&mock)
        .await;
    mock
}

#[tokio::test]
async fn update_request_refuses_an_unknown_operator_and_lists_the_valid_ones() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request("r", 1, "GET", "http://localhost/a")),
    ])
    .await;
    let before = std::fs::read_to_string(server.root().join("r.yml")).expect("read");
    let message = server
        .err(
            "update_request",
            json!({"path": "r.yml", "assertions": [{"expression": "res.status",
                "operator": "frobnicate", "value": "1", "enabled": true}]}),
        )
        .await;
    assert!(message.contains("`frobnicate`"), "{message}");
    for op in ["eq", "notMatches", "between", "isArray"] {
        assert!(message.contains(op), "{op} missing: {message}");
    }
    let after = std::fs::read_to_string(server.root().join("r.yml")).expect("read");
    assert_eq!(before, after);
}

#[tokio::test]
async fn update_request_keeps_an_unknown_operator_the_file_already_had() {
    let file = asserted("http://localhost/a", "res.status", "frobnicate", "1");
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", &file)]).await;
    let doc = server.ok("read_request", json!({"path": "r.yml"})).await;
    let mut assertions = doc["assertions"].clone();
    assertions
        .as_array_mut()
        .expect("assertions")
        .push(json!({"expression": "res.body", "operator": "isJson", "enabled": true}));
    server
        .ok(
            "update_request",
            json!({"path": "r.yml", "assertions": assertions}),
        )
        .await;
}

#[tokio::test]
async fn a_javascript_assertion_is_skipped_without_asking_for_approval() {
    let mock = json_ok().await;
    let file = asserted(&mock.uri(), "res.body.name.toUpperCase()", "eq", "ANN");
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", &file)]).await;
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    let assertion = &result["assertions"][0];
    assert_eq!(assertion["verdict"], "skipped", "{assertion}");
    assert!(
        server.inbox().pending_scripts().expect("inbox").is_empty(),
        "nothing filed"
    );
}

#[tokio::test]
async fn a_javascript_assertion_does_not_stop_a_folder_run() {
    let mock = json_ok().await;
    let file = asserted(&mock.uri(), "res.body.name.toUpperCase()", "eq", "ANN");
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("api/r.yml", &file)]).await;
    let result = server.ok("run_folder", json!({"path": "api"})).await;
    let assertion = &result["results"][0]["assertions"][0];
    assert_eq!(assertion["verdict"], "skipped", "{assertion}");
    assert!(server.inbox().pending_scripts().expect("inbox").is_empty());
}

#[tokio::test]
async fn an_assertion_result_names_its_operator_expected_and_actual_values() {
    let mock = json_ok().await;
    let file = asserted(&mock.uri(), "res.status", "eq", "201");
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", &file)]).await;
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    let assertion = &result["assertions"][0];
    assert_eq!(assertion["verdict"], "fail", "{assertion}");
    assert_eq!(assertion["operator"], "eq", "{assertion}");
    assert_eq!(assertion["value"], "201", "{assertion}");
    assert_eq!(assertion["actual"], "200", "{assertion}");
}

#[tokio::test]
async fn update_request_refuses_a_new_value_for_a_kept_unknown_operator() {
    let file = asserted("http://localhost/a", "res.status", "frobnicate", "1");
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", &file)]).await;
    let message = server
        .err(
            "update_request",
            json!({"path": "r.yml", "assertions": [{"expression": "res.status",
                "operator": "frobnicate", "value": "2", "enabled": true}]}),
        )
        .await;
    assert!(
        message.contains("unknown operator `frobnicate`"),
        "{message}"
    );
}

#[tokio::test]
async fn a_skipped_javascript_assertion_says_how_to_approve_it() {
    let mock = json_ok().await;
    let file = asserted(&mock.uri(), "res.body.name.toUpperCase()", "eq", "ANN");
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", &file)]).await;
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    let message = result["assertions"][0]["message"].as_str().expect("reason");
    assert!(message.contains("approve"), "{message}");
    assert!(message.contains("Apinata app"), "{message}");
}

#[tokio::test]
async fn an_approved_javascript_assertion_runs() {
    let mock = json_ok().await;
    let expression = "res.body.name.toUpperCase()";
    let file = asserted(&mock.uri(), expression, "eq", "ANN");
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", &file)]).await;
    server.approve_scripts(&[expression]);
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    let assertion = &result["assertions"][0];
    assert_eq!(assertion["verdict"], "pass", "{assertion}");
}

#[tokio::test]
async fn a_javascript_assertion_in_a_blocked_collection_is_skipped_and_sent() {
    let mock = json_ok().await;
    let file = asserted(&mock.uri(), "res.body.name.toUpperCase()", "eq", "ANN");
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", &file)]).await;
    server.block_scripts();
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    let assertion = &result["assertions"][0];
    assert_eq!(assertion["verdict"], "skipped", "{assertion}");
    assert!(server.inbox().pending_scripts().expect("inbox").is_empty());
    assert_eq!(mock.received_requests().await.expect("recorded").len(), 1);
}

#[tokio::test]
async fn a_script_sharing_its_text_with_an_earlier_assertion_still_needs_approval() {
    let mock = json_ok().await;
    let text = "res.body.name.toUpperCase()";
    let a = asserted(&mock.uri(), text, "eq", "ANN");
    let b = format!(
        "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"{text}\"\n",
        request("b", 2, "GET", &mock.uri())
    );
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("api/a.yml", &a),
        ("api/b.yml", &b),
    ])
    .await;
    let message = server.err("run_folder", json!({"path": "api"})).await;
    assert!(message.contains("scripts need approval"), "{message}");
    let pending = server.inbox().pending_scripts().expect("inbox");
    assert_eq!(pending.len(), 1, "one filing");
    assert_eq!(pending[0].hashes, [apinata_core::script_hash(text)]);
    assert!(mock.received_requests().await.expect("recorded").is_empty());
}
