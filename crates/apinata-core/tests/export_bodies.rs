//! `export::postman` bodies, body pruning, and a round trip of every request through the
//! Postman importer.

use crate::export_support;

use apinata_core::export::postman;
use apinata_core::import::parse_postman;
use apinata_core::{BodySpec, Collection, KeyValue};
use export_support::*;
use serde_json::{Value, json};

const JSON_BODY: &str = "  body:\n    type: json\n    data: '{\"a\": 1}'\n";
const FORM: &str = "  body:\n    type: form-urlencoded\n    data:\n      - name: a\n        value: \"1\"\n      - name: b\n        value: x\n        description: note\n        disabled: true\n";
const MULTIPART: &str = "  body:\n    type: multipart-form\n    data:\n      - name: a\n        type: text\n        value: \"1\"\n      - name: up\n        type: file\n        value: [/data/up.bin]\n        contentType: text/plain\n      - name: two\n        type: file\n        value: [/a, /b]\n      - name: none\n        type: file\n        value: []\n        disabled: true\n";
const FILE: &str = "  body:\n    type: file\n    data:\n      - filePath: /data/a.json\n        selected: false\n      - filePath: /data/b.json\n        contentType: application/json\n        selected: true\n";
const XML: &str = "  body:\n    type: xml\n    data: <a/>\n";
const TEXT: &str = "  body:\n    type: text\n    data: hi\n";
const GRAPHQL: &str = "  body:\n    type: graphql\n";

fn raw(text: &str, language: &str) -> Option<Value> {
    Some(json!({"mode": "raw", "raw": text, "options": {"raw": {"language": language}}}))
}

#[test]
fn bodies_map_by_mode_and_a_get_with_a_body_disables_pruning() {
    let cases: Vec<(&str, &str, Option<Value>)> = vec![
        ("POST", JSON_BODY, raw("{\"a\": 1}", "json")),
        (
            "POST",
            FORM,
            Some(json!({"mode": "urlencoded", "urlencoded": [
            {"key": "a", "value": "1", "disabled": false, "type": "default", "description": ""},
            {"key": "b", "value": "x", "disabled": true, "type": "default", "description": "note"}]})),
        ),
        (
            "POST",
            MULTIPART,
            Some(json!({"mode": "formdata", "formdata": [
            {"key": "a", "disabled": false, "type": "text", "description": "", "value": "1"},
            {"key": "up", "disabled": false, "type": "file", "description": "", "src": "/data/up.bin", "contentType": "text/plain"},
            {"key": "two", "disabled": false, "type": "file", "description": "", "src": ["/a", "/b"]},
            {"key": "none", "disabled": true, "type": "file", "description": "", "src": null}]})),
        ),
        (
            "PUT",
            FILE,
            Some(json!({"mode": "file", "file": {"src": "/data/b.json"}})),
        ),
        ("POST", XML, raw("<a/>", "xml")),
        ("POST", TEXT, raw("hi", "text")),
        ("POST", GRAPHQL, Some(json!({"mode": "raw", "raw": ""}))),
        ("GET", JSON_BODY, raw("{\"a\": 1}", "json")),
        ("GET", "", None),
    ];
    let files: Vec<(String, String)> = cases
        .iter()
        .enumerate()
        .map(|(i, (method, body, _))| {
            (
                format!("{i}.yml"),
                http(
                    &format!("r{i}"),
                    method,
                    "http://x.test",
                    &format!("{body}{INHERIT}"),
                ),
            )
        })
        .collect();
    let mut all = vec![("opencollection.yml", ROOT)];
    all.extend(files.iter().map(|(p, t)| (p.as_str(), t.as_str())));
    let (_d, c) = open(&all);
    let v = exported(&c);
    for (i, (_, _, want)) in cases.iter().enumerate() {
        assert_eq!(
            v["item"][i]["request"].get("body"),
            want.as_ref(),
            "case {i}"
        );
        // A file without a settings block forwards Authorization, which the export states.
        let behavior = if i == 7 {
            json!({"disableBodyPruning": true, "followAuthorizationHeader": true})
        } else {
            json!({"followAuthorizationHeader": true})
        };
        assert_eq!(
            v["item"][i].get("protocolProfileBehavior"),
            Some(&behavior),
            "case {i}"
        );
    }
    assert_eq!(
        keys(&v["item"][7]),
        ["name", "protocolProfileBehavior", "request"]
    );
    assert_eq!(
        keys(&v["item"][0]["request"]),
        ["method", "header", "description", "url", "body"]
    );
    assert_eq!(
        notes(&c),
        pairs(&[(
            "r6 / body",
            "the graphql body cannot be exported; an empty body was written"
        )])
    );
    assert_round_trip("bodies", &c);
}

#[test]
fn every_corpus_request_survives_a_round_trip_through_the_importer() {
    for (label, root) in corpus() {
        assert_round_trip(label, &Collection::open(&root).expect("opens"));
    }
}

/// Export, import again, and compare each request's name, method, URL, enabled headers and body.
fn assert_round_trip(label: &str, c: &Collection) {
    let back =
        parse_postman(postman(c).json.as_bytes()).unwrap_or_else(|e| panic!("{label}: {e:?}"));
    assert_eq!(back.name, c.name(), "{label}");
    let ours = source_requests(c.tree(), "");
    let theirs = imported_requests(&back.items, "");
    let our_names: Vec<&str> = ours.iter().map(|(n, _)| n.as_str()).collect();
    let their_names: Vec<&str> = theirs.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(our_names, their_names, "{label}");
    for ((name, r), (_, item)) in ours.iter().zip(&theirs) {
        let i = &item.request;
        assert_eq!(i.method, r.method().to_uppercase(), "{label}: {name}");
        assert_eq!(i.url, r.url(), "{label}: {name}");
        assert_eq!(
            enabled(&i.headers),
            enabled(&r.headers()),
            "{label}: {name}"
        );
        assert!(
            same_body(&r.body(), &i.body),
            "{label}: {name}: {:?} became {:?}",
            r.body(),
            i.body
        );
    }
}

fn enabled(headers: &[KeyValue]) -> Vec<(&str, &str)> {
    headers
        .iter()
        .filter(|h| h.enabled)
        .map(|h| (h.name.as_str(), h.value.as_str()))
        .collect()
}

// A file body comes back as its one selected path; an unsupported body has nothing to compare.
fn same_body(ours: &BodySpec, theirs: &BodySpec) -> bool {
    match (ours, theirs) {
        (BodySpec::File(files), BodySpec::File(back)) => {
            let selected = files
                .iter()
                .find(|f| f.selected)
                .map_or("", |f| f.path.as_str());
            back.len() == 1 && back[0].path == selected
        }
        (BodySpec::Unsupported { .. }, _) => true,
        _ => ours == theirs,
    }
}

#[test]
fn graphql_requests_export_as_graphql_bodies_and_round_trip() {
    let full = "info:\n  name: g1\n  type: graphql\n  seq: 1\ngraphql:\n  method: POST\n  url: https://x.test/graphql\n  body:\n    query: \"{ me }\"\n    variables: '{\"a\": 1}'\n";
    let bare =
        "info:\n  name: g2\n  type: graphql\n  seq: 2\ngraphql:\n  url: https://x.test/graphql\n";
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        ("g1.yml", full),
        ("g2.yml", bare),
    ]);
    let v = exported(&c);
    assert_eq!(
        v["item"][0]["request"]["body"],
        json!({"mode": "graphql", "graphql": {"query": "{ me }", "variables": "{\"a\": 1}"}})
    );
    assert_eq!(
        v["item"][1]["request"]["body"],
        json!({"mode": "graphql", "graphql": {"query": "", "variables": ""}})
    );
    assert_eq!(v["item"][1]["request"]["method"], "POST");
    assert!(notes(&c).is_empty(), "{:?}", notes(&c));
    assert_round_trip("graphql", &c);
}
