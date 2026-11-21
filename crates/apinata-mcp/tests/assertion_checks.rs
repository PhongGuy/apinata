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
