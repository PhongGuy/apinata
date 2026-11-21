//! An operation whose path moved: the preview and the write name the same files, and an
//! operation id the spec keeps carries the request across the move.

use crate::import_update_support::{FakeBin, bundle, ip, paths};
use apinata_core::import::{
    CollectionOptions, ImportBundle, NoteKind, SpecLocation, UpdateOptions, apply_update,
    parse_collections, plan_update, recorded_spec, write_collection,
};
use apinata_core::{Collection, MemoryStore, ScriptPhase, Trash};
use std::path::{Component, Path};
use tempfile::TempDir;

/// A one-operation store spec at `path`; `id` is its operationId, when it has one.
fn store(path: &str, id: Option<&str>) -> Vec<u8> {
    let id = id.map_or(String::new(), |id| format!(r#""operationId":"{id}","#));
    format!(
        r#"{{"openapi":"3.0.0","info":{{"title":"Shop","version":"1"}},
"servers":[{{"url":"http://127.0.0.1:1"}}],
"paths":{{"{path}":{{"get":{{{id}"summary":"List orders","tags":["store"]}}}}}}}}"#
    )
    .into_bytes()
}

fn import(parent: &Path, spec: &[u8]) -> Collection {
    let b = bundle(spec);
    write_collection(parent, &b.collection, &b.environments, &MemoryStore::new())
        .expect("writes")
        .0
}

fn update(c: &mut Collection, b: &ImportBundle) -> String {
    let planned = plan_update(c, b, &UpdateOptions::default()).unwrap();
    let applied = apply_update(c, b, &UpdateOptions::default(), None).unwrap();
    assert_eq!(format!("{planned:?}"), format!("{applied:?}"));
    format!("{applied:?}")
}

#[test]
fn a_preview_names_the_file_the_update_writes() {
    let dir = TempDir::new().unwrap();
    let mut c = import(dir.path(), &store("/store/orders", None));
    let moved = bundle(&store("/store/order-list", None));
    let planned = plan_update(&c, &moved, &UpdateOptions::default()).unwrap();
    assert_eq!(paths(&planned.added), ["store/List orders 1.yml"]);
    let s = apply_update(&mut c, &moved, &UpdateOptions::default(), None).unwrap();
    assert_eq!(paths(&s.added), paths(&planned.added));
    assert!(c.root().join("store/List orders 1.yml").is_file());
}

#[test]
fn a_moved_path_with_the_same_operation_id_updates_the_request_in_place() {
    let dir = TempDir::new().unwrap();
    let mut c = import(dir.path(), &store("/store/orders", Some("listOrders")));
    let moved = bundle(&store("/store/order-list", Some("listOrders")));
    let planned = plan_update(&c, &moved, &UpdateOptions::default()).unwrap();
    let s = apply_update(&mut c, &moved, &UpdateOptions::default(), None).unwrap();
    assert_eq!(format!("{planned:?}"), format!("{s:?}"));
    assert!(s.added.is_empty() && s.orphaned.is_empty(), "{s:?}");
    assert_eq!(s.changed.len(), 1, "{s:?}");
    assert_eq!(s.changed[0].path.as_str(), "store/List orders.yml");
    let reopened = Collection::open(c.root()).unwrap();
    let file = reopened.request(&ip("store/List orders.yml")).unwrap();
    assert_eq!(file.url(), "{{baseUrl}}/store/order-list");
    assert!(!c.root().join("store/List orders 1.yml").exists());

    // The record follows the move, so the next update finds nothing to do.
    let mut again = Collection::open(c.root()).unwrap();
    let second = apply_update(&mut again, &moved, &UpdateOptions::default(), None).unwrap();
    assert_eq!(paths(&second.kept), ["store/List orders.yml"], "{second:?}");
    assert!(second.added.is_empty() && second.orphaned.is_empty());
}

#[test]
fn a_moved_path_keeps_the_users_edits() {
    let dir = TempDir::new().unwrap();
    let mut c = import(dir.path(), &store("/store/orders", Some("listOrders")));
    let f = c.request_mut(&ip("store/List orders.yml")).unwrap();
    f.set_docs("my notes");
    f.save().unwrap();
    update(
        &mut c,
        &bundle(&store("/store/order-list", Some("listOrders"))),
    );
    let reopened = Collection::open(c.root()).unwrap();
    let file = reopened.request(&ip("store/List orders.yml")).unwrap();
    assert_eq!(file.url(), "{{baseUrl}}/store/order-list");
    assert_eq!(file.docs(), "my notes");
}

#[test]
fn an_operation_id_the_spec_repeats_is_not_matched() {
    let dir = TempDir::new().unwrap();
    let mut c = import(dir.path(), &store("/store/orders", Some("listOrders")));
    let twice = br#"{"openapi":"3.0.0","info":{"title":"Shop","version":"1"},
"paths":{"/store/order-list":{"get":{"operationId":"listOrders","summary":"List orders","tags":["store"]}},
"/store/all-orders":{"get":{"operationId":"listOrders","summary":"All orders","tags":["store"]}}}}"#;
    let summary = update(&mut c, &bundle(twice));
    assert!(summary.contains("List orders 1.yml"), "{summary}");
    let orphaned = r#"orphaned: [ItemPath("store/List orders.yml")]"#;
    assert!(summary.contains(orphaned), "{summary}");
}

#[test]
fn a_prune_note_names_the_method_and_the_request_path() {
    let dir = TempDir::new().unwrap();
    let mut c = import(dir.path(), &store("/store/orders/{id}", None));
    let f = c.request_mut(&ip("store/List orders.yml")).unwrap();
    f.set_script(ScriptPhase::Tests, "test('ok', () => {});");
    f.save().unwrap();
    let mut o = UpdateOptions::default();
    o.prune = true;
    let bin = FakeBin::new(true);
    let other = store("/store/other", None);
    let s = apply_update(&mut c, &bundle(&other), &o, Some(&bin as &dyn Trash)).unwrap();
    let note = s
        .notes
        .iter()
        .find(|n| n.detail.contains("not pruned"))
        .expect("prune note");
    assert_eq!(note.source, "GET /store/orders/:id");
}

#[test]
fn an_update_without_a_baseline_is_not_noted_as_dropped() {
    let dir = TempDir::new().unwrap();
    let c = Collection::create(&dir.path().join("plain"), "plain").unwrap();
    let s = plan_update(&c, &bundle(&store("/x", None)), &UpdateOptions::default()).unwrap();
    let note = &s.notes[0];
    assert!(note.detail.starts_with("no update baseline"), "{note:?}");
    assert_eq!(note.kind, NoteKind::Skipped);
    assert_eq!(note.kind.id(), "skipped");
}

#[test]
fn a_recorded_file_source_is_normalised() {
    let dir = TempDir::new().unwrap();
    let spec_path = dir.path().join("spec.json");
    std::fs::write(&spec_path, store("/x", None)).unwrap();
    let mut options = CollectionOptions::default();
    options.source = Some(SpecLocation::File(spec_path));
    let b = parse_collections(None, &store("/x", None), &options)
        .unwrap()
        .remove(0);
    let (c, _) = write_collection(dir.path(), &b.collection, &[], &MemoryStore::new()).unwrap();
    let Some(SpecLocation::File(path)) = recorded_spec(&c).map(|r| r.location) else {
        panic!("a file source is recorded");
    };
    assert!(
        !path.components().any(|p| p == Component::ParentDir),
        "{}",
        path.display()
    );
    assert!(path.ends_with("spec.json"), "{}", path.display());
    assert!(path.is_file());
}
