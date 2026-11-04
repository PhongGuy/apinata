//! Retrying a request whose send failed in transit or got a 502, 503 or 504.

use crate::script_support::{ROOT, console, on, run_dir, script, write_tree};

use apinata_core::{Outcome, RunOptions};
use std::time::{Duration, Instant};
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn request(method: &str, url: &str, scripts: &str) -> String {
    let scripts = if scripts.is_empty() {
        String::new()
    } else {
        format!("  scripts:\n{scripts}")
    };
    format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: {method}\n  url: \"{url}\"\nruntime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n{scripts}"
    )
}

/// A server answering `fail` first with `status`, then 200.
async fn flaky(status: u16, fail: u64) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(status))
        .up_to_n_times(fail)
        .mount(&s)
        .await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    s
}

async fn hits(s: &MockServer) -> usize {
    s.received_requests().await.expect("recorded").len()
}

async fn one(
    method: &str,
    url: &str,
    scripts: &str,
    opts: RunOptions,
) -> apinata_core::RequestResult {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request(method, url, scripts)),
    ]);
    let report = run_dir(dir.path(), opts).await;
    assert_eq!(report.results.len(), 1);
    (*report.results[0]).clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_503s_then_200_passes_on_the_third_attempt() {
    let s = flaky(503, 2).await;
    let r = one("GET", &s.uri(), "", on().retries(2)).await;
    assert_eq!(r.outcome, Outcome::Passed, "{r:?}");
    assert_eq!(r.attempts(), 3);
    assert_eq!(hits(&s).await, 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn running_out_of_retries_keeps_the_last_response() {
    let s = flaky(503, 5).await;
    let r = one("GET", &s.uri(), "", on().retries(1)).await;
    assert_eq!(r.outcome, Outcome::Failed, "{r:?}");
    assert_eq!(r.attempts(), 2);
    assert_eq!(r.response.as_ref().map(|x| x.status()), Some(503));
    assert_eq!(hits(&s).await, 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_default_never_retries() {
    let s = flaky(503, 2).await;
    let r = one("GET", &s.uri(), "", on()).await;
    assert_eq!(r.attempts(), 1);
    assert_eq!(hits(&s).await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_500_is_not_retried() {
    let s = flaky(500, 2).await;
    let r = one("GET", &s.uri(), "", on().retries(3)).await;
    assert_eq!(r.attempts(), 1);
    assert_eq!(hits(&s).await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_assertion_runs_once() {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(201))
        .mount(&s)
        .await;
    let r = one("GET", &s.uri(), "", on().retries(3)).await;
    assert_eq!(r.outcome, Outcome::Failed);
    assert_eq!(r.attempts(), 1);
    assert_eq!(hits(&s).await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_is_retried_only_when_unsafe_retries_are_on() {
    let s = flaky(503, 2).await;
    let r = one("POST", &s.uri(), "", on().retries(2)).await;
    assert_eq!(r.attempts(), 1);
    assert_eq!(hits(&s).await, 1);

    let s = flaky(503, 2).await;
    let r = one("POST", &s.uri(), "", on().retries(2).retry_unsafe(true)).await;
    assert_eq!(r.outcome, Outcome::Passed, "{r:?}");
    assert_eq!(r.attempts(), 3);
    assert_eq!(hits(&s).await, 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pre_request_script_runs_once_across_retries() {
    let s = flaky(503, 2).await;
    let scripts = script("before-request", "console.log('pre');");
    let r = one("GET", &s.uri(), &scripts, on().retries(2)).await;
    assert_eq!(r.attempts(), 3);
    assert_eq!(console(&r), ["log:pre"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_connection_is_retried_then_reported() {
    // Bind and drop a listener so the port refuses connections.
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        l.local_addr().expect("addr").port()
    };
    let url = format!("http://127.0.0.1:{port}");
    let r = one("GET", &url, "", on().retries(2)).await;
    assert_eq!(r.outcome, Outcome::Errored, "{r:?}");
    assert_eq!(r.attempts(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retry_after_stretches_the_pause() {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(503).insert_header("Retry-After", "1"))
        .up_to_n_times(1)
        .mount(&s)
        .await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let started = Instant::now();
    let r = one("GET", &s.uri(), "", on().retries(1)).await;
    assert_eq!(r.attempts(), 2);
    assert!(started.elapsed() >= Duration::from_millis(950));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_retry_delay_waits_between_attempts() {
    let s = flaky(503, 2).await;
    let started = Instant::now();
    let r = one(
        "GET",
        &s.uri(),
        "",
        on().retries(2).retry_delay(Duration::from_millis(300)),
    )
    .await;
    assert_eq!(r.attempts(), 3);
    assert!(started.elapsed() >= Duration::from_millis(580));
}
