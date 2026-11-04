//! A removed directory keeps a folder for the dirty drafts under it; directory changes list
//! the item files they cover.

use crate::tree_support;

use apinata_core::{ChangeKind::*, Collection, DiskState, ItemKind::*};
use std::fs;
use tempfile::TempDir;
use tree_support::{ROOT, exists, folder, ip, item, open, read, reload, request, write};

/// `f/` with a folder file, a clean and a dirty request, an opaque item and a clean
/// sub-folder; `f/g/` with a dirty request.
fn tree() -> (TempDir, Collection) {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("Named", 1)),
        ("f/clean.yml", &request("clean", 1)),
        ("f/dirty.yml", &request("dirty", 2)),
        ("f/q.yml", &item("grpc", "q", 3)),
        ("f/h/x.yml", &request("x", 1)),
        ("f/g/deep.yml", &request("deep", 1)),
    ]);
    for rel in ["f/dirty.yml", "f/g/deep.yml"] {
        c.request_mut(&ip(rel)).expect(rel).set_method("POST");
    }
    (dir, c)
}

fn rel_items(dir: &TempDir, c: &apinata_core::Change) -> Vec<String> {
    c.items
        .iter()
        .map(|p| {
            p.strip_prefix(dir.path())
                .expect("inside")
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

#[test]
fn a_removed_directory_keeps_only_its_dirty_drafts() {
    let (dir, mut c) = tree();
    fs::remove_dir_all(dir.path().join("f")).expect("rm");
    let changes = c.reload_path(&dir.path().join("f"));
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert_eq!((changes[0].kind, changes[0].item), (Removed, Folder));
    assert_eq!(
        rel_items(&dir, &changes[0]),
        [
            "f/clean.yml",
            "f/dirty.yml",
            "f/folder.yml",
            "f/g/deep.yml",
            "f/h/x.yml",
            "f/q.yml"
        ]
    );
    let f = c.folder(&ip("f")).expect("kept");
    assert_eq!((f.disk_state(), f.name()), (DiskState::Removed, "f"));
    assert!(f.file().is_none(), "a clean folder file goes");
    assert!(f.opaque_items().is_empty());
    let names: Vec<_> = f.requests().iter().map(|r| r.file_name()).collect();
    assert_eq!(names, ["dirty.yml"]);
    assert_eq!(f.requests()[0].disk_state(), DiskState::Removed);
    let subfolders: Vec<_> = f.folders().iter().map(|g| g.dir_name()).collect();
    assert_eq!(subfolders, ["g"], "a sub-folder without drafts goes");
    assert_eq!(f.folders()[0].disk_state(), DiskState::Removed);
    assert!(c.load_errors().is_empty());
    assert!(reload(&dir, &mut c, "f").is_empty(), "reported once");
    assert!(c.reload().is_empty(), "a full reload reports nothing new");
    assert!(c.folder(&ip("f/g")).is_some());
}

#[test]
fn a_dirty_folder_file_keeps_its_folder() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("f", 1)),
    ]);
    c.folder_file_mut(&ip("f")).expect("f").set_name("Draft");
    fs::remove_dir_all(dir.path().join("f")).expect("rm");
    reload(&dir, &mut c, "f");
    let f = c.folder(&ip("f")).expect("kept");
    assert_eq!((f.disk_state(), f.name()), (DiskState::Removed, "Draft"));
    assert_eq!(f.file().expect("draft").disk_state(), DiskState::Removed);
}

#[test]
fn saving_a_kept_draft_recreates_its_directories_and_the_reload_restores_them() {
    let (dir, mut c) = tree();
    fs::remove_dir_all(dir.path().join("f")).expect("rm");
    reload(&dir, &mut c, "f");
    c.request_mut(&ip("f/g/deep.yml"))
        .expect("kept")
        .save()
        .expect("recreates");
    assert!(read(&dir, "f/g/deep.yml").contains("method: POST"));
    write(&dir, "f/new.yml", &request("new", 9));
    let changes = reload(&dir, &mut c, "f");
    assert_eq!(
        changes,
        vec![
            ("f".to_owned(), Restored, Folder),
            ("f/g".to_owned(), Restored, Folder),
            ("f/new.yml".to_owned(), Added, Request),
        ]
    );
    let restored = c.reload_path(&dir.path().join("f"));
    assert!(restored.is_empty(), "{restored:?}");
    let f = c.folder(&ip("f")).expect("f");
    assert_eq!(f.disk_state(), DiskState::Present);
    assert_eq!(
        c.folder(&ip("f/g")).expect("g").disk_state(),
        DiskState::Present
    );
    let dirty = c
        .request(&ip("f/dirty.yml"))
        .expect("still a removed draft");
    assert_eq!(dirty.disk_state(), DiskState::Removed);
}

#[test]
fn an_added_directory_lists_its_item_files() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    fs::create_dir_all(dir.path().join("n/m")).expect("mkdir");
    write(&dir, "n/folder.yml", &folder("n", 1));
    write(&dir, "n/a.yml", &request("a", 1));
    write(&dir, "n/m/b.yml", &request("b", 1));
    let changes = c.reload_path(&dir.path().join("n"));
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert_eq!(changes[0].kind, Added);
    assert_eq!(
        rel_items(&dir, &changes[0]),
        ["n/a.yml", "n/folder.yml", "n/m/b.yml"]
    );
}

#[test]
fn a_file_change_lists_no_items() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    write(&dir, "a.yml", &request("a", 1));
    let changes = c.reload_path(&dir.path().join("a.yml"));
    assert!(changes[0].items.is_empty());
}

#[test]
fn a_sibling_delete_does_not_renumber_a_kept_removed_folder() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request("r", 1)),
        ("f/folder.yml", &folder("f", 2)),
        ("f/a.yml", &request("a", 1)),
    ]);
    c.request_mut(&ip("f/a.yml")).expect("a").set_method("POST");
    fs::remove_dir_all(dir.path().join("f")).expect("rm");
    reload(&dir, &mut c, "f");
    c.delete(&ip("r.yml"))
        .expect("the kept folder is not on disk to renumber");
    assert!(!dir.path().join("f").exists(), "no directory recreated");
    assert!(c.folder(&ip("f")).is_some());
}

#[test]
fn a_sibling_delete_does_not_recreate_a_removed_request_draft() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("b.yml", &request("b", 2)),
    ]);
    c.request_mut(&ip("b.yml")).expect("b").set_method("POST");
    fs::remove_file(dir.path().join("b.yml")).expect("rm");
    reload(&dir, &mut c, "b.yml");
    c.delete(&ip("a.yml")).expect("deletes");
    assert!(
        !dir.path().join("b.yml").exists(),
        "the removed draft stays unsaved"
    );
    let b = c.request(&ip("b.yml")).expect("kept");
    assert_eq!(b.disk_state(), DiskState::Removed);
    assert!(b.is_dirty());
}

#[test]
fn renaming_a_removed_request_draft_fails_before_touching_anything() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("f/a.yml", &request("a", 1)),
        ("b.yml", &request("b", 1)),
    ]);
    for rel in ["f/a.yml", "b.yml"] {
        c.request_mut(&ip(rel)).expect(rel).set_method("POST");
    }
    fs::remove_dir_all(dir.path().join("f")).expect("rm dir");
    fs::remove_file(dir.path().join("b.yml")).expect("rm file");
    reload(&dir, &mut c, "f");
    reload(&dir, &mut c, "b.yml");
    for (rel, rename_file) in [("f/a.yml", false), ("f/a.yml", true), ("b.yml", false)] {
        let err = c.rename(&ip(rel), "renamed", rename_file);
        assert!(err.is_err(), "{rel}: {err:?}");
        assert!(!exists(&dir, rel), "{rel} is not recreated");
        let r = c.request(&ip(rel)).expect("kept");
        assert_eq!(
            (r.disk_state(), r.name()),
            (DiskState::Removed, rel_name(rel))
        );
        assert!(r.is_dirty());
    }
    assert!(!exists(&dir, "f"), "no directory recreated");
    assert!(!exists(&dir, "f/renamed.yml"));
}

fn rel_name(rel: &str) -> &str {
    rel.trim_start_matches("f/").trim_end_matches(".yml")
}
