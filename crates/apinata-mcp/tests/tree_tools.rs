//! Tree tools: Bruno's bytes for created, cloned and renamed items (the tree parity corpus), the
//! paths they return, and the refusals.

use crate::common;

use common::{FakeTrash, ROOT, TestServer, files_under, request, research};
use serde_json::json;
use std::fs;
use std::sync::Arc;

/// The bytes a new request is written with: Bruno's, but with an inherited timeout.
fn new_request_bytes(id: &str) -> String {
    let path = research("2026-10-17-new-request-timeout-inherit").join(format!("{id}.yml"));
    fs::read_to_string(path).expect("new request case")
}

fn tree_parity(id: &str) -> String {
    let path = research("2026-10-09-tree-parity/expected").join(format!("{id}.yml"));
    fs::read_to_string(path).expect("tree parity case")
}

fn normalised(fixture: &str) -> String {
    let path = research("2026-10-09-edit-parity/normalized").join(fixture);
    fs::read_to_string(path).expect("normalised fixture")
}

fn folder(name: &str, seq: u64) -> String {
    format!("info:\n  name: {name}\n  type: folder\n  seq: {seq}\n")
}

fn item(kind: &str, name: &str, seq: u64) -> String {
    format!(
        "info:\n  name: {name}\n  type: {kind}\n  seq: {seq}\n{kind}:\n  url: https://x.test/{name}\n"
    )
}

fn read(server: &TestServer, rel: &str) -> String {
    fs::read_to_string(server.root().join(rel)).expect("read")
}

#[tokio::test]
async fn created_items_have_brunos_bytes() {
    // The sibling sets of core's tree parity tests, so the seq Bruno chose is reproduced.
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1, "GET", "https://x.test/a")),
        ("f/folder.yml", &folder("f", 2)),
        ("ui.yml", &item("app", "ui", 1)),
        ("x.yml", &item("custom", "x", 4)),
    ])
    .await;
    let made = server
        .ok("create_request", json!({"parent": "", "name": "Get users"}))
        .await;
    assert_eq!(made, json!({"path": "Get users.yml"}));
    assert_eq!(
        read(&server, "Get users.yml"),
        new_request_bytes("new-request")
    );

    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1, "GET", "https://x.test/a")),
        ("b.yml", &request("b", 2, "GET", "https://x.test/b")),
        ("f/folder.yml", &folder("f", 3)),
    ])
    .await;
    let made = server
        .ok("create_folder", json!({"parent": "", "name": "Admin"}))
        .await;
    assert_eq!(made, json!({"path": "Admin"}));
    assert_eq!(read(&server, "Admin/folder.yml"), tree_parity("new-folder"));
}

#[tokio::test]
async fn create_request_applies_method_and_url_through_the_setters() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("f", 1)),
    ])
    .await;
    let made = server
        .ok(
            "create_request",
            json!({"parent": "f", "name": "Find", "method": "POST",
            "url": "http://localhost/find?q=1"}),
        )
        .await;
    assert_eq!(made["path"], "f/Find.yml");
    let doc = server
        .ok("read_request", json!({"path": "f/Find.yml"}))
        .await;
    assert_eq!(doc["method"], "POST");
    assert_eq!(doc["url"], "http://localhost/find?q=1");
    assert_eq!(
        doc["params"][0]["name"], "q",
        "the URL's query became a param"
    );
}

#[tokio::test]
async fn clone_and_rename_have_brunos_bytes() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("request_1.yml", &normalised("cli-docs/request_1.yml")),
        ("b.yml", &request("b", 2, "GET", "https://x.test/b")),
        ("ui.yml", &item("app", "ui", 1)),
        ("f/folder.yml", &folder("f", 3)),
    ])
    .await;
    let copy = server
        .ok(
            "clone_request",
            json!({"path": "request_1.yml", "name": "request_1 copy"}),
        )
        .await;
    assert_eq!(copy, json!({"path": "request_1 copy.yml"}));
    assert_eq!(
        read(&server, "request_1 copy.yml"),
        tree_parity("clone-request")
    );

    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        (
            "folder_1/folder.yml",
            &normalised("cli-docs/folder_1/folder.yml"),
        ),
        (
            "folder_1/x.yml",
            &request("x", 1, "GET", "https://x.test/x"),
        ),
    ])
    .await;
    let renamed = server
        .ok(
            "rename_item",
            json!({"path": "folder_1", "name": "Renamed"}),
        )
        .await;
    assert_eq!(renamed, json!({"path": "Renamed"}));
    assert_eq!(
        read(&server, "Renamed/folder.yml"),
        tree_parity("folder-rename")
    );
    assert!(server.root().join("Renamed/x.yml").is_file());
}

#[tokio::test]
async fn move_and_delete_return_the_new_state() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1, "GET", "https://x.test/a")),
        ("b.yml", &request("b", 2, "GET", "https://x.test/b")),
        ("f/folder.yml", &folder("f", 3)),
        ("q.yml", &item("grpc", "q", 4)),
    ])
    .await;
    let moved = server
        .ok("move_item", json!({"path": "a.yml", "inside": "f"}))
        .await;
    assert_eq!(moved, json!({"path": "f/a.yml"}));
    let moved = server
        .ok("move_item", json!({"path": "q.yml", "before": "b.yml"}))
        .await;
    assert_eq!(moved, json!({"path": "q.yml"}));
    let listed = server.ok("list_items", json!({})).await;
    let seq = |path: &str| {
        let items = listed["items"].as_array().expect("items");
        items.iter().find(|i| i["path"] == path).expect("listed")["seq"].clone()
    };
    assert_eq!(
        (seq("q.yml"), seq("b.yml")),
        (json!(1), json!(2)),
        "{listed}"
    );
    assert_eq!(
        server.ok("delete_item", json!({"path": "f"})).await,
        json!({"deleted": "f", "destination": "trash"})
    );
    assert!(!server.root().join("f").exists());
}

#[tokio::test]
async fn delete_item_refuses_unknown_fields_and_deletes_nothing() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("a/folder.yml", &folder("a", 1)),
        ("a/x.yml", &request("x", 1, "GET", "https://x.test/x")),
    ])
    .await;
    // A misspelt or invented safety flag must not be silently dropped on a destructive call.
    let outcome = server
        .try_call("delete_item", json!({"path": "a", "recursive": false}))
        .await;
    assert!(
        outcome.as_ref().map_or(true, |r| r.is_error == Some(true)),
        "{outcome:?}"
    );
    assert!(server.root().join("a/x.yml").is_file());
}

#[tokio::test]
async fn tree_tools_refuse_bad_targets_by_name() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1, "GET", "https://x.test/a")),
        ("q.yml", &item("grpc", "q", 2)),
    ])
    .await;
    let err = server
        .err(
            "move_item",
            json!({"path": "a.yml", "inside": "", "after": "q.yml"}),
        )
        .await;
    assert!(err.contains("exactly one"), "{err}");
    let err = server
        .err("rename_item", json!({"path": "q.yml", "name": "Q2"}))
        .await;
    assert!(err.contains("q.yml") && err.contains("grpc"), "{err}");
    assert!(
        !server
            .err("delete_item", json!({"path": ""}))
            .await
            .is_empty()
    );
    let err = server
        .err("create_request", json!({"parent": "nope", "name": "x"}))
        .await;
    assert!(err.contains("nope"), "{err}");
    assert!(server.root().join("q.yml").is_file() && server.root().join("a.yml").is_file());
}

#[tokio::test]
async fn only_delete_item_is_destructive() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let tools = server.tools().await;
    let tree = [
        "create_request",
        "create_folder",
        "clone_request",
        "clone_folder",
        "rename_item",
        "move_item",
        "delete_item",
    ];
    for name in tree {
        let tool = tools
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("{name}"));
        let hints = tool.annotations.clone().expect("annotated");
        assert_eq!(
            hints.destructive_hint,
            Some(name == "delete_item"),
            "{name}"
        );
        assert_eq!(hints.open_world_hint, Some(false), "{name}");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn delete_item_succeeds_with_a_warning_when_siblings_cannot_be_renumbered() {
    use std::os::unix::fs::PermissionsExt;
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1, "GET", "https://x.test/a")),
        ("f/folder.yml", &folder("f", 2)),
    ])
    .await;
    let locked = server.root().join("f");
    let _g = crate::common::ModeGuard::set(&locked, 0o555);
    // Permissions are not enforced for root, so the failure cannot be staged there.
    let enforced = fs::write(locked.join("probe"), "").is_err();
    let out = server.ok("delete_item", json!({"path": "a.yml"})).await;
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("chmod");
    if !enforced {
        return;
    }
    assert!(!server.root().join("a.yml").exists());
    assert_eq!(out["deleted"], json!("a.yml"));
    let warnings = out["warnings"].as_array().expect("warnings");
    assert_eq!(warnings.len(), 1, "{out}");
    let text = warnings[0].as_str().expect("text");
    assert!(
        text.starts_with("deleted, but could not renumber the siblings: "),
        "{text}"
    );
    assert!(text.contains("f/folder.yml"), "{text}");
    assert!(
        !text.contains(&server.root().display().to_string()),
        "{text}"
    );
}

#[tokio::test]
async fn create_request_uppercases_a_custom_method_and_refuses_a_bad_one_before_creating() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("f", 1)),
    ])
    .await;
    server
        .ok(
            "create_request",
            json!({"parent": "f", "name": "Find", "method": "m-search"}),
        )
        .await;
    let doc = server
        .ok("read_request", json!({"path": "f/Find.yml"}))
        .await;
    assert_eq!(doc["method"], "M-SEARCH");
    let err = server
        .err(
            "create_request",
            json!({"parent": "f", "name": "Bad", "method": "GE T"}),
        )
        .await;
    assert!(err.contains("method"), "{err}");
    assert!(!server.root().join("f/Bad.yml").exists());
}

#[tokio::test]
async fn delete_item_moves_a_request_to_the_trash() {
    let trash = Arc::new(FakeTrash::new(true));
    let server = TestServer::start_with(
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &request("a", 1, "GET", "https://x.test/a")),
            ("b.yml", &request("b", 2, "GET", "https://x.test/b")),
        ],
        |o| o.trash(trash.clone()),
    )
    .await;
    assert_eq!(
        server.ok("delete_item", json!({"path": "a.yml"})).await,
        json!({"deleted": "a.yml", "destination": "trash"})
    );
    assert_eq!(trash.taken(), 1);
    assert!(!server.root().join("a.yml").exists());
}

#[tokio::test]
async fn delete_item_moves_a_folder_with_its_contents_to_the_trash() {
    let trash = Arc::new(FakeTrash::new(true));
    let server = TestServer::start_with(
        &[
            ("opencollection.yml", ROOT),
            ("f/folder.yml", &folder("f", 1)),
            ("f/x.yml", &request("x", 1, "GET", "https://x.test/x")),
        ],
        |o| o.trash(trash.clone()),
    )
    .await;
    let out = server.ok("delete_item", json!({"path": "f"})).await;
    assert_eq!(out["destination"], json!("trash"), "{out}");
    assert_eq!(out["deleted"], json!("f"), "{out}");
    assert_eq!(trash.taken(), 1);
    assert!(!server.root().join("f").exists());
}

#[tokio::test]
async fn delete_item_without_a_usable_trash_is_refused_and_changes_nothing() {
    let trash = Arc::new(FakeTrash::new(false));
    let server = TestServer::start_with(
        &[
            ("opencollection.yml", ROOT),
            ("f/folder.yml", &folder("f", 1)),
            ("f/x.yml", &request("x", 1, "GET", "https://x.test/x")),
            ("b.yml", &request("b", 2, "GET", "https://x.test/b")),
        ],
        |o| o.trash(trash.clone()),
    )
    .await;
    let before = files_under(server.root());
    for path in ["f", "b.yml"] {
        let err = server.err("delete_item", json!({"path": path})).await;
        assert_eq!(
            err,
            "deleting moves the item to the trash and none is available, so nothing was changed"
        );
    }
    assert_eq!(files_under(server.root()), before);
    assert_eq!(trash.taken(), 0);
}
