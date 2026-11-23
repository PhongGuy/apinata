//! Renaming a collection: only `info.name` in `opencollection.yml` changes, never the directory.

use crate::config_parity;

use apinata_core::{Collection, CollectionPresets, TreeError};
use config_parity::check;
use std::fs;
use tempfile::TempDir;

const ROOT: &str = "opencollection: 1.0.0\n\ninfo:\n  name: Old\nconfig:\n  proxy:\n    inherit: true\nbundled: false\nextensions: {}\n";

fn open_root(text: &str) -> (TempDir, Collection) {
    let dir = TempDir::new().expect("tempdir");
    fs::write(dir.path().join("opencollection.yml"), text).expect("root");
    let c = Collection::open(dir.path()).expect("opens");
    (dir, c)
}

fn root_text(dir: &TempDir) -> String {
    fs::read_to_string(dir.path().join("opencollection.yml")).expect("read")
}

#[test]
fn a_rename_writes_brunos_bytes() {
    check("collection-name", |_, c| {
        c.rename_collection("renamed").expect("renames");
        assert!(!c.file().is_dirty());
    });
}

#[test]
fn a_rename_changes_only_the_name_line_and_keeps_the_directory() {
    let (dir, mut c) = open_root(ROOT);
    c.rename_collection("  New name  ").expect("renames");
    assert_eq!(root_text(&dir), ROOT.replace("name: Old", "name: New name"));
    assert_eq!(c.name(), "New name");
    assert_eq!(c.root(), dir.path());
    let reopened = Collection::open(dir.path()).expect("reopens");
    assert_eq!(reopened.name(), "New name");
}

#[test]
fn a_rename_keeps_unsaved_edits_unsaved() {
    let (dir, mut c) = open_root(ROOT);
    let presets = CollectionPresets::new("", "http://127.0.0.1:1");
    c.file_mut().set_presets(&presets);
    c.rename_collection("New").expect("renames");
    assert_eq!(root_text(&dir), ROOT.replace("name: Old", "name: New"));
    assert!(c.file().is_dirty(), "the preset edit stays unsaved");
    assert_eq!(c.file().presets().request_url, "http://127.0.0.1:1");
    assert_eq!(c.name(), "New");
}

#[test]
fn a_blank_or_whitespace_name_is_refused_and_writes_nothing() {
    let (dir, mut c) = open_root(ROOT);
    for name in ["", "   ", "\t\n"] {
        let err = c.rename_collection(name).expect_err("refused");
        assert!(
            matches!(&err, TreeError::InvalidName { name: n } if n == name),
            "{err:?}"
        );
    }
    assert_eq!(root_text(&dir), ROOT);
    assert_eq!(c.name(), "Old");
}
