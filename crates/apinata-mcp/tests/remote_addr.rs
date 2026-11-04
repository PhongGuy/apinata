//! `send_request` names the address each hop reached, nothing when unknown.

use crate::common;

use common::{ROOT, TestServer};
use serde_json::json;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn get(url: &str) -> String {
    format!("info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\n")
}

#[tokio::test]
async fn the_result_and_every_hop_carry_the_servers_address() {
    let mock = MockServer::start().await;
    Mock::given(path("/start"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/end"))
        .mount(&mock)
        .await;
    Mock::given(path("/end"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;
    let request = get(&format!("{}/start", mock.uri()));
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", &request)]).await;
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    let want = mock.address().to_string();
    assert_eq!(result["remote_addr"], want, "{result}");
    assert!(result.get("via_proxy").is_none(), "{result}");
    let timeline = result["timeline"].as_array().expect("timeline");
    assert_eq!(timeline.len(), 2);
    for hop in timeline {
        assert_eq!(hop["remote_addr"], want, "{hop}");
    }
}
