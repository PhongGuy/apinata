//! `--group-by` for OpenAPI and `--name` for HAR on `apinata import collection`.

use crate::common;

use apinata_core::MemoryStore;
use apinata_core::import::{
    CollectionFormat, CollectionOptions, GroupBy, parse_collections, write_collection,
};
use common::apinata;
use predicates::str::contains;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const OPENAPI: &str = r#"{"openapi":"3.0.3","info":{"title":"Shop","version":"1"},
    "paths":{"/users/{id}":{"get":{"tags":["people"],"operationId":"getUser","responses":{"200":{"description":"ok"}}}}}}"#;
const HAR: &str = r#"{"log":{"version":"1.2","entries":[{"request":{"method":"GET","url":"https://x.test/a","headers":[]}}]}}"#;
const HOPPSCOTCH: &str = r#"{"v":12,"name":"Solo","folders":[],"requests":[]}"#;
const POSTMAN: &str = r#"{"info":{"name":"P","schema":"https://schema.getpostman.com/json/collection/v2.1.0/collection.json"},"item":[]}"#;

fn file(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, text).unwrap();
    path
}

/// Every file under `root` by its `/`-separated relative path.
fn tree(root: &Path) -> BTreeMap<String, String> {
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
                out.insert(rel, fs::read_to_string(&path).unwrap());
            }
        }
    }
    out
}

fn import(input: &Path, extra: &[&str], into: &Path) -> assert_cmd::assert::Assert {
    apinata()
        .args(["import", "collection"])
        .arg(input)
        .args(extra)
        .arg("--into")
        .arg(into)
        .assert()
}

#[test]
fn group_by_path_writes_what_core_writes() {
    let input = tempfile::tempdir().unwrap();
    let api = file(input.path(), "api.json", OPENAPI);
    let (by_path, by_tag, core) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    import(&api, &["--group-by", "path"], by_path.path()).code(0);
    import(&api, &[], by_tag.path()).code(0);
    let mut options = CollectionOptions::default();
    options.group_by = GroupBy::Path;
    let bundles = parse_collections(
        Some(CollectionFormat::OpenApi),
        OPENAPI.as_bytes(),
        &options,
    )
    .unwrap();
    write_collection(
        core.path(),
        &bundles[0].collection,
        &bundles[0].environments,
        &MemoryStore::new(),
    )
    .unwrap();
    assert_eq!(
        tree(&by_path.path().join("Shop")),
        tree(&core.path().join("Shop"))
    );
    assert_ne!(
        tree(&by_path.path().join("Shop")),
        tree(&by_tag.path().join("Shop")),
        "the flag changes the layout"
    );
}

#[test]
fn group_by_with_another_format_is_refused() {
    let input = tempfile::tempdir().unwrap();
    let into = tempfile::tempdir().unwrap();
    let message = "error: --group-by applies to OpenAPI input only";
    import(
        &file(input.path(), "h.json", HOPPSCOTCH),
        &["--group-by", "path"],
        into.path(),
    )
    .code(3)
    .stderr(contains(message));
    import(
        &file(input.path(), "p.json", POSTMAN),
        &["--from", "postman", "--group-by", "tags"],
        into.path(),
    )
    .code(3)
    .stderr(contains(message));
    import(
        &file(input.path(), "a.json", OPENAPI),
        &["--group-by", "tag"],
        into.path(),
    )
    .code(3)
    .stderr(contains("tags"));
    assert_eq!(fs::read_dir(into.path()).unwrap().count(), 0);
}

#[test]
fn a_har_capture_is_named_by_the_flag_else_the_file_stem() {
    let input = tempfile::tempdir().unwrap();
    let into = tempfile::tempdir().unwrap();
    let har = file(input.path(), "session.har", HAR);
    import(&har, &[], into.path()).code(0);
    import(&har, &["--name", "Checkout flow"], into.path()).code(0);
    assert!(into.path().join("session/opencollection.yml").is_file());
    assert!(
        into.path()
            .join("Checkout flow/opencollection.yml")
            .is_file()
    );
}

#[test]
fn name_with_another_format_is_refused() {
    let input = tempfile::tempdir().unwrap();
    let into = tempfile::tempdir().unwrap();
    import(
        &file(input.path(), "h.json", HOPPSCOTCH),
        &["--name", "x"],
        into.path(),
    )
    .code(3)
    .stderr(contains("error: --name applies to HAR input only"));
    assert_eq!(fs::read_dir(into.path()).unwrap().count(), 0);
}

#[test]
fn help_lists_the_flags_and_from_lists_every_format() {
    let out = apinata()
        .args(["import", "collection", "--help"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    for flag in ["--group-by", "--name", "tags", "path"] {
        assert!(text.contains(flag), "{flag}: {text}");
    }
    assert!(!text.to_lowercase().contains("bruno"), "{text}");
    let input = tempfile::tempdir().unwrap();
    let into = tempfile::tempdir().unwrap();
    import(
        &file(input.path(), "h.json", HOPPSCOTCH),
        &["--from", "nope"],
        into.path(),
    )
    .code(3)
    .stderr(contains(
        "expected auto, postman, openapi, insomnia, hoppscotch, har",
    ));
}
