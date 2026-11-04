//! `send_request` on a GraphQL request against a local mock server.

use crate::common;

use common::{ROOT, TestServer};
use serde_json::json;
use wiremock::matchers::{body_json, header, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn gql(url: &str, variables: &str) -> String {
    format!(
        "info:\n  name: g\n  type: graphql\ngraphql:\n  url: \"{url}/graphql\"\n  body:\n    query: \"{{ me }}\"\n    variables: '{variables}'\n"
    )
}

#[tokio::test]
async fn sends_the_composed_json_body() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("content-type", "application/json"))
        .and(body_json(json!({"query": "{ me }", "variables": {"a": 1}})))
        .respond_with(ResponseTemplate::new(200).set_body_raw("{\"data\":{}}", "application/json"))
        .mount(&mock)
        .await;
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("g.yml", &gql(&mock.uri(), "{\"a\": 1}")),
    ])
    .await;
    let result = server.ok("send_request", json!({"path": "g.yml"})).await;
    assert_eq!(result["status"], 200);
}

#[tokio::test]
async fn bad_variables_send_nothing() {
    let mock = MockServer::start().await;
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("g.yml", &gql(&mock.uri(), "{oops")),
    ])
    .await;
    let message = server.err("send_request", json!({"path": "g.yml"})).await;
    assert!(
        message.contains("Failed to parse GraphQL variables"),
        "{message}"
    );
    assert!(mock.received_requests().await.expect("recorded").is_empty());
}
