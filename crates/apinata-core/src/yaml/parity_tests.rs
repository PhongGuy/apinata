//! Byte parity with Bruno's writer, against the committed Node reference corpus.

use super::emit::to_bruno_yaml as emit_mapping;
use std::fs;
use std::path::{Path, PathBuf};

/// Emit a document whose root is known to be a mapping.
fn to_bruno_yaml(value: &yaml_serde::Value) -> String {
    emit_mapping(value.as_mapping().expect("corpus documents are mappings"))
}

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/reference/2026-10-09-writer-parity")
}

#[derive(serde::Deserialize)]
struct Case {
    id: String,
    value: serde_json::Value,
}

/// Emit every corpus case whose id starts with one of `prefixes`, and report all mismatches at once.
fn check(prefixes: &[&str]) {
    let text = fs::read_to_string(corpus_dir().join("cases.json")).expect("cases.json");
    let cases: Vec<Case> = serde_json::from_str(&text).expect("valid cases.json");
    let selected: Vec<Case> = cases
        .into_iter()
        .filter(|c| prefixes.iter().any(|p| c.id.starts_with(p)))
        .collect();
    assert!(!selected.is_empty(), "no corpus cases match {prefixes:?}");

    let mut failures = Vec::new();
    for case in &selected {
        let value: yaml_serde::Value =
            yaml_serde::to_value(&case.value).expect("json converts to yaml");
        let expected_path = corpus_dir()
            .join("expected/cases")
            .join(format!("{}.yml", case.id));
        let expected = fs::read_to_string(&expected_path).expect("expected output");
        let actual = to_bruno_yaml(&value);
        if actual != expected {
            failures.push(format!(
                "{}\n  expected {expected:?}\n  actual   {actual:?}",
                case.id
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases differ:\n{}",
        failures.len(),
        selected.len(),
        failures.join("\n")
    );
}

#[test]
fn plain_and_quoted_scalars_match_bruno() {
    check(&["scalar-"]);
}

#[test]
fn block_scalars_match_bruno() {
    check(&["block-"]);
}

#[test]
fn keys_match_bruno() {
    check(&["key-"]);
}

#[test]
fn numbers_booleans_and_null_match_bruno() {
    check(&["number-", "literal-"]);
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bruno")
}

fn yml_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).expect("readable dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            out.extend(yml_files(&path));
        } else if path.extension().is_some_and(|e| e == "yml") {
            out.push(path);
        }
    }
    out.sort();
    out
}

#[test]
fn structures_match_bruno() {
    check(&["struct-"]);
}

#[test]
fn bruno_fixtures_match_bruno_bytes() {
    let fixtures = fixtures_dir();
    let files = yml_files(&fixtures);
    assert_eq!(files.len(), 27);
    let mut failures = Vec::new();
    for file in &files {
        let rel = file.strip_prefix(&fixtures).expect("under fixtures");
        let value: yaml_serde::Value =
            yaml_serde::from_str(&fs::read_to_string(file).expect("fixture")).expect("parses");
        let expected = fs::read_to_string(corpus_dir().join("expected/fixtures").join(rel))
            .expect("expected fixture");
        let actual = to_bruno_yaml(&value);
        if actual != expected {
            failures.push(format!(
                "{}\n  expected {expected:?}\n  actual   {actual:?}",
                rel.display()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} fixtures differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn bruno_fixtures_round_trip_to_the_same_value() {
    for file in yml_files(&fixtures_dir()) {
        let value: yaml_serde::Value =
            yaml_serde::from_str(&fs::read_to_string(&file).expect("fixture")).expect("parses");
        let emitted = to_bruno_yaml(&value);
        let again: yaml_serde::Value = yaml_serde::from_str(&emitted).expect("emitted yaml parses");
        assert_eq!(value, again, "{}", file.display());
        // Mapping equality ignores order, so compare key order through a canonical dump too.
        assert_eq!(
            yaml_serde::to_string(&value).expect("dump"),
            yaml_serde::to_string(&again).expect("dump"),
            "{}",
            file.display()
        );
    }
}
