//! `list_cookies` and `clear_cookies`: the jar a send fills, listed without values.

use crate::common;

use common::{ROOT, TestServer};
use serde_json::json;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn get(url: &str) -> String {
    format!("info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\n")
}

async fn server_with_cookies() -> (MockServer, TestServer) {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .append_header("set-cookie", "sid=cookie-Value-1; Path=/; HttpOnly")
                .append_header("set-cookie", "theme=dark; Path=/; Max-Age=3600"),
        )
        .mount(&mock)
        .await;
    let server =
        TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", &get(&mock.uri()))]).await;
    server.ok("send_request", json!({"path": "r.yml"})).await;
    (mock, server)
}

#[tokio::test]
async fn lists_names_domains_and_flags_but_never_values() {
    let (_mock, server) = server_with_cookies().await;
    let listed = server.call("list_cookies", json!({})).await;
    let shown = serde_json::to_string(&listed).expect("serialises");
    assert!(!shown.contains("cookie-Value-1"), "{shown}");
    assert!(!shown.contains("dark"), "{shown}");
    let cookies = listed.structured_content.expect("structured")["cookies"].clone();
    let mut names: Vec<&str> = cookies
        .as_array()
        .expect("cookies")
        .iter()
        .map(|c| c["name"].as_str().expect("name"))
        .collect();
    names.sort();
    assert_eq!(names, ["sid", "theme"]);
    let sid = cookies
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "sid")
        .unwrap();
    assert_eq!(sid["domain"], "127.0.0.1");
    assert_eq!(sid["http_only"], true);
    assert_eq!(sid["expires"], serde_json::Value::Null);
    let theme = cookies
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "theme")
        .unwrap();
    assert!(theme["expires"].as_u64().is_some());
}

#[tokio::test]
async fn clears_one_domain_or_everything() {
    let (_mock, server) = server_with_cookies().await;
    let other = server
        .ok("clear_cookies", json!({"domain": "example.com"}))
        .await;
    assert_eq!(other, json!({"removed": 0}));
    let all = server.ok("clear_cookies", json!({})).await;
    assert_eq!(all, json!({"removed": 2}));
    assert_eq!(
        server.ok("list_cookies", json!({})).await,
        json!({"cookies": []})
    );
}

#[tokio::test]
async fn clearing_a_domain_removes_only_its_cookies() {
    let (_mock, server) = server_with_cookies().await;
    let removed = server
        .ok("clear_cookies", json!({"domain": "127.0.0.1"}))
        .await;
    assert_eq!(removed, json!({"removed": 2}));
}
