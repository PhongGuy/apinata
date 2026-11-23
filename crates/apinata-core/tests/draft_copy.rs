//! Copying a request as a draft: an unsaved copy in the same folder, nothing written.

use crate::tree_support;

use apinata_core::TreeError;
use tree_support::{ROOT, folder, ip, open, request};

fn files_under(dir: &std::path::Path) -> usize {
    walk(dir)
}

fn walk(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir)
        .expect("readable")
        .map(|e| {
            let path = e.expect("entry").path();
            if path.is_dir() { walk(&path) } else { 1 }
        })
        .sum()
}

#[test]
fn a_copy_is_a_draft_in_the_same_folder_named_as_a_copy() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("Users", 1)),
        ("users/list.yml", &request("List", 1)),
    ]);
    let before = files_under(dir.path());
    let copy = c.create_draft_copy(&ip("users/list.yml")).expect("copied");
    assert!(c.is_draft(&copy));
    assert_eq!(copy.parent(), Some(ip("users")));
    let draft = c.request(&copy).expect("the draft");
    assert_eq!(draft.name(), "List (copy)");
    assert_eq!(draft.url(), "https://x.test/List");
    assert_eq!(files_under(dir.path()), before, "nothing is written");
    assert_eq!(
        c.request(&ip("users/list.yml")).expect("source").name(),
        "List"
    );
}

#[test]
fn a_second_copy_takes_the_next_copy_name() {
    let (_dir, mut c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request("a", 1))]);
    c.create_draft_copy(&ip("a.yml")).expect("first");
    let second = c.create_draft_copy(&ip("a.yml")).expect("second");
    assert_eq!(c.request(&second).expect("draft").name(), "a (copy 2)");
}

#[test]
fn a_draft_can_be_copied_too() {
    let (_dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    let draft = c
        .create_draft(&apinata_core::ItemPath::root())
        .expect("draft");
    let copy = c.create_draft_copy(&draft).expect("copied");
    assert_ne!(copy, draft);
    assert_eq!(
        c.request(&copy).expect("copy").name(),
        "Untitled request (copy)"
    );
}

#[test]
fn an_unknown_item_is_not_found() {
    let (_dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    assert!(matches!(
        c.create_draft_copy(&ip("missing.yml")),
        Err(TreeError::NotFound { .. })
    ));
}
