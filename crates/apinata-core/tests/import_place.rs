//! `ensure_folder` and `place_items`: an import placed into a folder of an existing collection.

use crate::import_update_support::{bundle, ip, spec};
use apinata_core::import::{ImportError, UpdateOptions, apply_update, ensure_folder, place_items};
use apinata_core::{Collection, MemoryStore};
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
