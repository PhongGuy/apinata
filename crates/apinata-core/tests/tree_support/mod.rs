//! Helpers shared by the reload and tree operation tests.
#![allow(dead_code)] // each test binary uses a different subset

use apinata_core::{Change, ChangeKind, Collection, ItemKind, ItemPath};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

pub const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: c\n";
pub const BROKEN: &str = "info: [";

pub fn request(name: &str, seq: u64) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: https://x.test/{name}\n"
    )
}

/// An item of another type, such as `graphql` or `app`.
pub fn item(kind: &str, name: &str, seq: u64) -> String {
    format!(
        "info:\n  name: {name}\n  type: {kind}\n  seq: {seq}\n{kind}:\n  url: https://x.test/{name}\n"
    )
}

pub fn folder(name: &str, seq: u64) -> String {
    format!("info:\n  name: {name}\n  type: folder\n  seq: {seq}\n")
}

pub fn open(files: &[(&str, &str)]) -> (TempDir, Collection) {
    let dir = TempDir::new().expect("tempdir");
    for (rel, text) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(path, text).expect("write");
    }
    let c = Collection::open(dir.path()).expect("opens");
    (dir, c)
}

pub fn ip(rel: &str) -> ItemPath {
    ItemPath::new(rel).expect("valid item path")
}

pub fn write(dir: &TempDir, rel: &str, text: &str) {
    fs::write(dir.path().join(rel), text).expect("write");
}

/// `request(name, seq)` with another URL: a change that is not seq-only.
pub fn changed_url(name: &str, seq: u64) -> String {
    request(name, seq).replace(&format!("x.test/{name}"), "x.test/changed")
}

/// Reload `rel` and summarise what changed.
pub fn reload(dir: &TempDir, c: &mut Collection, rel: &str) -> Vec<(String, ChangeKind, ItemKind)> {
    let changes = c.reload_path(&dir.path().join(rel));
    summary(dir, &changes)
}

/// Each change as (path relative to the collection, kind, item kind).
pub fn summary(dir: &TempDir, changes: &[Change]) -> Vec<(String, ChangeKind, ItemKind)> {
    changes
        .iter()
        .map(|c| {
            let rel = c
                .path
                .strip_prefix(dir.path())
                .expect("inside the collection");
            (rel.to_string_lossy().into_owned(), c.kind, c.item)
        })
        .collect()
}

/// One expected change, in `summary`'s form.
pub fn one(rel: &str, kind: ChangeKind, item: ItemKind) -> Vec<(String, ChangeKind, ItemKind)> {
    vec![(rel.to_owned(), kind, item)]
}

/// How many load errors name `rel`.
pub fn errors_for(dir: &TempDir, c: &Collection, rel: &str) -> usize {
    let path = dir.path().join(rel);
    c.load_errors().iter().filter(|e| e.path() == path).count()
}

pub fn read(dir: &TempDir, rel: &str) -> String {
    fs::read_to_string(dir.path().join(rel)).expect("read")
}

pub fn yaml(dir: &TempDir, rel: &str) -> yaml_serde::Value {
    yaml_serde::from_str(&read(dir, rel)).expect("parses")
}

pub fn exists(dir: &TempDir, rel: &str) -> bool {
    dir.path().join(rel).exists()
}

/// The `info.seq` on disk for a file, or for a directory in its `folder.yml`.
pub fn disk_seq(dir: &TempDir, rel: &str) -> Option<u64> {
    let file = if dir.path().join(rel).is_dir() {
        format!("{rel}/folder.yml")
    } else {
        rel.to_owned()
    };
    yaml(dir, &file)["info"]["seq"].as_u64()
}

/// The bytes a new request is written with: Bruno's, but with an inherited timeout.
pub fn new_request_bytes(id: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reference/2026-10-17-new-request-timeout-inherit")
        .join(format!("{id}.yml"));
    fs::read_to_string(path).expect("new request case")
}

pub fn tree_parity(id: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reference/2026-10-09-tree-parity/expected")
        .join(format!("{id}.yml"));
    fs::read_to_string(path).expect("tree parity case")
}
