//! `retries`, `retry_delay_ms` and `retry_unsafe` on `run_folder`.

use crate::common;

use common::{ROOT, TestServer};
use serde_json::json;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn flaky(fail: u64) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(fail)
        .mount(&s)
        .await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    s
}

fn request(method: &str, url: &str) -> String {
    format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: {method}\n  url: \"{url}\"\nruntime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n"
    )
}

async fn start(method: &str, url: &str) -> TestServer {
    TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request(method, url)),
    ])
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_folder_retries_a_503() {
    let s = flaky(2).await;
    let server = start("GET", &s.uri()).await;
    let result = server
        .ok(
            "run_folder",
            json!({"path": "", "retries": 2, "retry_delay_ms": 10}),
        )
        .await;
    assert_eq!(result["results"][0]["outcome"], "passed", "{result}");
    assert_eq!(result["results"][0]["attempts"], 3, "{result}");
    assert_eq!(s.received_requests().await.expect("rec").len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_folder_sends_once_by_default() {
    let s = flaky(2).await;
    let server = start("GET", &s.uri()).await;
    let result = server.ok("run_folder", json!({"path": ""})).await;
    assert_eq!(result["results"][0]["outcome"], "failed", "{result}");
    assert_eq!(s.received_requests().await.expect("rec").len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_folder_retries_a_post_only_when_unsafe() {
    let s = flaky(2).await;
    let server = start("POST", &s.uri()).await;
    server
        .ok("run_folder", json!({"path": "", "retries": 2}))
        .await;
    assert_eq!(s.received_requests().await.expect("rec").len(), 1);

    let s = flaky(2).await;
    let server = start("POST", &s.uri()).await;
    let result = server
        .ok(
            "run_folder",
            json!({"path": "", "retries": 2, "retry_unsafe": true}),
        )
        .await;
    assert_eq!(result["results"][0]["attempts"], 3, "{result}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn too_many_retries_is_an_error_result() {
    let server = start("GET", "http://127.0.0.1:9/").await;
    let err = server
        .err("run_folder", json!({"path": "", "retries": 11}))
        .await;
    assert!(err.contains("retries"), "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_run_folder_description_names_last_attempt_error() {
    let server = start("GET", "http://127.0.0.1:9/").await;
    let tools = server.tools().await;
    let tool = tools
        .iter()
        .find(|t| t.name == "run_folder")
        .expect("listed");
    let description = tool.description.as_deref().unwrap_or_default();
    assert!(description.contains("last_attempt_error"), "{description}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_retried_run_that_ends_on_a_502_reports_attempts_and_no_last_attempt_error() {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(502))
        .mount(&s)
        .await;
    let server = start("GET", &s.uri()).await;
    let result = server
        .ok(
            "run_folder",
            json!({"path": "", "retries": 1, "retry_delay_ms": 10}),
        )
        .await;
    assert_eq!(result["results"][0]["attempts"], 2, "{result}");
    assert_eq!(result["results"][0]["status"], 502, "{result}");
    assert!(
        result["results"][0].get("last_attempt_error").is_none(),
        "{result}"
    );
}
