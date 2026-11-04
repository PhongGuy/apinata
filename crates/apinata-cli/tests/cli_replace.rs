//! `apinata replace`: dry runs write nothing, real runs change only the matched bytes.

use crate::common;

use common::{ROOT, apinata, write_tree};
use serde_json::Value;

const DEV: &str = "name: dev\nvariables:\n  - name: token\n    secret: true\n";

fn request(name: &str, url: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: 1\n\nhttp:\n  method: GET\n  url: {url}\n  headers:\n    - name: X-Keep\n      value: hay\n"
    )
}

fn stdout(assert: assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stdout.clone()).unwrap()
}

#[test]
fn dry_run_lists_hits_by_path_and_writes_nothing() {
    let a = request("a", "http://x/needle");
    let dir = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let out = stdout(
        apinata()
            .current_dir(dir.path())
            .args(["replace", "needle", "thread", "--dry-run"])
            .assert()
            .success(),
    );
    assert!(out.contains("a.yml"), "{out}");
    assert!(out.contains("http://x/needle"), "{out}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.yml")).unwrap(),
        a
    );

    let json = stdout(
        apinata()
            .current_dir(dir.path())
            .args(["replace", "needle", "thread", "--dry-run", "--json"])
            .assert()
            .success(),
    );
    let json: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(json["dry_run"], true);
    assert_eq!(json["hits"][0]["path"], "a.yml");
}

#[test]
fn real_run_prints_the_summary_and_changes_only_the_match() {
    let a = request("a", "http://x/needle");
    let dir = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let out = stdout(
        apinata()
            .current_dir(dir.path())
            .args(["replace", "needle", "thread"])
            .assert()
            .success(),
    );
    assert_eq!(out, "Replaced 1 in 1 file\n");
    let after = std::fs::read_to_string(dir.path().join("a.yml")).unwrap();
    assert_eq!(after, a.replace("needle", "thread"));
}

#[test]
fn real_run_json_reports_the_counts() {
    let a = request("a", "http://x/needle");
    let dir = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let json = stdout(
        apinata()
            .current_dir(dir.path())
            .args(["replace", "--json", "needle", "thread"])
            .assert()
            .success(),
    );
    let json: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(json["replaced"], 1);
    assert_eq!(json["files_changed"], 1);
    assert_eq!(json["scripts_changed"], false);
    assert_eq!(json["summary"], "Replaced 1 in 1 file");
}

#[test]
fn regex_groups_expand_in_the_replacement() {
    let a = request("a", "http://x/user-42");
    let dir = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    apinata()
        .current_dir(dir.path())
        .args(["replace", "--regex", r"user-(\d+)", "id/$1"])
        .assert()
        .success();
    let after = std::fs::read_to_string(dir.path().join("a.yml")).unwrap();
    assert_eq!(after, a.replace("user-42", "id/42"));
}

#[test]
fn matching_ignores_case_unless_asked() {
    let a = request("a", "http://x/Needle");
    let dir = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    apinata()
        .current_dir(dir.path())
        .args(["replace", "--case", "needle", "thread"])
        .assert()
        .success();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.yml")).unwrap(),
        a
    );
    apinata()
        .current_dir(dir.path())
        .args(["replace", "needle", "thread"])
        .assert()
        .success();
    let after = std::fs::read_to_string(dir.path().join("a.yml")).unwrap();
    assert_eq!(after, a.replace("Needle", "thread"));
}

#[test]
fn an_invalid_regex_exits_3_and_writes_nothing() {
    let a = request("a", "http://x/needle");
    let dir = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    apinata()
        .current_dir(dir.path())
        .args(["replace", "--regex", "(", "x"])
        .assert()
        .code(3)
        .stderr(predicates::str::contains("invalid regular expression"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.yml")).unwrap(),
        a
    );
}

#[test]
fn secret_values_and_dotenv_are_never_matched() {
    let a = request("a", "http://x/needle");
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &a),
        ("environments/dev.yml", DEV),
        (".env", "token=s3cr3t-dotenv-value\n"),
    ]);
    let out = stdout(
        apinata()
            .current_dir(dir.path())
            .args(["replace", "s3cr3t-dotenv-value", "x"])
            .assert()
            .success(),
    );
    assert_eq!(out, "Replaced 0 in 0 files\n");
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".env")).unwrap(),
        "token=s3cr3t-dotenv-value\n"
    );
}

#[test]
fn no_matches_exits_0() {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &request("a", "u"))]);
    apinata()
        .current_dir(dir.path())
        .args(["replace", "absent", "x"])
        .assert()
        .success();
}

#[test]
fn dry_run_notes_the_cap_in_text_and_json() {
    let files: Vec<(String, String)> = (0..501)
        .map(|i| {
            (
                format!("r{i:03}.yml"),
                request(&format!("r{i:03}"), "http://x/needle"),
            )
        })
        .collect();
    let mut refs: Vec<(&str, &str)> = vec![("opencollection.yml", ROOT)];
    refs.extend(files.iter().map(|(p, t)| (p.as_str(), t.as_str())));
    let dir = write_tree(&refs);
    let text = stdout(
        apinata()
            .current_dir(dir.path())
            .args(["replace", "needle", "thread", "--dry-run"])
            .assert()
            .success(),
    );
    assert!(text.contains("showing the first 500 hits"), "{text}");
    let json = stdout(
        apinata()
            .current_dir(dir.path())
            .args(["replace", "needle", "thread", "--dry-run", "--json"])
            .assert()
            .success(),
    );
    let json: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(json["capped"], true);
}
