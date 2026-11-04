//! `list_trusted_hosts` and the scrubbing every tool result passes through.

use crate::common;

use common::{ROOT, TestServer};
use serde_json::json;

fn get(url: &str) -> String {
    format!("info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\n")
}

async fn start() -> TestServer {
    TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &get("http://127.0.0.1:9/")),
    ])
    .await
}

#[tokio::test]
async fn list_trusted_hosts_reads_the_file_fresh_on_every_call() {
    let server = start().await;
    let empty = server.ok("list_trusted_hosts", json!({})).await;
    assert_eq!(empty, json!({"trusted": [], "denied": []}));
    server.trust("api.example.com");
    server.deny("evil.example");
    let lists = server.ok("list_trusted_hosts", json!({})).await;
    assert_eq!(
        lists,
        json!({"trusted": ["api.example.com"], "denied": ["evil.example"]})
    );
}

#[tokio::test]
async fn a_malformed_trust_file_lists_nothing() {
    let server = start().await;
    std::fs::write(server.trust_file(), "{ not json").expect("write");
    let lists = server.ok("list_trusted_hosts", json!({})).await;
    assert_eq!(lists, json!({"trusted": [], "denied": []}));
}

#[tokio::test]
async fn every_result_is_scrubbed_of_values_seen_this_session() {
    // A value the human supplied with --env-var is scrubbed even from a plain read.
    let server = TestServer::start_with(
        &[
            ("opencollection.yml", ROOT),
            ("r.yml", &get("http://127.0.0.1:9/cli-Secret-5")),
        ],
        |o| o.env_var("key", "cli-Secret-5"),
    )
    .await;
    let result = server.call("read_request", json!({"path": "r.yml"})).await;
    let shown = serde_json::to_string(&result).expect("serialises");
    assert!(!shown.contains("cli-Secret-5"), "{shown}");
    assert!(shown.contains("/<secret>"), "{shown}");
}

#[tokio::test]
async fn errors_quoting_an_argument_are_scrubbed() {
    let server = TestServer::start_with(
        &[
            ("opencollection.yml", ROOT),
            ("r.yml", &get("http://127.0.0.1:9/")),
        ],
        |o| o.env_var("key", "cli-Secret-5"),
    )
    .await;
    // serde names an unknown field; rmcp 3.5.1 returns it as an isError tool result.
    let message = server
        .err(
            "update_request",
            json!({"path": "r.yml", "cli-Secret-5": 1}),
        )
        .await;
    assert!(message.contains("unknown field `<secret>`"), "{message}");
}
