//! Edge cases of following an operation by its id, and of naming the requests an update adds.

use crate::import_update_support::{FakeBin, bundle, ip, paths};
use apinata_core::import::{
    ImportBundle, SpecBaseline, TakeSpec, UpdateOptions, UpdateSummary, apply_update, plan_update,
    write_collection,
};
use apinata_core::{Collection, MemoryStore, Trash};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// One GET operation: its path, operationId, summary and tag.
type Op<'a> = (&'a str, Option<&'a str>, &'a str, &'a str);

fn spec(ops: &[Op]) -> ImportBundle {
    let paths: Vec<String> = ops
        .iter()
        .map(|(path, id, summary, tag)| {
            let id = id.map_or(String::new(), |id| format!(r#""operationId":"{id}","#));
            format!(r#""{path}":{{"get":{{{id}"summary":"{summary}","tags":["{tag}"]}}}}"#)
        })
        .collect();
    let text = format!(
        r#"{{"openapi":"3.0.0","info":{{"title":"Shop","version":"1"}},
"servers":[{{"url":"http://127.0.0.1:1"}}],"paths":{{{}}}}}"#,
        paths.join(",")
    );
    bundle(text.as_bytes())
}

fn import(parent: &Path, ops: &[Op]) -> Collection {
    let b = spec(ops);
    write_collection(parent, &b.collection, &b.environments, &MemoryStore::new())
        .expect("writes")
        .0
}

/// Apply `b` with `o` after checking the preview predicts the same summary.
fn update_with(c: &mut Collection, b: &ImportBundle, o: &UpdateOptions) -> UpdateSummary {
    let planned = plan_update(c, b, o).unwrap();
    let applied = apply_update(c, b, o, None).unwrap();
    assert_eq!(format!("{planned:?}"), format!("{applied:?}"));
    applied
}

fn update(c: &mut Collection, b: &ImportBundle) -> UpdateSummary {
    update_with(c, b, &UpdateOptions::default())
}

fn url(c: &Collection, rel: &str) -> String {
    let reopened = Collection::open(c.root()).unwrap();
    reopened.request(&ip(rel)).unwrap().url().to_owned()
}

fn baseline(c: &Collection) -> SpecBaseline {
    Collection::open(c.root())
        .unwrap()
        .file()
        .spec_baseline()
        .unwrap()
}

const A: Op = ("/a", Some("listA"), "List A", "store");
const B: Op = ("/b", Some("listB"), "List B", "store");

// A key match wins over an id match, so in a swap each file keeps its path and takes the
// operation now there; nothing is added or lost.
#[test]
fn a_swap_keeps_each_request_at_its_path() {
    let dir = TempDir::new().unwrap();
    let mut c = import(dir.path(), &[A, B]);
    let swapped = spec(&[
        ("/a", Some("listB"), "List B", "store"),
        ("/b", Some("listA"), "List A", "store"),
    ]);
    let s = update(&mut c, &swapped);
    assert!(s.added.is_empty() && s.orphaned.is_empty(), "{s:?}");
    assert_eq!(url(&c, "store/List A.yml"), "{{baseUrl}}/a");
    let b = baseline(&c);
    assert_eq!(b.operation_id("GET /a"), Some("listB"));
    assert_eq!(b.operation_id("GET /b"), Some("listA"));
    let mut again = Collection::open(c.root()).unwrap();
    let second = update(&mut again, &swapped);
    assert_eq!(second.kept.len(), 2, "{second:?}");
}

#[test]
fn two_records_sharing_an_id_are_not_followed() {
    let dir = TempDir::new().unwrap();
    let mut c = import(dir.path(), &[A, B]);
    let mut b = baseline(&c);
    b.set_operation_id("GET /b".to_owned(), Some("listA".to_owned()));
    c.file_mut().set_spec_baseline(&b);
    c.file_mut().save().unwrap();
    let s = update(
        &mut c,
        &spec(&[("/moved", Some("listA"), "List A", "store"), B]),
    );
    assert_eq!(paths(&s.added), ["store/List A 1.yml"], "{s:?}");
    assert_eq!(paths(&s.orphaned), ["store/List A.yml"]);
}

#[test]
fn an_id_reused_while_its_old_operation_stays_adds_the_new_one() {
    let dir = TempDir::new().unwrap();
    let mut c = import(dir.path(), &[A]);
    let s = update(
        &mut c,
        &spec(&[
            ("/a", Some("listOld"), "List A", "store"),
            ("/new", Some("listA"), "New A", "store"),
        ]),
    );
    assert_eq!(paths(&s.added), ["store/New A.yml"], "{s:?}");
    assert!(s.orphaned.is_empty());
    assert_eq!(url(&c, "store/List A.yml"), "{{baseUrl}}/a");
    assert_eq!(baseline(&c).operation_id("GET /a"), Some("listOld"));
}

#[test]
fn a_moved_operation_follows_a_renamed_file_and_relinks_it() {
    let dir = TempDir::new().unwrap();
    let c0 = import(dir.path(), &[A]);
    fs::rename(
        c0.root().join("store/List A.yml"),
        c0.root().join("store/Mine.yml"),
    )
    .unwrap();
    let mut c = Collection::open(c0.root()).unwrap();
    let s = update(
        &mut c,
        &spec(&[("/moved", Some("listA"), "List A", "store")]),
    );
    assert!(s.added.is_empty() && s.orphaned.is_empty(), "{s:?}");
    assert_eq!(s.changed[0].path.as_str(), "store/Mine.yml");
    assert_eq!(url(&c, "store/Mine.yml"), "{{baseUrl}}/moved");
    assert_eq!(baseline(&c).file("GET /moved"), Some("store/Mine.yml"));
}

#[test]
fn a_moved_operation_can_be_taken_from_the_spec() {
    let dir = TempDir::new().unwrap();
    let mut c = import(dir.path(), &[A]);
    let f = c.request_mut(&ip("store/List A.yml")).unwrap();
    f.set_url("{{baseUrl}}/mine");
    f.save().unwrap();
    let mut o = UpdateOptions::default();
    o.take_spec = vec![TakeSpec::request(ip("store/List A.yml"))];
    let moved = spec(&[("/moved", Some("listA"), "List A", "store")]);
    let s = update_with(&mut c, &moved, &o);
    assert!(s.added.is_empty() && s.orphaned.is_empty(), "{s:?}");
    assert_eq!(url(&c, "store/List A.yml"), "{{baseUrl}}/moved");
}

#[test]
fn a_legacy_baseline_records_ids_on_its_next_update_and_then_follows_a_move() {
    let dir = TempDir::new().unwrap();
    let mut c = import(dir.path(), &[A]);
    let mut b = baseline(&c);
    b.set_operation_id("GET /a".to_owned(), None);
    c.file_mut().set_spec_baseline(&b);
    c.file_mut().save().unwrap();
    // Without a recorded id the move cannot be followed, and the preview still agrees.
    let probe = plan_update(
        &c,
        &spec(&[("/moved", Some("listA"), "List A", "store")]),
        &UpdateOptions::default(),
    )
    .unwrap();
    assert_eq!(paths(&probe.added), ["store/List A 1.yml"]);
    update(&mut c, &spec(&[A]));
    assert_eq!(baseline(&c).operation_id("GET /a"), Some("listA"));
    let mut again = Collection::open(c.root()).unwrap();
    let s = update(
        &mut again,
        &spec(&[("/moved", Some("listA"), "List A", "store")]),
    );
    assert!(s.added.is_empty() && s.orphaned.is_empty(), "{s:?}");
}

#[test]
fn adds_claiming_one_file_or_folder_name_each_get_their_own() {
    let dir = TempDir::new().unwrap();
    let mut c = import(dir.path(), &[A]);
    let s = update(
        &mut c,
        &spec(&[
            A,
            ("/x", None, "a/b", "store"),
            ("/y", None, "a:b", "store"),
            ("/z", None, "Z", "n/t"),
            ("/w", None, "W", "n:t"),
        ]),
    );
    let added = paths(&s.added);
    for p in [
        "store/a-b.yml",
        "store/a-b 1.yml",
        "n-t/Z.yml",
        "n-t 1/W.yml",
    ] {
        assert!(added.contains(&p), "{p} not in {added:?}");
    }
    for p in &added {
        assert!(c.root().join(p).is_file(), "{p}");
    }
}

/// The update of `ops` lists the new operation `label` only in a `not added` note.
fn refused(ops: &[Op], label: &str) {
    let dir = TempDir::new().unwrap();
    let mut c = import(dir.path(), &[A]);
    let s = update(&mut c, &spec(ops));
    assert!(s.added.is_empty(), "{s:?}");
    let note = s
        .notes
        .iter()
        .find(|n| n.detail.starts_with("not added"))
        .unwrap_or_else(|| panic!("no note: {:?}", s.notes));
    assert_eq!(note.source, label);
}

#[test]
fn a_root_folder_the_writer_refuses_is_noted_not_dropped() {
    refused(&[A, ("/e", None, "E", "environments")], "GET /e");
}

#[test]
fn a_request_name_the_writer_refuses_is_noted_not_dropped() {
    refused(&[A, ("/f", None, "folder", "store")], "GET /f");
}

#[test]
fn a_prune_note_names_the_path_of_an_absolute_url() {
    let dir = TempDir::new().unwrap();
    let mut c = import(dir.path(), &[A, B]);
    let f = c.request_mut(&ip("store/List B.yml")).unwrap();
    f.set_url("https://api.test/v1/b?x=1");
    f.save().unwrap();
    let mut o = UpdateOptions::default();
    o.prune = true;
    let bin = FakeBin::new(true);
    let s = apply_update(&mut c, &spec(&[A]), &o, Some(&bin as &dyn Trash)).unwrap();
    let note = s
        .notes
        .iter()
        .find(|n| n.detail.contains("not pruned"))
        .expect("prune note");
    assert_eq!(note.source, "GET /v1/b");
}
