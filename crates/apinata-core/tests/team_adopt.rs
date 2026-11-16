//! A conflict copy opening as a request's unsaved edits, and the copy folder.

use crate::tree_support::{
    ROOT, changed_url, folder, ip, item, one, open, read, reload, request, summary, write,
};

use std::fs;

use apinata_core::team::ConflictCopies;
use apinata_core::{AdoptError, ChangeKind::*, DiskState, ItemKind::*};

fn mine(name: &str) -> String {
    request(name, 1).replace(&format!("x.test/{name}"), "x.test/mine")
}

#[test]
fn a_copy_of_an_item_changed_on_both_sides_loads_as_its_unsaved_edits() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request("a", 1))]);
    write(&dir, "a.yml", &changed_url("a", 1));
    let change = c
        .adopt_conflict_copy(&ip("a.yml"), mine("a").as_bytes())
        .unwrap();
    assert_eq!(summary(&dir, &[change]), one("a.yml", Conflict, Request));
    let a = c.request(&ip("a.yml")).unwrap();
    assert_eq!(
        (a.url(), a.is_dirty(), a.disk_state()),
        ("https://x.test/mine", true, DiskState::Present)
    );
    assert!(
        reload(&dir, &mut c, "a.yml").is_empty(),
        "the reset's bytes were already read"
    );
}

#[test]
fn a_copy_equal_to_the_disk_still_stays_dirty() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request("a", 1))]);
    write(&dir, "a.yml", &changed_url("a", 1));
    c.adopt_conflict_copy(&ip("a.yml"), changed_url("a", 1).as_bytes())
        .unwrap();
    assert!(c.request(&ip("a.yml")).unwrap().is_dirty());
}

#[test]
fn reload_takes_the_remote_and_overwrite_writes_the_copy() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("b.yml", &request("b", 2)),
    ]);
    write(&dir, "a.yml", &changed_url("a", 1));
    write(&dir, "b.yml", &changed_url("b", 2));
    c.adopt_conflict_copy(&ip("a.yml"), mine("a").as_bytes())
        .unwrap();
    c.adopt_conflict_copy(&ip("b.yml"), mine("b").as_bytes())
        .unwrap();
    c.revert(&dir.path().join("a.yml")).unwrap();
    let a = c.request(&ip("a.yml")).unwrap();
    assert_eq!((a.url(), a.is_dirty()), ("https://x.test/changed", false));
    c.save_overwrite(&dir.path().join("b.yml")).unwrap();
    assert!(read(&dir, "b.yml").contains("x.test/mine"));
    assert!(!c.request(&ip("b.yml")).unwrap().is_dirty());
}

#[test]
fn a_copy_of_an_item_deleted_remotely_comes_back_removed_on_disk() {
    for reloaded_first in [false, true] {
        let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request("a", 1))]);
        fs::remove_file(dir.path().join("a.yml")).unwrap();
        if reloaded_first {
            assert_eq!(
                reload(&dir, &mut c, "a.yml"),
                one("a.yml", Removed, Request)
            );
        }
        let change = c
            .adopt_conflict_copy(&ip("a.yml"), mine("a").as_bytes())
            .unwrap();
        assert_eq!(summary(&dir, &[change]), one("a.yml", Conflict, Request));
        let a = c.request_mut(&ip("a.yml")).expect("back in the tree");
        assert_eq!(
            (a.url(), a.is_dirty(), a.disk_state()),
            ("https://x.test/mine", true, DiskState::Removed)
        );
        a.save().unwrap();
        assert!(
            read(&dir, "a.yml").contains("x.test/mine"),
            "reloaded first: {reloaded_first}"
        );
    }
}

#[test]
fn a_copy_whose_folder_was_deleted_remotely_brings_the_folder_back() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("f", 1)),
        ("f/a.yml", &request("a", 1)),
    ]);
    fs::remove_dir_all(dir.path().join("f")).unwrap();
    reload(&dir, &mut c, "f");
    c.adopt_conflict_copy(&ip("f/a.yml"), mine("a").as_bytes())
        .unwrap();
    let a = c
        .request_mut(&ip("f/a.yml"))
        .expect("kept under its folder");
    assert_eq!(a.disk_state(), DiskState::Removed);
    a.save().unwrap();
    assert!(read(&dir, "f/a.yml").contains("x.test/mine"));
}

#[test]
fn a_copy_that_does_not_load_changes_nothing() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request("a", 1))]);
    let got = c.adopt_conflict_copy(&ip("a.yml"), b"info: [");
    assert!(matches!(got, Err(AdoptError::Unreadable { .. })), "{got:?}");
    assert!(!c.request(&ip("a.yml")).unwrap().is_dirty());
    assert!(reload(&dir, &mut c, "a.yml").is_empty());
}

#[test]
fn only_a_request_copy_is_adopted() {
    let (_dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("o.yml", &item("app", "o", 1)),
    ]);
    assert!(matches!(
        c.adopt_conflict_copy(&ip("o.yml"), item("app", "o", 2).as_bytes()),
        Err(AdoptError::NotARequest)
    ));
    assert!(matches!(
        c.adopt_conflict_copy(&ip("opencollection.yml"), ROOT.as_bytes()),
        Err(AdoptError::NotARequest)
    ));
}

#[test]
fn the_copy_folder_lists_reads_and_prunes() {
    let dir = tempfile::TempDir::new().unwrap();
    let copies = ConflictCopies::new(dir.path().join("b1"));
    assert!(copies.list().unwrap().is_empty());
    for (rel, text) in [("f/r.yml", "r"), ("a.yml", "a")] {
        let path = dir.path().join("b1").join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    assert_eq!(copies.list().unwrap(), ["a.yml", "f/r.yml"]);
    assert_eq!(copies.read("f/r.yml").unwrap(), b"r");
    copies.remove("f/r.yml").unwrap();
    assert!(!dir.path().join("b1/f").exists());
    copies.remove("f/r.yml").unwrap();
    assert_eq!(copies.list().unwrap(), ["a.yml"]);
}

#[test]
fn a_copy_and_a_delete_mark_move_with_their_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let copies = ConflictCopies::new(dir.path().join("b1"));
    for (rel, text) in [("b1/f/r.yml", "r"), ("b1.deleted/f/d.yml", "")] {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    copies.rename("f/r.yml", "g/s.yml").unwrap();
    assert_eq!(copies.list().unwrap(), ["g/s.yml"]);
    assert_eq!(copies.read("g/s.yml").unwrap(), b"r");
    assert!(!dir.path().join("b1/f").exists());
    copies.rename_deleted("f/d.yml", "e.yml").unwrap();
    assert_eq!(copies.deleted().unwrap(), ["e.yml"]);
    assert!(!dir.path().join("b1.deleted/f").exists());
    copies.rename("gone.yml", "x.yml").unwrap();
    copies.rename_deleted("gone.yml", "x.yml").unwrap();
    assert_eq!(copies.list().unwrap(), ["g/s.yml"]);
    assert!(copies.rename("g/s.yml", "../s.yml").is_err());
    assert!(copies.rename_deleted("e.yml", "../e.yml").is_err());
}

#[test]
fn a_dirty_request_keeps_its_newer_edits_when_its_copy_is_adopted() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request("a", 1))]);
    c.request_mut(&ip("a.yml"))
        .unwrap()
        .set_url("https://x.test/newer");
    write(&dir, "a.yml", &changed_url("a", 1));
    c.adopt_conflict_copy(&ip("a.yml"), mine("a").as_bytes())
        .unwrap();
    let a = c.request(&ip("a.yml")).unwrap();
    assert_eq!((a.url(), a.is_dirty()), ("https://x.test/newer", true));
    c.request_mut(&ip("a.yml"))
        .unwrap()
        .set_url("https://x.test/changed");
    assert!(
        c.request(&ip("a.yml")).unwrap().is_dirty(),
        "marked unsaved, not only edited"
    );
}

#[test]
fn a_copy_of_an_ignored_file_is_refused() {
    let root = format!("{ROOT}extensions:\n  bruno:\n    ignore:\n      - 'skip'\n");
    let (dir, _) = open(&[
        ("opencollection.yml", &root),
        ("skip/a.yml", &request("a", 1)),
    ]);
    let mut c = apinata_core::Collection::open_with(
        dir.path(),
        &apinata_core::OpenOptions::new().ignore(apinata_core::IgnorePolicy::BrunoApp),
    )
    .unwrap();
    let got = c.adopt_conflict_copy(&ip("skip/a.yml"), mine("a").as_bytes());
    assert!(matches!(got, Err(AdoptError::Ignored)), "{got:?}");
    assert!(c.request(&ip("skip/a.yml")).is_none());
}
