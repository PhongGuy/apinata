//! The folder, collection and environment edit corpus
//! `2026-10-09-config-edit-parity`: for each case id, the bytes Bruno writes to
//! each file the edit touches. Inputs are the collections normalised by the edit-parity corpus.
#![allow(dead_code)] // each test binary uses a different subset

use apinata_core::Collection;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const CONFIG_CORPUS: &str = "2026-10-09-config-edit-parity";

pub fn corpus_dir() -> PathBuf {
    research_dir(CONFIG_CORPUS)
}

fn research_dir(corpus: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reference")
        .join(corpus)
}

/// The normalised collections this corpus edits.
pub fn inputs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reference/2026-10-09-edit-parity/normalized")
}

#[derive(serde::Deserialize)]
pub struct Case {
    pub id: String,
    pub collection: String,
    pub files: Vec<String>,
}

pub fn all_cases() -> Vec<Case> {
    corpus_cases(CONFIG_CORPUS)
}

fn corpus_cases(corpus: &str) -> Vec<Case> {
    let text = fs::read_to_string(research_dir(corpus).join("cases.json")).expect("cases.json");
    serde_json::from_str(&text).expect("valid cases.json")
}

/// The cases whose id starts with `prefix`; panics when there are none, so a typo cannot pass.
pub fn cases(prefix: &str) -> Vec<Case> {
    corpus_selection(CONFIG_CORPUS, prefix)
}

fn corpus_selection(corpus: &str, prefix: &str) -> Vec<Case> {
    let selected: Vec<Case> = corpus_cases(corpus)
        .into_iter()
        .filter(|c| c.id.starts_with(prefix))
        .collect();
    assert!(!selected.is_empty(), "no parity cases start with {prefix}");
    selected
}

/// Bruno's bytes for `file` after edit `id`.
pub fn expected(id: &str, file: &str) -> String {
    corpus_expected(CONFIG_CORPUS, id, file)
}

fn corpus_expected(corpus: &str, id: &str, file: &str) -> String {
    fs::read_to_string(research_dir(corpus).join("expected").join(id).join(file))
        .expect("expected bytes")
}

/// A copy of the normalised `collection` in a fresh directory, so a save never touches the corpus.
pub fn copy_collection(collection: &str) -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    copy_tree(&inputs_dir().join(collection), dir.path());
    dir
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("mkdir");
    for entry in fs::read_dir(from).expect("read dir") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("copy");
        }
    }
}

/// For every case under `prefix`: open a copy of its collection, apply `edit` (which saves), and
/// compare each file the case lists with Bruno's bytes. Every mismatch is reported at once.
pub fn check(prefix: &str, edit: impl Fn(&str, &mut Collection)) {
    check_corpus(CONFIG_CORPUS, prefix, edit);
}

/// [`check`] against the named parity corpus. A corpus with its own `normalized/`
/// directory takes its inputs from there.
pub fn check_corpus(corpus: &str, prefix: &str, edit: impl Fn(&str, &mut Collection)) {
    let selected = corpus_selection(corpus, prefix);
    let mut failures = Vec::new();
    for case in &selected {
        let own = research_dir(corpus)
            .join("normalized")
            .join(&case.collection);
        let dir = if own.is_dir() {
            let dir = TempDir::new().expect("tempdir");
            copy_tree(&own, dir.path());
            dir
        } else {
            copy_collection(&case.collection)
        };
        let mut collection = Collection::open(dir.path()).expect("normalised collection opens");
        edit(&case.id, &mut collection);
        for file in &case.files {
            let actual = fs::read_to_string(dir.path().join(file)).unwrap_or_default();
            let expected = corpus_expected(corpus, &case.id, file);
            if actual != expected {
                failures.push(format!(
                    "{} {file}\n  expected {expected:?}\n  actual   {actual:?}",
                    case.id
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches over {} cases:\n{}",
        failures.len(),
        selected.len(),
        failures.join("\n")
    );
}
