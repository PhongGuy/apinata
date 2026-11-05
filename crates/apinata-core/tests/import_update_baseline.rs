//! The provenance baseline written on import and kept in the root file.

use crate::import_update_support::{imported, ip};
use apinata_core::import::{
    ChangedField, CollectionFormat, CollectionOptions, FieldHashes, SpecBaseline,
    parse_collections, write_collection,
};
use apinata_core::{Collection, CollectionPresets, MemoryStore};
use tempfile::TempDir;

#[test]
fn an_openapi_import_records_one_entry_per_operation() {
    let dir = TempDir::new().unwrap();
    let c = imported(dir.path(), "update-v1.json");
    // Pins the layout the later tests rely on (task 2 fixture).
    for rel in [
        "people/getUser.yml",
        "people/createUser.yml",
        "legacy/removeLegacy.yml",
    ] {
        assert!(c.request(&ip(rel)).is_some(), "{rel}");
    }
    let baseline = c.file().spec_baseline().expect("baseline");
    let keys: Vec<_> = baseline.keys().collect();
    assert_eq!(keys, ["GET /legacy", "GET /users/{}", "POST /users"]);
    for key in keys {
        assert_eq!(baseline.get(key).unwrap().iter().count(), 8, "{key}");
    }
}

#[test]
fn the_baseline_is_on_disk_and_survives_a_reopen() {
    let dir = TempDir::new().unwrap();
    let c = imported(dir.path(), "update-v1.json");
    let again = Collection::open(c.root()).unwrap();
    assert_eq!(again.file().spec_baseline(), c.file().spec_baseline());
    let text = std::fs::read_to_string(c.root().join("opencollection.yml")).unwrap();
    assert!(text.contains("extensions:") && text.contains("openapi:") && text.contains("ops:"));
}

#[test]
fn a_plain_import_from_another_format_records_nothing() {
    let har = br#"{"log":{"version":"1.2","entries":[{"request":{"method":"GET","url":"https://x.test/a","headers":[]}}]}}"#;
    let mut options = CollectionOptions::default();
    options.name = Some("h".into());
    let b = parse_collections(Some(CollectionFormat::Har), har, &options)
        .unwrap()
        .remove(0);
    let dir = TempDir::new().unwrap();
    let (c, _) = write_collection(
        dir.path(),
        &b.collection,
        &b.environments,
        &MemoryStore::new(),
    )
    .unwrap();
    assert!(c.file().spec_baseline().is_none());
}

#[test]
fn number_looking_hashes_stay_strings_and_other_extensions_stay() {
    let dir = TempDir::new().unwrap();
    let mut c = Collection::create(&dir.path().join("c"), "c").unwrap();
    c.file_mut()
        .set_presets(&CollectionPresets::new("http", "https://x.test"));
    let mut baseline = SpecBaseline::default();
    baseline.insert(
        "GET /a".into(),
        FieldHashes::new([
            (ChangedField::Target, "1234567890123456".to_owned()),
            (ChangedField::Body, "12345678e1234567".to_owned()),
            (ChangedField::Docs, "0000000000000000".to_owned()),
        ]),
    );
    c.file_mut().set_spec_baseline(&baseline);
    c.file_mut().save().unwrap();
    let again = Collection::open(c.root()).unwrap();
    assert_eq!(again.file().spec_baseline(), Some(baseline));
    assert_eq!(again.file().presets().request_url, "https://x.test");
}

#[test]
fn an_empty_baseline_keeps_an_empty_ops_and_clear_removes_it() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    c.file_mut().set_spec_baseline(&SpecBaseline::default());
    c.file_mut().save().unwrap();
    let again = Collection::open(c.root()).unwrap();
    assert_eq!(again.file().spec_baseline(), Some(SpecBaseline::default()));
    let mut c = again;
    c.file_mut().clear_spec_baseline();
    c.file_mut().save().unwrap();
    let text = std::fs::read_to_string(c.root().join("opencollection.yml")).unwrap();
    assert!(!text.contains("ops:") && !text.contains("openapi:"));
    assert!(
        Collection::open(c.root())
            .unwrap()
            .file()
            .spec_baseline()
            .is_none()
    );
}

#[test]
fn an_import_records_the_file_of_each_operation_and_it_survives_a_reopen() {
    let dir = TempDir::new().unwrap();
    let c = imported(dir.path(), "update-v1.json");
    let b = c.file().spec_baseline().unwrap();
    assert_eq!(b.file("GET /users/{}"), Some("people/getUser.yml"));
    assert_eq!(b.file("GET /legacy"), Some("legacy/removeLegacy.yml"));
    let again = Collection::open(c.root()).unwrap();
    assert_eq!(again.file().spec_baseline(), c.file().spec_baseline());
    // The hashes are still reported as before.
    assert_eq!(b.get("GET /legacy").unwrap().iter().count(), 8);
}

#[test]
fn set_file_and_remove_keep_the_file_beside_the_entry() {
    let mut b = SpecBaseline::default();
    b.insert("GET /a".into(), FieldHashes::default());
    assert_eq!(b.file("GET /a"), None);
    b.set_file("GET /a".into(), "a.yml".into());
    assert_eq!(b.file("GET /a"), Some("a.yml"));
    assert!(b.remove("GET /a"));
    assert_eq!(b.file("GET /a"), None);
}

#[test]
fn a_baseline_entry_without_a_file_still_reads() {
    let dir = TempDir::new().unwrap();
    let c = imported(dir.path(), "update-v1.json");
    let path = c.root().join("opencollection.yml");
    let text: String = std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .filter(|l| !l.trim_start().starts_with("file:"))
        .map(|l| format!("{l}\n"))
        .collect();
    std::fs::write(&path, text).unwrap();
    let again = Collection::open(c.root()).unwrap();
    let b = again.file().spec_baseline().unwrap();
    assert_eq!(b.len(), 3);
    assert_eq!(b.file("GET /legacy"), None);
    assert_eq!(b.get("GET /legacy").unwrap().iter().count(), 8);
}
