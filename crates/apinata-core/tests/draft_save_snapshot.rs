//! `Collection::save_snapshot` writes a snapshot as a request and leaves no draft behind.

use crate::tree_support;

use apinata_core::{DraftSnapshot, TreeError};
use tree_support::{ROOT, folder, ip, open, request};

const SNAP: &str = r#"{"info":{"name":"Echo","type":"http"},"http":{"method":"POST","url":"http://127.0.0.1:1/echo"}}"#;

#[test]
fn it_writes_the_request_and_keeps_no_draft() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("users", 1)),
        ("users/u.yml", &request("u", 1)),
    ]);
    let snap = DraftSnapshot::from_json(SNAP).unwrap();
    let saved = c.save_snapshot(&ip("users"), "Echo", &snap).unwrap();
    assert!(!saved.is_draft());
    assert_eq!(c.drafts().count(), 0);
    let text = std::fs::read_to_string(saved.to_path(dir.path())).unwrap();
    assert!(text.contains("http://127.0.0.1:1/echo"), "{text}");
}

#[test]
fn an_unknown_folder_fails_and_leaves_no_draft() {
    let (_dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    let snap = DraftSnapshot::from_json(SNAP).unwrap();
    let err = c.save_snapshot(&ip("nope"), "Echo", &snap).unwrap_err();
    assert!(matches!(err, TreeError::NotFound { .. }), "{err:?}");
    assert_eq!(c.drafts().count(), 0);
}
