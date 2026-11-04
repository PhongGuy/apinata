//! `send_request` with an unknown environment names the ones that exist.

use crate::common;

use common::{ROOT, TestServer, request};
use serde_json::json;

const ENV: &str = "name: x\nvariables: []\n";

#[tokio::test]
async fn send_lists_the_environments_and_suggests_one() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", ENV),
        ("environments/staging.yml", ENV),
        ("r.yml", &request("r", 1, "GET", "http://127.0.0.1:1/")),
    ])
    .await;
    let message = server
        .err(
            "send_request",
            json!({"path": "r.yml", "environment": "stagin"}),
        )
        .await;
    assert_eq!(
        message,
        "environment \"stagin\" not found; available: dev, staging; did you mean staging?; nothing was sent"
    );
}

#[tokio::test]
async fn send_says_when_the_collection_has_no_environments() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request("r", 1, "GET", "http://127.0.0.1:1/")),
    ])
    .await;
    let message = server
        .err("send_request", json!({"path": "r.yml", "environment": "x"}))
        .await;
    assert_eq!(
        message,
        "environment \"x\" not found; this collection has no environments; nothing was sent"
    );
}

#[tokio::test]
async fn read_and_update_environment_name_the_available_ones() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", ENV),
        ("environments/prod.yml", ENV),
    ])
    .await;
    let want = "environment \"prd\" not found; available: dev, prod; did you mean prod?";
    let read = server.err("read_environment", json!({"name": "prd"})).await;
    assert!(read.contains(want), "{read}");
    let update = server
        .err(
            "update_environment",
            json!({"name": "prd", "set": [{"name": "a", "value": "b"}]}),
        )
        .await;
    assert!(update.contains(want), "{update}");
    let delete = server
        .err("delete_environment", json!({"name": "prd"}))
        .await;
    assert!(delete.contains(want), "{delete}");
}
