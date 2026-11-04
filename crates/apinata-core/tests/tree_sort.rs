//! Sorting a folder's items by name: seq values, files on disk, and no write when sorted.

use crate::tree_support;

use apinata_core::{ItemPath, TreeError};
use std::fs;
use tree_support::{ROOT, disk_seq, folder, ip, open, request};

#[test]
fn renumbers_siblings_in_name_order_on_disk_and_in_memory() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("c.yml", &request("c", 1)),
        ("a.yml", &request("a", 2)),
        ("b.yml", &request("b", 3)),
    ]);
    let changed = c.sort_by_name(&ItemPath::root()).expect("sorts");
    assert_eq!(changed.len(), 3);
    let seqs: Vec<_> = ["a.yml", "b.yml", "c.yml"]
        .iter()
        .map(|r| disk_seq(&dir, r))
        .collect();
    assert_eq!(seqs, [Some(1), Some(2), Some(3)]);
    assert_eq!(c.request(&ip("a.yml")).expect("a").seq(), Some(1));
}

#[test]
fn order_is_case_insensitive_and_natural() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("i10.yml", &request("item 10", 1)),
        ("i2.yml", &request("Item 2", 2)),
        ("i1.yml", &request("item 1", 3)),
        ("z.yml", &request("zeta", 4)),
        ("b.yml", &request("Beta", 5)),
    ]);
    c.sort_by_name(&ItemPath::root()).expect("sorts");
    let order = ["i1.yml", "i2.yml", "i10.yml", "b.yml", "z.yml"];
    // Beta sorts before "item", so it takes seq 1.
    let seqs: Vec<_> = order.iter().map(|r| disk_seq(&dir, r)).collect();
    assert_eq!(seqs, [Some(2), Some(3), Some(4), Some(1), Some(5)]);
}

#[test]
fn folders_and_requests_share_one_numbering() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("m.yml", &request("m", 1)),
        ("k/folder.yml", &folder("k", 2)),
        ("a/folder.yml", &folder("a", 3)),
    ]);
    c.sort_by_name(&ItemPath::root()).expect("sorts");
    assert_eq!(disk_seq(&dir, "a/folder.yml"), Some(1));
    assert_eq!(disk_seq(&dir, "k/folder.yml"), Some(2));
    assert_eq!(disk_seq(&dir, "m.yml"), Some(3));
}

#[test]
fn sorts_a_subfolder_and_leaves_other_folders_alone() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("g/folder.yml", &folder("g", 1)),
        ("g/y.yml", &request("y", 1)),
        ("g/x.yml", &request("x", 2)),
        ("b.yml", &request("b", 5)),
        ("a.yml", &request("a", 7)),
    ]);
    c.sort_by_name(&ip("g")).expect("sorts");
    assert_eq!(disk_seq(&dir, "g/x.yml"), Some(1));
    assert_eq!(disk_seq(&dir, "g/y.yml"), Some(2));
    assert_eq!(disk_seq(&dir, "b.yml"), Some(5));
    assert_eq!(disk_seq(&dir, "a.yml"), Some(7));
}

#[test]
fn an_already_sorted_folder_writes_nothing() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("b.yml", &request("b", 2)),
    ]);
    let before: Vec<_> = ["a.yml", "b.yml"]
        .iter()
        .map(|r| {
            fs::metadata(dir.path().join(r))
                .expect("meta")
                .modified()
                .expect("mtime")
        })
        .collect();
    std::thread::sleep(std::time::Duration::from_millis(30));
    let changed = c.sort_by_name(&ItemPath::root()).expect("sorts");
    assert!(changed.is_empty());
    let after: Vec<_> = ["a.yml", "b.yml"]
        .iter()
        .map(|r| {
            fs::metadata(dir.path().join(r))
                .expect("meta")
                .modified()
                .expect("mtime")
        })
        .collect();
    assert_eq!(before, after);
}

#[test]
fn equal_names_keep_their_current_relative_order() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("one.yml", &request("same", 1)),
        ("two.yml", &request("same", 2)),
    ]);
    let changed = c.sort_by_name(&ItemPath::root()).expect("sorts");
    assert!(changed.is_empty());
    assert_eq!(disk_seq(&dir, "two.yml"), Some(2));
}

#[test]
fn an_unknown_folder_is_not_found() {
    let (_dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    let err = c.sort_by_name(&ip("nope")).unwrap_err();
    assert!(matches!(err, TreeError::NotFound { .. }));
}

#[test]
fn a_file_changed_on_disk_is_a_conflict_and_nothing_is_written() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("b.yml", &request("b", 1)),
        ("a.yml", &request("a", 2)),
    ]);
    tree_support::write(&dir, "a.yml", &tree_support::changed_url("a", 2));
    let err = c.sort_by_name(&ItemPath::root()).unwrap_err();
    assert!(matches!(err, TreeError::Conflict { .. }), "{err:?}");
    assert_eq!(disk_seq(&dir, "b.yml"), Some(1));
}
