//! The global environment tools: the collection environment tools on the profile's global
//! environments, under the same secret rules.

use crate::common;

use common::{ROOT, TestServer};
use serde_json::json;
use std::fs;

const SECRET: &str = "sentinel-gl0bal-s3cret";

async fn server() -> TestServer {
    TestServer::start(&[("opencollection.yml", ROOT)]).await
}

fn read(server: &TestServer, name: &str) -> String {
    fs::read_to_string(server.global_dir().join(format!("{name}.yml"))).expect("read")
}

#[tokio::test]
async fn create_list_update_and_read_a_global_environment() {
    let server = server().await;
    let listed = server.ok("list_global_environments", json!({})).await;
    assert_eq!(listed, json!({"environments": []}));
    let made = server
        .ok("create_global_environment", json!({"name": "shared"}))
        .await;
    assert_eq!(made, json!({"name": "shared", "warnings": []}));
    let listed = server.ok("list_global_environments", json!({})).await;
    assert_eq!(listed, json!({"environments": ["shared"]}));

    let args = json!({"name": "shared", "set": [{"name": "host", "value": "a.test"}]});
    let doc = server.ok("update_global_environment", args).await;
    assert_eq!(doc["variables"][0]["value"], "a.test", "{doc}");
    assert!(read(&server, "shared").contains("a.test"));
    let doc = server
        .ok("read_global_environment", json!({"name": "shared"}))
        .await;
    assert_eq!(doc["name"], "shared");
    assert!(
        !server.root().join("environments").exists(),
        "no collection file"
    );
}

#[tokio::test]
async fn secrets_are_never_set_shown_or_removed_and_making_one_moves_the_value() {
    let server = server().await;
    server.write_global(
        "shared",
        "name: shared\nvariables:\n  - name: plain\n    value: v1\n  - secret: true\n    name: tok\n",
    );
    server.set_global_secret("shared", "tok", SECRET);
    let doc = server
        .ok("read_global_environment", json!({"name": "shared"}))
        .await;
    assert!(!doc.to_string().contains(SECRET), "{doc}");

    let args = json!({"name": "shared", "set": [{"name": "tok", "value": "x"}]});
    let err = server.err("update_global_environment", args).await;
    assert!(err.contains("tok"), "{err}");
    let err = server
        .err(
            "update_global_environment",
            json!({"name": "shared", "remove": ["tok"]}),
        )
        .await;
    assert!(err.contains("tok is secret"), "{err}");
    assert_eq!(
        server.global_secret("shared", "tok").as_deref(),
        Some(SECRET)
    );

    let args = json!({"name": "shared", "set": [{"name": "plain", "secret": true}]});
    server.ok("update_global_environment", args).await;
    assert_eq!(
        server.global_secret("shared", "plain").as_deref(),
        Some("v1")
    );
    assert!(!read(&server, "shared").contains("v1"));
}

#[tokio::test]
async fn duplicate_and_rename_carry_secrets_and_delete_refuses_one_holding_a_secret() {
    let server = server().await;
    server.write_global(
        "base",
        "name: base\nvariables:\n  - secret: true\n    name: tok\n",
    );
    server.write_global("dev", "name: dev\nextends: base\n");
    server.write_global("tmp", "name: tmp\n");
    server.set_global_secret("base", "tok", SECRET);

    let copied = server
        .ok("duplicate_global_environment", json!({"name": "base"}))
        .await;
    assert_eq!(copied, json!({"name": "base copy", "warnings": []}));
    assert_eq!(
        server.global_secret("base copy", "tok").as_deref(),
        Some(SECRET)
    );

    let renamed = server
        .ok(
            "rename_global_environment",
            json!({"from": "base", "to": "root"}),
        )
        .await;
    assert_eq!(renamed, json!({"name": "root", "warnings": []}));
    assert!(read(&server, "dev").contains("extends: root"));
    assert_eq!(server.global_secret("root", "tok").as_deref(), Some(SECRET));
    assert_eq!(server.global_secret("base", "tok"), None);

    let err = server
        .err("delete_global_environment", json!({"name": "base copy"}))
        .await;
    assert!(err.contains("secret") && !err.contains(SECRET), "{err}");
    assert!(server.global_dir().join("base copy.yml").is_file());
    let deleted = server
        .ok("delete_global_environment", json!({"name": "tmp"}))
        .await;
    assert_eq!(deleted, json!({"deleted": "tmp"}));
    assert!(!server.global_dir().join("tmp.yml").exists());
    let err = server
        .err("delete_global_environment", json!({"name": "nope"}))
        .await;
    assert!(
        err.contains("global environment") && err.contains("nope"),
        "{err}"
    );
}

#[tokio::test]
async fn the_global_tools_are_annotated_like_the_collection_ones() {
    let server = server().await;
    let tools = server.tools().await;
    for (name, read_only, destructive) in [
        ("list_global_environments", true, false),
        ("read_global_environment", true, false),
        ("create_global_environment", false, false),
        ("duplicate_global_environment", false, false),
        ("rename_global_environment", false, false),
        ("delete_global_environment", false, true),
        ("update_global_environment", false, false),
    ] {
        let tool = tools
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("{name}"));
        let hints = tool.annotations.clone().expect("annotated");
        assert_eq!(hints.read_only_hint.unwrap_or(false), read_only, "{name}");
        if !read_only {
            assert_eq!(hints.destructive_hint, Some(destructive), "{name}");
        }
        assert_eq!(hints.open_world_hint, Some(false), "{name}");
    }
}
