//! `Collection::file_texts` and `text_hunks`: the disk and draft text a Compare dialog shows.

use crate::tree_support;

use apinata_core::{LineKind, text_hunks};
use tree_support::{ROOT, changed_url, ip, open, reload, request, write};

#[test]
fn a_changed_request_gives_the_disk_text_and_the_serialized_draft() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request("a", 1))]);
    c.request_mut(&ip("a.yml")).expect("a").set_method("POST");
    write(&dir, "a.yml", &changed_url("a", 1));
    reload(&dir, &mut c, "a.yml");
    let texts = c.file_texts(&dir.path().join("a.yml")).expect("texts");
    assert_eq!(texts.disk.as_deref(), Some(changed_url("a", 1).as_str()));
    assert!(texts.draft.contains("POST"), "{}", texts.draft);
    assert!(!texts.draft.contains("changed"), "{}", texts.draft);
}

#[test]
fn a_removed_file_has_no_disk_text() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request("a", 1))]);
    // A draft keeps its request in the collection after the file goes.
    c.request_mut(&ip("a.yml")).expect("a").set_method("POST");
    std::fs::remove_file(dir.path().join("a.yml")).unwrap();
    reload(&dir, &mut c, "a.yml");
    let texts = c.file_texts(&dir.path().join("a.yml")).expect("texts");
    assert_eq!(texts.disk, None);
}

#[test]
fn an_unknown_path_has_no_texts() {
    let (dir, c) = open(&[("opencollection.yml", ROOT)]);
    assert!(c.file_texts(&dir.path().join("nope.yml")).is_none());
}

#[test]
fn text_hunks_number_the_changed_lines() {
    let hunks = text_hunks("a\nb\nc\n", "a\nB\nc\n");
    assert_eq!(hunks.len(), 1);
    let kinds: Vec<_> = hunks[0].lines.iter().map(|l| l.kind).collect();
    assert!(kinds.contains(&LineKind::Added) && kinds.contains(&LineKind::Removed));
    assert!(text_hunks("same\n", "same\n").is_empty());
}
