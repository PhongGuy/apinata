//! Drafts: unsaved requests held in memory until saved under a chosen name and folder.

use crate::tree_support;

use apinata_core::{Collection, DropTarget, ItemPath, SaveError, TreeError};
use std::path::Path;
use tempfile::TempDir;
use tree_support::{ROOT, disk_seq, folder, ip, open, read, reload, request};

fn tree() -> (TempDir, Collection) {
    open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("users/folder.yml", &folder("users", 2)),
        ("users/u.yml", &request("u", 1)),
    ])
}

fn files(root: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let rel = path.strip_prefix(root).unwrap();
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

#[test]
fn a_draft_lives_in_memory_only() {
    let (dir, mut c) = tree();
    let before = files(dir.path());
    let d = c.create_draft(&ip("users")).expect("a draft");
    assert!(d.is_draft(), "{d:?}");
    assert_eq!(d.parent(), Some(ip("users")));
    assert!(c.is_draft(&d));
    let r = c.request(&d).expect("looked up like a request");
    assert_eq!(r.name(), "Untitled request");
    assert_eq!(r.method(), "GET");
    assert_eq!(r.url(), "");
    assert!(!r.is_dirty(), "an untouched draft is clean");
    c.request_mut(&d).unwrap().set_url("http://x/1");
    assert!(c.request(&d).unwrap().is_dirty());
    let other = c.create_draft(&ItemPath::root()).expect("a second draft");
    assert_ne!(d, other);
    assert_eq!(c.drafts().count(), 2);
    assert_eq!(files(dir.path()), before, "nothing written");
    let err = c.create_draft(&ip("nope")).unwrap_err();
    assert!(matches!(err, TreeError::NotFound { .. }), "{err:?}");
}

#[test]
fn a_draft_is_planned_alone_and_never_in_runs() {
    let (dir, mut c) = tree();
    let d = c.create_draft(&ip("users")).unwrap();
    let root = c.root().to_path_buf();
    let alone = c.plan(&d.to_path(&root)).expect("plans");
    assert_eq!(alone.items().cloned().collect::<Vec<_>>(), vec![d.clone()]);
    let all = c.plan(&root).expect("plans");
    assert!(all.items().all(|i| !i.is_draft()));
    let users = c.plan(&dir.path().join("users")).expect("plans");
    assert_eq!(
        users.items().cloned().collect::<Vec<_>>(),
        vec![ip("users/u.yml")]
    );
}

#[test]
fn a_saved_draft_inherits_the_default_timeout() {
    let (dir, mut c) = tree();
    let d = c.create_draft(&ItemPath::root()).unwrap();
    c.save_draft(&d, &ItemPath::root(), "b").expect("saves");
    assert!(read(&dir, "b.yml").contains("\n  timeout: inherit\n"));
}

#[test]
fn saving_a_draft_writes_the_same_bytes_as_create_then_edit() {
    let (dir, mut c) = tree();
    let d = c.create_draft(&ItemPath::root()).unwrap();
    c.request_mut(&d).unwrap().set_url("http://x/list");
    let saved = c.save_draft(&d, &ip("users"), "List users").expect("saves");
    assert_eq!(saved, ip("users/List users.yml"));
    assert!(!c.is_draft(&d));
    assert!(c.request(&d).is_none());
    let r = c.request(&saved).expect("in the tree");
    assert_eq!(r.name(), "List users");
    assert_eq!(r.url(), "http://x/list");
    assert!(!r.is_dirty());
    assert_eq!(disk_seq(&dir, "users/List users.yml"), Some(2));
    assert!(files(dir.path()).iter().all(|f| !f.ends_with(".draft")));
    assert!(reload(&dir, &mut c, "users/List users.yml").is_empty());

    let (twin_dir, mut twin) = tree();
    let made = twin.create_request(&ip("users"), "List users").unwrap();
    twin.request_mut(&made).unwrap().set_url("http://x/list");
    twin.request_mut(&made).unwrap().save().unwrap();
    assert_eq!(
        read(&dir, "users/List users.yml"),
        read(&twin_dir, "users/List users.yml")
    );
}

#[test]
fn a_taken_name_is_numbered_and_a_refused_save_keeps_the_draft() {
    let (_dir, mut c) = tree();
    let d = c.create_draft(&ItemPath::root()).unwrap();
    assert_eq!(
        c.save_draft(&d, &ItemPath::root(), "a").unwrap(),
        ip("a 1.yml")
    );
    let d = c.create_draft(&ItemPath::root()).unwrap();
    let err = c.save_draft(&d, &ItemPath::root(), "folder").unwrap_err();
    assert!(matches!(err, TreeError::InvalidName { .. }), "{err:?}");
    assert!(c.is_draft(&d), "a refused save keeps the draft");
    let err = c.save_draft(&d, &ip("nope"), "x").unwrap_err();
    assert!(matches!(err, TreeError::NotFound { .. }), "{err:?}");
    assert!(c.is_draft(&d));
}

#[test]
fn tree_operations_refuse_drafts_and_discard_removes_them() {
    let (dir, mut c) = tree();
    let before = files(dir.path());
    let d = c.create_draft(&ip("users")).unwrap();
    c.request_mut(&d).unwrap().set_url("http://x/1");
    assert!(c.rename(&d, "x", true).is_err());
    assert!(c.clone_request(&d, "x").is_err());
    assert!(c.delete(&d).is_err());
    assert_eq!(files(dir.path()), before, "nothing written");
    assert!(c.remove_draft(&d));
    assert!(c.request(&d).is_none());
    assert!(!c.remove_draft(&d));
}

#[test]
fn drafts_follow_folder_renames_and_moves_and_go_with_a_deleted_folder() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("users", 1)),
        ("admin/folder.yml", &folder("admin", 2)),
    ]);
    let d = c.create_draft(&ip("users")).unwrap();
    c.request_mut(&d).unwrap().set_url("http://x/1");
    let renamed = c.rename(&ip("users"), "people", true).expect("renames");
    let moved = renamed.join(d.file_name()).unwrap();
    assert!(c.request(&d).is_none());
    assert_eq!(c.request(&moved).expect("rebased").url(), "http://x/1");
    assert_eq!(
        c.request(&moved).unwrap().path(),
        moved.to_path(c.root()).as_path(),
        "the draft's disk path follows, so folder context does too"
    );
    let saved = c.save_draft(&moved, &renamed, "In people").expect("saves");
    assert!(dir.path().join(saved.as_str()).exists());
    let other = c.create_draft(&ip("admin")).unwrap();
    c.delete(&ip("admin")).expect("deletes");
    assert!(
        c.request(&other).is_none(),
        "a deleted folder takes its drafts"
    );
    assert_eq!(c.drafts().count(), 0);
}

#[test]
fn revert_resets_a_draft_in_memory() {
    let (dir, mut c) = tree();
    let before = files(dir.path());
    let d = c.create_draft(&ItemPath::root()).unwrap();
    c.request_mut(&d).unwrap().set_url("http://x/1");
    let path = c.request(&d).unwrap().path().to_path_buf();
    c.revert(&path).expect("reverts");
    let r = c.request(&d).expect("still a draft");
    assert_eq!(r.url(), "");
    assert!(!r.is_dirty());
    assert_eq!(files(dir.path()), before);
}

#[test]
fn a_rescan_keeps_drafts() {
    let (_dir, mut c) = tree();
    let dirty = c.create_draft(&ip("users")).unwrap();
    c.request_mut(&dirty).unwrap().set_url("http://x/1");
    let clean = c.create_draft(&ItemPath::root()).unwrap();
    c.reload();
    assert_eq!(c.request(&dirty).unwrap().url(), "http://x/1");
    assert!(c.request(&clean).is_some(), "a clean draft is not dropped");
}

#[test]
fn no_save_path_writes_a_draft() {
    let (dir, mut c) = tree();
    let before = files(dir.path());
    let d = c.create_draft(&ip("users")).unwrap();
    c.request_mut(&d).unwrap().set_url("http://x/1");
    let err = c.request_mut(&d).unwrap().save().unwrap_err();
    assert!(matches!(err, SaveError::Io { .. }), "{err:?}");
    let path = c.request(&d).unwrap().path().to_path_buf();
    assert!(c.save_overwrite(&path).is_err());
    assert!(c.request(&d).unwrap().is_dirty(), "the edit is kept");
    assert_eq!(files(dir.path()), before, "nothing written");
}

#[test]
fn a_moved_folder_takes_its_drafts_and_a_draft_cannot_be_moved() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("users", 1)),
        ("admin/folder.yml", &folder("admin", 2)),
    ]);
    let d = c.create_draft(&ip("users")).unwrap();
    c.request_mut(&d).unwrap().set_url("http://x/1");
    let before = files(dir.path());
    let err = c
        .move_item(&d, DropTarget::Inside(ip("admin")))
        .unwrap_err();
    assert!(matches!(err, TreeError::NotFound { .. }), "{err:?}");
    assert_eq!(files(dir.path()), before, "nothing written");
    let moved = c
        .move_item(&ip("users"), DropTarget::Inside(ip("admin")))
        .expect("moves");
    let rebased = moved.join(d.file_name()).unwrap();
    assert!(c.request(&d).is_none());
    assert_eq!(c.request(&rebased).expect("rebased").url(), "http://x/1");
    assert_eq!(
        c.request(&rebased).unwrap().path(),
        rebased.to_path(c.root()).as_path()
    );
}

#[test]
fn only_cores_own_draft_names_are_drafts() {
    assert!(!ip("x.draft").is_draft());
    assert!(!ip("untitled-.draft").is_draft());
    assert!(ip("f/untitled-12.draft").is_draft());
}

#[test]
fn each_open_draft_gets_its_own_default_name() {
    let (_dir, mut c) = tree();
    let name = |c: &Collection, d: &ItemPath| c.request(d).unwrap().name().to_owned();
    let first = c.create_draft(&ItemPath::root()).unwrap();
    let second = c.create_draft(&ItemPath::root()).unwrap();
    let third = c.create_draft(&ip("users")).unwrap();
    assert_eq!(name(&c, &first), "Untitled request");
    assert_eq!(name(&c, &second), "Untitled request 2");
    assert_eq!(name(&c, &third), "Untitled request 3", "across folders");
    assert!(
        !c.request(&second).unwrap().is_dirty(),
        "the name is the template"
    );
    assert!(c.remove_draft(&first));
    let again = c.create_draft(&ItemPath::root()).unwrap();
    assert_eq!(
        name(&c, &again),
        "Untitled request",
        "a freed name is reused"
    );
}
