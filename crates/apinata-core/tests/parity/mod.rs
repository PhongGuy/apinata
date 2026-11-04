//! The edit parity corpus `2026-10-09-edit-parity`: fixtures normalised by
//! Bruno's writer, and for each case id the bytes Bruno writes after the matching edit.
#![allow(dead_code)] // each test binary uses a different subset

use apinata_core::RequestFile;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

pub fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/reference/2026-10-09-edit-parity")
}

#[derive(serde::Deserialize)]
pub struct Case {
    pub id: String,
    pub fixture: String,
}

pub fn all_cases() -> Vec<Case> {
    let text = fs::read_to_string(corpus_dir().join("cases.json")).expect("cases.json");
    serde_json::from_str(&text).expect("valid cases.json")
}

/// The cases whose id starts with `prefix`; panics when there are none, so a typo cannot pass.
pub fn cases(prefix: &str) -> Vec<Case> {
    let selected: Vec<Case> = all_cases()
        .into_iter()
        .filter(|c| c.id.starts_with(prefix))
        .collect();
    assert!(!selected.is_empty(), "no parity cases start with {prefix}");
    selected
}

/// The normalised fixture's text.
pub fn normalized(fixture: &str) -> String {
    fs::read_to_string(corpus_dir().join("normalized").join(fixture)).expect("normalised fixture")
}

/// Bruno's bytes after edit `id`.
pub fn expected(id: &str) -> String {
    fs::read_to_string(corpus_dir().join("expected").join(format!("{id}.yml")))
        .expect("expected bytes")
}

/// A copy of the normalised `fixture` in a fresh directory, so a save never touches the corpus.
pub fn copy_fixture(fixture: &str) -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("tempdir");
    let name = Path::new(fixture).file_name().expect("file name");
    let path = dir.path().join(name);
    fs::write(&path, normalized(fixture)).expect("write copy");
    (dir, path)
}

/// For every case under `prefix`: load a copy of its fixture, apply `edit`, save, and compare the
/// file with Bruno's bytes. Every mismatch is reported at once.
pub fn check_requests(prefix: &str, edit: impl Fn(&str, &mut RequestFile)) {
    let selected = cases(prefix);
    let mut failures = Vec::new();
    for case in &selected {
        let (_dir, path) = copy_fixture(&case.fixture);
        let mut request = RequestFile::load(&path).expect("normalised fixture loads");
        edit(&case.id, &mut request);
        request.save().expect("saves");
        let actual = fs::read_to_string(&path).expect("read back");
        let expected = expected(&case.id);
        if actual != expected {
            failures.push(format!(
                "{}\n  expected {expected:?}\n  actual   {actual:?}",
                case.id
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases differ:\n{}",
        failures.len(),
        selected.len(),
        failures.join("\n")
    );
}

/// A request file holding `text` in a fresh directory.
pub fn file_with(text: &str) -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("r.yml");
    fs::write(&path, text).expect("write");
    (dir, path)
}

/// The file at `path`, parsed.
pub fn saved(path: &Path) -> yaml_serde::Value {
    yaml_serde::from_str(&fs::read_to_string(path).expect("read")).expect("parses")
}

/// A mapping's keys in order.
pub fn keys(value: &yaml_serde::Value) -> Vec<String> {
    value
        .as_mapping()
        .expect("mapping")
        .keys()
        .map(|k| k.as_str().expect("string key").to_owned())
        .collect()
}
