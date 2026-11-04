//! Tag edits: `folder.yml` tags match Bruno's bytes, setters normalise, and the shared
//! normaliser is public.

use crate::config_parity;

use apinata_core::{Collection, ItemPath, normalize_tags};
use std::fs;
use tempfile::TempDir;

const CORPUS: &str = "2026-10-21-folder-tags-parity";

fn item(path: &str) -> ItemPath {
    ItemPath::new(path).expect("valid item path")
}

fn strings(tags: &[&str]) -> Vec<String> {
    tags.iter().map(|t| (*t).to_owned()).collect()
}

fn collection_with(folder_yml: Option<&str>) -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    fs::write(
        dir.path().join("opencollection.yml"),
        "opencollection: 1.0.0\ninfo:\n  name: c\n",
    )
    .expect("root");
    fs::create_dir(dir.path().join("f")).expect("mkdir");
    if let Some(text) = folder_yml {
        fs::write(dir.path().join("f/folder.yml"), text).expect("folder.yml");
    }
    dir
}

#[test]
fn folder_tag_edits_match_bruno() {
    config_parity::check_corpus(CORPUS, "folder-tags-", |id, c| {
        let file = c.folder_file_mut(&item("folder_1")).expect("folder file");
        match id {
            "folder-tags-add" => file.set_tags(strings(&["smoke", "v2"])),
            "folder-tags-normalised" => file.set_tags(strings(&["  smoke ", "", "v2", "smoke"])),
            other => panic!("no Apinata edit for {other}"),
        }
        file.save().expect("saves");
    });
}

#[test]
fn normalize_tags_trims_drops_empties_and_keeps_the_first_of_each() {
    assert_eq!(
        normalize_tags(strings(&[" a ", "", "b", "a", "A"])),
        strings(&["a", "b", "A"])
    );
}

#[test]
fn folder_tags_round_trip_and_clearing_removes_the_key() {
    let dir = collection_with(Some("info:\n  name: f\n  type: folder\n"));
    let mut c = Collection::open(dir.path()).expect("opens");
    let file = c.folder_file_mut(&item("f")).expect("file");
    file.set_tags(strings(&["x", " y "]));
    assert_eq!(file.tags(), strings(&["x", "y"]));
    file.save().expect("saves");
    let c = Collection::open(dir.path()).expect("reopens");
    assert_eq!(
        c.folder(&item("f"))
            .expect("folder")
            .file()
            .expect("file")
            .tags(),
        strings(&["x", "y"])
    );
    let mut c = c;
    let file = c.folder_file_mut(&item("f")).expect("file");
    file.set_tags(Vec::new());
    file.save().expect("saves");
    assert!(
        !fs::read_to_string(dir.path().join("f/folder.yml"))
            .unwrap()
            .contains("tags")
    );
}

#[test]
fn unchanged_folder_tags_leave_the_document_clean() {
    let dir = collection_with(Some(
        "info:\n  name: f\n  type: folder\n  tags:\n    - a\n    - 7\n",
    ));
    let mut c = Collection::open(dir.path()).expect("opens");
    let file = c.folder_file_mut(&item("f")).expect("file");
    file.set_tags(file.tags());
    assert!(!file.is_dirty());
}

#[test]
fn request_set_tags_normalises() {
    let dir = collection_with(None);
    fs::write(
        dir.path().join("r.yml"),
        "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: u\n",
    )
    .expect("request");
    let mut c = Collection::open(dir.path()).expect("opens");
    let r = c.request_mut(&item("r.yml")).expect("request");
    r.set_tags(strings(&[" a", "a", ""]));
    assert_eq!(r.tags(), strings(&["a"]));
}
