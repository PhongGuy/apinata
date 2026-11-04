//! Host overrides under the MCP host policy: the mapped address is what gets approved, a local
//! or trusted name is no exemption, and no test connects beyond 127.0.0.1 (the check refuses
//! before any dial).

use crate::common;

use common::{ROOT, TestServer, request};
use serde_json::json;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn root_with(rows: &[(&str, &str)]) -> String {
    let mut out = format!("{ROOT}extensions:\n  apinata:\n    hostOverrides:\n");
    for (host, address) in rows {
        out.push_str(&format!(
            "      - host: \"{host}\"\n        address: \"{address}\"\n"
        ));
    }
    out
}

async fn start(rows: &[(&str, &str)], url: &str) -> TestServer {
    TestServer::start(&[
        ("opencollection.yml", &root_with(rows)),
        ("r.yml", &request("r", 1, "GET", url)),
    ])
    .await
}

#[tokio::test]
async fn localhost_mapped_off_loopback_needs_approval_and_shows_the_address() {
    let server = start(&[("localhost", "203.0.113.5")], "http://localhost:9/x").await;
    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(message.contains("`203.0.113.5`"), "{message}");
    assert!(message.contains("`localhost`"), "{message}");
    assert!(
        message.contains("retry after the user approves"),
        "{message}"
    );
    let pending = server.inbox().pending().expect("inbox");
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0].host, "203.0.113.5");
    assert_eq!(pending[0].mapped_from.as_deref(), Some("localhost"));
}

#[tokio::test]
async fn an_ipv4_mapped_address_is_shown_dotted_and_filed_under_its_normalised_form() {
    let server = start(&[("api.test", "::ffff:203.0.113.5")], "http://api.test:9/x").await;
    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(message.contains("`[::ffff:203.0.113.5]`"), "{message}");
    assert!(message.contains("`api.test`"), "{message}");
    let pending = server.inbox().pending().expect("inbox");
    assert_eq!(pending[0].host, "[::ffff:cb00:7105]");
    assert_eq!(pending[0].mapped_from.as_deref(), Some("api.test"));
}

#[tokio::test]
async fn a_trusted_name_mapped_off_loopback_still_needs_approval() {
    let server = start(&[("api.test", "203.0.113.5")], "http://api.test:9/x").await;
    server.trust("api.test");
    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(message.contains("`203.0.113.5`"), "{message}");
    assert!(message.contains("api.test"), "{message}");
    assert_eq!(server.inbox().pending().expect("inbox").len(), 1);
}

#[tokio::test]
async fn a_redirect_to_an_overridden_name_is_judged_by_its_mapped_address() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "http://fake.test/next"))
        .mount(&mock)
        .await;
    let server = start(
        &[("fake.test", "203.0.113.5")],
        &format!("{}/start", mock.uri()),
    )
    .await;
    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(message.contains("`203.0.113.5`"), "{message}");
    assert!(message.contains("(hop 1"), "{message}");
    assert_eq!(server.inbox().pending().expect("inbox").len(), 1);
}

#[tokio::test]
async fn loopback_mappings_send_without_approval_and_unspecified_ones_are_refused() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("hit"))
        .mount(&mock)
        .await;
    let port = common::port_of(&mock.uri());
    let server = start(
        &[("fake.test", "::ffff:127.0.0.1"), ("v4.test", "127.0.0.1")],
        &format!("http://v4.test:{port}/x"),
    )
    .await;
    server.ok("send_request", json!({"path": "r.yml"})).await;
    assert!(server.inbox().pending().expect("inbox").is_empty());

    let server = start(&[("fake.test", "0.0.0.0")], "http://fake.test:9/x").await;
    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(message.contains("0.0.0.0"), "{message}");
    assert!(server.inbox().pending().expect("inbox").is_empty());
}

#[tokio::test]
async fn the_timeline_names_the_override_a_hop_used() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;
    let port = common::port_of(&mock.uri());
    let server = start(
        &[("fake.test", "127.0.0.1")],
        &format!("http://fake.test:{port}/x"),
    )
    .await;
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    let hop = &result["timeline"][0];
    assert_eq!(hop["host_override"], "fake.test -> 127.0.0.1", "{result}");
    assert_eq!(hop["remote_addr"], mock.address().to_string(), "{result}");
}

#[tokio::test]
async fn a_mapped_name_the_redactor_changes_is_not_filed_as_mapped_from() {
    let server = TestServer::start_with(
        &[
            (
                "opencollection.yml",
                &root_with(&[("leakyname9.test", "203.0.113.5")]),
            ),
            (
                "r.yml",
                &request("r", 1, "GET", "http://leakyname9.test:9/x"),
            ),
        ],
        |o| o.env_var("key", "LeakyName9"),
    )
    .await;
    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(message.contains("`203.0.113.5`"), "{message}");
    assert!(!message.to_lowercase().contains("leakyname9"), "{message}");
    let pending = server.inbox().pending().expect("inbox");
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0].host, "203.0.113.5");
    assert_eq!(pending[0].mapped_from, None);
    for entry in std::fs::read_dir(server.inbox().dir()).unwrap() {
        let raw = std::fs::read_to_string(entry.unwrap().path()).unwrap();
        assert!(!raw.to_lowercase().contains("leakyname9"), "{raw}");
    }
}
