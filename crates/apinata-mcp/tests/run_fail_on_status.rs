//! `fail_on_status` on `run_folder`.

use crate::common;

use common::{ROOT, TestServer};
use serde_json::json;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn replying(status: u16) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(status))
        .mount(&s)
        .await;
    s
}

async fn start(url: &str) -> TestServer {
    let req = format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n"
    );
    TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", &req)]).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_404_fails_the_request_when_asked() {
    let s = replying(404).await;
    let server = start(&s.uri()).await;
    let result = server
        .ok("run_folder", json!({"path": "", "fail_on_status": "error"}))
        .await;
    assert_eq!(result["results"][0]["outcome"], "failed", "{result}");
    assert_eq!(
        result["results"][0]["status_failure"], "status 404 is in the fail-on-status range (error)",
        "{result}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn it_is_off_by_default_and_the_class_is_honoured() {
    let s = replying(500).await;
    let server = start(&s.uri()).await;
    let off = server.ok("run_folder", json!({"path": ""})).await;
    assert_eq!(off["results"][0]["outcome"], "passed", "{off}");
    let client = server
        .ok("run_folder", json!({"path": "", "fail_on_status": "4xx"}))
        .await;
    assert_eq!(client["results"][0]["outcome"], "passed", "{client}");
    let srv = server
        .ok("run_folder", json!({"path": "", "fail_on_status": "5xx"}))
        .await;
    assert_eq!(srv["results"][0]["outcome"], "failed", "{srv}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_value_is_refused() {
    let s = replying(200).await;
    let server = start(&s.uri()).await;
    let text = server
        .err("run_folder", json!({"path": "", "fail_on_status": "404"}))
        .await;
    assert!(text.contains("fail_on_status"), "{text}");
}
