//! `apinata import collection|environment` over files holding several collections or environments.

use crate::common;

use apinata_core::import::{CollectionFormat, CollectionOptions, parse_collections};
use common::{ROOT, apinata, write_tree};
use predicates::str::contains;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

const DEV: &str = r#"{"name":"dev","values":[{"key":"host","value":"x.test","enabled":true}]}"#;
const TWO_WORKSPACES: &str = r#"{"_type":"export","__export_format":4,"resources":[
    {"_id":"wrk_a","_type":"workspace","parentId":null,"name":"Alpha"},
    {"_id":"env_a","_type":"environment","parentId":"wrk_a","name":"A","data":{"host":"a.test"}},
    {"_id":"wrk_b","_type":"workspace","parentId":null,"name":"Beta"},
    {"_id":"env_b","_type":"environment","parentId":"wrk_b","name":"B","data":{"host":"b.test"}}]}"#;
const ENVS: &str = r#"[{"v":0,"name":"Env-I","variables":[{"key":"a","value":"1"}]},
    {"v":1,"name":"Env-II","variables":[{"key":"b","value":"2","secret":false}]}]"#;

/// A Hoppscotch workspace export holding one empty collection per name.
fn hoppscotch(names: &[&str]) -> String {
    let each: Vec<String> = names
        .iter()
        .map(|n| format!(r#"{{"v":12,"name":"{n}","folders":[],"requests":[]}}"#))
        .collect();
    format!("[{}]", each.join(","))
}

fn file(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, text).unwrap();
    path
}

fn names(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

#[test]
fn hoppscotch_keeps_the_failing_name_verbatim() {
    let text = hoppscotch(&["Alpha", "CON", "Gamma"]);
    let bundles = parse_collections(
        Some(CollectionFormat::Hoppscotch),
        text.as_bytes(),
        &CollectionOptions::default(),
    )
    .unwrap();
    let names: Vec<&str> = bundles.iter().map(|b| b.collection.name.as_str()).collect();
    assert_eq!(
        names,
        ["Alpha", "CON", "Gamma"],
        "the partial-failure tests need `CON` to reach the writer"
    );
}

#[test]
fn every_collection_in_the_file_is_written_and_listed() {
    let input = tempfile::tempdir().unwrap();
    let into = tempfile::tempdir().unwrap();
    let out = apinata()
        .args(["import", "collection"])
        .arg(file(
            input.path(),
            "ws.json",
            &hoppscotch(&["Alpha", "Beta"]),
        ))
        .arg("--into")
        .arg(into.path())
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let root = fs::canonicalize(into.path()).unwrap();
    let want = format!(
        "{}\n{}\n",
        root.join("Alpha").display(),
        root.join("Beta").display()
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), want);
    assert_eq!(names(into.path()), ["Alpha", "Beta"]);
}

#[test]
fn each_collection_keeps_its_own_environments() {
    let input = tempfile::tempdir().unwrap();
    let into = tempfile::tempdir().unwrap();
    apinata()
        .args(["import", "collection", "--no-keychain"])
        .arg(file(input.path(), "two.json", TWO_WORKSPACES))
        .arg("--into")
        .arg(into.path())
        .assert()
        .code(0);
    assert_eq!(names(&into.path().join("Alpha/environments")), ["A.yml"]);
    assert_eq!(names(&into.path().join("Beta/environments")), ["B.yml"]);
}

#[test]
fn json_prints_one_report_per_collection() {
    let input = tempfile::tempdir().unwrap();
    let into = tempfile::tempdir().unwrap();
    let out = apinata()
        .args(["import", "collection", "--format", "json"])
        .arg(file(
            input.path(),
            "ws.json",
            &hoppscotch(&["Alpha", "Beta"]),
        ))
        .arg("--into")
        .arg(into.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(
        out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let reports: Vec<Value> = serde_json::Deserializer::from_slice(&out.stdout)
        .into_iter::<Value>()
        .map(|v| v.unwrap())
        .collect();
    let roots: Vec<&str> = reports
        .iter()
        .map(|r| r["collection"].as_str().unwrap())
        .collect();
    assert_eq!(roots.len(), 2);
    assert!(
        roots[0].ends_with("Alpha") && roots[1].ends_with("Beta"),
        "{roots:?}"
    );
}

#[test]
fn env_files_join_only_a_single_collection() {
    let input = tempfile::tempdir().unwrap();
    let into = tempfile::tempdir().unwrap();
    let dev = file(input.path(), "dev.json", DEV);
    apinata()
        .args(["import", "collection"])
        .arg(file(
            input.path(),
            "ws.json",
            &hoppscotch(&["Alpha", "Beta"]),
        ))
        .arg("--env")
        .arg(&dev)
        .arg("--into")
        .arg(into.path())
        .assert()
        .code(3)
        .stderr(contains(
            "error: --env needs an input with one collection; this file holds 2",
        ));
    assert!(names(into.path()).is_empty());
    apinata()
        .args(["import", "collection", "--no-keychain"])
        .arg(file(input.path(), "solo.json", &hoppscotch(&["Solo"])))
        .arg("--env")
        .arg(&dev)
        .arg("--into")
        .arg(into.path())
        .assert()
        .code(0);
    assert!(into.path().join("Solo/environments/dev.yml").is_file());
}

#[test]
fn a_failed_write_keeps_earlier_collections_and_names_both_sets() {
    let input = tempfile::tempdir().unwrap();
    let into = tempfile::tempdir().unwrap();
    let out = apinata()
        .args(["import", "collection"])
        .arg(file(
            input.path(),
            "ws.json",
            &hoppscotch(&["Alpha", "CON", "Gamma"]),
        ))
        .arg("--into")
        .arg(into.path())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        (out.status.code(), out.stdout.is_empty()),
        (Some(3), true),
        "{stderr}"
    );
    let root = fs::canonicalize(into.path()).unwrap();
    let tail: Vec<&str> = stderr.lines().rev().take(3).collect();
    let want = [
        "not written: CON, Gamma".to_owned(),
        format!("written: {}", root.join("Alpha").display()),
        "error: `CON` is not a valid collection name".to_owned(),
    ];
    assert_eq!(tail, want, "{stderr}");
    assert_eq!(
        names(into.path()),
        ["Alpha"],
        "no staging directory or later collection is left"
    );
}

#[test]
fn a_first_write_failure_prints_only_the_error() {
    let input = tempfile::tempdir().unwrap();
    let into = tempfile::tempdir().unwrap();
    let out = apinata()
        .args(["import", "collection"])
        .arg(file(input.path(), "ws.json", &hoppscotch(&["CON", "Beta"])))
        .arg("--into")
        .arg(into.path())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(3));
    // The writer's own error, not a refusal of the file: the plural path reached the first write.
    assert_eq!(
        stderr.lines().last(),
        Some("error: `CON` is not a valid collection name"),
        "{stderr}"
    );
    assert!(!stderr.contains("written:"), "{stderr}");
    assert!(names(into.path()).is_empty());
}

#[test]
fn an_environment_file_with_several_environments_writes_each() {
    let input = tempfile::tempdir().unwrap();
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    apinata()
        .args(["import", "environment", "--no-keychain"])
        .arg(file(input.path(), "bulk.json", ENVS))
        .arg("--collection")
        .arg(col.path())
        .assert()
        .code(0)
        .stdout("Env-I\nEnv-II\n");
    assert_eq!(
        names(&col.path().join("environments")),
        ["Env-I.yml", "Env-II.yml"]
    );
}

#[test]
fn an_env_file_with_several_environments_joins_whole() {
    let input = tempfile::tempdir().unwrap();
    let into = tempfile::tempdir().unwrap();
    apinata()
        .args(["import", "collection", "--no-keychain"])
        .arg(file(input.path(), "solo.json", &hoppscotch(&["Solo"])))
        .arg("--env")
        .arg(file(input.path(), "bulk.json", ENVS))
        .arg("--into")
        .arg(into.path())
        .assert()
        .code(0);
    assert_eq!(
        names(&into.path().join("Solo/environments")),
        ["Env-I.yml", "Env-II.yml"]
    );
}
