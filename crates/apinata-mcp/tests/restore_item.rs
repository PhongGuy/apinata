//! `restore_item`: puts back the newest item `delete_item` moved to the trash, through the
//! persisted trash log, refusing rather than overwriting.

use crate::common;

use common::{FakeTrash, ROOT, TestServer, request};
use serde_json::json;
use std::fs;
use std::sync::Arc;

const NOTHING: &str = "nothing deleted from this collection is left to restore";

async fn three(trash: &Arc<FakeTrash>) -> TestServer {
    let trash = trash.clone();
    TestServer::start_with(
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &request("a", 1, "GET", "https://x.test/a")),
            ("b.yml", &request("b", 2, "GET", "https://x.test/b")),
            ("c.yml", &request("c", 3, "GET", "https://x.test/c")),
        ],
        move |o| o.trash(trash),
    )
    .await
}

fn seq(server: &TestServer, rel: &str) -> String {
    let text = fs::read_to_string(server.root().join(rel)).expect("read");
    let line = text.lines().find(|l| l.trim_start().starts_with("seq:"));
    line.expect("a seq line").trim().to_owned()
}

#[tokio::test]
async fn a_deleted_request_comes_back_with_its_siblings_renumbered_back() {
    let trash = Arc::new(FakeTrash::new(true));
    let server = three(&trash).await;
    server.ok("delete_item", json!({"path": "a.yml"})).await;
    assert_eq!(seq(&server, "b.yml"), "seq: 1");
    assert_eq!(
        server.ok("restore_item", json!({})).await,
        json!({"restored": "a.yml", "warnings": []})
    );
    assert!(server.root().join("a.yml").is_file());
    assert_eq!(seq(&server, "b.yml"), "seq: 2");
    assert_eq!(seq(&server, "c.yml"), "seq: 3");
    assert_eq!(trash.restored(), ["item1"]);
}

#[tokio::test]
async fn restoring_with_nothing_deleted_is_an_error_that_never_reaches_the_bin() {
    let trash = Arc::new(FakeTrash::new(true));
    let server = three(&trash).await;
    // An item in the trash this collection never deleted stays there.
    fs::write(trash.stash(7), "someone else's").expect("plant");
    assert_eq!(server.err("restore_item", json!({})).await, NOTHING);
    assert!(trash.restored().is_empty(), "{:?}", trash.restored());
    assert!(trash.stash(7).is_file());
}

#[tokio::test]
async fn a_clash_is_refused_untouched_then_restores_once_cleared() {
    let trash = Arc::new(FakeTrash::new(true));
    let server = three(&trash).await;
    server.ok("delete_item", json!({"path": "a.yml"})).await;
    let clash = server.root().join("a.yml");
    fs::write(&clash, "mine\n").expect("write");
    let err = server.err("restore_item", json!({})).await;
    assert!(
        err.contains("a.yml") && err.contains("move or rename"),
        "{err}"
    );
    assert_eq!(fs::read_to_string(&clash).expect("read"), "mine\n");
    assert!(trash.restored().is_empty());
    fs::remove_file(&clash).expect("rm");
    let out = server.ok("restore_item", json!({})).await;
    assert_eq!(out["restored"], json!("a.yml"), "{out}");
    assert!(clash.is_file());
}

#[tokio::test]
async fn two_deletes_restore_newest_first() {
    let trash = Arc::new(FakeTrash::new(true));
    let server = three(&trash).await;
    server.ok("delete_item", json!({"path": "a.yml"})).await;
    server.ok("delete_item", json!({"path": "c.yml"})).await;
    let first = server.ok("restore_item", json!({})).await;
    assert_eq!(first["restored"], json!("c.yml"), "{first}");
    let second = server.ok("restore_item", json!({})).await;
    assert_eq!(second["restored"], json!("a.yml"), "{second}");
    assert_eq!(server.err("restore_item", json!({})).await, NOTHING);
    assert_eq!(trash.restored(), ["item2", "item1"]);
    assert_eq!(
        ["a.yml", "b.yml", "c.yml"].map(|f| seq(&server, f)),
        ["seq: 1", "seq: 2", "seq: 3"]
    );
}

#[tokio::test]
async fn an_item_emptied_from_the_trash_is_reported_and_skipped() {
    let trash = Arc::new(FakeTrash::new(true));
    let server = three(&trash).await;
    server.ok("delete_item", json!({"path": "a.yml"})).await;
    server.ok("delete_item", json!({"path": "c.yml"})).await;
    fs::remove_file(trash.stash(2)).expect("empty");
    let err = server.err("restore_item", json!({})).await;
    assert!(
        err.contains("c.yml") && err.contains("no longer in the trash"),
        "{err}"
    );
    let out = server.ok("restore_item", json!({})).await;
    assert_eq!(out["restored"], json!("a.yml"), "{out}");
}

#[tokio::test]
async fn without_a_data_directory_delete_warns_and_restore_is_unavailable() {
    let trash = Arc::new(FakeTrash::new(true));
    let bin = trash.clone();
    let server = TestServer::start_with(
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &request("a", 1, "GET", "https://x.test/a")),
        ],
        move |o| o.trash(bin).without_trash_log(),
    )
    .await;
    let out = server.ok("delete_item", json!({"path": "a.yml"})).await;
    assert_eq!(
        out["warnings"],
        json!(["moved to the trash, but it cannot be restored from here: no data directory"])
    );
    assert_eq!(
        server.err("restore_item", json!({})).await,
        "restoring is not available: no data directory"
    );
    assert!(!server.trash_log_dir().exists());
}

#[tokio::test]
async fn restore_item_is_listed_as_a_non_destructive_local_write() {
    let trash = Arc::new(FakeTrash::new(true));
    let server = three(&trash).await;
    let tools = server.tools().await;
    let tool = tools
        .iter()
        .find(|t| t.name == "restore_item")
        .expect("listed");
    let a = tool.annotations.as_ref().expect("annotations");
    assert_eq!(
        (
            a.read_only_hint,
            a.destructive_hint,
            a.idempotent_hint,
            a.open_world_hint
        ),
        (None, Some(false), Some(false), Some(false))
    );
    assert!(tool.output_schema.is_some());
}
