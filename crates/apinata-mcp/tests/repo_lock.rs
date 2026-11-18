//! While another process holds the repository lock (the app mid-pull or mid-switch), MCP's
//! writes report busy and change nothing, and its reads still answer.

use crate::common;
use crate::git_support;

use std::fs;
use std::time::Duration;

use apinata_core::repo_lock;
use common::{ROOT, TestServer, request};
use git_support::Sandbox;
use serde_json::json;

async fn committed(sb: &Sandbox) -> TestServer {
    let get = request("get", 1, "GET", "https://example.com/v1");
    let server = TestServer::start_with(
        &[("opencollection.yml", ROOT), ("get.yml", get.as_str())],
        |options| {
            let git = sb.options().env(
                "GIT_CEILING_DIRECTORIES",
                options.root().parent().expect("parent"),
            );
            options.git(git)
        },
    )
    .await;
    sb.git(server.root(), &["init", "-q", "-b", "main"]);
    sb.commit_all(server.root(), "base");
    server
}

fn assert_busy(err: &str) {
    assert!(
        err.contains("a pull or branch switch is running in the app") && err.contains("try again"),
        "{err}"
    );
}

#[tokio::test]
async fn a_write_under_a_held_lock_is_busy_and_changes_nothing() {
    let sb = Sandbox::new();
    let server = committed(&sb).await;
    let before = fs::read_to_string(server.root().join("get.yml")).unwrap();
    let held = repo_lock(server.root(), Duration::ZERO, &sb.options()).unwrap();
    assert!(held.is_held());

    let err = server
        .err(
            "update_request",
            json!({"path": "get.yml", "method": "POST"}),
        )
        .await;
    assert_busy(&err);
    assert_eq!(
        fs::read_to_string(server.root().join("get.yml")).unwrap(),
        before
    );
    assert_busy(
        &server
            .err("create_folder", json!({"parent": "", "name": "made"}))
            .await,
    );
    assert!(!server.root().join("made").exists());

    drop(held);
    server
        .ok(
            "update_request",
            json!({"path": "get.yml", "method": "POST"}),
        )
        .await;
}

#[tokio::test]
async fn reads_answer_under_a_held_lock() {
    let sb = Sandbox::new();
    let server = committed(&sb).await;
    let _held = repo_lock(server.root(), Duration::ZERO, &sb.options()).unwrap();

    let doc = server.ok("read_request", json!({"path": "get.yml"})).await;
    assert_eq!(doc["method"], "GET");
    server.ok("list_items", json!({})).await;
    assert_eq!(server.ok("git_status", json!({})).await["state"], "repo");
    server.ok("git_branches", json!({})).await;
}
