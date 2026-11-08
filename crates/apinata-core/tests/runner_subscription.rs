//! `run` on a subscription: a failed socket errors the result, and handshake retries.

use crate::common::ws_server::{QUERY, Step, WsScript, raw_server, tick, ws_server};
use crate::script_support::{ROOT, run_dir, write_tree};
use apinata_core::{Outcome, RunOptions};
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A GraphQL item POSTing `query` to `url` with `assertions` as `runtime.assertions` items.
fn item(url: &str, query: &str, assertions: &str) -> String {
    format!(
        "info:\n  name: s\n  type: graphql\n  seq: 1\ngraphql:\n  method: POST\n  url: \"{url}\"\n  body:\n    query: {}\nruntime:\n  assertions:\n{assertions}",
        serde_json::to_string(query).expect("query")
    )
}

async fn one(
    url: &str,
    query: &str,
    assertions: &str,
    opts: RunOptions,
) -> apinata_core::RequestResult {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("s.yml", &item(url, query, assertions)),
    ]);
    let report = run_dir(dir.path(), opts).await;
    assert_eq!(report.results.len(), 1);
    (*report.results[0]).clone()
}

const BODY_HAS_TICK: &str =
    "    - expression: res.body\n      operator: contains\n      value: \"tick\"\n";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_completed_subscription_passes_its_assertions_on_the_transcript() {
    let s = ws_server(WsScript::new(vec![tick(1), Step::Complete])).await;
    let r = one(&s.http("/"), QUERY, BODY_HAS_TICK, RunOptions::new()).await;
    assert_eq!(r.outcome, Outcome::Passed, "{r:?}");
    assert_eq!(r.assertions.len(), 1);
    assert!(r.error.is_none(), "{r:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_handshake_errors_the_request_and_keeps_the_reply() {
    let port = raw_server("HTTP/1.1 404 Not Found\r\ncontent-length: 4\r\n\r\nnope").await;
    let url = format!("http://127.0.0.1:{port}/graphql");
    let r = one(&url, QUERY, "", RunOptions::new()).await;
    assert_eq!(r.outcome, Outcome::Errored, "{r:?}");
    assert_eq!(
        r.error.as_ref().map(ToString::to_string),
        Some(format!(
            "WebSocket handshake to ws://127.0.0.1:{port}/graphql failed: 404 Not Found"
        ))
    );
    assert_eq!(r.response.as_ref().map(|x| x.status()), Some(404));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bad_close_code_errors_the_request() {
    let s = ws_server(WsScript::new(vec![tick(1), Step::Close(4403, "Forbidden")])).await;
    let r = one(&s.http("/"), QUERY, BODY_HAS_TICK, RunOptions::new()).await;
    assert_eq!(r.outcome, Outcome::Errored, "{r:?}");
    assert_eq!(
        r.error.as_ref().map(ToString::to_string).as_deref(),
        Some("closed by server: 4403 Forbidden")
    );
    assert_eq!(r.response.as_ref().map(|x| x.status()), Some(101));
    // Assertions still ran on what arrived.
    assert_eq!(r.assertions.len(), 1);
    // The message may quote server text, so Debug shows only its length.
    let debug = format!("{:?}", r.error.expect("error"));
    assert!(debug.contains("message_len: 32"), "{debug}");
    assert!(!debug.contains("Forbidden"), "{debug}");
}

async fn always(status: u16) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(status))
        .mount(&s)
        .await;
    s
}

async fn methods(s: &MockServer) -> Vec<String> {
    let seen = s.received_requests().await.expect("recorded");
    seen.iter().map(|r| r.method.to_string()).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_handshake_is_retried_as_a_get() {
    let s = always(503).await;
    let r = one(&s.uri(), QUERY, "", RunOptions::new().retries(2)).await;
    assert_eq!(r.attempts(), 3, "{r:?}");
    assert_eq!(methods(&s).await, ["GET", "GET", "GET"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_graphql_query_is_still_a_post_that_is_not_retried() {
    let s = always(503).await;
    let r = one(&s.uri(), "query { a }", "", RunOptions::new().retries(2)).await;
    assert_eq!(r.attempts(), 1, "{r:?}");
    assert_eq!(methods(&s).await, ["POST"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_socket_that_fails_after_the_101_is_not_retried() {
    let s = ws_server(WsScript::new(vec![tick(1), Step::Close(4403, "Forbidden")])).await;
    let r = one(&s.http("/"), QUERY, "", RunOptions::new().retries(2)).await;
    assert_eq!(r.attempts(), 1, "{r:?}");
    let opened = s
        .seen
        .lines()
        .iter()
        .filter(|l| l.starts_with("path: "))
        .count();
    assert_eq!(opened, 1);
}
