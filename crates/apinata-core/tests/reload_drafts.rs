//! Dirty request drafts outlive a removed or broken file.

use crate::tree_support;

use apinata_core::{ChangeKind::*, DiskState, ItemKind::*, SaveError};
use std::fs;
use tree_support::{
    BROKEN, ROOT, changed_url, errors_for, ip, item, one, open, read, reload, request, write,
};

/// A collection holding `a.yml`, with an unsaved method edit.
fn dirty_a() -> (tempfile::TempDir, apinata_core::Collection) {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request("a", 1))]);
    c.request_mut(&ip("a.yml")).expect("a").set_method("POST");
    (dir, c)
}

#[test]
fn a_dirty_request_outlives_its_deleted_file_and_saving_recreates_it() {
    let (dir, mut c) = dirty_a();
    fs::remove_file(dir.path().join("a.yml")).expect("rm");
    assert_eq!(
        reload(&dir, &mut c, "a.yml"),
        one("a.yml", Conflict, Request)
    );
    assert!(reload(&dir, &mut c, "a.yml").is_empty(), "reported once");
    assert_eq!(errors_for(&dir, &c, "a.yml"), 0);
    let a = c.request_mut(&ip("a.yml")).expect("kept");
    assert_eq!((a.method(), a.disk_state()), ("POST", DiskState::Removed));
    a.save().expect("recreates the file");
    assert_eq!(a.disk_state(), DiskState::Present);
    assert!(read(&dir, "a.yml").contains("method: POST"));
    assert!(reload(&dir, &mut c, "a.yml").is_empty(), "own write");
}

#[test]
fn a_dirty_request_outlives_a_broken_file_until_it_loads_again() {
    let (dir, mut c) = dirty_a();
    write(&dir, "a.yml", BROKEN);
    assert_eq!(
        reload(&dir, &mut c, "a.yml"),
        one("a.yml", Conflict, Request)
    );
    assert!(reload(&dir, &mut c, "a.yml").is_empty(), "reported once");
    assert_eq!(errors_for(&dir, &c, "a.yml"), 1);
    let a = c.request_mut(&ip("a.yml")).expect("kept");
    assert_eq!((a.method(), a.disk_state()), ("POST", DiskState::Broken));
    assert!(matches!(a.save(), Err(SaveError::Conflict { .. })));
    assert_eq!(read(&dir, "a.yml"), BROKEN);
    write(&dir, "a.yml", &request("a", 1));
    assert_eq!(
        reload(&dir, &mut c, "a.yml"),
        one("a.yml", Restored, Request)
    );
    assert_eq!(errors_for(&dir, &c, "a.yml"), 0);
    let a = c.request_mut(&ip("a.yml")).expect("kept");
    assert_eq!((a.method(), a.disk_state()), ("POST", DiskState::Present));
    a.save()
        .expect("no conflict once the original content is back");
}

#[test]
fn a_file_that_returns_with_other_content_is_a_conflict() {
    let (dir, mut c) = dirty_a();
    fs::remove_file(dir.path().join("a.yml")).expect("rm");
    reload(&dir, &mut c, "a.yml");
    write(&dir, "a.yml", &changed_url("a", 1));
    assert_eq!(
        reload(&dir, &mut c, "a.yml"),
        one("a.yml", Conflict, Request)
    );
    let a = c.request_mut(&ip("a.yml")).expect("kept");
    assert_eq!(a.disk_state(), DiskState::Present);
    assert!(matches!(a.save(), Err(SaveError::Conflict { .. })));
}

#[test]
fn a_dirty_request_whose_file_becomes_another_item_type_stays_a_broken_request() {
    let (dir, mut c) = dirty_a();
    write(&dir, "a.yml", &item("grpc", "a", 1));
    assert_eq!(
        reload(&dir, &mut c, "a.yml"),
        one("a.yml", Conflict, Request)
    );
    assert!(c.opaque_item(&ip("a.yml")).is_none());
    assert_eq!(
        c.request(&ip("a.yml")).expect("kept").disk_state(),
        DiskState::Broken
    );
}

#[test]
fn a_clean_request_still_leaves_with_its_file() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request("a", 1))]);
    write(&dir, "a.yml", BROKEN);
    assert_eq!(
        reload(&dir, &mut c, "a.yml"),
        one("a.yml", Removed, Request)
    );
    assert!(c.request(&ip("a.yml")).is_none());
}

#[test]
fn a_full_reload_keeps_a_removed_draft_without_reporting_it_again() {
    let (dir, mut c) = dirty_a();
    fs::remove_file(dir.path().join("a.yml")).expect("rm");
    reload(&dir, &mut c, "a.yml");
    assert!(c.reload().is_empty());
    assert!(c.request(&ip("a.yml")).is_some());
}

#[test]
fn a_conflict_is_reported_once_per_content_and_again_for_new_content() {
    let (dir, mut c) = dirty_a();
    write(&dir, "a.yml", &changed_url("a", 1));
    assert_eq!(
        reload(&dir, &mut c, "a.yml"),
        one("a.yml", Conflict, Request)
    );
    assert!(reload(&dir, &mut c, "a.yml").is_empty(), "same content");
    write(
        &dir,
        "a.yml",
        &request("a", 1).replace("x.test/a", "x.test/other"),
    );
    assert_eq!(
        reload(&dir, &mut c, "a.yml"),
        one("a.yml", Conflict, Request)
    );
}

#[test]
fn a_conflict_is_reported_again_after_the_file_returns_to_saved_content() {
    let (dir, mut c) = dirty_a();
    let other = changed_url("a", 1);
    write(&dir, "a.yml", &other);
    assert_eq!(
        reload(&dir, &mut c, "a.yml"),
        one("a.yml", Conflict, Request)
    );
    write(&dir, "a.yml", &request("a", 1));
    assert!(reload(&dir, &mut c, "a.yml").is_empty(), "back to saved");
    write(&dir, "a.yml", &other);
    assert_eq!(
        reload(&dir, &mut c, "a.yml"),
        one("a.yml", Conflict, Request),
        "the same content conflicts again"
    );
}

#[test]
fn a_conflict_is_reported_again_after_a_seq_only_change() {
    let (dir, mut c) = dirty_a();
    let other = changed_url("a", 1);
    write(&dir, "a.yml", &other);
    assert_eq!(
        reload(&dir, &mut c, "a.yml"),
        one("a.yml", Conflict, Request)
    );
    write(&dir, "a.yml", &request("a", 2));
    reload(&dir, &mut c, "a.yml");
    write(&dir, "a.yml", &other);
    assert_eq!(
        reload(&dir, &mut c, "a.yml"),
        one("a.yml", Conflict, Request),
        "the same content conflicts again"
    );
}
