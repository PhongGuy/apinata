//! A Postman script that uses `CryptoJS` gets `const CryptoJS = require('crypto-js');` on top,
//! because the engine serves the library through `require` only.

use crate::script_support::{console, on, send};
use apinata_core::MemoryStore;
use apinata_core::import::{ImportReport, NoteKind, parse_postman, write_collection};
use serde_json::json;
use std::fs;
use tempfile::TempDir;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const HMAC: &str = "78da91511e675587f5b9df78bedebaf5560da2abb88162ee875dcdf744951d9e";

/// Imports one request with the given pre-request script; returns the report, the written
/// request file's text and the temp dir holding the collection.
fn import(exec: &str, url: &str) -> (ImportReport, String, TempDir) {
    let doc = json!({
        "info": {
            "name": "C",
            "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
        },
        "item": [{
            "name": "R",
            "event": [{"listen": "prerequest", "script": {"type": "text/javascript", "exec": [exec]}}],
            "request": {"method": "GET", "url": url}
        }]
    });
    let imported = parse_postman(doc.to_string().as_bytes()).expect("parses");
    let parent = TempDir::new().expect("tempdir");
    let (collection, report) =
        write_collection(parent.path(), &imported, &[], &MemoryStore::new()).expect("writes");
    let text = fs::read_to_string(collection.root().join("R.yml")).expect("R.yml");
    (report, text, parent)
}

fn cryptojs_notes(report: &ImportReport) -> usize {
    report
        .notes
        .iter()
        .filter(|n| n.kind == NoteKind::Converted && n.detail.contains("CryptoJS"))
        .count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_imported_cryptojs_script_runs() {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let (report, _, parent) = import(
        "console.log(CryptoJS.HmacSHA256('a', 'k').toString());",
        &format!("{}/ok", s.uri()),
    );
    let root = fs::read_dir(parent.path())
        .expect("dir")
        .next()
        .expect("collection")
        .expect("entry")
        .path();
    let result = send(&root, "R.yml", on()).await;
    assert_eq!(
        console(&result),
        [format!("log:{HMAC}")],
        "{:?}",
        result.error
    );
    assert_eq!(cryptojs_notes(&report), 1, "{:?}", report.notes);
}

#[test]
fn the_import_adds_the_require_and_one_note() {
    let (report, text, _p) = import("var h = CryptoJS.MD5('x');", "http://127.0.0.1:1/");
    assert!(
        text.contains("const CryptoJS = require('crypto-js');"),
        "{text}"
    );
    assert_eq!(cryptojs_notes(&report), 1, "{:?}", report.notes);
    let note = report
        .notes
        .iter()
        .find(|n| n.detail.contains("CryptoJS"))
        .unwrap();
    assert!(
        note.detail.contains("require('crypto-js')"),
        "{}",
        note.detail
    );
}

#[test]
fn an_existing_require_is_not_doubled() {
    let code = "const CryptoJS = require('crypto-js');\nvar h = CryptoJS.MD5('x');";
    let (report, text, _p) = import(code, "http://127.0.0.1:1/");
    assert_eq!(text.matches("require('crypto-js')").count(), 1, "{text}");
    assert_eq!(cryptojs_notes(&report), 0, "{:?}", report.notes);
}

#[test]
fn a_comment_or_string_mention_is_not_a_use() {
    let code = "// CryptoJS is great\nvar s = 'CryptoJS.MD5'; var o = {}; o.CryptoJS = 1;";
    let (report, text, _p) = import(code, "http://127.0.0.1:1/");
    assert!(!text.contains("require('crypto-js')"), "{text}");
    assert_eq!(cryptojs_notes(&report), 0, "{:?}", report.notes);
}

#[test]
fn a_template_hole_counts_as_code() {
    let (_, text, _p) = import("var s = `${CryptoJS.MD5('x')}`;", "http://127.0.0.1:1/");
    assert!(
        text.contains("const CryptoJS = require('crypto-js');"),
        "{text}"
    );
}

fn position(text: &str, needle: &str) -> usize {
    text.find(needle)
        .unwrap_or_else(|| panic!("{needle} missing in {text}"))
}

#[test]
fn a_use_strict_directive_stays_first() {
    for directive in ["'use strict';", "\"use strict\";", "'use strict'"] {
        let code = format!("{directive}\nvar h = CryptoJS.MD5('x');");
        let (_, text, _p) = import(&code, "http://127.0.0.1:1/");
        let strict = position(&text, "use strict");
        let require = position(&text, "const CryptoJS = require('crypto-js');");
        assert!(strict < require, "{directive}: {text}");
    }
}

#[test]
fn a_prologue_with_comments_and_two_directives_stays_first() {
    let code = "// header\n'use strict'; /* c */ 'other';\nvar h = CryptoJS.MD5('x');";
    let (_, text, _p) = import(code, "http://127.0.0.1:1/");
    let require = position(&text, "const CryptoJS = require('crypto-js');");
    assert!(position(&text, "'other'") < require, "{text}");
    assert!(position(&text, "use strict") < require, "{text}");
}

#[test]
fn an_object_key_is_not_a_use() {
    for code in ["var o = {CryptoJS: 1};", "var o = {a: 2, CryptoJS: 1};"] {
        let (report, text, _p) = import(code, "http://127.0.0.1:1/");
        assert!(!text.contains("require('crypto-js')"), "{text}");
        assert_eq!(cryptojs_notes(&report), 0, "{:?}", report.notes);
    }
}

#[test]
fn a_member_name_is_not_a_use() {
    let (report, text, _p) = import("var v = x.CryptoJS;", "http://127.0.0.1:1/");
    assert!(!text.contains("require('crypto-js')"), "{text}");
    assert_eq!(cryptojs_notes(&report), 0, "{:?}", report.notes);
}

#[test]
fn a_shorthand_property_is_a_use() {
    let (_, text, _p) = import("var o = {CryptoJS};", "http://127.0.0.1:1/");
    assert!(
        text.contains("const CryptoJS = require('crypto-js');"),
        "{text}"
    );
}

#[test]
fn a_spaced_or_double_quoted_require_is_detected() {
    for req in [
        "const CryptoJS = require ( 'crypto-js' );",
        "const CryptoJS = require(\"crypto-js\");",
    ] {
        let code = format!("{req}\nvar h = CryptoJS.MD5('x');");
        let (report, text, _p) = import(&code, "http://127.0.0.1:1/");
        assert_eq!(text.matches("crypto-js").count(), 1, "{text}");
        assert_eq!(cryptojs_notes(&report), 0, "{:?}", report.notes);
    }
}

#[test]
fn a_require_in_a_comment_does_not_count() {
    let code = "// const CryptoJS = require('crypto-js');\nvar h = CryptoJS.MD5('x');";
    let (_, text, _p) = import(code, "http://127.0.0.1:1/");
    assert_eq!(
        text.matches("const CryptoJS = require('crypto-js');")
            .count(),
        2,
        "{text}"
    );
}

#[test]
fn an_unavailable_global_gets_a_note() {
    for name in ["cheerio", "xml2Json"] {
        let (report, _, _p) = import(&format!("var d = {name}(x);"), "http://127.0.0.1:1/");
        let notes: Vec<_> = report
            .notes
            .iter()
            .filter(|n| n.detail.contains(name))
            .collect();
        assert_eq!(notes.len(), 1, "{:?}", report.notes);
        assert_eq!(notes[0].kind, NoteKind::Converted);
        assert!(
            notes[0].detail.contains("not available"),
            "{}",
            notes[0].detail
        );
    }
}

#[test]
fn a_declared_or_key_use_of_an_unavailable_global_gets_no_note() {
    for code in ["var cheerio = 1; cheerio.x;", "var o = {xml2Json: 1};"] {
        let (report, _, _p) = import(code, "http://127.0.0.1:1/");
        let n = report
            .notes
            .iter()
            .filter(|n| n.detail.contains("not available"))
            .count();
        assert_eq!(n, 0, "{:?}", report.notes);
    }
}
