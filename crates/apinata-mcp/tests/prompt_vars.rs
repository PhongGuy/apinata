//! `send_request` and `run_folder` refuse prompt variables by name and send nothing.

use crate::common;

use common::{ROOT, TestServer, request};
use serde_json::json;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::any};

#[tokio::test]
async fn send_and_run_name_the_labels_and_send_nothing() {
    let mock = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;
    let url = format!("{}/{{{{?code}}}}", mock.uri());
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("api/r.yml", &request("r", 1, "GET", &url)),
    ])
    .await;
    let message = server
        .err("send_request", json!({"path": "api/r.yml"}))
        .await;
    assert!(
        message.contains("no value for prompt variables: code"),
        "{message}"
    );
    assert!(message.contains("nothing was sent"), "{message}");
    let result = server.ok("run_folder", json!({"path": "api"})).await;
    let error = result["results"][0]["error"].as_str().expect("error");
    assert!(
        error.contains("no value for prompt variables: code"),
        "{error}"
    );
    assert!(mock.received_requests().await.expect("recorded").is_empty());
}
