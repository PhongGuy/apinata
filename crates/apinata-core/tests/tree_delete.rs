//! Deleting items: Bruno's renumbering of the remaining siblings, conflicts and I/O failures.

use crate::tree_support;

use apinata_core::{ItemPath, TreeError};
#[cfg(unix)]
use std::fs;
use tree_support::{
    BROKEN, ROOT, disk_seq, exists, folder, ip, item, open, read, reload, request, tree_parity,
    write, yaml,
};

#[test]
fn delete_renumbers_folders_and_request_types_only() {
    // Not renumbered: an app item, an unknown kind, an unparseable file and a folder whose
    // folder.yml fails to load.
    let custom = item("custom", "x", 9);
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("b.yml", &request("b", 2)),
        ("c.yml", &request("c", 3)),
        ("f/folder.yml", &folder("f", 4)),
        ("q.yml", &item("grpc", "q", 5)),
        ("ui.yml", &item("app", "ui", 6)),
        ("x.yml", &custom),
        ("bad.yml", BROKEN),
        ("g/folder.yml", BROKEN),
    ]);
    let mut expected_q = yaml(&dir, "q.yml");
    expected_q["info"]["seq"] = yaml_serde::Value::from(4);
    c.delete(&ip("b.yml")).expect("deletes");
    assert!(!exists(&dir, "b.yml") && c.request(&ip("b.yml")).is_none());
    let seqs: Vec<_> = ["a.yml", "c.yml", "f", "q.yml", "ui.yml"]
        .iter()
        .map(|r| disk_seq(&dir, r))
        .collect();
    assert_eq!(seqs, [Some(1), Some(2), Some(3), Some(4), Some(6)]);
    assert_eq!(
        yaml(&dir, "q.yml"),
        expected_q,
        "an opaque item gets only its seq edited"
    );
    assert_eq!(
        (read(&dir, "x.yml"), read(&dir, "bad.yml")),
        (custom, BROKEN.to_owned())
    );
    assert_eq!(read(&dir, "g/folder.yml"), BROKEN);
    assert_eq!(c.request(&ip("c.yml")).expect("c").seq(), Some(2));
    assert_eq!(c.folder(&ip("f")).expect("f").seq(), Some(3));
    assert!(
        reload(&dir, &mut c, "c.yml").is_empty(),
        "renumber writes are not echoed"
    );
    assert!(reload(&dir, &mut c, "b.yml").is_empty());
}

#[test]
fn unchanged_siblings_are_not_rewritten() {
    let a = format!("# keep this comment\n{}", request("a", 1));
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &a),
        ("b.yml", &request("b", 2)),
        ("c.yml", &request("c", 3)),
    ]);
    c.delete(&ip("c.yml")).expect("deletes");
    assert_eq!(read(&dir, "a.yml"), a);
    assert_eq!(read(&dir, "b.yml"), request("b", 2));
}

#[test]
fn a_folder_without_a_folder_file_gets_brunos_minimal_one() {
    // Bruno's order is a(1), plain (no seq), c(3); without c, plain becomes 2.
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("c.yml", &request("c", 3)),
        ("plain/x.yml", &request("x", 1)),
    ]);
    c.delete(&ip("c.yml")).expect("deletes");
    assert_eq!(
        read(&dir, "plain/folder.yml"),
        tree_parity("folder-file-from-seq")
    );
    assert_eq!(c.folder(&ip("plain")).expect("plain").seq(), Some(2));
}

#[test]
fn deleting_a_folder_removes_its_whole_tree() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("f", 1)),
        ("f/g/x.yml", &request("x", 1)),
    ]);
    c.delete(&ip("f")).expect("deletes");
    assert!(!exists(&dir, "f"));
    assert!(c.folder(&ip("f")).is_none() && c.request(&ip("f/g/x.yml")).is_none());
}

#[test]
fn a_renumbered_dirty_sibling_keeps_its_draft() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("b.yml", &request("b", 2)),
    ]);
    c.request_mut(&ip("b.yml")).expect("b").set_method("POST");
    c.delete(&ip("a.yml")).expect("deletes");
    assert_eq!(disk_seq(&dir, "b.yml"), Some(1));
    assert!(
        read(&dir, "b.yml").contains("method: GET"),
        "the draft is not saved"
    );
    let b = c.request_mut(&ip("b.yml")).expect("b");
    assert_eq!((b.seq(), b.method(), b.is_dirty()), (Some(1), "POST", true));
    b.save().expect("saves without conflict");
}

#[test]
fn a_sibling_changed_on_disk_stops_the_delete_before_any_write() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("b.yml", &request("b", 2)),
    ]);
    write(
        &dir,
        "b.yml",
        &request("b", 2).replace("x.test/b", "x.test/other"),
    );
    match c.delete(&ip("a.yml")).unwrap_err() {
        TreeError::Conflict { path } => assert_eq!(path, dir.path().join("b.yml")),
        other => panic!("expected Conflict, got {other:?}"),
    }
    assert!(exists(&dir, "a.yml"));
}

#[test]
fn the_root_cannot_be_deleted_and_a_missing_item_is_not_found() {
    let (_dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    assert!(matches!(
        c.delete(&ItemPath::root()),
        Err(TreeError::InvalidMove { .. })
    ));
    assert!(matches!(
        c.delete(&ip("nope.yml")),
        Err(TreeError::NotFound { .. })
    ));
}

#[cfg(unix)]
#[test]
fn an_io_error_mid_renumber_reports_the_path_and_reloads_the_tree() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("b.yml", &request("b", 2)),
        ("f/folder.yml", &folder("f", 3)),
        ("g.yml", &request("g", 4)),
    ]);
    let locked = dir.path().join("f");
    let _g = crate::common::ModeGuard::set(&locked, 0o555);
    if fs::write(locked.join("probe"), "").is_ok() {
        // Running as root: permissions do not block writes, so the failure cannot be staged.
        fs::remove_file(locked.join("probe")).expect("cleanup");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("chmod");
        return;
    }
    let err = c.delete(&ip("a.yml")).unwrap_err();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("chmod");
    match err {
        TreeError::DeletedRenumber { source } => match *source {
            TreeError::Io { path, .. } => assert_eq!(path, locked.join("folder.yml")),
            other => panic!("expected Io inside, got {other:?}"),
        },
        other => panic!("expected DeletedRenumber, got {other:?}"),
    }
    assert!(!exists(&dir, "a.yml"));
    assert_eq!(
        c.folder(&ip("f")).expect("reloaded").seq(),
        disk_seq(&dir, "f")
    );
    for rel in ["b.yml", "g.yml"] {
        assert_eq!(
            c.request(&ip(rel)).expect("reloaded").seq(),
            disk_seq(&dir, rel),
            "{rel}"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_delete_that_fails_part_way_reloads_the_tree() {
    use std::os::unix::fs::PermissionsExt;
    // A read-only parent lets every file inside `p/f` go and fails only the final removal of
    // `f` itself, whatever order the directory is read in.
    let names = ["a.yml", "b.yml", "g/x.yml"];
    let mut files: Vec<(String, String)> = vec![
        ("opencollection.yml".into(), ROOT.into()),
        ("p/f/folder.yml".into(), folder("f", 1)),
    ];
    for name in names {
        files.push((format!("p/f/{name}"), request("r", 1)));
    }
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(p, t)| (p.as_str(), t.as_str()))
        .collect();
    let (dir, mut c) = open(&refs);
    let locked = dir.path().join("p");
    let _g = crate::common::ModeGuard::set(&locked, 0o555);
    if fs::write(locked.join("probe"), "").is_ok() {
        // Running as root: permissions do not block deletes, so the failure cannot be staged.
        fs::remove_file(locked.join("probe")).expect("cleanup");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("chmod");
        return;
    }
    let err = c.delete(&ip("p/f")).unwrap_err();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("chmod");
    assert!(matches!(err, TreeError::Io { .. }), "{err:?}");
    assert!(exists(&dir, "p/f") && c.folder(&ip("p/f")).is_some());
    for name in names {
        let rel = format!("p/f/{name}");
        assert!(!exists(&dir, &rel), "{rel} is gone from disk");
        assert!(c.request(&ip(&rel)).is_none(), "{rel} is gone from memory");
    }
    assert!(c.folder(&ip("p/f/g")).is_none());
}
