//! Folder and collection names and folder seq: Bruno's bytes on normalised files, minimal
//! edits elsewhere, and a `folder.yml` created on first save.

use crate::config_parity;

use apinata_core::{Collection, ItemPath, SaveError};
use config_parity::check;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn item(path: &str) -> ItemPath {
    ItemPath::new(path).expect("valid item path")
}

/// A collection with one folder per `(dir, folder.yml text)`; `None` leaves the file out.
fn collection_with(folders: &[(&str, Option<&str>)]) -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    fs::write(
        dir.path().join("opencollection.yml"),
        "opencollection: 1.0.0\ninfo:\n  name: c\n",
    )
    .expect("root");
    for (name, text) in folders {
        fs::create_dir(dir.path().join(name)).expect("mkdir");
        if let Some(text) = text {
            fs::write(dir.path().join(name).join("folder.yml"), text).expect("folder.yml");
        }
    }
    dir
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).expect("read")
}

#[test]
fn folder_names_and_seqs_match_bruno() {
    check("folder-name", |_, c| {
        let file = c
            .folder_file_mut(&item("response-time"))
            .expect("folder file");
        file.set_name("renamed");
        file.save().expect("saves");
    });
    check("folder-seq-", |id, c| {
        let (path, seq) = match id {
            "folder-seq-edit" => ("folder_1", 4),
            "folder-seq-add" => ("response-time", 2),
            other => panic!("unexpected case {other}"),
        };
        let file = c.folder_file_mut(&item(path)).expect("folder file");
        file.set_seq(seq);
        file.save().expect("saves");
    });
}

#[test]
fn the_collection_name_matches_bruno() {
    check("collection-name", |_, c| {
        c.file_mut().set_name("renamed");
        c.file_mut().save().expect("saves");
    });
}

#[test]
fn names_and_seqs_read_through_the_edited_files() {
    let dir = collection_with(&[("f", Some("info:\n  name: Old\n  type: folder\n  seq: 2\n"))]);
    let mut c = Collection::open(dir.path()).expect("opens");
    assert_eq!(c.name(), "c");
    c.file_mut().set_name("New collection");
    let file = c.folder_file_mut(&item("f")).expect("folder file");
    file.set_name("New folder");
    file.set_seq(5);
    assert_eq!(c.name(), "New collection");
    let folder = c.folder(&item("f")).expect("folder");
    assert_eq!(folder.name(), "New folder");
    assert_eq!(folder.seq(), Some(5));
    assert_eq!(folder.file().expect("file").name(), Some("New folder"));
    assert!(c.file().is_dirty());
}

#[test]
fn a_folder_without_a_file_gets_one_on_first_save() {
    let dir = collection_with(&[("bare", None), ("numbered", None)]);
    let mut c = Collection::open(dir.path()).expect("opens");
    assert!(c.folder(&item("bare")).expect("folder").file().is_none());

    let file = c.folder_file_mut(&item("bare")).expect("created");
    assert!(file.is_dirty());
    assert_eq!(file.name(), Some("bare"));
    assert!(
        !dir.path().join("bare/folder.yml").exists(),
        "nothing is written before save"
    );
    file.save().expect("saves");
    assert_eq!(
        read(&dir.path().join("bare/folder.yml")),
        "info:\n  name: bare\n  type: folder\n"
    );

    let file = c.folder_file_mut(&item("numbered")).expect("created");
    file.set_seq(3);
    file.save().expect("saves");
    assert_eq!(
        read(&dir.path().join("numbered/folder.yml")),
        "info:\n  name: numbered\n  type: folder\n  seq: 3\n"
    );
}

#[test]
fn a_created_file_never_replaces_one_that_appeared_on_disk() {
    let dir = collection_with(&[("bare", None)]);
    let mut c = Collection::open(dir.path()).expect("opens");
    let file = c.folder_file_mut(&item("bare")).expect("created");
    fs::write(
        dir.path().join("bare/folder.yml"),
        "info:\n  name: theirs\n",
    )
    .expect("write");
    assert!(matches!(file.save(), Err(SaveError::Conflict { .. })));
    assert_eq!(
        read(&dir.path().join("bare/folder.yml")),
        "info:\n  name: theirs\n"
    );
}

#[test]
fn the_root_unknown_paths_and_broken_files_have_no_folder_file() {
    let dir = collection_with(&[("broken", Some("info: [unclosed\n"))]);
    let mut c = Collection::open(dir.path()).expect("opens");
    assert!(c.folder_file_mut(&ItemPath::root()).is_none());
    assert!(c.folder_file_mut(&item("missing")).is_none());
    assert!(c.folder_file_mut(&item("broken")).is_none());
    assert_eq!(
        read(&dir.path().join("broken/folder.yml")),
        "info: [unclosed\n"
    );
}

#[test]
fn a_minimal_edit_keeps_unknown_keys_and_their_order() {
    let text =
        "info:\n  type: folder\n  name: f\n  x-extra: 1\nx-top: keep\nrequest:\n  x-note: n\n";
    let dir = collection_with(&[("f", Some(text))]);
    let mut c = Collection::open(dir.path()).expect("opens");
    let file = c.folder_file_mut(&item("f")).expect("folder file");
    file.set_seq(2);
    file.save().expect("saves");
    assert_eq!(
        read(&dir.path().join("f/folder.yml")),
        "info:\n  type: folder\n  name: f\n  seq: 2\n  x-extra: 1\nx-top: keep\n\nrequest:\n  x-note: n\n"
    );
}

#[test]
fn debug_shows_the_path_not_the_content() {
    let dir = collection_with(&[("f", Some("info:\n  name: f\ndocs: secret-docs\n"))]);
    let c = Collection::open(dir.path()).expect("opens");
    let shown = format!("{:?} {:?}", c.file(), c.folder(&item("f")).unwrap().file());
    assert!(shown.contains("opencollection.yml"), "{shown}");
    assert!(!shown.contains("secret-docs"), "{shown}");
}
