//! `apinata import collection --update` against the v1 and v2 fixtures.

use crate::common;
use common::apinata;
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../apinata-core/tests/fixtures/openapi")
        .join(name)
}

fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(rel, fs::read(&path).unwrap());
            }
        }
    }
    out
}

/// Import v1 and return the new collection root.
fn first_import(parent: &Path) -> PathBuf {
    apinata()
        .args(["import", "collection", "--no-keychain"])
        .arg(fixture("update-v1.json"))
        .arg("--into")
        .arg(parent)
        .assert()
        .code(0);
    parent.join("Shop")
}

fn update(root: &Path, extra: &[&str]) -> assert_cmd::assert::Assert {
    apinata()
        .args(["import", "collection", "--update"])
        .arg(fixture("update-v2.json"))
        .arg("--into")
        .arg(root)
        .args(extra)
        .assert()
}

#[test]
fn dry_run_prints_the_summary_and_writes_nothing() {
    let parent = tempfile::tempdir().unwrap();
    let root = first_import(parent.path());
    let before = tree(&root);
    update(&root, &["--dry-run"])
        .code(0)
        .stdout(contains(
            "would add: 1, would change: 1, kept: 1, edited: 0, orphaned: 1, would prune: 0",
        ))
        .stdout(contains("would add people/listUsers.yml"))
        .stdout(contains("would change people/getUser.yml (target, docs)"))
        .stdout(contains("orphaned legacy/removeLegacy.yml"));
    assert_eq!(tree(&root), before);
}

#[test]
fn a_prune_dry_run_says_would_prune_and_names_no_trash() {
    let parent = tempfile::tempdir().unwrap();
    let root = first_import(parent.path());
    let before = tree(&root);
    let out = update(&root, &["--prune", "--dry-run"])
        .code(0)
        .stdout(contains("would prune legacy/removeLegacy.yml"))
        .stdout(contains("would prune: 1"))
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    assert!(!text.contains("moved to the trash"), "{text}");
    assert!(!text.contains("pruned"), "{text}");
    assert_eq!(tree(&root), before);
}

#[test]
fn a_real_run_keeps_the_past_tense_text() {
    let parent = tempfile::tempdir().unwrap();
    let root = first_import(parent.path());
    update(&root, &[])
        .code(0)
        .stdout(contains(
            "added: 1, changed: 1, kept: 1, edited: 0, orphaned: 1, pruned: 0",
        ))
        .stdout(contains("added people/listUsers.yml"))
        .stdout(contains("changed people/getUser.yml (target, docs)"))
        .stdout(contains("orphaned legacy/removeLegacy.yml"))
        .stdout(contains("would").not());
}

#[test]
fn update_applies_and_a_second_run_changes_nothing() {
    let parent = tempfile::tempdir().unwrap();
    let root = first_import(parent.path());
    update(&root, &[])
        .code(0)
        .stdout(contains("added people/listUsers.yml"));
    assert!(root.join("people/listUsers.yml").exists());
    assert!(
        root.join("legacy/removeLegacy.yml").exists(),
        "no prune: nothing deleted"
    );
    let after = tree(&root);
    update(&root, &[])
        .code(0)
        .stdout(contains("added: 0, changed: 0"));
    assert_eq!(tree(&root), after);
}

/// A prune run whose home and data directory are scratch directories beside the collection,
/// so the trash crate files into a trash on the same filesystem and never the user's own.
fn prune_in_scratch_home(
    root: &Path,
    home: &Path,
    data: &Path,
    extra: &[&str],
) -> assert_cmd::assert::Assert {
    apinata()
        .env("HOME", home)
        .env("XDG_DATA_HOME", data)
        .args(["import", "collection", "--update"])
        .arg(fixture("update-v2.json"))
        .arg("--into")
        .arg(root)
        .arg("--prune")
        .args(extra)
        .assert()
}

#[test]
fn prune_moves_the_orphan_to_the_trash() {
    let parent = tempfile::tempdir().unwrap();
    let root = first_import(parent.path());
    let home = parent.path().join("home");
    let data = home.join("data");
    fs::create_dir_all(&data).unwrap();
    prune_in_scratch_home(&root, &home, &data, &[])
        .code(0)
        .stdout(contains(
            "pruned legacy/removeLegacy.yml (moved to the trash)",
        ));
    assert!(!root.join("legacy/removeLegacy.yml").exists());
    let trashed = data.join("Trash/files/removeLegacy.yml");
    assert!(trashed.is_file(), "{trashed:?}");
}

#[test]
fn prune_json_names_the_trash_as_the_destination() {
    let parent = tempfile::tempdir().unwrap();
    let root = first_import(parent.path());
    let home = parent.path().join("home");
    let data = home.join("data");
    fs::create_dir_all(&data).unwrap();
    let out = prune_in_scratch_home(&root, &home, &data, &["--format", "json"])
        .code(0)
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["pruned"][0], "legacy/removeLegacy.yml");
    assert_eq!(json["pruned_destination"], "trash");
}

#[test]
fn prune_is_refused_when_there_is_no_trash() {
    let parent = tempfile::tempdir().unwrap();
    let root = first_import(parent.path());
    let home = parent.path().join("home");
    fs::create_dir_all(&home).unwrap();
    // A regular file where the data directory's parent should be: the trash cannot be made.
    let blocker = parent.path().join("blocker");
    fs::write(&blocker, "").unwrap();
    let before = tree(&root);
    prune_in_scratch_home(&root, &home, &blocker.join("share"), &[])
        .failure()
        .stderr(contains("trash"));
    assert_eq!(tree(&root), before);
    assert!(root.join("legacy/removeLegacy.yml").exists());
}

#[test]
fn prune_and_dry_run_need_update() {
    let parent = tempfile::tempdir().unwrap();
    for flag in ["--prune", "--dry-run"] {
        apinata()
            .args(["import", "collection", "--no-keychain"])
            .arg(fixture("update-v1.json"))
            .arg("--into")
            .arg(parent.path())
            .arg(flag)
            .assert()
            .failure();
    }
    assert!(!parent.path().join("Shop").exists());
}

#[test]
fn update_needs_an_existing_collection_root() {
    let parent = tempfile::tempdir().unwrap();
    update(parent.path(), &[]).code(3);
}

#[test]
fn update_refuses_a_missing_directory_and_creates_nothing() {
    let parent = tempfile::tempdir().unwrap();
    let missing = parent.path().join("a").join("b");
    update(&missing, &[])
        .code(3)
        .stderr(contains("cannot open"));
    assert!(!parent.path().join("a").exists());
}

#[test]
fn update_refuses_input_that_is_not_an_openapi_description() {
    let parent = tempfile::tempdir().unwrap();
    let root = first_import(parent.path());
    let har = parent.path().join("a.har");
    fs::write(&har, r#"{"log":{"version":"1.2","entries":[]}}"#).unwrap();
    apinata()
        .args(["import", "collection", "--update"])
        .arg(&har)
        .arg("--into")
        .arg(&root)
        .assert()
        .code(3)
        .stderr(contains("--update needs an OpenAPI description"));
}

#[test]
fn json_output_carries_the_same_summary() {
    let parent = tempfile::tempdir().unwrap();
    let root = first_import(parent.path());
    let out = update(&root, &["--dry-run", "--format", "json"])
        .code(0)
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let mut keys: Vec<&str> = json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "added",
            "changed",
            "collection",
            "dry_run",
            "edited",
            "edited_fields",
            "kept",
            "notes",
            "orphaned",
            "pruned"
        ]
    );
    assert_eq!(json["dry_run"], true);
    assert_eq!(json["added"][0], "people/listUsers.yml");
    assert_eq!(json["orphaned"][0], "legacy/removeLegacy.yml");
}

#[test]
fn update_rejects_name() {
    let parent = tempfile::tempdir().unwrap();
    let root = first_import(parent.path());
    update(&root, &["--name", "Other"]).failure();
}

#[test]
fn an_edited_path_stays_the_same_request_in_dry_run_and_apply() {
    let parent = tempfile::tempdir().unwrap();
    let root = first_import(parent.path());
    let file = root.join("people/getUser.yml");
    let text = fs::read_to_string(&file).unwrap();
    assert!(text.contains("/users/:id"), "{text}");
    fs::write(&file, text.replace("/users/:id", "/v9/users/:id")).unwrap();
    let edited = tree(&root);
    for extra in [&["--dry-run"][..], &[][..]] {
        let (counts, added) = if extra.is_empty() {
            ("added: 1, changed: 1, kept: 1, edited: 1", "added people/")
        } else {
            (
                "would add: 1, would change: 1, kept: 1, edited: 1",
                "would add people/",
            )
        };
        update(&root, extra)
            .code(0)
            .stdout(contains(counts))
            .stdout(contains("edited people/getUser.yml"))
            .stdout(contains(format!("{added}listUsers.yml")));
        assert!(!root.join("people/getUser 1.yml").exists());
        if extra.is_empty() {
            assert!(root.join("people/listUsers.yml").exists());
        } else {
            assert_eq!(tree(&root), edited);
        }
    }
    assert!(
        fs::read_to_string(&file).unwrap().contains("/v9/users/:id"),
        "the edited path is kept"
    );
}

/// A collection whose spec lives in the folder `Sub`: the v1 import moved under it and its
/// baseline recorded there, as core does for an import into a folder.
fn folder_import(parent: &Path) -> PathBuf {
    use apinata_core::import::record_spec_baseline;
    use apinata_core::{Collection, ItemPath};
    let root = first_import(parent);
    let mut c = Collection::open(&root).unwrap();
    let sub = c.create_folder(&ItemPath::root(), "Sub").unwrap();
    for dir in ["people", "legacy"] {
        fs::rename(root.join(dir), root.join("Sub").join(dir)).unwrap();
    }
    c.reload();
    let items = [
        ItemPath::new("Sub/people/getUser.yml").unwrap(),
        ItemPath::new("Sub/legacy/removeLegacy.yml").unwrap(),
    ];
    record_spec_baseline(&mut c, &sub, &items).unwrap();
    root
}

#[test]
fn folder_update_changes_only_that_folder_and_names_it() {
    let parent = tempfile::tempdir().unwrap();
    let root = folder_import(parent.path());
    update(&root, &["--folder", "Sub"])
        .code(0)
        .stdout(contains("folder: Sub"))
        .stdout(contains("added Sub/people/listUsers.yml"))
        .stdout(contains("changed Sub/people/getUser.yml (target, docs)"));
    assert!(root.join("Sub/people/listUsers.yml").exists());
    assert!(!root.join("people").exists(), "nothing lands in the root");
}

#[test]
fn folder_update_json_carries_the_folder() {
    let parent = tempfile::tempdir().unwrap();
    let root = folder_import(parent.path());
    let out = update(&root, &["--folder", "Sub", "--dry-run", "--format", "json"])
        .code(0)
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["folder"], "Sub");
    assert_eq!(json["added"][0], "Sub/people/listUsers.yml");
}

#[test]
fn a_missing_folder_prints_the_no_baseline_note() {
    let parent = tempfile::tempdir().unwrap();
    let root = folder_import(parent.path());
    let before = tree(&root);
    update(&root, &["--folder", "Nope"])
        .code(0)
        .stderr(contains("no update baseline for folder Nope"));
    assert_eq!(tree(&root), before);
}

#[test]
fn folder_without_a_collection_at_into_writes_nothing() {
    let parent = tempfile::tempdir().unwrap();
    apinata()
        .args(["import", "collection", "--no-keychain"])
        .arg(fixture("update-v1.json"))
        .arg("--into")
        .arg(parent.path())
        .args(["--folder", "Sub"])
        .assert()
        .failure();
    assert!(!parent.path().join("Shop").exists());
}

#[test]
fn without_folder_the_json_has_no_folder_key() {
    let parent = tempfile::tempdir().unwrap();
    let root = first_import(parent.path());
    let out = update(&root, &["--dry-run", "--format", "json"])
        .code(0)
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert!(json.get("folder").is_none());
}

#[test]
fn a_changed_method_is_reported_as_edited_naming_the_method() {
    let parent = tempfile::tempdir().unwrap();
    let root = first_import(parent.path());
    let file = root.join("people/getUser.yml");
    let text = fs::read_to_string(&file).unwrap();
    assert!(text.contains("method: GET"), "{text}");
    fs::write(&file, text.replace("method: GET", "method: POST")).unwrap();
    update(&root, &["--dry-run"])
        .code(0)
        .stdout(contains("edited: 1"))
        .stdout(contains("edited people/getUser.yml (method)"));
    let out = update(&root, &["--format", "json"])
        .code(0)
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["edited"][0], "people/getUser.yml");
    assert_eq!(json["edited_fields"][0]["path"], "people/getUser.yml");
    assert_eq!(json["edited_fields"][0]["fields"][0], "method");
    assert_eq!(json["edited_fields"].as_array().unwrap().len(), 1);
    assert_eq!(json["changed"][0]["path"], "people/getUser.yml");
    assert!(fs::read_to_string(&file).unwrap().contains("method: POST"));
}
