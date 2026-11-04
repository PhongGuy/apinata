//! The edit parity corpus itself, and the M1 setters against it.

use crate::parity;

use apinata_core::RequestFile;
use parity::{all_cases, check_requests, corpus_dir};
use std::fs;
use std::path::{Path, PathBuf};

/// Every case prefix, one per test that claims it.
const PREFIXES: &[&str] = &[
    "info-",
    "opaque-",
    "headers-",
    "tags-",
    "docs-",
    "vars-",
    "assert-",
    "settings-",
    "body-",
    "auth-",
    "url-",
    "params-",
];

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

fn relative(files: Vec<PathBuf>, base: &Path) -> Vec<PathBuf> {
    files
        .into_iter()
        .map(|f| f.strip_prefix(base).expect("under base").to_path_buf())
        .collect()
}

#[test]
fn every_vendored_fixture_is_normalised() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bruno");
    let normalized = corpus_dir().join("normalized");
    let mut expected = relative(yml_files(&fixtures), &fixtures);
    expected.push(PathBuf::from("synthetic/graphql.yml"));
    expected.sort();
    assert_eq!(relative(yml_files(&normalized), &normalized), expected);
}

#[test]
fn every_case_has_exactly_one_owner_prefix() {
    let cases = all_cases();
    for case in &cases {
        let owners = PREFIXES.iter().filter(|p| case.id.starts_with(*p)).count();
        assert_eq!(owners, 1, "case {} has {owners} owner prefixes", case.id);
        assert!(
            corpus_dir()
                .join("normalized")
                .join(&case.fixture)
                .is_file(),
            "case {} names a missing fixture",
            case.id
        );
    }
    for prefix in PREFIXES {
        assert!(
            cases.iter().any(|c| c.id.starts_with(prefix)),
            "no cases for {prefix}"
        );
    }
}

#[test]
fn normalised_requests_load_clean_and_save_nothing() {
    let normalized = corpus_dir().join("normalized");
    let requests: Vec<PathBuf> = yml_files(&normalized)
        .into_iter()
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name != "folder.yml"
                && name != "opencollection.yml"
                && !p.parent().is_some_and(|d| d.ends_with("environments"))
                && !p.starts_with(normalized.join("synthetic"))
        })
        .collect();
    assert_eq!(requests.len(), 13);
    for source in requests {
        let rel = source.strip_prefix(&normalized).expect("under corpus");
        let (_dir, path) = parity::copy_fixture(rel.to_str().expect("utf-8"));
        let mut request = RequestFile::load(&path).expect("loads");
        assert!(!request.is_dirty(), "{}", rel.display());
        request.save().expect("saves");
        assert_eq!(
            fs::read_to_string(&path).expect("read"),
            fs::read_to_string(&source).expect("read"),
            "{}",
            rel.display()
        );
    }
}

#[test]
fn m1_setters_match_bruno() {
    check_requests("info-", |id, r| match id {
        _ if id.starts_with("info-name-") => r.set_name("renamed"),
        "info-method" => r.set_method("POST"),
        "info-seq" => r.set_seq(9),
        "info-url-plain" => r.set_url("https://api.example.com/changed"),
        other => panic!("no Apinata edit for {other}"),
    });
}
