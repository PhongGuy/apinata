//! `send_request`'s `body_filter`: the matched text is returned, sized against the budget,
//! with `total_bytes` still the whole body.

use crate::common;

use common::{ROOT, TestServer};
use serde_json::{Value, json};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const DEV: &str = "name: dev\nvariables:\n  - name: token\n    secret: true\n";

fn get(url: &str) -> String {
    format!("info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\n")
}

async fn mock(body: &str, content_type: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body.to_owned(), content_type))
        .mount(&server)
        .await;
    server
}

async fn start(url: &str) -> TestServer {
    TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("r.yml", &get(url)),
    ])
    .await
}

const LIST: &str = r#"[{"name":"first","n":1},{"name":"second","n":2}]"#;

#[tokio::test]
async fn the_matched_text_comes_back_with_the_size_of_the_whole_body() {
    let mock = mock(LIST, "application/json").await;
    let server = start(&mock.uri()).await;
    let result = server
        .ok(
            "send_request",
            json!({"path": "r.yml", "body_filter": "$[1].name"}),
        )
        .await;
    assert_eq!(result["body"]["text"], "\"second\"");
    assert_eq!(result["body"]["total_bytes"], LIST.len());
    assert_eq!(result["body"]["truncated"], false);
    assert_eq!(result["body"]["filter_error"], Value::Null);
}

#[tokio::test]
async fn no_match_is_an_empty_array_and_xml_uses_xpath() {
    let mock_json = mock(LIST, "application/json").await;
    let server = start(&mock_json.uri()).await;
    let result = server
        .ok(
            "send_request",
            json!({"path": "r.yml", "body_filter": "$[*].nope"}),
        )
        .await;
    assert_eq!(result["body"]["text"], "[]");

    let xml = mock("<r><i>x</i><i>y</i></r>", "application/xml").await;
    server.write("r.yml", &get(&xml.uri()));
    let result = server
        .ok(
            "send_request",
            json!({"path": "r.yml", "body_filter": "//i"}),
        )
        .await;
    assert_eq!(result["body"]["text"], "<i>x</i>\n<i>y</i>\n");
    let result = server
        .ok(
            "send_request",
            json!({"path": "r.yml", "body_filter": "string(//nope)"}),
        )
        .await;
    assert_eq!(
        result["body"]["text"], "\n",
        "an empty value is not no match"
    );
}

#[tokio::test]
async fn a_bad_expression_names_itself_and_sends_nothing() {
    let mock = mock(LIST, "application/json").await;
    let server = start(&mock.uri()).await;
    let message = server
        .err(
            "send_request",
            json!({"path": "r.yml", "body_filter": "$[?("}),
        )
        .await;
    assert!(message.contains("$[?("), "{message}");
    assert!(message.contains("nothing was sent"), "{message}");
    assert!(mock.received_requests().await.expect("recorded").is_empty());
}

#[tokio::test]
async fn a_body_that_cannot_be_filtered_reports_why_instead_of_text() {
    let mock = mock("plain words", "text/plain").await;
    let server = start(&mock.uri()).await;
    let result = server
        .ok(
            "send_request",
            json!({"path": "r.yml", "body_filter": "$.a"}),
        )
        .await;
    assert_eq!(result["body"]["text"], Value::Null);
    let why = result["body"]["filter_error"].as_str().expect("reason");
    assert!(why.contains("text/plain"), "{why}");
    assert_eq!(result["body"]["total_bytes"], 11);
}

#[tokio::test]
async fn truncated_means_the_filtered_text_was_cut() {
    let mock = mock(LIST, "application/json").await;
    let server = start(&mock.uri()).await;
    let cut = server
        .ok(
            "send_request",
            json!({"path": "r.yml", "body_filter": "$[*].name", "max_body_bytes": 10}),
        )
        .await;
    assert_eq!(cut["body"]["truncated"], true);
    assert_eq!(cut["body"]["text"].as_str().expect("text").len(), 10);
    // The whole body is far over the budget, but the match fits, so nothing is cut.
    let fits = server
        .ok(
            "send_request",
            json!({"path": "r.yml", "body_filter": "$[0].n", "max_body_bytes": 10}),
        )
        .await;
    assert_eq!(fits["body"]["truncated"], false);
    assert_eq!(fits["body"]["text"], "1");
}

#[tokio::test]
async fn secrets_are_scrubbed_before_the_filter_runs() {
    let mock = mock(r#"{"token":"Sec-Val-99","n":1}"#, "application/json").await;
    let server = start(&format!("{}/?k={{{{token}}}}", mock.uri())).await;
    server.set_secret("dev", "token", "Sec-Val-99");
    let result = server
        .call(
            "send_request",
            json!({"path": "r.yml", "environment": "dev", "body_filter": "$.token"}),
        )
        .await;
    let shown = serde_json::to_string(&result).expect("serialises");
    assert!(!shown.contains("Sec-Val-99"), "{shown}");
    let text = &result.structured_content.expect("structured")["body"]["text"];
    assert_eq!(text, "\"<secret>\"");
}

#[tokio::test]
async fn a_secret_hidden_by_a_json_escape_is_scrubbed_after_the_filter() {
    let mock = mock(r#"{"b":"Sec\u002dVal-99"}"#, "application/json").await;
    let server = start(&format!("{}/?k={{{{token}}}}", mock.uri())).await;
    server.set_secret("dev", "token", "Sec-Val-99");
    let result = server
        .call(
            "send_request",
            json!({"path": "r.yml", "environment": "dev", "body_filter": "$.b"}),
        )
        .await;
    let shown = serde_json::to_string(&result).expect("serialises");
    assert!(!shown.contains("Sec-Val-99"), "{shown}");
}
