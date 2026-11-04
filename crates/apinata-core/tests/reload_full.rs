//! The full rescan, and I/O failures during reload.

use crate::tree_support;

use apinata_core::{ChangeKind::*, ItemKind::*};
use std::fs;
use tree_support::{
    ROOT, changed_url, errors_for, folder, ip, one, open, reload, request, summary, write,
};

#[test]
fn a_full_reload_reports_each_difference_once_in_path_order() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("c.yml", &request("c", 2)),
        ("f/folder.yml", &folder("f", 3)),
        ("f/x.yml", &request("x", 1)),
        ("f/y.yml", &request("y", 2)),
    ]);
    write(&dir, "a.yml", &changed_url("a", 1));
    write(&dir, "b.yml", &request("b", 4));
    write(&dir, "c.yml", &request("c", 7));
    fs::remove_dir_all(dir.path().join("f")).expect("rm");
    fs::create_dir(dir.path().join("environments")).expect("mkdir");
    write(&dir, "environments/dev.yml", "name: dev\n");
    let expected = [
        one("a.yml", Modified, Request),
        one("b.yml", Added, Request),
        one("c.yml", SeqOnly, Request),
        one("environments/dev.yml", Added, Environment),
        one("f", Removed, Folder),
    ]
    .concat();
    assert_eq!(summary(&dir, &c.reload()), expected);
    assert!(c.reload().is_empty());
    assert!(c.request(&ip("b.yml")).is_some() && c.folder(&ip("f")).is_none());
}

#[test]
fn a_full_reload_keeps_drafts() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request("a", 1))]);
    c.request_mut(&ip("a.yml")).expect("a").set_method("POST");
    write(&dir, "a.yml", &changed_url("a", 1));
    assert_eq!(summary(&dir, &c.reload()), one("a.yml", Conflict, Request));
    assert_eq!(c.request(&ip("a.yml")).expect("a").method(), "POST");
}

#[cfg(unix)]
#[test]
fn an_unreadable_file_becomes_a_load_error() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request("a", 1))]);
    let a = dir.path().join("a.yml");
    write(&dir, "a.yml", &changed_url("a", 1));
    let _g = crate::common::ModeGuard::set(&a, 0o000);
    if fs::read(&a).is_ok() {
        return; // running as root: permissions do not block reads
    }
    let changes = c.reload_path(&a);
    fs::set_permissions(&a, fs::Permissions::from_mode(0o644)).expect("chmod");
    assert_eq!(summary(&dir, &changes), one("a.yml", Removed, Request));
    assert_eq!(errors_for(&dir, &c, "a.yml"), 1);
}

#[test]
fn files_open_loads_are_the_files_a_full_reload_finds() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    write(&dir, ".yml", &request("hidden", 1));
    let reopened = apinata_core::Collection::open(dir.path()).expect("opens");
    assert!(reopened.request(&ip(".yml")).is_some());
    assert_eq!(summary(&dir, &c.reload()), one(".yml", Added, Request));
}

#[test]
fn a_nested_collection_file_is_not_an_item() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("f", 1)),
    ]);
    write(&dir, "f/opencollection.yml", &request("nested", 1));
    assert!(
        c.reload_path(&dir.path().join("f/opencollection.yml"))
            .is_empty()
    );
}

#[cfg(unix)]
#[test]
fn a_directory_read_error_clears_once_it_is_readable() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, _) = open(&[("opencollection.yml", ROOT), ("f/a.yml", &request("a", 1))]);
    let f = dir.path().join("f");
    let _g = crate::common::ModeGuard::set(&f, 0o000);
    if fs::read_dir(&f).is_ok() {
        fs::set_permissions(&f, fs::Permissions::from_mode(0o755)).expect("chmod");
        return; // running as root: permissions do not block reads
    }
    let opened = apinata_core::Collection::open(dir.path());
    fs::set_permissions(&f, fs::Permissions::from_mode(0o755)).expect("chmod");
    let mut c = opened.expect("opens");
    assert_eq!(errors_for(&dir, &c, "f"), 1);
    assert_eq!(summary(&dir, &c.reload()), one("f/a.yml", Added, Request));
    assert_eq!(errors_for(&dir, &c, "f"), 0);
}

const WORKSPACE: &str =
    "opencollection: 1.0.0\ninfo:\n  name: Team\n  type: workspace\ncollections: []\n";

#[test]
fn workspace_yml_at_the_root_is_not_an_item() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("workspace.yml", WORKSPACE),
        ("a.yml", &request("a", 1)),
        ("f/folder.yml", &folder("f", 2)),
        ("f/workspace.yml", &request("workspace", 1)),
    ]);
    assert!(c.opaque_item(&ip("workspace.yml")).is_none());
    assert_eq!(errors_for(&dir, &c, "workspace.yml"), 0);
    assert!(c.request(&ip("a.yml")).is_some());
    // Only the root's file is the workspace's; one in a folder is an ordinary request.
    assert!(c.request(&ip("f/workspace.yml")).is_some());
    assert!(c.reload().is_empty());
    assert!(c.opaque_item(&ip("workspace.yml")).is_none());
}

#[test]
fn a_workspace_yml_appearing_at_the_root_is_not_reloaded_as_an_item() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request("a", 1))]);
    write(&dir, "workspace.yml", WORKSPACE);
    assert!(reload(&dir, &mut c, "workspace.yml").is_empty());
    assert!(c.reload().is_empty());
    assert!(c.opaque_item(&ip("workspace.yml")).is_none());
    assert_eq!(errors_for(&dir, &c, "workspace.yml"), 0);
}
