//! Creating requests and folders: Bruno's file names, defaults and seq values.

use crate::tree_support;

use apinata_core::{ItemPath, TreeError};
use tree_support::{
    ROOT, disk_seq, exists, folder, ip, item, new_request_bytes, open, read, reload, request,
    tree_parity,
};

#[test]
fn a_new_request_has_the_reference_bytes_and_seq() {
    // Folders and request-type items count; app items and unknown kinds do not, and an
    // unknown kind's seq is not passed either.
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("f/folder.yml", &folder("f", 2)),
        ("ui.yml", &item("app", "ui", 1)),
        ("x.yml", &item("custom", "x", 4)),
    ]);
    let created = c
        .create_request(&ItemPath::root(), "Get users")
        .expect("creates");
    assert_eq!(created, ip("Get users.yml"));
    assert_eq!(
        read(&dir, "Get users.yml"),
        new_request_bytes("new-request")
    );
    let r = c.request(&created).expect("in the tree");
    assert_eq!(r.seq(), Some(3));
    assert!(!r.is_dirty());
    assert!(
        reload(&dir, &mut c, "Get users.yml").is_empty(),
        "its own write is not a change"
    );
}

#[test]
fn graphql_siblings_count_toward_the_seq() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("q.yml", &item("graphql", "q", 1)),
    ]);
    c.create_request(&ItemPath::root(), "r").expect("creates");
    assert_eq!(disk_seq(&dir, "r.yml"), Some(2));
}

#[test]
fn a_taken_file_name_gets_brunos_numeric_suffix() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("Get users.yml", &request("Get users", 1)),
        ("Get users 1.yml", &request("Get users", 2)),
    ]);
    let created = c
        .create_request(&ItemPath::root(), "Get users")
        .expect("creates");
    assert_eq!(created, ip("Get users 2.yml"));
    assert_eq!(disk_seq(&dir, "Get users 2.yml"), Some(3));
}

#[test]
fn the_file_name_is_the_sanitised_display_name() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    let created = c
        .create_request(&ItemPath::root(), "a/b?")
        .expect("creates");
    assert_eq!(created, ip("a-b-.yml"));
    assert_eq!(c.request(&created).expect("in the tree").name(), "a/b?");
    assert!(exists(&dir, "a-b-.yml"));
}

#[test]
fn invalid_names_are_rejected_before_anything_is_written() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    // The last name is 200 characters but 400 bytes, past the file system's limit.
    let too_long = "\u{e9}".repeat(200);
    for name in [
        "",
        "   ",
        "...",
        "CON",
        "folder",
        "collection",
        too_long.as_str(),
    ] {
        let err = c.create_request(&ItemPath::root(), name).unwrap_err();
        assert!(
            matches!(err, TreeError::InvalidName { .. }),
            "{name:?}: {err:?}"
        );
    }
    assert_eq!(std::fs::read_dir(dir.path()).expect("list").count(), 1);
}

#[test]
fn a_missing_parent_is_not_found() {
    let (_dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    let err = c.create_request(&ip("nope"), "r").unwrap_err();
    assert!(matches!(err, TreeError::NotFound { .. }), "{err:?}");
}

#[test]
fn a_request_in_a_subfolder_is_numbered_among_its_siblings() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("f/folder.yml", &folder("f", 2)),
    ]);
    let created = c.create_request(&ip("f"), "x").expect("creates");
    assert_eq!(created, ip("f/x.yml"));
    assert_eq!(disk_seq(&dir, "f/x.yml"), Some(1));
    assert!(c.request(&created).is_some());
}

#[test]
fn a_new_folder_has_brunos_folder_file() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("b.yml", &request("b", 2)),
        ("f/folder.yml", &folder("f", 3)),
    ]);
    let created = c
        .create_folder(&ItemPath::root(), "Admin")
        .expect("creates");
    assert_eq!(created, ip("Admin"));
    assert_eq!(read(&dir, "Admin/folder.yml"), tree_parity("new-folder"));
    let f = c.folder(&created).expect("in the tree");
    assert_eq!((f.name(), f.seq()), ("Admin", Some(4)));
    assert!(
        reload(&dir, &mut c, "Admin").is_empty()
            && reload(&dir, &mut c, "Admin/folder.yml").is_empty()
    );
}

#[test]
fn folder_names_get_suffixes_and_the_root_reserves_environments() {
    let (_dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("Admin/folder.yml", &folder("Admin", 1)),
    ]);
    assert_eq!(
        c.create_folder(&ItemPath::root(), "Admin")
            .expect("creates"),
        ip("Admin 1")
    );
    let err = c
        .create_folder(&ItemPath::root(), "My Environments")
        .unwrap_err();
    assert!(matches!(err, TreeError::InvalidName { .. }), "{err:?}");
    assert_eq!(
        c.create_folder(&ip("Admin"), "environments")
            .expect("creates"),
        ip("Admin/environments")
    );
}
