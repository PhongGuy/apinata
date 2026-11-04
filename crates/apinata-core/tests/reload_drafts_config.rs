//! Dirty `folder.yml`, environment and `opencollection.yml` drafts outlive a removed or broken
//! file.

use crate::tree_support;

use apinata_core::{ChangeKind::*, DiskState, ItemKind, SaveError};
use std::fs;
use tree_support::{BROKEN, ROOT, errors_for, folder, ip, one, open, read, reload, write};

const ENV: &str = "name: dev\nvariables:\n  - name: host\n    value: a\n";

#[test]
fn a_dirty_folder_file_outlives_its_removal_and_saving_recreates_it() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("f", 1)),
    ]);
    c.folder_file_mut(&ip("f")).expect("f").set_name("Draft");
    fs::remove_file(dir.path().join("f/folder.yml")).expect("rm");
    let changes = reload(&dir, &mut c, "f/folder.yml");
    assert_eq!(changes, one("f/folder.yml", Conflict, ItemKind::Folder));
    assert!(
        reload(&dir, &mut c, "f/folder.yml").is_empty(),
        "reported once"
    );
    assert_eq!(c.folder(&ip("f")).expect("f").name(), "Draft");
    let file = c.folder_file_mut(&ip("f")).expect("draft kept");
    assert_eq!(file.disk_state(), DiskState::Removed);
    file.save().expect("recreates");
    assert!(read(&dir, "f/folder.yml").contains("name: Draft"));
}

#[test]
fn a_dirty_folder_file_outlives_a_broken_file_and_recovers() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("f", 1)),
    ]);
    c.folder_file_mut(&ip("f")).expect("f").set_name("Draft");
    write(&dir, "f/folder.yml", BROKEN);
    let changes = reload(&dir, &mut c, "f/folder.yml");
    assert_eq!(changes, one("f/folder.yml", Conflict, ItemKind::Folder));
    assert_eq!(errors_for(&dir, &c, "f/folder.yml"), 1);
    let file = c
        .folder_file_mut(&ip("f"))
        .expect("draft kept and still editable");
    assert_eq!(file.disk_state(), DiskState::Broken);
    assert!(matches!(file.save(), Err(SaveError::Conflict { .. })));
    write(&dir, "f/folder.yml", &folder("f", 1));
    let changes = reload(&dir, &mut c, "f/folder.yml");
    assert_eq!(changes, one("f/folder.yml", Restored, ItemKind::Folder));
    assert_eq!(errors_for(&dir, &c, "f/folder.yml"), 0);
    let file = c.folder_file_mut(&ip("f")).expect("f");
    assert_eq!(
        (file.name(), file.disk_state()),
        (Some("Draft"), DiskState::Present)
    );
}

#[test]
fn a_dirty_environment_outlives_its_removal_and_saving_recreates_it() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("environments/dev.yml", ENV)]);
    let env = |kind| one("environments/dev.yml", kind, ItemKind::Environment);
    c.environment_mut("dev")
        .expect("dev")
        .set_color(Some("#ff0000".into()));
    fs::remove_dir_all(dir.path().join("environments")).expect("rm");
    assert_eq!(reload(&dir, &mut c, "environments/dev.yml"), env(Conflict));
    assert!(reload(&dir, &mut c, "environments/dev.yml").is_empty());
    let dev = c.environment_mut("dev").expect("kept");
    assert_eq!(dev.disk_state(), DiskState::Removed);
    dev.save().expect("recreates the file and environments/");
    assert!(read(&dir, "environments/dev.yml").contains("#ff0000"));
}

#[test]
fn a_dirty_environment_outlives_a_broken_file() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("environments/dev.yml", ENV)]);
    let env = |kind| one("environments/dev.yml", kind, ItemKind::Environment);
    c.environment_mut("dev")
        .expect("dev")
        .set_color(Some("#ff0000".into()));
    write(&dir, "environments/dev.yml", BROKEN);
    assert_eq!(reload(&dir, &mut c, "environments/dev.yml"), env(Conflict));
    assert_eq!(errors_for(&dir, &c, "environments/dev.yml"), 1);
    let dev = c.environment("dev").expect("kept");
    assert_eq!(dev.disk_state(), DiskState::Broken);
    assert_eq!(dev.color().as_deref(), Some("#ff0000"));
}

#[test]
fn a_clean_environment_still_leaves_with_its_file() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("environments/dev.yml", ENV)]);
    write(&dir, "environments/dev.yml", BROKEN);
    let changes = reload(&dir, &mut c, "environments/dev.yml");
    assert_eq!(
        changes,
        one("environments/dev.yml", Removed, ItemKind::Environment)
    );
    assert!(c.environment("dev").is_none());
}

#[test]
fn a_dirty_collection_file_is_marked_broken_and_restored() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    let root = |kind| one("opencollection.yml", kind, ItemKind::Collection);
    c.file_mut().set_name("Draft");
    write(&dir, "opencollection.yml", BROKEN);
    assert_eq!(reload(&dir, &mut c, "opencollection.yml"), root(Conflict));
    assert!(reload(&dir, &mut c, "opencollection.yml").is_empty());
    assert_eq!(c.file().disk_state(), DiskState::Broken);
    write(&dir, "opencollection.yml", ROOT);
    assert_eq!(reload(&dir, &mut c, "opencollection.yml"), root(Restored));
    assert_eq!(
        (c.name(), c.file().disk_state()),
        ("Draft", DiskState::Present)
    );
}

#[test]
fn a_clean_broken_collection_file_keeps_its_content_unmarked() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    write(&dir, "opencollection.yml", BROKEN);
    assert!(reload(&dir, &mut c, "opencollection.yml").is_empty());
    assert_eq!((c.name(), c.file().disk_state()), ("c", DiskState::Present));
    assert_eq!(errors_for(&dir, &c, "opencollection.yml"), 1);
}

#[test]
fn a_conflict_is_reported_again_after_the_environment_returns_to_saved_bytes() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("environments/dev.yml", ENV)]);
    let env = |kind| one("environments/dev.yml", kind, ItemKind::Environment);
    let other = ENV.replace("value: a", "value: b");
    c.environment_mut("dev")
        .expect("dev")
        .set_color(Some("#ff0000".into()));
    write(&dir, "environments/dev.yml", &other);
    assert_eq!(reload(&dir, &mut c, "environments/dev.yml"), env(Conflict));
    write(&dir, "environments/dev.yml", ENV);
    assert!(reload(&dir, &mut c, "environments/dev.yml").is_empty());
    write(&dir, "environments/dev.yml", &other);
    assert_eq!(reload(&dir, &mut c, "environments/dev.yml"), env(Conflict));
}

#[test]
fn a_conflict_is_reported_again_after_the_folder_file_returns_to_saved_bytes() {
    let saved = folder("f", 1);
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("f/folder.yml", &saved)]);
    let item = |kind| one("f/folder.yml", kind, ItemKind::Folder);
    let other = folder("g", 1);
    c.folder_file_mut(&ip("f")).expect("f").set_name("Draft");
    write(&dir, "f/folder.yml", &other);
    assert_eq!(reload(&dir, &mut c, "f/folder.yml"), item(Conflict));
    write(&dir, "f/folder.yml", &saved);
    assert!(reload(&dir, &mut c, "f/folder.yml").is_empty());
    write(&dir, "f/folder.yml", &other);
    assert_eq!(reload(&dir, &mut c, "f/folder.yml"), item(Conflict));
}

#[test]
fn a_conflict_is_reported_again_after_the_collection_file_returns_to_saved_bytes() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    let root = |kind| one("opencollection.yml", kind, ItemKind::Collection);
    let other = ROOT.replace("name: c", "name: other");
    c.file_mut().set_name("Draft");
    write(&dir, "opencollection.yml", &other);
    assert_eq!(reload(&dir, &mut c, "opencollection.yml"), root(Conflict));
    write(&dir, "opencollection.yml", ROOT);
    assert!(reload(&dir, &mut c, "opencollection.yml").is_empty());
    write(&dir, "opencollection.yml", &other);
    assert_eq!(reload(&dir, &mut c, "opencollection.yml"), root(Conflict));
}
