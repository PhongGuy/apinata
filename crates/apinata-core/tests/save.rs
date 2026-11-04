//! Setters touch only their key, insert new keys where Bruno writes them, and save refuses external changes.

use apinata_core::{RequestFile, SaveError};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const HAND_WRITTEN: &str = "# a comment Bruno would drop\ninfo:\n  name: r\n  type: http\n  description: d\nhttp:\n  method: GET\n  url: 'https://example.test/a'\n";

fn file_with(text: &str) -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("r.yml");
    fs::write(&path, text).expect("write");
    (dir, path)
}

fn info_keys(path: &Path) -> Vec<String> {
    let value: yaml_serde::Value =
        yaml_serde::from_str(&fs::read_to_string(path).expect("read")).expect("parses");
    value["info"]
        .as_mapping()
        .expect("info map")
        .keys()
        .map(|k| k.as_str().expect("str key").to_owned())
        .collect()
}

#[test]
fn loading_and_saving_without_edits_writes_nothing() {
    let (_dir, path) = file_with(HAND_WRITTEN);
    let mut r = RequestFile::load(&path).expect("loads");
    assert!(!r.is_dirty());
    r.save().expect("saves");
    assert_eq!(fs::read_to_string(&path).expect("read"), HAND_WRITTEN);
}

#[test]
fn setting_the_same_value_writes_nothing() {
    let (_dir, path) = file_with(HAND_WRITTEN);
    let mut r = RequestFile::load(&path).expect("loads");
    r.set_url("https://example.test/a");
    assert!(!r.is_dirty());
    r.save().expect("saves");
    assert_eq!(fs::read_to_string(&path).expect("read"), HAND_WRITTEN);
}

#[test]
fn setting_a_value_back_writes_nothing() {
    let (_dir, path) = file_with(HAND_WRITTEN);
    let mut r = RequestFile::load(&path).expect("loads");
    r.set_url("https://example.test/changed");
    assert!(r.is_dirty());
    r.set_url("https://example.test/a");
    assert!(!r.is_dirty());
    r.save().expect("saves");
    assert_eq!(fs::read_to_string(&path).expect("read"), HAND_WRITTEN);
}

#[test]
fn a_url_edit_on_a_bruno_file_writes_brunos_bytes() {
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bruno/cli-docs/request_2.yml");
    let (_dir, path) = file_with(&fs::read_to_string(source).expect("fixture"));
    let mut r = RequestFile::load(&path).expect("loads");
    r.set_url("{{host}}/two");
    assert_eq!(r.url(), "{{host}}/two");
    r.save().expect("saves");
    assert_eq!(
        fs::read_to_string(&path).expect("read"),
        "info:\n  name: request_2\n  type: http\n  seq: 2\n\nhttp:\n  method: GET\n  url: \"{{host}}/two\"\n"
    );
}

#[test]
fn an_edit_to_a_hand_written_file_changes_only_the_targeted_value() {
    let (_dir, path) = file_with(HAND_WRITTEN);
    let mut r = RequestFile::load(&path).expect("loads");
    r.set_method("POST");
    r.save().expect("saves");
    let saved = fs::read_to_string(&path).expect("read");
    assert!(!saved.contains("# a comment"));
    let mut expected: yaml_serde::Value = yaml_serde::from_str(HAND_WRITTEN).expect("parses");
    expected["http"]["method"] = yaml_serde::Value::from("POST");
    let after: yaml_serde::Value = yaml_serde::from_str(&saved).expect("parses");
    assert_eq!(after, expected);
    // Mapping equality ignores order, so compare key order through a canonical dump too.
    assert_eq!(
        yaml_serde::to_string(&after).expect("dump"),
        yaml_serde::to_string(&expected).expect("dump")
    );
}

#[test]
fn set_seq_inserts_seq_after_type_and_keeps_unknown_keys_in_place() {
    let (_dir, path) = file_with(
        "info:\n  name: r\n  type: http\n  x-custom: keep\n  description: d\nhttp:\n  method: GET\n  url: u\n",
    );
    let mut r = RequestFile::load(&path).expect("loads");
    r.set_seq(3);
    r.save().expect("saves");
    assert_eq!(
        info_keys(&path),
        ["name", "type", "seq", "x-custom", "description"]
    );
    assert_eq!(RequestFile::load(&path).expect("reloads").seq(), Some(3));
}

#[test]
fn a_new_key_with_no_predecessor_goes_before_its_first_successor() {
    let (_dir, path) = file_with("info:\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: u\n");
    let mut r = RequestFile::load(&path).expect("loads");
    r.set_name("named");
    r.save().expect("saves");
    assert_eq!(info_keys(&path), ["name", "type", "seq"]);
}

#[test]
fn save_refuses_when_the_file_changed_on_disk() {
    let (_dir, path) = file_with(HAND_WRITTEN);
    let mut r = RequestFile::load(&path).expect("loads");
    let external = HAND_WRITTEN.replace("description: d", "description: changed elsewhere");
    fs::write(&path, &external).expect("external write");
    r.set_url("https://example.test/b");
    assert!(matches!(r.save(), Err(SaveError::Conflict { .. })));
    assert_eq!(fs::read_to_string(&path).expect("read"), external);
}

#[test]
fn two_edit_and_save_cycles_both_succeed() {
    let (_dir, path) = file_with(HAND_WRITTEN);
    let mut r = RequestFile::load(&path).expect("loads");
    r.set_url("https://example.test/b");
    r.save().expect("first save");
    assert!(!r.is_dirty());
    r.set_url("https://example.test/c");
    r.save().expect("second save");
    assert_eq!(
        RequestFile::load(&path).expect("reloads").url(),
        "https://example.test/c"
    );
}

#[test]
fn save_leaves_no_temporary_files() {
    let (dir, path) = file_with(HAND_WRITTEN);
    let mut r = RequestFile::load(&path).expect("loads");
    r.set_url("https://example.test/b");
    r.save().expect("saves");
    let entries: Vec<String> = fs::read_dir(dir.path())
        .expect("list")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(entries, ["r.yml"]);
}

#[cfg(unix)]
#[test]
fn saving_through_a_symlink_keeps_the_link() {
    let (dir, target) = file_with(HAND_WRITTEN);
    let link = dir.path().join("link.yml");
    std::os::unix::fs::symlink(&target, &link).expect("symlink");
    let mut r = RequestFile::load(&link).expect("loads");
    r.set_url("https://example.test/b");
    r.save().expect("saves");
    assert!(fs::symlink_metadata(&link).expect("meta").is_symlink());
    assert!(
        fs::read_to_string(&target)
            .expect("read")
            .contains("example.test/b")
    );
}

#[cfg(unix)]
#[test]
fn save_keeps_the_file_mode() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, path) = file_with(HAND_WRITTEN);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("chmod");
    let mut r = RequestFile::load(&path).expect("loads");
    r.set_url("https://example.test/b");
    r.save().expect("saves");
    let mode = fs::metadata(&path).expect("meta").permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
}
