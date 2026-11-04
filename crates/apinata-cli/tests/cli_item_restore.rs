//! `apinata item restore`: undoes the newest `item rm` (or MCP `delete_item`) in the
//! collection from the scratch freedesktop trash, sharing one record with `apinata mcp`.

use crate::cli_item::{Copy, stdout};
use crate::common::{ROOT, write_tree};

use apinata_core::TrashLog;
use predicates::str::contains;
use serde_json::{Value, json};
use std::fs;

fn seq(copy: &Copy, rel: &str) -> String {
    let text = fs::read_to_string(copy.root().join(rel)).expect("read");
    let line = text.lines().find(|l| l.trim_start().starts_with("seq:"));
    line.expect("a seq line").trim().to_owned()
}

#[test]
fn restore_puts_back_what_rm_trashed_with_the_old_numbers() {
    let a = Copy::new();
    a.item(&["rm", "users/get.yml"]).code(0);
    assert_eq!(seq(&a, "users/list.yml"), "seq: 1");
    a.item(&["restore"])
        .code(0)
        .stdout("restored users/get.yml\n");
    assert!(a.root().join("users/get.yml").is_file());
    assert_eq!(seq(&a, "users/list.yml"), "seq: 2");
    assert!(!a.data().join("Trash/files/get.yml").exists());
    // The restored file is byte for byte what was trashed; list.yml was rewritten twice.
    let fresh = Copy::new().tree();
    assert_eq!(a.tree()["users/get.yml"], fresh["users/get.yml"]);
}

#[test]
fn restore_prints_the_mcp_shape_as_json() {
    let a = Copy::new();
    a.item(&["rm", "users"]).code(0);
    let out = a.item(&["--format", "json", "restore"]).code(0);
    let doc: Value = serde_json::from_str(&stdout(&out)).expect("json");
    assert_eq!(doc, json!({"restored": "users", "warnings": []}));
    assert_eq!(a.tree(), Copy::new().tree());
}

#[test]
fn a_clash_exits_3_and_leaves_the_file_there() {
    let a = Copy::new();
    a.item(&["rm", "a.yml"]).code(0);
    fs::write(a.root().join("a.yml"), "mine\n").expect("write");
    a.item(&["restore"])
        .code(3)
        .stdout("")
        .stderr(contains("a.yml"))
        .stderr(contains("move or rename"));
    assert_eq!(
        fs::read_to_string(a.root().join("a.yml")).expect("read"),
        "mine\n"
    );
}

#[test]
fn restore_with_nothing_deleted_exits_3() {
    let a = Copy::new();
    a.item(&["restore"])
        .code(3)
        .stdout("")
        .stderr("error: nothing deleted from this collection is left to restore\n");
}

#[tokio::test]
async fn an_rm_from_the_cli_is_restored_by_the_mcp_tool() {
    let a = Copy::new();
    a.item(&["rm", "a.yml"]).code(0);
    let out = a.mcp("restore_item", json!({})).await.expect("restored");
    assert_eq!(out, json!({"restored": "a.yml", "warnings": []}));
    assert_eq!(a.tree(), Copy::new().tree());
}

/// A root holding a, b and c in that order, deleted as `order` runs, with the outer two deletes
/// recorded as one prune.
fn deleted_in_order(order: [&str; 3]) -> Copy {
    let request = |name: &str, seq: u32| {
        format!(
            "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: http://localhost/{name}\n"
        )
    };
    let a = Copy::of(&write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("b.yml", &request("b", 2)),
        ("c.yml", &request("c", 3)),
    ]));
    for item in order {
        a.item(&["rm", item]).code(0);
    }
    // The CLI cannot prune around a single delete by itself, so the record is shaped as an
    // interleaved update would have left it: the outer two entries are one prune.
    let log = TrashLog::new(a.data().join("apinata/trash-log"));
    let file = log.file_for(&a.root());
    let text = fs::read_to_string(&file).expect("read");
    let mut doc: Value = serde_json::from_str(&text).expect("json");
    for i in [0, 2] {
        doc["entries"][i]["batch"] = "run".into();
    }
    fs::write(&file, doc.to_string()).expect("write");
    a
}

#[test]
fn a_restore_after_a_prune_places_the_item_by_its_tail() {
    let a = deleted_in_order(["b.yml", "a.yml", "c.yml"]);
    a.item(&["restore", "--pruned"]).code(0);
    a.item(&["restore"]).code(0).stdout("restored a.yml\n");
    assert_eq!(seq(&a, "b.yml"), "seq: 1");
    assert_eq!(seq(&a, "a.yml"), "seq: 2");
    assert_eq!(seq(&a, "c.yml"), "seq: 3");
}

#[test]
fn a_restore_after_a_prune_keeps_the_old_order_when_the_tail_allows() {
    let a = deleted_in_order(["c.yml", "a.yml", "b.yml"]);
    a.item(&["restore", "--pruned"]).code(0);
    a.item(&["restore"]).code(0).stdout("restored a.yml\n");
    assert_eq!(seq(&a, "a.yml"), "seq: 1");
    assert_eq!(seq(&a, "b.yml"), "seq: 2");
    assert_eq!(seq(&a, "c.yml"), "seq: 3");
}
