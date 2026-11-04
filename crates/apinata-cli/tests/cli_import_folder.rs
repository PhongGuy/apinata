//! `apinata import collection --into <collection> --folder <path>` without `--update`.

use crate::common::apinata;
use apinata_core::Collection;
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
                    .into_owned();
                out.insert(rel, fs::read(&path).unwrap());
            }
        }
    }
    out
}

fn collection(parent: &Path) -> PathBuf {
    Collection::create(&parent.join("coll"), "coll").unwrap();
    parent.join("coll")
}

fn import(root: &Path, spec: &str, folder: &str) -> assert_cmd::assert::Assert {
    apinata()
        .args(["import", "collection", "--no-keychain"])
        .arg(fixture(spec))
        .arg("--into")
        .arg(root)
        .args(["--folder", folder])
        .assert()
}

#[test]
fn two_specs_in_two_folders_update_independently() {
    let parent = tempfile::tempdir().unwrap();
    let root = collection(parent.path());
    import(&root, "update-v1.json", "Shop").code(0);
    import(&root, "petstore/openapi.yaml", "Pets").code(0);
    assert!(root.join("Shop/people/getUser.yml").is_file());
    let text = fs::read_to_string(root.join("opencollection.yml")).unwrap();
    for name in ["Shop", "Pets"] {
        assert!(text.contains(&format!("{name}:")), "{text}");
    }
    assert!(text.contains("folders:"), "{text}");
    let pets_before: BTreeMap<_, _> = tree(&root.join("Pets"));

    apinata()
        .args(["import", "collection", "--update"])
        .arg(fixture("update-v2.json"))
        .arg("--into")
        .arg(&root)
        .args(["--folder", "Shop"])
        .assert()
        .code(0)
        .stdout(contains("added Shop/people/listUsers.yml"));
    assert_eq!(tree(&root.join("Pets")), pets_before);
}

#[test]
fn a_folder_that_holds_requests_is_refused_and_nothing_is_written() {
    let parent = tempfile::tempdir().unwrap();
    let root = collection(parent.path());
    import(&root, "update-v1.json", "Shop").code(0);
    let before = tree(&root);
    import(&root, "update-v2.json", "Shop")
        .code(3)
        .stderr(contains("already holds requests"));
    // A descendant counts too.
    import(&root, "update-v2.json", "Shop/people")
        .code(3)
        .stderr(contains("already holds requests"));
    assert_eq!(tree(&root), before);
}

#[test]
fn a_missing_nested_folder_is_created() {
    let parent = tempfile::tempdir().unwrap();
    let root = collection(parent.path());
    import(&root, "update-v1.json", "a/b").code(0);
    assert!(root.join("a/b/folder.yml").is_file());
    assert!(root.join("a/b/people/getUser.yml").is_file());
}

#[test]
fn folder_with_env_is_rejected() {
    let parent = tempfile::tempdir().unwrap();
    let root = collection(parent.path());
    let env = parent.path().join("env.json");
    fs::write(&env, r#"{"name":"e","values":[]}"#).unwrap();
    let before = tree(&root);
    apinata()
        .args(["import", "collection", "--no-keychain"])
        .arg(fixture("update-v1.json"))
        .arg("--into")
        .arg(&root)
        .args(["--folder", "Shop", "--env"])
        .arg(&env)
        .assert()
        .code(3)
        .stderr(contains("--env"));
    assert_eq!(tree(&root), before);
}

#[test]
fn folder_needs_an_existing_collection_and_creates_no_directory() {
    let parent = tempfile::tempdir().unwrap();
    let target = parent.path().join("nowhere");
    import(&target, "update-v1.json", "Shop").code(3);
    assert!(!target.exists());
}

#[test]
fn a_symlinked_folder_is_refused() {
    let parent = tempfile::tempdir().unwrap();
    let root = collection(parent.path());
    let outside = parent.path().join("outside");
    fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
    import(&root, "update-v1.json", "link")
        .code(3)
        .stderr(contains("symlink"));
    assert!(fs::read_dir(&outside).unwrap().next().is_none());
}

#[test]
fn json_output_keeps_its_shape() {
    let parent = tempfile::tempdir().unwrap();
    let root = collection(parent.path());
    let out = apinata()
        .args(["import", "collection", "--no-keychain", "--format", "json"])
        .arg(fixture("update-v1.json"))
        .arg("--into")
        .arg(&root)
        .args(["--folder", "Shop"])
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(
        json["collection"],
        fs::canonicalize(&root).unwrap().display().to_string()
    );
    assert!(json["items"].as_array().unwrap().len() >= 3);
}

#[test]
fn a_traversing_or_absolute_folder_is_rejected_and_nothing_is_written() {
    let parent = tempfile::tempdir().unwrap();
    let root = collection(parent.path());
    let before = tree(parent.path());
    let absolute = parent.path().join("abs").display().to_string();
    for bad in ["../x", "a/../../x", absolute.as_str(), ""] {
        import(&root, "update-v1.json", bad)
            .code(3)
            .stderr(contains("invalid folder path"));
    }
    assert_eq!(tree(parent.path()), before);
}
