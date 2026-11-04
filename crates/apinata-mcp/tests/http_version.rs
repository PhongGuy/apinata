//! `send_request` reports each hop's negotiated version and follows the launch policy.

use crate::common;

use apinata_core::HttpVersionPolicy;
use common::{ROOT, TestServer};
use serde_json::json;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn get(url: &str) -> String {
    format!("info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\n")
}

/// `/start` redirects to `/end`, which answers 200.
async fn redirecting() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(path("/start"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/end"))
        .mount(&s)
        .await;
    Mock::given(path("/end"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    s
}

#[tokio::test]
async fn every_hop_reports_http1_by_default() {
    let mock = redirecting().await;
    let request = get(&format!("{}/start", mock.uri()));
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", &request)]).await;
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    let timeline = result["timeline"].as_array().expect("timeline");
    assert_eq!(timeline.len(), 2);
    for hop in timeline {
        assert_eq!(hop["version"], "HTTP/1.1", "{hop}");
    }
}

#[tokio::test]
async fn the_launch_policy_reaches_every_send() {
    let mock = redirecting().await;
    let request = get(&format!("{}/start", mock.uri()));
    let server =
        TestServer::start_with(&[("opencollection.yml", ROOT), ("r.yml", &request)], |o| {
            o.http_version(HttpVersionPolicy::Http2PriorKnowledge)
        })
        .await;
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    assert_eq!(result["status"], 200);
    let versions: Vec<&str> = result["timeline"]
        .as_array()
        .expect("timeline")
        .iter()
        .map(|h| h["version"].as_str().expect("a string"))
        .collect();
    assert_eq!(versions, ["HTTP/2", "HTTP/2"]);
}
