//! `resources/list` and `resources/read`: collection items as read-only JSON documents.

use crate::common;

use common::{ROOT, TestServer, request};
use serde_json::Value;

const DEV: &str =
    "name: dev\nvariables:\n  - name: token\n    secret: true\n  - name: plain\n    value: shown\n";

async fn shop() -> TestServer {
    TestServer::start(&[
        ("opencollection.yml", ROOT),
        (
            "ping.yml",
            &request("Ping", 1, "GET", "http://127.0.0.1:1/ping"),
        ),
        (
            "users/get.yml",
            &request("Get user", 1, "GET", "http://127.0.0.1:1/users"),
        ),
        ("users/folder.yml", "info:\n  name: Users\n"),
        ("empty/.keep", ""),
        ("environments/dev.yml", DEV),
    ])
    .await
}

fn uris(resources: &[rmcp::model::Resource]) -> Vec<String> {
    resources.iter().map(|r| r.uri.clone()).collect()
}

#[tokio::test]
async fn the_list_names_requests_folders_environments_and_the_root() {
    let server = shop().await;
    let uris = uris(&server.resources().await);
    for expected in [
        "apinata://shop/",
        "apinata://shop/ping.yml",
        "apinata://shop/users/",
        "apinata://shop/users/get.yml",
        "apinata://shop/empty/",
        "apinata://shop/@environment/dev",
    ] {
        assert!(uris.iter().any(|u| u == expected), "{expected} in {uris:?}");
    }
    assert_eq!(uris.len(), 6, "{uris:?}");
}

#[tokio::test]
async fn every_listed_resource_is_json_and_reads() {
    let server = shop().await;
    for resource in server.resources().await {
        assert_eq!(resource.mime_type.as_deref(), Some("application/json"));
        let text = server.resource_text(&resource.uri).await;
        serde_json::from_str::<Value>(&text).expect("json");
    }
}

#[tokio::test]
async fn a_request_reads_as_the_read_request_tool_returns_it() {
    let server = shop().await;
    let expected = server
        .ok("read_request", serde_json::json!({"path": "users/get.yml"}))
        .await;
    let text = server.resource_text("apinata://shop/users/get.yml").await;
    assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), expected);
}

#[tokio::test]
async fn folders_and_the_root_read_as_read_folder() {
    let server = shop().await;
    for (uri, path) in [
        ("apinata://shop/", ""),
        ("apinata://shop/users/", "users"),
        ("apinata://shop/empty/", "empty"),
    ] {
        let expected = server
            .ok("read_folder", serde_json::json!({"path": path}))
            .await;
        let text = server.resource_text(uri).await;
        assert_eq!(
            serde_json::from_str::<Value>(&text).unwrap(),
            expected,
            "{uri}"
        );
    }
}

#[tokio::test]
async fn an_environment_reads_as_read_environment_with_no_secret_value() {
    let server = shop().await;
    server.set_secret("dev", "token", "s3cr3t-token-value");
    let expected = server
        .ok("read_environment", serde_json::json!({"name": "dev"}))
        .await;
    let text = server
        .resource_text("apinata://shop/@environment/dev")
        .await;
    assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), expected);
    assert!(!text.contains("s3cr3t-token-value"));
}

#[tokio::test]
async fn an_unknown_path_is_an_error() {
    let server = shop().await;
    for uri in [
        "apinata://shop/nope.yml",
        "apinata://shop/nope/",
        "apinata://shop/@environment/nope",
        "apinata://shop/users",
        "apinata://shop/ping.yml/",
        "http://shop/ping.yml",
        "apinata://shop",
        "not a uri",
    ] {
        assert!(server.try_read_resource(uri).await.is_err(), "{uri}");
    }
}

#[tokio::test]
async fn a_name_with_spaces_round_trips_through_the_listing() {
    let server = TestServer::start(&[
        (
            "opencollection.yml",
            "opencollection: \"1.0.0\"\ninfo:\n  name: my shop\n",
        ),
        (
            "a b/c d.yml",
            &request("C", 1, "GET", "http://127.0.0.1:1/c"),
        ),
    ])
    .await;
    let listed = uris(&server.resources().await);
    let uri = listed
        .iter()
        .find(|u| u.ends_with("c%20d.yml"))
        .expect("listed");
    assert_eq!(uri, "apinata://my%20shop/a%20b/c%20d.yml");
    server.resource_text(uri).await;
}

#[tokio::test]
async fn a_mutating_tool_announces_a_changed_list() {
    let server = shop().await;
    assert_eq!(server.list_changed().await, 0);
    server
        .ok(
            "create_folder",
            serde_json::json!({"parent": "", "name": "fresh"}),
        )
        .await;
    assert_eq!(server.list_changed().await, 1);
    assert!(
        uris(&server.resources().await)
            .iter()
            .any(|u| u == "apinata://shop/fresh/")
    );
}

#[tokio::test]
async fn a_read_tool_announces_nothing() {
    let server = shop().await;
    server.ok("get_collection", serde_json::json!({})).await;
    server
        .ok("read_request", serde_json::json!({"path": "ping.yml"}))
        .await;
    server.resources().await;
    assert_eq!(server.list_changed().await, 0);
}
