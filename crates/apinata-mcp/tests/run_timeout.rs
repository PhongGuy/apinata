//! `timeout_secs` on `send_request` and `run_folder`: a run-wide cap on request timeouts.

use crate::common;

use std::time::{Duration, Instant};

use common::{ROOT, TestServer};
use serde_json::json;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn slow() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(3)))
        .mount(&s)
        .await;
    s
}

fn request(url: &str, settings: &str) -> String {
    format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n{settings}"
    )
}

async fn start(url: &str, settings: &str) -> TestServer {
    TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request(url, settings)),
    ])
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_request_cap_times_out_and_names_itself() {
    let s = slow().await;
    let server = start(&s.uri(), "").await;
    let started = Instant::now();
    let err = server
        .err("send_request", json!({"path": "r.yml", "timeout_secs": 1}))
        .await;
    assert!(started.elapsed() < Duration::from_millis(2800));
    assert!(err.contains("timed out (run-wide timeout of 1 s)"), "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_folder_cap_times_out_and_names_itself() {
    let s = slow().await;
    let server = start(&s.uri(), "").await;
    let started = Instant::now();
    let result = server
        .ok("run_folder", json!({"path": "", "timeout_secs": 1}))
        .await;
    assert!(started.elapsed() < Duration::from_millis(2800));
    let text = result.to_string();
    assert!(
        text.contains("timed out (run-wide timeout of 1 s)"),
        "{text}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_shorter_own_timeout_keeps_the_plain_message() {
    let s = slow().await;
    let server = start(&s.uri(), "settings:\n  timeout: 300\n").await;
    let err = server
        .err("send_request", json!({"path": "r.yml", "timeout_secs": 2}))
        .await;
    assert!(err.contains("timed out"), "{err}");
    assert!(!err.contains("run-wide"), "{err}");
    let result = server
        .ok("run_folder", json!({"path": "", "timeout_secs": 2}))
        .await;
    let text = result.to_string();
    assert!(text.contains("timed out"), "{text}");
    assert!(!text.contains("run-wide"), "{text}");
}

#[tokio::test]
async fn an_out_of_range_cap_is_an_error_result() {
    let server = start("http://127.0.0.1:9/", "").await;
    for tool in ["send_request", "run_folder"] {
        let path = if tool == "run_folder" { "" } else { "r.yml" };
        for bad in [0, 86_401] {
            let err = server
                .err(tool, json!({"path": path, "timeout_secs": bad}))
                .await;
            assert!(err.contains("timeout_secs"), "{err}");
            assert!(err.contains("86400"), "{err}");
        }
    }
}
