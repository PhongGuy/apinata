//! `apinata init`: start an empty collection with the same files the app's New collection writes.

use crate::common::apinata;
use apinata_core::Collection;
use std::fs;
use std::path::Path;

/// Every file under `dir` as (relative path, bytes), sorted, so two trees compare whole.
fn files(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path
                    .strip_prefix(dir)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                out.push((rel, fs::read(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn init_in_an_empty_dir_writes_what_core_create_writes_and_ls_opens_it() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("shop");
    fs::create_dir(&dir).unwrap();
    let out = apinata().current_dir(&dir).arg("init").assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    assert!(stdout.contains("shop"), "{stdout}");

    let reference = tmp.path().join("reference").join("shop");
    fs::create_dir(reference.parent().unwrap()).unwrap();
    Collection::create(&reference, "shop").unwrap();
    assert_eq!(files(&dir), files(&reference));

    apinata().current_dir(&dir).arg("ls").assert().success();
}

#[test]
fn init_twice_refuses_naming_the_path() {
    let tmp = tempfile::tempdir().unwrap();
    apinata().arg("init").arg(tmp.path()).assert().success();
    let before = files(tmp.path());
    let out = apinata().arg("init").arg(tmp.path()).assert().code(3);
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("opencollection.yml"), "{stderr}");
    assert!(stderr.contains("already"), "{stderr}");
    assert_eq!(files(tmp.path()), before);
}

#[test]
fn name_flag_sets_the_collection_name() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("api");
    apinata()
        .args(["init", "--name", "My Shop"])
        .arg(&dir)
        .assert()
        .success();
    assert_eq!(Collection::open(&dir).unwrap().name(), "My Shop");
}

#[test]
fn init_creates_a_missing_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("new-api");
    apinata().arg("init").arg(&dir).assert().success();
    assert!(dir.join("opencollection.yml").is_file());
    assert_eq!(Collection::open(&dir).unwrap().name(), "new-api");
}
