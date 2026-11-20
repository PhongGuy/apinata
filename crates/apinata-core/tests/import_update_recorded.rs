//! The source an import or update came from is recorded, so the next update can offer it.

use crate::import_update_support::{bundle, imported, spec};
use apinata_core::Collection;
use apinata_core::MemoryStore;
use apinata_core::import::{
    CollectionFormat, CollectionOptions, GroupBy, ImportBundle, ImportNote, SpecLocation,
    UpdateOptions, apply_update, parse_collections, plan_update, recorded_spec, write_collection,
};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// The fixture parsed as if read from `source`, grouped by `group_by`.
fn from(fixture: &str, source: SpecLocation, group_by: GroupBy) -> ImportBundle {
    let mut options = CollectionOptions::default();
    options.source = Some(source);
    options.group_by = group_by;
    parse_collections(Some(CollectionFormat::OpenApi), &spec(fixture), &options)
        .expect("parses")
        .remove(0)
}

fn root_text(root: &Path) -> String {
    fs::read_to_string(root.join("opencollection.yml")).unwrap()
}

#[test]
fn an_import_from_a_link_records_the_link_and_its_grouping() {
    let dir = TempDir::new().unwrap();
    let url = "http://127.0.0.1:9/spec.json";
    let b = from(
        "update-v1.json",
        SpecLocation::Link(url.to_owned()),
        GroupBy::Path,
    );
    let (c, _) = write_collection(
        dir.path(),
        &b.collection,
        &b.environments,
        &MemoryStore::new(),
    )
    .unwrap();
    let c = Collection::open(c.root()).unwrap();
    let recorded = recorded_spec(&c).expect("recorded");
    assert_eq!(recorded.location, SpecLocation::Link(url.to_owned()));
    assert_eq!(recorded.group_by, GroupBy::Path);
    assert!(
        root_text(c.root()).contains(&format!("      source: {url}\n      groupBy: path\n")),
        "{}",
        root_text(c.root())
    );
}

#[test]
fn an_import_from_a_file_outside_the_collection_records_it_relative_to_the_root() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("specs/spec.json");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, spec("update-v1.json")).unwrap();
    let b = from("update-v1.json", SpecLocation::File(file), GroupBy::Tags);
    let into = dir.path().join("collections");
    fs::create_dir_all(&into).unwrap();
    let (c, _) =
        write_collection(&into, &b.collection, &b.environments, &MemoryStore::new()).unwrap();
    let text = root_text(c.root());
    assert!(
        text.contains("      source: ../../specs/spec.json\n"),
        "{text}"
    );
    let recorded = recorded_spec(&Collection::open(c.root()).unwrap()).expect("recorded");
    assert_eq!(
        recorded.location,
        SpecLocation::File(c.root().join("../../specs/spec.json"))
    );
    assert_eq!(recorded.group_by, GroupBy::Tags);
}

/// Whether any note says the link was not remembered, and none repeats the secret.
fn says_not_remembered(notes: &[ImportNote]) -> bool {
    assert!(
        notes.iter().all(|n| !n.detail.contains("s3cret")),
        "{notes:?}"
    );
    notes.iter().any(|n| n.detail.contains("not remembered"))
}

#[test]
fn an_import_from_a_link_with_a_query_records_nothing_and_says_so() {
    let dir = TempDir::new().unwrap();
    let url = "http://127.0.0.1:9/spec.json?token=s3cret";
    let b = from(
        "update-v1.json",
        SpecLocation::Link(url.to_owned()),
        GroupBy::Tags,
    );
    let (c, report) = write_collection(
        dir.path(),
        &b.collection,
        &b.environments,
        &MemoryStore::new(),
    )
    .unwrap();
    assert!(says_not_remembered(&report.notes), "{:?}", report.notes);
    assert!(!root_text(c.root()).contains("s3cret"));
    assert!(recorded_spec(&Collection::open(c.root()).unwrap()).is_none());
}

#[test]
fn an_update_from_a_link_with_a_fragment_keeps_the_earlier_record_and_says_so() {
    let dir = TempDir::new().unwrap();
    let plain = "http://127.0.0.1:9/spec.json";
    let b = from(
        "update-v1.json",
        SpecLocation::Link(plain.to_owned()),
        GroupBy::Tags,
    );
    let (mut c, _) = write_collection(
        dir.path(),
        &b.collection,
        &b.environments,
        &MemoryStore::new(),
    )
    .unwrap();
    let signed = from(
        "update-v2.json",
        SpecLocation::Link("http://127.0.0.1:9/spec.json#s3cret".to_owned()),
        GroupBy::Tags,
    );
    let planned = plan_update(&c, &signed, &UpdateOptions::default()).unwrap();
    assert!(says_not_remembered(&planned.notes), "{planned:?}");
    let s = apply_update(&mut c, &signed, &UpdateOptions::default(), None).unwrap();
    assert!(says_not_remembered(&s.notes), "{s:?}");
    assert!(!root_text(c.root()).contains("s3cret"));
    let recorded = recorded_spec(&Collection::open(c.root()).unwrap()).expect("recorded");
    assert_eq!(recorded.location, SpecLocation::Link(plain.to_owned()));
}

#[test]
fn an_update_from_a_file_inside_the_collection_records_it_relative_to_the_root() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let file = c.root().join("specs/api.json");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, spec("update-v2.json")).unwrap();
    let b = from(
        "update-v2.json",
        SpecLocation::File(file.clone()),
        GroupBy::Tags,
    );
    apply_update(&mut c, &b, &UpdateOptions::default(), None).unwrap();
    let text = root_text(c.root());
    assert!(text.contains("      source: specs/api.json\n"), "{text}");
    let recorded = recorded_spec(&Collection::open(c.root()).unwrap()).expect("recorded");
    assert_eq!(
        recorded.location,
        SpecLocation::File(c.root().join("specs/api.json"))
    );
}

#[test]
fn an_update_records_its_source_over_the_reference_one_and_leaves_that_alone() {
    let dir = TempDir::new().unwrap();
    let c = imported(dir.path(), "update-v1.json");
    let file = c.root().join("opencollection.yml");
    // The import already writes a `bruno` section; the reference record goes first inside it.
    let text = root_text(c.root()).replacen(
        "\n  bruno:\n",
        "\n  bruno:\n    openapi:\n      - sourceUrl: old.json\n        groupBy: path\n",
        1,
    );
    fs::write(&file, &text).unwrap();
    let mut c = Collection::open(c.root()).unwrap();
    let url = "http://127.0.0.1:9/new.json";
    let b = from(
        "update-v2.json",
        SpecLocation::Link(url.to_owned()),
        GroupBy::Tags,
    );
    apply_update(&mut c, &b, &UpdateOptions::default(), None).unwrap();
    let after = root_text(c.root());
    assert!(
        after.contains("      - sourceUrl: old.json\n        groupBy: path\n"),
        "{after}"
    );
    let recorded = recorded_spec(&Collection::open(c.root()).unwrap()).expect("recorded");
    assert_eq!(recorded.location, SpecLocation::Link(url.to_owned()));
    assert_eq!(recorded.group_by, GroupBy::Tags);
}

#[test]
fn an_update_without_a_source_keeps_the_recorded_one() {
    let dir = TempDir::new().unwrap();
    let url = "http://127.0.0.1:9/spec.json";
    let b = from(
        "update-v1.json",
        SpecLocation::Link(url.to_owned()),
        GroupBy::Tags,
    );
    let (mut c, _) = write_collection(
        dir.path(),
        &b.collection,
        &b.environments,
        &MemoryStore::new(),
    )
    .unwrap();
    apply_update(
        &mut c,
        &bundle(&spec("update-v2.json")),
        &UpdateOptions::default(),
        None,
    )
    .unwrap();
    let recorded = recorded_spec(&Collection::open(c.root()).unwrap()).expect("recorded");
    assert_eq!(recorded.location, SpecLocation::Link(url.to_owned()));
}

#[test]
fn an_import_with_no_source_records_none() {
    let dir = TempDir::new().unwrap();
    let c = imported(dir.path(), "update-v1.json");
    assert!(!root_text(c.root()).contains("source:"));
    assert!(recorded_spec(&c).is_none());
}
