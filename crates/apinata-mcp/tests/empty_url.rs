//! `send_request` names an empty URL instead of quoting a scheme nobody wrote.

use crate::common;

use common::{ROOT, TestServer, request};
use serde_json::json;

#[tokio::test]
async fn a_blank_url_is_reported_as_empty() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request("r", 1, "GET", "  ")),
    ])
    .await;
    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    assert_eq!(message, "the URL is empty; nothing was sent");
}
