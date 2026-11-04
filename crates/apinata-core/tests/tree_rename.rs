//! Cloning requests and renaming requests and folders.

use crate::parity;
use crate::tree_support;

use apinata_core::{Collection, ItemPath, TreeError};
use tempfile::TempDir;
use tree_support::{ROOT, exists, folder, ip, item, open, read, reload, request, tree_parity};

fn with_request_1(extra: &[(&str, &str)]) -> (TempDir, Collection) {
    let source = parity::normalized("cli-docs/request_1.yml");
    let mut files = vec![
        ("opencollection.yml", ROOT),
        ("request_1.yml", source.as_str()),
    ];
    files.extend_from_slice(extra);
    open(&files)
}

#[test]
fn a_clone_has_brunos_bytes_and_the_next_seq() {
    // The clone takes the next seq among these siblings, 4, which the reference bytes hold.
    let (dir, mut c) = with_request_1(&[
        ("b.yml", &request("b", 2)),
        ("ui.yml", &item("app", "ui", 1)),
        ("f/folder.yml", &folder("f", 3)),
    ]);
    let before = read(&dir, "request_1.yml");
    let cloned = c
        .clone_request(&ip("request_1.yml"), "request_1 copy")
        .expect("clones");
    assert_eq!(cloned, ip("request_1 copy.yml"));
    assert_eq!(
        read(&dir, "request_1 copy.yml"),
        tree_parity("clone-request")
    );
    assert_eq!(read(&dir, "request_1.yml"), before);
    assert!(c.request(&cloned).is_some());
}

#[test]
fn a_clone_copies_unsaved_edits_and_leaves_the_source_dirty() {
    let (dir, mut c) = with_request_1(&[]);
    c.request_mut(&ip("request_1.yml"))
        .expect("source")
        .set_method("POST");
    let cloned = c
        .clone_request(&ip("request_1.yml"), "  copy  ")
        .expect("clones");
    assert_eq!(cloned, ip("copy.yml"));
    let clone = c.request(&cloned).expect("clone");
    assert_eq!(
        (clone.method(), clone.name(), clone.seq()),
        ("POST", "copy", Some(2))
    );
    assert!(!clone.is_dirty());
    assert!(c.request(&ip("request_1.yml")).expect("source").is_dirty());
    assert!(read(&dir, "request_1.yml").contains("method: GET"));
}

#[test]
fn only_requests_can_be_cloned() {
    let (_dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("f", 1)),
        ("q.yml", &item("grpc", "q", 2)),
    ]);
    for target in ["f", "q.yml", "nope.yml"] {
        let result = c.clone_request(&ip(target), "x");
        assert!(
            matches!(result, Err(TreeError::NotFound { .. })),
            "{target}: {result:?}"
        );
    }
}

#[test]
fn a_name_only_rename_rewrites_info_name() {
    let (dir, mut c) = with_request_1(&[]);
    assert_eq!(
        c.rename(&ip("request_1.yml"), "renamed", false)
            .expect("renames"),
        ip("request_1.yml")
    );
    assert_eq!(
        read(&dir, "request_1.yml"),
        parity::expected("info-name-cli-docs-request-1")
    );
}

#[test]
fn a_file_rename_moves_the_request_under_its_sanitised_name() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("Alpha", 1)),
        ("b.yml", &request("b", 2)),
    ]);
    let renamed = c.rename(&ip("a.yml"), "New: one", true).expect("renames");
    assert_eq!(renamed, ip("New- one.yml"));
    assert!(!exists(&dir, "a.yml") && c.request(&ip("a.yml")).is_none());
    assert_eq!(c.request(&renamed).expect("renamed").name(), "New: one");
    assert_eq!(
        c.rename(&ip("b.yml"), "New: one", true).expect("renames"),
        ip("New- one 1.yml")
    );
    assert_eq!(
        c.rename(&renamed, "New- one", true).expect("renames"),
        renamed,
        "same stem keeps the file"
    );
    assert!(
        reload(&dir, &mut c, "a.yml").is_empty() && reload(&dir, &mut c, "New- one.yml").is_empty()
    );
}

#[test]
fn a_dirty_request_keeps_its_draft_through_a_rename() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request("a", 1))]);
    c.request_mut(&ip("a.yml")).expect("a").set_method("POST");
    let renamed = c.rename(&ip("a.yml"), "b", true).expect("renames");
    let disk = read(&dir, "b.yml");
    assert!(
        disk.contains("name: b") && disk.contains("method: GET"),
        "{disk}"
    );
    let b = c.request_mut(&renamed).expect("b");
    assert_eq!((b.name(), b.method(), b.is_dirty()), ("b", "POST", true));
    b.save().expect("saves at the new path");
    assert!(read(&dir, "b.yml").contains("method: POST"));
}

#[test]
fn a_folder_rename_writes_its_folder_file_and_then_its_directory() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        (
            "folder_1/folder.yml",
            &parity::normalized("cli-docs/folder_1/folder.yml"),
        ),
        ("folder_1/x.yml", &request("x", 1)),
    ]);
    assert_eq!(
        c.rename(&ip("folder_1"), "Renamed", false)
            .expect("renames"),
        ip("folder_1")
    );
    assert_eq!(
        read(&dir, "folder_1/folder.yml"),
        tree_parity("folder-rename")
    );
    let moved = c.rename(&ip("folder_1"), "Renamed", true).expect("renames");
    assert_eq!(moved, ip("Renamed"));
    assert!(!exists(&dir, "folder_1"));
    assert_eq!(c.folder(&moved).expect("folder").name(), "Renamed");
    let x = c.request_mut(&ip("Renamed/x.yml")).expect("x");
    x.set_method("POST");
    x.save().expect("saves under the renamed directory");
    assert!(read(&dir, "Renamed/x.yml").contains("method: POST"));
    assert!(
        reload(&dir, &mut c, "Renamed").is_empty() && reload(&dir, &mut c, "folder_1").is_empty()
    );
}

#[test]
fn a_folder_without_a_folder_file_gets_one_on_rename() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("plain/a.yml", &request("a", 1)),
    ]);
    c.rename(&ip("plain"), "Renamed", false).expect("renames");
    assert_eq!(
        read(&dir, "plain/folder.yml"),
        tree_parity("folder-file-from-rename")
    );
    assert_eq!(c.folder(&ip("plain")).expect("plain").name(), "Renamed");
}

#[test]
fn the_root_opaque_items_and_blank_names_are_rejected() {
    let (_dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("q.yml", &item("grpc", "q", 1)),
    ]);
    assert!(matches!(
        c.rename(&ItemPath::root(), "x", false),
        Err(TreeError::InvalidMove { .. })
    ));
    assert!(matches!(
        c.rename(&ip("q.yml"), "x", false),
        Err(TreeError::NotFound { .. })
    ));
    assert!(matches!(
        c.rename(&ip("a.yml"), "   ", false),
        Err(TreeError::InvalidName { .. })
    ));
}
