//! `apinata search`: hits as text or JSON, the cap note, and secret hygiene.

use crate::common;

use common::{ROOT, apinata, write_tree};
use serde_json::Value;

const DEV: &str = "name: dev\nvariables:\n  - name: token\n    secret: true\n";

fn request(name: &str, url: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n  headers:\n    - name: X-Needle\n      value: hay\n"
    )
}

#[test]
fn search_prints_text_hits_and_json_hits_that_agree() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/needle")),
        ("environments/dev.yml", DEV),
        (".env", "token=s3cr3t-dotenv-value\n"),
    ]);
    let text = apinata()
        .current_dir(dir.path())
        .args(["search", "needle"])
        .assert()
        .success();
    let text = String::from_utf8(text.get_output().stdout.clone()).unwrap();
    assert!(text.contains("a.yml"), "{text}");
    assert!(text.contains("http://x/needle"), "{text}");
    assert!(text.contains("X-Needle"), "{text}");

    let json = apinata()
        .current_dir(dir.path())
        .args(["search", "--json", "needle"])
        .assert()
        .success();
    let json: Value = serde_json::from_slice(&json.get_output().stdout).unwrap();
    assert_eq!(json["capped"], false);
    let fields: Vec<&str> = json["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["field"].as_str().unwrap())
        .collect();
    assert_eq!(fields, ["url", "header_name"]);
    assert_eq!(json["hits"][0]["path"], "a.yml");

    // A secret variable's value, even in .env, is never searched.
    let none = apinata()
        .current_dir(dir.path())
        .args(["search", "--json", "s3cr3t-dotenv-value"])
        .assert()
        .success();
    let none: Value = serde_json::from_slice(&none.get_output().stdout).unwrap();
    assert_eq!(none["hits"].as_array().unwrap().len(), 0);
}

#[test]
fn search_notes_the_cap_in_text_and_json() {
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
    let out = apinata()
        .current_dir(dir.path())
        .args(["search", "needle"])
        .assert()
        .success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(text.contains("showing the first 500 hits"), "{text}");
    let out = apinata()
        .current_dir(dir.path())
        .args(["search", "--json", "needle"])
        .assert()
        .success();
    let json: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(json["capped"], true);
    assert_eq!(json["hits"].as_array().unwrap().len(), 500);
}

fn many(count: usize) -> tempfile::TempDir {
    let files: Vec<(String, String)> = (0..count)
        .map(|i| {
            (
                format!("r{i:04}.yml"),
                request(&format!("r{i:04}"), "http://x/needle"),
            )
        })
        .collect();
    let mut refs: Vec<(&str, &str)> = vec![("opencollection.yml", ROOT)];
    refs.extend(files.iter().map(|(p, t)| (p.as_str(), t.as_str())));
    write_tree(&refs)
}

fn search_out(dir: &std::path::Path, args: &[&str]) -> String {
    let out = apinata()
        .current_dir(dir)
        .arg("search")
        .args(args)
        .assert()
        .success();
    String::from_utf8(out.get_output().stdout.clone()).unwrap()
}

#[test]
fn search_without_paging_flags_prints_exactly_the_legacy_output() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/needle")),
    ]);
    assert_eq!(
        search_out(dir.path(), &["needle"]),
        "a.yml  url  http://x/needle\na.yml  header_name X-Needle  X-Needle\n"
    );
    let json: Value = serde_json::from_str(&search_out(dir.path(), &["--json", "needle"])).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"hits": [
            {"path": "a.yml", "request": "a", "field": "url", "key": null,
             "snippet": "http://x/needle", "match_start": 9, "match_end": 15},
            {"path": "a.yml", "request": "a", "field": "header_name", "key": "X-Needle",
             "snippet": "X-Needle", "match_start": 2, "match_end": 8}
        ], "capped": false})
    );
}

#[test]
fn search_pages_past_the_cap_with_offset_and_limit() {
    let dir = many(1200);
    let tail = search_out(
        dir.path(),
        &["--json", "--offset", "2200", "--limit", "500", "needle"],
    );
    let tail: Value = serde_json::from_str(&tail).unwrap();
    assert_eq!(tail["hits"].as_array().unwrap().len(), 200);
    assert_eq!(tail["hits"][0]["path"], "r1100.yml");
    assert_eq!(
        (&tail["total"], &tail["has_more"]),
        (&2400.into(), &false.into())
    );
    assert!(tail.get("capped").is_none());
    let mid = search_out(dir.path(), &["--offset", "500", "--limit", "2", "needle"]);
    assert!(mid.ends_with("(showing 501-502 of 2400)\n"), "{mid}");
    assert!(mid.starts_with("r0250.yml  url"), "{mid}");
    let past = search_out(dir.path(), &["--offset", "5000", "needle"]);
    assert_eq!(past, "(no hits at offset 5000 of 2400)\n");
}

#[test]
fn search_limit_zero_exits_3_and_negative_values_are_rejected() {
    let dir = many(2);
    apinata()
        .current_dir(dir.path())
        .args(["search", "--limit", "0", "needle"])
        .assert()
        .code(3);
    for flag in ["--limit=-1", "--offset=-1"] {
        apinata()
            .current_dir(dir.path())
            .args(["search", flag, "needle"])
            .assert()
            .failure();
    }
}
