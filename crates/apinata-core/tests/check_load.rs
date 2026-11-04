use crate::common;
use apinata_core::{CheckError, CheckOptions, FindingKind, Severity, check_collection};
use common::{ROOT, check_tree, write_tree};

fn req(name: &str, seq: u32, url: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{url}\"\n"
    )
}

#[test]
fn a_clean_collection_has_no_findings() {
    let (_d, r) = check_tree(
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &req("a", 1, "http://x/a")),
        ],
        &CheckOptions::default(),
    );
    assert!(r.findings().is_empty(), "{:?}", r.findings());
    assert_eq!((r.errors(), r.warnings(), r.items_checked()), (0, 0, 1));
    assert!(r.environments().is_empty());
}

#[test]
fn an_unparsable_file_is_an_error_finding_with_its_relative_path() {
    let (_d, r) = check_tree(
        &[("opencollection.yml", ROOT), ("sub/bad.yml", "info: [")],
        &CheckOptions::default(),
    );
    let f = &r.findings()[0];
    assert_eq!(
        (f.item(), f.kind(), f.severity()),
        ("sub/bad.yml", FindingKind::UnparsableItem, Severity::Error)
    );
    assert_eq!(r.errors(), 1);
}

#[test]
fn a_broken_environment_is_reported_and_left_out() {
    let (_d, r) = check_tree(
        &[
            ("opencollection.yml", ROOT),
            ("environments/dev.yml", "name: dev\nvariables: []\n"),
            ("environments/bad.yml", "name: [\n"),
        ],
        &CheckOptions::default(),
    );
    assert_eq!(r.environments(), ["dev"]);
    assert!(
        r.findings()
            .iter()
            .any(|f| f.item() == "environments/bad.yml" && f.kind() == FindingKind::UnparsableItem)
    );
}

#[test]
fn env_selects_one_environment_and_an_unknown_name_is_an_error() {
    let files = [
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", "name: dev\nvariables: []\n"),
        ("environments/prod.yml", "name: prod\nvariables: []\n"),
    ];
    let (d, all) = check_tree(&files, &CheckOptions::default());
    assert_eq!(all.environments(), ["dev", "prod"]);
    let one = check_collection(d.path(), &CheckOptions::default().env("prod")).unwrap();
    assert_eq!(one.environments(), ["prod"]);
    let err = check_collection(d.path(), &CheckOptions::default().env("nope")).unwrap_err();
    assert!(matches!(err, CheckError::EnvironmentNotFound { ref name, .. } if name == "nope"));
}

#[test]
fn a_directory_that_is_not_a_collection_is_a_check_error() {
    let d = write_tree(&[("x.txt", "hi")]);
    assert!(matches!(
        check_collection(d.path(), &CheckOptions::default()),
        Err(CheckError::Open(_))
    ));
}

#[test]
fn equal_sequence_numbers_among_siblings_warn_on_the_later_item() {
    let (_d, r) = check_tree(
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &req("a", 1, "http://x/a")),
            ("b.yml", &req("b", 1, "http://x/b")),
            (
                "f1/folder.yml",
                "info:\n  name: f1\n  type: folder\n  seq: 3\n",
            ),
            (
                "f2/folder.yml",
                "info:\n  name: f2\n  type: folder\n  seq: 3\n",
            ),
            ("c.yml", &req("c", 0, "http://x/c")),
            ("d.yml", &req("d", 0, "http://x/d")),
        ],
        &CheckOptions::default(),
    );
    let dups: Vec<(&str, Severity)> = r
        .findings()
        .iter()
        .filter(|f| f.kind() == FindingKind::DuplicateSequence)
        .map(|f| (f.item(), f.severity()))
        .collect();
    assert_eq!(
        dups,
        [("b.yml", Severity::Warning), ("f2", Severity::Warning)]
    );
}

#[test]
fn the_report_serialises_with_snake_case_kinds_and_a_stable_shape() {
    let (_d, r) = check_tree(
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &req("a", 1, "http://x")),
            ("b.yml", &req("b", 1, "http://x")),
        ],
        &CheckOptions::default(),
    );
    let v = r.to_json();
    assert_eq!(v["findings"][0]["kind"], "duplicate_sequence");
    assert_eq!(v["findings"][0]["severity"], "warning");
    assert_eq!(v["findings"][0]["environment"], serde_json::Value::Null);
    assert_eq!(v["summary"]["warnings"], 1);
    assert_eq!(v["summary"]["items_checked"], 2);
    assert_eq!(v, serde_json::to_value(&r).unwrap());
    assert!(
        r.to_json_versioned()
            .to_string()
            .starts_with("{\"version\":1,\"summary\"")
    );
}
