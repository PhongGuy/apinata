//! A send that fails keeps what went out: the hops that completed and the request left
//! unanswered.

use crate::common;

use apinata_core::{
    CancelHandle, ContextOptions, ExecError, ExecOptions, Executor, ItemPath, RequestResult,
    RunError, RunOptions, send_one,
};
use common::{dead_port, request, setup};
use std::sync::Arc;
use std::time::Duration;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";
const HEADERS: &str = "  headers:\n    - name: Authorization\n      value: Bearer tok-123\n    - name: X-Trace\n      value: abc\n";

fn file(url: &str) -> String {
    format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\n{HEADERS}settings:\n  timeout: 300\n"
    )
}

async fn sent(url: &str) -> RequestResult {
    let (_dir, c, _) = setup(&[("opencollection.yml", ROOT), ("r.yml", &file(url))]);
    let ctx = apinata_core::VarContext::new(&c, ContextOptions::new()).expect("context");
    let exec = Arc::new(Executor::new(ExecOptions::new()).expect("client"));
    send_one(
        ItemPath::new("r.yml").expect("path"),
        request(&c, "r.yml"),
        &ctx,
        &exec,
        &RunOptions::default(),
        &CancelHandle::new(),
    )
    .await
}

fn header<'a>(pairs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    (pairs.iter())
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

#[tokio::test]
async fn a_timeout_after_a_redirect_keeps_the_hop_and_the_unanswered_request() {
    let server = MockServer::start().await;
    Mock::given(path("/start"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/slow"))
        .mount(&server)
        .await;
    Mock::given(path("/slow"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
        .mount(&server)
        .await;
    let r = sent(&format!("{}/start", server.uri())).await;
    assert!(r.response.is_none());
    assert!(
        matches!(r.error, Some(RunError::Exec(ExecError::Timeout { .. }))),
        "{:?}",
        r.error
    );
    let [hop] = r.timeline.as_slice() else {
        panic!("one completed hop: {:?}", r.timeline)
    };
    assert_eq!(hop.status, 302);
    assert!(hop.url.ends_with("/start"), "{}", hop.url);
    let unanswered = r.unanswered.as_ref().expect("the request left unanswered");
    assert_eq!(unanswered.method, "GET");
    assert!(unanswered.url.ends_with("/slow"), "{}", unanswered.url);
    assert_eq!(header(&unanswered.headers, "x-trace"), Some("abc"));
    assert_eq!(
        header(&unanswered.headers, "authorization"),
        Some("Bearer tok-123")
    );
}

#[tokio::test]
async fn a_refused_connection_keeps_the_request_it_tried_to_send() {
    let r = sent(&format!("http://127.0.0.1:{}/down", dead_port())).await;
    assert!(r.response.is_none());
    assert!(r.timeline.is_empty(), "{:?}", r.timeline);
    let unanswered = r.unanswered.as_ref().expect("the request left unanswered");
    assert!(unanswered.url.ends_with("/down"), "{}", unanswered.url);
    assert_eq!(header(&unanswered.headers, "x-trace"), Some("abc"));
    // Debug never prints a header value.
    let debug = format!("{unanswered:?}");
    assert!(!debug.contains("tok-123"), "{debug}");
}

#[tokio::test]
async fn a_credential_placed_on_a_failed_send_is_redacted() {
    let url = format!("http://user:pw-9137@127.0.0.1:{}/down", dead_port());
    let r = sent(&url).await;
    assert!(r.unanswered.is_some());
    assert!(r.derived_credentials().iter().any(|v| v == "pw-9137"));
    assert_eq!(r.redact("echo pw-9137"), "echo [redacted]");
}

#[tokio::test]
async fn an_answered_send_keeps_its_hops_on_the_response_only() {
    let server = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let r = sent(&format!("{}/ok", server.uri())).await;
    assert_eq!(
        r.response.as_ref().map(|resp| resp.timeline().len()),
        Some(1)
    );
    assert!(r.timeline.is_empty());
    assert!(r.unanswered.is_none());
}
