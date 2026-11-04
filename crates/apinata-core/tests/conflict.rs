//! `revert` and `save_overwrite`: settling a draft against a changed, removed or broken file.

use crate::tree_support;

use apinata_core::{Collection, DiskState, SaveError};
use std::fs;
use tempfile::TempDir;
use tree_support::{
    BROKEN, ROOT, changed_url, errors_for, folder, ip, open, read, reload, request, write,
};

const ENV: &str = "name: dev\nvariables:\n  - name: host\n    value: a\n";

/// A collection holding `a.yml`, with an unsaved method edit.
fn dirty_a() -> (TempDir, Collection) {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request("a", 1))]);
    c.request_mut(&ip("a.yml")).expect("a").set_method("POST");
    (dir, c)
}

#[test]
fn revert_takes_the_changed_file() {
    let (dir, mut c) = dirty_a();
    write(&dir, "a.yml", &changed_url("a", 1));
    reload(&dir, &mut c, "a.yml");
    c.revert(&dir.path().join("a.yml")).expect("reverts");
    let a = c.request(&ip("a.yml")).expect("a");
    assert_eq!((a.method(), a.url()), ("GET", "https://x.test/changed"));
    assert!(!a.is_dirty());
}

#[test]
fn reverting_a_clean_unchanged_request_changes_nothing() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request("a", 1))]);
    c.revert(&dir.path().join("a.yml")).expect("reverts");
    assert_eq!(
        c.request(&ip("a.yml")).expect("a").disk_state(),
        DiskState::Present
    );
}

#[test]
fn save_overwrite_writes_the_draft_over_the_changed_file() {
    let (dir, mut c) = dirty_a();
    write(&dir, "a.yml", &changed_url("a", 1));
    reload(&dir, &mut c, "a.yml");
    c.save_overwrite(&dir.path().join("a.yml"))
        .expect("overwrites");
    let disk = read(&dir, "a.yml");
    assert!(
        disk.contains("method: POST") && disk.contains("x.test/a"),
        "{disk}"
    );
    assert!(!c.request(&ip("a.yml")).expect("a").is_dirty());
    assert!(reload(&dir, &mut c, "a.yml").is_empty(), "own write");
}

#[test]
fn reverting_a_removed_request_drops_it() {
    let (dir, mut c) = dirty_a();
    fs::remove_file(dir.path().join("a.yml")).expect("rm");
    reload(&dir, &mut c, "a.yml");
    c.revert(&dir.path().join("a.yml")).expect("reverts");
    assert!(c.request(&ip("a.yml")).is_none());
    assert!(c.load_errors().is_empty());
}

#[test]
fn reverting_the_last_draft_under_a_removed_directory_drops_its_folders() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("f/g/a.yml", &request("a", 1)),
        ("f/g/b.yml", &request("b", 2)),
    ]);
    for rel in ["f/g/a.yml", "f/g/b.yml"] {
        c.request_mut(&ip(rel)).expect(rel).set_method("POST");
    }
    fs::remove_dir_all(dir.path().join("f")).expect("rm");
    reload(&dir, &mut c, "f");
    c.revert(&dir.path().join("f/g/a.yml")).expect("reverts");
    assert!(c.folder(&ip("f/g")).is_some(), "b is still a draft");
    c.revert(&dir.path().join("f/g/b.yml")).expect("reverts");
    assert!(c.folder(&ip("f")).is_none());
}

#[test]
fn a_broken_request_is_reverted_away_or_overwritten() {
    let (dir, mut c) = dirty_a();
    write(&dir, "a.yml", BROKEN);
    reload(&dir, &mut c, "a.yml");
    c.save_overwrite(&dir.path().join("a.yml"))
        .expect("overwrites");
    assert!(read(&dir, "a.yml").contains("method: POST"));
    assert_eq!(errors_for(&dir, &c, "a.yml"), 0);
    assert_eq!(
        c.request(&ip("a.yml")).expect("a").disk_state(),
        DiskState::Present
    );

    c.request_mut(&ip("a.yml")).expect("a").set_method("PUT");
    write(&dir, "a.yml", BROKEN);
    reload(&dir, &mut c, "a.yml");
    c.revert(&dir.path().join("a.yml")).expect("reverts");
    assert!(c.request(&ip("a.yml")).is_none());
    assert_eq!(errors_for(&dir, &c, "a.yml"), 1);
}

#[test]
fn a_broken_folder_file_is_reverted_away_or_overwritten() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("f", 1)),
    ]);
    let path = dir.path().join("f/folder.yml");
    c.folder_file_mut(&ip("f")).expect("f").set_name("Draft");
    write(&dir, "f/folder.yml", BROKEN);
    reload(&dir, &mut c, "f/folder.yml");
    c.save_overwrite(&path).expect("overwrites");
    assert!(read(&dir, "f/folder.yml").contains("name: Draft"));
    assert_eq!(errors_for(&dir, &c, "f/folder.yml"), 0);

    c.folder_file_mut(&ip("f")).expect("f").set_name("Again");
    write(&dir, "f/folder.yml", BROKEN);
    reload(&dir, &mut c, "f/folder.yml");
    c.revert(&path).expect("reverts");
    let f = c.folder(&ip("f")).expect("f");
    assert!(f.file().is_none());
    assert_eq!(f.name(), "f");
    assert!(
        c.folder_file_mut(&ip("f")).is_none(),
        "a broken file is not replaced by edits"
    );
}

#[test]
fn a_removed_environment_is_reverted_away_or_recreated() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("environments/dev.yml", ENV)]);
    let path = dir.path().join("environments/dev.yml");
    c.environment_mut("dev")
        .expect("dev")
        .set_color(Some("#ff0000".into()));
    fs::remove_file(&path).expect("rm");
    reload(&dir, &mut c, "environments/dev.yml");
    c.save_overwrite(&path).expect("recreates");
    assert!(read(&dir, "environments/dev.yml").contains("#ff0000"));
    assert_eq!(
        c.environment("dev").expect("dev").disk_state(),
        DiskState::Present
    );

    c.environment_mut("dev").expect("dev").set_color(None);
    fs::remove_file(&path).expect("rm");
    reload(&dir, &mut c, "environments/dev.yml");
    c.revert(&path).expect("reverts");
    assert!(c.environment("dev").is_none());
}

#[test]
fn a_broken_collection_file_is_reverted_but_stays_marked_or_is_overwritten() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    let path = dir.path().join("opencollection.yml");
    c.file_mut().set_name("Draft");
    write(&dir, "opencollection.yml", BROKEN);
    reload(&dir, &mut c, "opencollection.yml");
    c.revert(&path).expect("reverts");
    assert_eq!((c.name(), c.file().is_dirty()), ("c", false));
    assert_eq!(c.file().disk_state(), DiskState::Broken);
    c.save_overwrite(&path).expect("overwrites");
    assert!(read(&dir, "opencollection.yml").contains("name: c"));
    assert_eq!(c.file().disk_state(), DiskState::Present);
    assert_eq!(errors_for(&dir, &c, "opencollection.yml"), 0);
}

#[test]
fn an_unknown_path_is_an_error() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    for rel in ["missing.yml", ".env", "environments/none.yml", "folder.yml"] {
        let path = dir.path().join(rel);
        let err = c.revert(&path).expect_err(rel);
        assert_eq!(err.path(), path);
        match c.save_overwrite(&path) {
            Err(SaveError::Io { source, .. }) => {
                assert_eq!(source.kind(), std::io::ErrorKind::NotFound, "{rel}")
            }
            other => panic!("{rel}: {other:?}"),
        }
    }
}
