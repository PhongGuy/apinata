//! Reloading `folder.yml` files and directories.

use crate::tree_support;

use apinata_core::{ChangeKind::*, ItemKind::*};
use std::fs;
use tree_support::{BROKEN, ROOT, errors_for, folder, ip, one, open, reload, request, write};

#[test]
fn a_folder_file_reloads_seq_only_or_modified() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("f", 1)),
    ]);
    write(&dir, "f/folder.yml", &folder("f", 3));
    assert_eq!(
        reload(&dir, &mut c, "f/folder.yml"),
        one("f/folder.yml", SeqOnly, Folder)
    );
    assert_eq!(c.folder(&ip("f")).expect("f").seq(), Some(3));
    write(&dir, "f/folder.yml", &folder("Renamed", 3));
    assert_eq!(
        reload(&dir, &mut c, "f/folder.yml"),
        one("f/folder.yml", Modified, Folder)
    );
    assert_eq!(c.folder(&ip("f")).expect("f").name(), "Renamed");
    assert!(reload(&dir, &mut c, "f/folder.yml").is_empty());
}

#[test]
fn a_broken_folder_file_becomes_a_load_error_and_recovers() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("Named", 1)),
    ]);
    write(&dir, "f/folder.yml", BROKEN);
    assert_eq!(
        reload(&dir, &mut c, "f/folder.yml"),
        one("f/folder.yml", Removed, Folder)
    );
    assert_eq!(c.folder(&ip("f")).expect("f").name(), "f");
    assert_eq!(errors_for(&dir, &c, "f/folder.yml"), 1);
    write(&dir, "f/folder.yml", &folder("Named", 1));
    assert_eq!(
        reload(&dir, &mut c, "f/folder.yml"),
        one("f/folder.yml", Added, Folder)
    );
    assert_eq!(errors_for(&dir, &c, "f/folder.yml"), 0);
}

#[test]
fn directories_are_added_and_removed_once() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("f", 1)),
    ]);
    fs::create_dir(dir.path().join("g")).expect("mkdir");
    write(&dir, "g/x.yml", &request("x", 1));
    assert_eq!(reload(&dir, &mut c, "g"), one("g", Added, Folder));
    assert!(c.request(&ip("g/x.yml")).is_some());
    assert!(reload(&dir, &mut c, "g").is_empty());
    fs::remove_dir_all(dir.path().join("f")).expect("rm");
    assert_eq!(reload(&dir, &mut c, "f"), one("f", Removed, Folder));
    assert!(c.folder(&ip("f")).is_none());
}

#[test]
fn the_root_environments_directory_is_not_a_folder() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    fs::create_dir(dir.path().join("environments")).expect("mkdir");
    write(&dir, "environments/x.yml", &request("x", 1));
    assert!(reload(&dir, &mut c, "environments").is_empty());
    assert!(c.folder(&ip("environments")).is_none());
    assert!(c.request(&ip("environments/x.yml")).is_none());
}

#[test]
fn paths_outside_the_collection_or_ignored_produce_nothing() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    let other = tempfile::TempDir::new().expect("tempdir");
    std::fs::write(other.path().join("a.yml"), request("a", 1)).expect("write");
    assert!(c.reload_path(&other.path().join("a.yml")).is_empty());
    for rel in [".git/a.yml", "node_modules/a.yml", "notes.txt"] {
        let path = dir.path().join(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, request("a", 1)).expect("write");
        assert!(reload(&dir, &mut c, rel).is_empty(), "{rel}");
    }
}

#[test]
fn an_unsaved_folder_file_draft_survives_a_missing_file() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("f/x.yml", &request("x", 1))]);
    c.folder_file_mut(&ip("f")).expect("stub").set_name("Draft");
    assert!(reload(&dir, &mut c, "f/folder.yml").is_empty());
    assert_eq!(c.folder(&ip("f")).expect("f").name(), "Draft");
}

#[test]
fn an_unsaved_folder_file_draft_conflicts_with_a_new_file() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("f/x.yml", &request("x", 1))]);
    c.folder_file_mut(&ip("f")).expect("stub").set_name("Draft");
    write(&dir, "f/folder.yml", &folder("Disk", 1));
    assert_eq!(
        reload(&dir, &mut c, "f/folder.yml"),
        one("f/folder.yml", Conflict, Folder)
    );
    assert_eq!(c.folder(&ip("f")).expect("f").name(), "Draft");
}

#[test]
fn a_dirty_folder_file_keeps_its_draft_as_a_conflict() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("f", 1)),
    ]);
    c.folder_file_mut(&ip("f")).expect("file").set_name("Draft");
    write(&dir, "f/folder.yml", &folder("Renamed", 1));
    assert_eq!(
        reload(&dir, &mut c, "f/folder.yml"),
        one("f/folder.yml", Conflict, Folder)
    );
    assert_eq!(c.folder(&ip("f")).expect("f").name(), "Draft");
}

#[test]
fn a_dirty_folder_file_survives_its_file_breaking_until_it_parses_again() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("f", 1)),
    ]);
    c.folder_file_mut(&ip("f")).expect("file").set_name("Draft");
    write(&dir, "f/folder.yml", BROKEN);
    assert_eq!(
        reload(&dir, &mut c, "f/folder.yml"),
        one("f/folder.yml", Conflict, Folder)
    );
    assert_eq!(errors_for(&dir, &c, "f/folder.yml"), 1);
    assert_eq!(c.folder(&ip("f")).expect("f").name(), "Draft");
    let file = c.folder_file_mut(&ip("f")).expect("draft kept");
    assert!(file.is_dirty());
    assert!(matches!(
        file.save(),
        Err(apinata_core::SaveError::Conflict { .. })
    ));

    write(&dir, "f/folder.yml", &folder("f", 1));
    assert_eq!(
        reload(&dir, &mut c, "f/folder.yml"),
        one("f/folder.yml", Restored, Folder)
    );
    assert_eq!(errors_for(&dir, &c, "f/folder.yml"), 0);
    c.folder_file_mut(&ip("f"))
        .expect("file")
        .save()
        .expect("saves over the restored file");
    assert_eq!(c.folder(&ip("f")).expect("f").name(), "Draft");
}
