//! `export::openapi`: per-collection notes, corpus snapshots, and a round trip through the
//! OpenAPI importer.

use crate::export_support;

use apinata_core::Collection;
use apinata_core::export::{OpenApiExport, openapi};
use apinata_core::import::{CollectionFormat, CollectionOptions, parse_collections};
use export_support::*;
use serde_json::Value;

#[test]
fn members_openapi_has_no_place_for_are_counted_once_per_collection() {
    let root = format!(
        "{ROOT}request:\n  headers:\n    - name: X-C\n      value: c\n  scripts:\n    - type: before-request\n      code: \"a();\"\n"
    );
    let folder = "info:\n  name: f\n  type: folder\n  seq: 1\nrequest:\n  headers:\n    - name: X-F\n      value: f\n  variables:\n    - name: v\n      value: \"1\"\n  scripts:\n    - type: tests\n      code: \"t();\"\n";
    let r1 = http("r1", "GET", "https://x.test/1", INHERIT)
        + "runtime:\n  variables:\n    - name: x\n      value: \"1\"\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n  scripts:\n    - type: after-response\n      code: \"b();\"\n";
    let r2 = http("r2", "GET", "/relative", INHERIT)
        + "runtime:\n  scripts:\n    - type: tests\n      code: \"t();\"\n    - type: before-request\n      code: \"  \"\n";
    let (_d, c) = open(&[
        ("opencollection.yml", &root),
        ("f/folder.yml", folder),
        ("f/r1.yml", &r1),
        ("r2.yml", &r2),
    ]);
    let tail = "; OpenAPI has no place for them";
    let want: Vec<(String, String)> = [
        (
            "r2 / url",
            "the URL has no origin OpenAPI can express; the request was left out".to_owned(),
        ),
        (
            "collection / scripts",
            format!("2 items have scripts{tail}"),
        ),
        ("collection / tests", format!("2 items have tests{tail}")),
        (
            "collection / assertions",
            format!("1 item has assertions{tail}"),
        ),
        (
            "collection / variables",
            format!("2 items have variables{tail}"),
        ),
        (
            "collection / headers",
            format!("2 items have headers{tail}"),
        ),
    ]
    .into_iter()
    .map(|(s, d)| (s.to_owned(), d))
    .collect();
    assert_eq!(openapi_notes(&c), want);
}

/// The JSON, then one `source: detail` line per note.
fn rendered(out: &OpenApiExport) -> String {
    let mut text = format!("{}--- notes\n", out.json);
    for n in &out.notes {
        text.push_str(&format!("{}: {}\n", n.source, n.detail));
    }
    text
}

#[test]
fn every_corpus_collection_matches_its_snapshot_and_parses_as_openapi_3_1() {
    for (label, root) in corpus() {
        let out = openapi(&Collection::open(&root).expect("opens"));
        assert_valid_openapi(&out.json);
        insta::assert_snapshot!(format!("openapi-{label}"), rendered(&out));
    }
}

/// `path` with each `{x}` written `:x`, as the importer writes request URLs.
fn colon(path: &str) -> String {
    let mut out = String::new();
    let mut rest = path;
    while let Some(open) = rest.find('{') {
        let close = open + rest[open..].find('}').expect("closed");
        out.push_str(&rest[..open]);
        out.push(':');
        out.push_str(&rest[open + 1..close]);
        rest = &rest[close + 1..];
    }
    out + rest
}

#[test]
fn every_corpus_export_round_trips_through_the_openapi_importer() {
    for (label, root) in corpus() {
        let json = openapi(&Collection::open(&root).expect("opens")).json;
        let v: Value = serde_json::from_str(&json).unwrap();
        let mut ours: Vec<(String, String)> = operations(&v)
            .into_iter()
            .map(|(path, method)| (method.to_uppercase(), colon(&path)))
            .collect();
        ours.sort();
        let bundles = parse_collections(
            Some(CollectionFormat::OpenApi),
            json.as_bytes(),
            &CollectionOptions::default(),
        )
        .unwrap_or_else(|e| panic!("{label}: {e:?}"));
        let mut theirs: Vec<(String, String)> = imported_requests(&bundles[0].collection.items, "")
            .into_iter()
            .map(|(_, item)| {
                let url = &item.request.url;
                let path = url.strip_prefix("{{baseUrl}}").unwrap_or(url);
                let path = path.split('?').next().unwrap_or_default();
                (item.request.method.to_uppercase(), path.to_owned())
            })
            .collect();
        theirs.sort();
        assert!(!ours.is_empty(), "{label}");
        assert_eq!(theirs, ours, "{label}");
    }
}
