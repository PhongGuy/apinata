//! `apinata mcp`: the real binary over stdio, driven by an rmcp client as Claude Code would.

use crate::common;

use std::path::Path;
use std::time::Duration;

use common::{ROOT, apinata, get, write_tree};
use predicates::prelude::PredicateBooleanExt as _;
use predicates::str::contains;
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::TokioChildProcess;
use serde_json::json;

/// Spawn `apinata mcp <args>` in `cwd`, call `list_items` once, and return its structured content.
/// The whole exchange is bounded, and the child is killed when dropped, so a stuck server fails
/// the test instead of hanging it.
async fn list_items_via_stdio(cwd: &Path, args: &[&str]) -> serde_json::Value {
    let mut cmd = crate::common::apinata_tokio();
    cmd.arg("mcp")
        .args(args)
        .current_dir(cwd)
        .kill_on_drop(true);
    let exchange = async {
        let client =
            ().serve(TokioChildProcess::new(cmd).expect("spawns"))
                .await
                .expect("handshake");
        let result = client
            .call_tool(CallToolRequestParams::new("list_items"))
            .await
            .expect("tools/call");
        client.cancel().await.expect("closes");
        result
    };
    let result = tokio::time::timeout(Duration::from_secs(30), exchange)
        .await
        .expect("the stdio exchange finishes within 30 s");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    result.structured_content.expect("structured content")
}

#[tokio::test]
async fn serves_the_collection_named_by_collection() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "users/get.yml",
            &get("Get user", 1, "http://localhost/users/1", ""),
        ),
    ]);
    let elsewhere = tempfile::TempDir::new().expect("tempdir");
    let path = dir.path().to_str().expect("utf-8");
    let list = list_items_via_stdio(elsewhere.path(), &["--collection", path]).await;
    assert_eq!(list["items"][0]["path"], "users");
    assert_eq!(list["items"][0]["items"][0]["path"], "users/get.yml");
    assert_eq!(list["items"][0]["items"][0]["method"], "GET");
}

#[tokio::test]
async fn finds_the_collection_above_the_working_directory() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "users/get.yml",
            &get("Get user", 1, "http://localhost/users/1", ""),
        ),
    ]);
    let list = list_items_via_stdio(&dir.path().join("users"), &[]).await;
    assert_eq!(
        list["items"],
        json!([{
            "path": "users", "kind": "folder", "name": "users",
            "items": [{"path": "users/get.yml", "kind": "request", "name": "Get user", "seq": 1,
                       "method": "GET", "url": "http://localhost/users/1"}]
        }])
    );
}

#[test]
fn no_collection_exits_3_with_a_message_on_stderr() {
    let empty = tempfile::TempDir::new().expect("tempdir");
    apinata()
        .args(["mcp", "--collection"])
        .arg(empty.path())
        .timeout(Duration::from_secs(30))
        .assert()
        .code(3)
        .stdout("")
        .stderr(contains("no opencollection.yml"));
    apinata()
        .arg("mcp")
        .current_dir(empty.path())
        .timeout(Duration::from_secs(30))
        .assert()
        .code(3)
        .stderr(contains("no opencollection.yml"));
}

#[test]
fn mcp_help_lists_its_options() {
    apinata()
        .args(["mcp", "--help"])
        .assert()
        .code(0)
        .stdout(contains("--collection"))
        .stdout(contains("--env-var"))
        .stdout(contains("tools, resources and prompts"));
}

#[test]
fn an_unknown_global_environment_stops_the_server_at_launch() {
    let profile = tempfile::TempDir::new().expect("tempdir");
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    apinata()
        .env("APINATA_PROFILE_DIR", profile.path())
        .args(["mcp", "--collection"])
        .arg(dir.path())
        .args(["--global-env", "nope"])
        .timeout(Duration::from_secs(30))
        .assert()
        .code(3)
        .stdout("")
        .stderr(contains(
            "global environment \"nope\" not found; no global environments",
        ));
}

#[tokio::test]
async fn mcp_takes_proxy_pac_relative_to_its_working_directory() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "users/get.yml",
            &get("Get user", 1, "http://localhost/users/1", ""),
        ),
    ]);
    let list = list_items_via_stdio(dir.path(), &["--proxy-pac", "missing.pac"]).await;
    assert_eq!(list["items"][0]["path"], "users");
}

#[test]
fn a_bad_proxy_pac_stops_the_server_at_launch() {
    apinata()
        .args(["mcp", "--help"])
        .assert()
        .code(0)
        .stdout(contains("--proxy-pac"));
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    apinata()
        .args(["mcp", "--collection"])
        .arg(dir.path())
        .args(["--proxy-pac", "ftp://u:pw-SEKRIT@127.0.0.1/p.pac"])
        .timeout(Duration::from_secs(30))
        .assert()
        .code(3)
        .stdout("")
        .stderr(contains("not ftp"))
        .stderr(contains("pw-SEKRIT").not());
}
