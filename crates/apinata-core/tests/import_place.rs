//! `ensure_folder` and `place_items`: an import placed into a folder of an existing collection.

use crate::import_update_support::{bundle, ip, spec};
use apinata_core::import::{
    CollectionOptions, ImportError, NoteKind, UpdateOptions, apply_update, ensure_folder,
    parse_collections, place_items,
};
use apinata_core::{Collection, EnvironmentChoice, MemoryStore};
use tempfile::TempDir;

fn names(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| (*s).to_owned()).collect()
}

fn fresh() -> (TempDir, Collection) {
    let dir = TempDir::new().unwrap();
    let c = Collection::create(&dir.path().join("c"), "c").unwrap();
    (dir, c)
}

#[test]
fn ensure_folder_creates_nested_folders() {
    let (_dir, mut c) = fresh();
    let folder = ensure_folder(&mut c, &names(&["a", "b"])).unwrap();
    assert_eq!(folder.as_str(), "a/b");
    assert!(c.root().join("a/b/folder.yml").is_file());
    // A second call reuses what exists.
    assert_eq!(ensure_folder(&mut c, &names(&["a", "b"])).unwrap(), folder);
}

#[cfg(unix)]
#[test]
fn ensure_folder_refuses_a_symlink_and_a_file() {
    let (dir, mut c) = fresh();
    let outside = dir.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, c.root().join("link")).unwrap();
    std::fs::write(c.root().join("file"), "x").unwrap();
    for bad in ["link", "file"] {
        let err = ensure_folder(&mut c, &names(&[bad])).unwrap_err();
        assert!(matches!(err, ImportError::Folder { .. }), "{err}");
    }
    assert!(std::fs::read_dir(&outside).unwrap().next().is_none());
}

#[test]
fn place_items_puts_requests_directly_in_the_folder_and_records_the_baseline() {
    let (_dir, mut c) = fresh();
    let folder = ensure_folder(&mut c, &names(&["Shop"])).unwrap();
    let b = bundle(&spec("update-v1.json"));
    let placed = place_items(&mut c, &folder, &b, &MemoryStore::new()).unwrap();
    assert_eq!(placed.requests, 3);
    assert!(c.root().join("Shop/people/getUser.yml").is_file());
    assert!(!c.root().join("Shop/Shop").exists());
    assert!(c.request(&ip("Shop/people/getUser.yml")).is_some());
    assert!(c.file().spec_baseline_in(&folder).is_some());
    assert!(c.file().spec_baseline().is_none());
    // The placed folder is the one an update finds.
    let mut options = UpdateOptions::default();
    options.folder = Some(folder);
    let summary = apply_update(&mut c, &bundle(&spec("update-v2.json")), &options, None).unwrap();
    assert!(!summary.added.is_empty());
}

#[test]
fn place_items_leaves_the_target_root_file_and_folder_settings_alone() {
    let (_dir, mut c) = fresh();
    let folder = ensure_folder(&mut c, &names(&["Shop"])).unwrap();
    let before = std::fs::read_to_string(c.root().join("Shop/folder.yml")).unwrap();
    let b = bundle(&spec("update-v1.json"));
    place_items(&mut c, &folder, &b, &MemoryStore::new()).unwrap();
    assert_eq!(
        std::fs::read_to_string(c.root().join("Shop/folder.yml")).unwrap(),
        before
    );
    assert!(!c.root().join("Shop/environments").exists());
    assert!(!c.root().join("Shop/opencollection.yml").exists());
}

#[test]
fn a_name_already_in_the_folder_is_refused_and_nothing_is_touched() {
    let (_dir, mut c) = fresh();
    let folder = ensure_folder(&mut c, &names(&["Shop"])).unwrap();
    // `people` exists with a file of its own; `legacy` does not.
    let people = c.root().join("Shop/people");
    std::fs::create_dir(&people).unwrap();
    std::fs::write(people.join("mine.txt"), "keep").unwrap();
    let b = bundle(&spec("update-v1.json"));
    let err = place_items(&mut c, &folder, &b, &MemoryStore::new()).unwrap_err();
    assert!(matches!(err, ImportError::Exists { .. }), "{err}");
    assert_eq!(
        std::fs::read_to_string(people.join("mine.txt")).unwrap(),
        "keep"
    );
    assert!(!people.join("getUser.yml").exists());
    assert!(!c.root().join("Shop/legacy").exists());
    assert!(c.file().spec_baseline_in(&folder).is_none());
}

/// A Postman bundle whose one request writes `url`.
fn postman_bundle(url: &str) -> apinata_core::import::ImportBundle {
    let input = serde_json::json!({
        "info": {
            "name": "P",
            "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
        },
        "item": [{"name": "R", "request": {"method": "GET", "url": url}}]
    })
    .to_string();
    parse_collections(None, input.as_bytes(), &CollectionOptions::default())
        .expect("parses")
        .remove(0)
}

/// The target `c` with an environment `dev` defining `host`, made the default, and a request of
/// its own outside the import that writes the undefined `theirs`.
fn target_with_environment() -> (TempDir, Collection) {
    let (dir, mut c) = fresh();
    let store = MemoryStore::new();
    c.create_environment("dev", &store).unwrap();
    c.set_variable_text("dev", "host", "http://127.0.0.1:1")
        .unwrap();
    c.create_environment("prod", &store).unwrap();
    c.file_mut().set_default_environment("prod");
    c.file_mut().save().unwrap();
    std::fs::write(
        c.root().join("Mine.yml"),
        "info:\n  name: Mine\n  type: http\nhttp:\n  method: GET\n  url: \"{{theirs}}/x\"\n",
    )
    .unwrap();
    c.reload();
    (dir, c)
}

#[test]
fn a_placed_reference_nothing_in_the_target_defines_needs_a_value() {
    let (_dir, mut c) = target_with_environment();
    let folder = ensure_folder(&mut c, &names(&["Shop"])).unwrap();
    let b = postman_bundle("{{host}}/{{base}}");
    let placed = place_items(&mut c, &folder, &b, &MemoryStore::new()).unwrap();
    let listed: Vec<_> = placed
        .report
        .notes
        .iter()
        .filter(|n| n.kind == NoteKind::NeedsInput && n.detail.starts_with("needs a value"))
        .map(|n| n.detail.as_str())
        .collect();
    // `host` is declared by `dev`, and `theirs` belongs to a request the import did not write.
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert!(
        listed[0].starts_with("needs a value: `base`;"),
        "{}",
        listed[0]
    );
}

#[test]
fn a_placed_import_leaves_the_target_default_environment_alone() {
    let (_dir, mut c) = target_with_environment();
    let folder = ensure_folder(&mut c, &names(&["Shop"])).unwrap();
    place_items(
        &mut c,
        &folder,
        &postman_bundle("{{host}}/x"),
        &MemoryStore::new(),
    )
    .unwrap();
    assert_eq!(
        c.choose_environment(None),
        EnvironmentChoice::Preset("prod".into())
    );
}
