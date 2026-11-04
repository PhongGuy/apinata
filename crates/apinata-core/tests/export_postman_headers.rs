//! `export::postman` copies the collection and folder headers a request inherits into it.

use crate::export_support;

use apinata_core::export::postman;
use apinata_core::import::{parse_postman, write_collection};
use apinata_core::{Collection, MemoryStore};
use export_support::*;
use serde_json::{Value, json};

fn rows(pairs: &[(&str, &str)]) -> String {
    let mut out = String::from("request:\n  headers:\n");
    for (name, value) in pairs {
        out.push_str(&format!("    - name: {name}\n      value: \"{value}\"\n"));
    }
    out
}

fn folder(name: &str, pairs: &[(&str, &str)]) -> String {
    format!(
        "info:\n  name: {name}\n  type: folder\n  seq: 1\n{}",
        rows(pairs)
    )
}

fn own(pairs: &[(&str, &str)]) -> String {
    let mut out = String::from("  headers:\n");
    for (name, value) in pairs {
        out.push_str(&format!("    - name: {name}\n      value: \"{value}\"\n"));
    }
    out
}

fn header_pairs(v: &Value) -> Vec<(String, String, bool)> {
    v["header"]
        .as_array()
        .expect("headers")
        .iter()
        .map(|h| {
            (
                h["key"].as_str().unwrap().to_owned(),
                h["value"].as_str().unwrap().to_owned(),
                h["disabled"].as_bool().unwrap(),
            )
        })
        .collect()
}

fn triples(list: &[(&str, &str, bool)]) -> Vec<(String, String, bool)> {
    list.iter()
        .map(|(a, b, c)| ((*a).to_owned(), (*b).to_owned(), *c))
        .collect()
}

#[test]
fn a_request_gets_the_collection_then_the_folder_headers_before_its_own() {
    let root = format!("{ROOT}{}", rows(&[("X-C", "{{c}}")]));
    let (_d, c) = open(&[
        ("opencollection.yml", &root),
        ("f/folder.yml", &folder("f", &[("X-F", "f")])),
        (
            "f/r.yml",
            &http(
                "r",
                "GET",
                "http://x.test/r",
                &(INHERIT.to_owned() + &own(&[("X-R", "r")])),
            ),
        ),
    ]);
    let v = exported(&c);
    let req = &v["item"][0]["item"][0]["request"];
    assert_eq!(
        header_pairs(req),
        triples(&[
            ("X-C", "{{c}}", false),
            ("X-F", "f", false),
            ("X-R", "r", false)
        ])
    );
    assert_eq!(
        req["header"][0],
        json!({"key": "X-C", "value": "{{c}}", "description": "", "disabled": false, "type": "default"})
    );
}

#[test]
fn an_own_header_replaces_an_inherited_one_ignoring_case() {
    let root = format!("{ROOT}{}", rows(&[("Accept", "c"), ("X-Keep", "k")]));
    let (_d, c) = open(&[
        ("opencollection.yml", &root),
        ("f/folder.yml", &folder("f", &[("x-keep", "f")])),
        (
            "f/r.yml",
            &http(
                "r",
                "GET",
                "http://x.test/r",
                &(INHERIT.to_owned() + &own(&[("ACCEPT", "r")])),
            ),
        ),
    ]);
    let v = exported(&c);
    // The folder's x-keep wins over the collection's X-Keep, the request's ACCEPT over Accept.
    assert_eq!(
        header_pairs(&v["item"][0]["item"][0]["request"]),
        triples(&[("x-keep", "f", false), ("ACCEPT", "r", false)])
    );
}

#[test]
fn disabled_rows_are_not_inherited_but_own_disabled_rows_stay() {
    let root = format!(
        "{ROOT}request:\n  headers:\n    - name: X-Off\n      value: o\n      disabled: true\n    - name: X-On\n      value: n\n"
    );
    let own_off = "  headers:\n    - name: X-Mine\n      value: m\n      disabled: true\n";
    let (_d, c) = open(&[
        ("opencollection.yml", &root),
        (
            "r.yml",
            &http(
                "r",
                "GET",
                "http://x.test/r",
                &(INHERIT.to_owned() + own_off),
            ),
        ),
    ]);
    let v = exported(&c);
    assert_eq!(
        header_pairs(&v["item"][0]["request"]),
        triples(&[("X-On", "n", false), ("X-Mine", "m", true)])
    );
}

const MANY: &str = "headers were copied into 2 requests, because Postman has no collection or folder headers; importing the file back gives each of those requests its own copy";
const ONE: &str = "headers were copied into 1 request, because Postman has no collection or folder headers; importing the file back gives that request its own copy";
const NONE: &str = "headers were not copied anywhere: no request uses them (the folder is empty, or every request sets its own)";

#[test]
fn each_source_notes_how_many_requests_received_its_headers() {
    let root = format!("{ROOT}{}", rows(&[("X-C", "c")]));
    let req = |n: &str| http(n, "GET", "http://x.test/r", INHERIT);
    let (_d, c) = open(&[
        ("opencollection.yml", &root),
        ("many/folder.yml", &folder("many", &[("X-M", "m")])),
        ("many/a.yml", &req("a")),
        ("many/b.yml", &req("b")),
        ("one/folder.yml", &folder("one", &[("X-O", "o")])),
        ("one/a.yml", &req("a")),
        ("empty/folder.yml", &folder("empty", &[("X-E", "e")])),
    ]);
    assert_eq!(
        notes(&c),
        pairs(&[
            (
                "collection / headers",
                "headers were copied into 3 requests, because Postman has no collection or folder headers; importing the file back gives each of those requests its own copy"
            ),
            ("empty / headers", NONE),
            ("many / headers", MANY),
            ("one / headers", ONE),
        ])
    );
}

#[test]
fn headers_every_request_overrides_are_noted_as_copied_nowhere() {
    let root = format!("{ROOT}{}", rows(&[("X-C", "c")]));
    let (_d, c) = open(&[
        ("opencollection.yml", &root),
        (
            "r.yml",
            &http(
                "r",
                "GET",
                "http://x.test/r",
                &(INHERIT.to_owned() + &own(&[("x-c", "r")])),
            ),
        ),
    ]);
    assert_eq!(notes(&c), pairs(&[("collection / headers", NONE)]));
}

/// Each request's enabled headers, inherited first, as lowercase name and value pairs.
fn effective(c: &Collection) -> Vec<(String, Vec<(String, String)>)> {
    source_requests(c.tree(), "")
        .into_iter()
        .map(|(name, r)| {
            let path = c.item_path(r.path()).unwrap();
            let own = r.headers();
            let mut all: Vec<(String, String)> = c
                .inherited_headers(&path)
                .iter()
                .filter(|h| !h.overridden(&own))
                .map(|h| (h.name().to_lowercase(), h.value().to_owned()))
                .collect();
            all.extend(
                own.iter()
                    .filter(|h| h.enabled)
                    .map(|h| (h.name.to_lowercase(), h.value.clone())),
            );
            (name, all)
        })
        .collect()
}

#[test]
fn exporting_then_importing_keeps_every_requests_effective_headers() {
    let root = format!("{ROOT}{}", rows(&[("X-C", "{{c}}"), ("Accept", "c")]));
    let req = |n: &str, h: &str| http(n, "GET", "http://x.test/r", &(INHERIT.to_owned() + h));
    let (_d, c) = open(&[
        ("opencollection.yml", &root),
        (
            "f/folder.yml",
            &folder("f", &[("X-F", "f"), ("x-c", "folder")]),
        ),
        ("f/a.yml", &req("a", &own(&[("ACCEPT", "a")]))),
        ("f/b.yml", &req("b", "")),
        ("top.yml", &req("top", "")),
    ]);
    let before = effective(&c);
    let out = postman(&c);
    let parsed = parse_postman(out.json.as_bytes()).expect("parses");
    let target = tempfile::tempdir().unwrap();
    let (back, _) = write_collection(target.path(), &parsed, &[], &MemoryStore::new()).unwrap();
    assert_eq!(effective(&back), before);
    assert!(before.iter().all(|(_, h)| !h.is_empty()));
}
