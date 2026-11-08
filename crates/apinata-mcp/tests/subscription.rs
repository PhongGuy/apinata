//! send_request on a GraphQL subscription: a refused handshake's response carries its error,
//! and an untrusted handshake host files an approval like any send.

use crate::common::raw_server::raw_server;
use crate::common::{ROOT, TestServer, request};
use serde_json::json;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn subscription(url: &str) -> String {
    format!(
        "info:\n  name: s\n  type: graphql\n  seq: 1\ngraphql:\n  method: POST\n  url: \"{url}\"\n  body:\n    query: \"subscription {{ tick }}\"\n"
    )
}

async fn start(url: &str) -> TestServer {
    TestServer::start(&[("opencollection.yml", ROOT), ("s.yml", &subscription(url))]).await
}

async fn ok_mock() -> MockServer {
    let mock = MockServer::builder().start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
        .mount(&mock)
        .await;
    mock
}

#[tokio::test]
async fn a_refused_handshake_returns_the_response_with_its_error() {
    let port = raw_server(
        "HTTP/1.1 404 Not Found\r\nContent-Type: text/html\r\nContent-Length: 17\r\n\r\n<h1>Not here</h1>",
    )
    .await;
    let server = start(&format!("http://127.0.0.1:{port}/graphql")).await;
    let r = server.ok("send_request", json!({"path": "s.yml"})).await;
    assert_eq!(r["status"], 404);
    assert_eq!(r["stream_end"], "error");
    assert_eq!(r["body"]["text"], "<h1>Not here</h1>");
    let error = r["error"].as_str().expect("an error field");
    let want = format!("WebSocket handshake to ws://127.0.0.1:{port}/graphql failed: 404");
    assert!(error.starts_with(&want), "{error}");
}

#[tokio::test]
async fn a_send_that_did_not_error_has_no_error_field() {
    let mock = ok_mock().await;
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request("r", 1, "GET", &mock.uri())),
    ])
    .await;
    let r = server.ok("send_request", json!({"path": "r.yml"})).await;
    assert_eq!(r["status"], 200);
    assert!(r.get("error").is_none(), "{r}");
}

// Core records a thrown post-response script as a failed script-error test, not a run error,
// so it reaches the agent through `tests` and leaves `error` absent.
#[tokio::test]
async fn a_post_response_script_error_is_a_failed_test_not_an_error() {
    const POST: &str = "throw new Error('post broke')";
    let mock = ok_mock().await;
    let item = format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{}/ok\"\nruntime:\n  scripts:\n    - type: after-response\n      code: |-\n        {POST}\n",
        mock.uri()
    );
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", &item)]).await;
    server.approve_scripts(&[POST]);
    let r = server.ok("send_request", json!({"path": "r.yml"})).await;
    assert_eq!(r["status"], 200);
    assert!(r.get("error").is_none(), "{r}");
    let test = &r["tests"][0];
    assert_eq!(test["phase"], "post-response", "{r}");
    assert_eq!(test["status"], "fail", "{r}");
    assert_eq!(test["script_error"], true, "{r}");
    assert!(
        test["message"]
            .as_str()
            .is_some_and(|m| m.contains("post broke")),
        "{r}"
    );
}

// The allowlist is enforced on the handshake's http(s) URL in the executor's hop loop, so a
// subscription to an untrusted host is refused and filed like any send.
#[tokio::test]
async fn an_untrusted_handshake_host_files_one_approval() {
    let server = start("https://api.example.com/graphql").await;
    let message = server.err("send_request", json!({"path": "s.yml"})).await;
    assert!(message.contains("`api.example.com`"), "{message}");
    let pending = server.inbox().pending().expect("inbox");
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0].host, "api.example.com");
}
