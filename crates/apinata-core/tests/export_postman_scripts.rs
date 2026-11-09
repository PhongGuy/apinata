//! `export::postman_with`: scripts translated to Postman's API with a note per script, or kept as written.

use crate::export_support;

use apinata_core::Collection;
use apinata_core::export::{PostmanOptions, postman, postman_with};
use export_support::*;
use serde_json::{Value, json};
use tempfile::TempDir;

const ONE: &str = "script translated; 1 call rewritten for Postman";

fn scripted() -> (TempDir, Collection) {
    let root = format!(
        "{ROOT}request:\n  scripts:\n    - type: before-request\n      code: \"bru.setVar('a', 1);\"\n"
    );
    open(&[
        ("opencollection.yml", &root),
        (
            "f/folder.yml",
            "info:\n  name: f\n  type: folder\n  seq: 1\nrequest:\n  scripts:\n    - type: tests\n      code: \"test('f', () => { expect(res.getStatus()).to.equal(200); });\"\n",
        ),
        (
            "a.yml",
            &(http("a", "GET", "http://x.test/a", INHERIT)
                + "runtime:\n  scripts:\n    - type: after-response\n      code: |-\n        const b = res.getBody();\n        bru.sleep(10);\n        bru.sleep(20);\n"),
        ),
    ])
}

fn with_pre_request(code_yaml: &str) -> (TempDir, Collection) {
    let a = http("a", "GET", "http://x.test/a", INHERIT)
        + "runtime:\n  scripts:\n    - type: before-request\n      code: "
        + code_yaml;
    open(&[("opencollection.yml", ROOT), ("a.yml", &a)])
}

fn exec(item: &Value) -> Value {
    item["event"][0]["script"]["exec"].clone()
}

#[test]
fn scripts_are_translated_by_default() {
    let (_d, c) = scripted();
    let v = exported(&c);
    assert_eq!(exec(&v), json!(["pm.variables.set('a', 1);"]));
    assert_eq!(
        exec(&v["item"][0]),
        json!([
            "// Tests",
            "pm.test('f', () => { pm.expect(pm.response.code).to.equal(200); });"
        ])
    );
    assert_eq!(
        exec(&v["item"][1]),
        json!([
            "const b = pm.response.json();",
            "bru.sleep(10);",
            "bru.sleep(20);"
        ])
    );
}

#[test]
fn each_script_gets_a_note_and_each_leftover_one_more() {
    let (_d, c) = scripted();
    assert_eq!(
        notes(&c),
        pairs(&[
            (
                "f / tests script",
                "script translated; 3 calls rewritten for Postman"
            ),
            ("a / post-response script", ONE),
            (
                "a / post-response script",
                "uses `bru.sleep`, which was not translated for Postman; the line was kept as written"
            ),
            ("collection / pre-request script", ONE),
        ])
    );
}

#[test]
fn kept_scripts_are_written_as_they_are_with_one_note() {
    let (_d, c) = scripted();
    let out = postman_with(&c, &PostmanOptions::new().keep_scripts(true));
    let v: Value = serde_json::from_str(&out.json).unwrap();
    assert_eq!(exec(&v), json!(["bru.setVar('a', 1);"]));
    assert_eq!(
        exec(&v["item"][1]),
        json!([
            "const b = res.getBody();",
            "bru.sleep(10);",
            "bru.sleep(20);"
        ])
    );
    let got: Vec<(String, String)> = out
        .notes
        .into_iter()
        .map(|n| (n.source, n.detail))
        .collect();
    assert_eq!(
        got,
        pairs(&[(
            "scripts",
            "scripts exported as written; their API calls are not translated"
        )])
    );
}

#[test]
fn keeping_scripts_adds_no_note_when_there_are_none() {
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &http("a", "GET", "http://x.test/a", INHERIT)),
    ]);
    assert!(
        postman_with(&c, &PostmanOptions::new().keep_scripts(true))
            .notes
            .is_empty()
    );
    assert!(postman(&c).notes.is_empty());
}

#[test]
fn a_script_whose_brackets_do_not_balance_is_kept_with_its_leftovers() {
    let (_d, c) = with_pre_request("\"bru.getVar('a');\\nif (x) {\"\n");
    assert_eq!(
        exec(&exported(&c)["item"][0]),
        json!(["bru.getVar('a');", "if (x) {"])
    );
    assert_eq!(
        notes(&c),
        pairs(&[
            (
                "a / pre-request script",
                "script kept as written; its brackets do not balance, so no calls were rewritten"
            ),
            (
                "a / pre-request script",
                "uses `bru.getVar`, which was not translated for Postman; the line was kept as written"
            ),
        ])
    );
}

#[test]
fn every_mapped_call_is_translated() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reference/2026-11-09-postman-export-translation/table.json");
    let table: Value = serde_json::from_str(&std::fs::read_to_string(fixture).unwrap()).unwrap();
    let keys: Vec<&str> = ["simple", "callToProperty"]
        .iter()
        .flat_map(|t| table[*t].as_object().unwrap().keys())
        .map(String::as_str)
        .collect();
    assert_eq!(keys.len(), 118);
    let code: String = keys.iter().map(|k| format!("        {k}(1);\n")).collect();
    let (_d, c) = with_pre_request(&format!("|-\n{code}"));
    let lines: Vec<String> = exec(&exported(&c)["item"][0])
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(lines.len(), 118);
    assert!(lines.iter().all(|l| l.starts_with("pm.")), "{lines:?}");
    assert_eq!(
        notes(&c),
        pairs(&[(
            "a / pre-request script",
            "script translated; 118 calls rewritten for Postman"
        )])
    );
}

#[test]
fn every_reshaped_call_is_translated() {
    let code = "|-\n        req.setUrl(\"u\");\n        req.setMethod(\"POST\");\n        req.setHeader(\"h\", \"v\");\n        req.setBody({a: 1});\n        req.setHeaders({h: \"v\"});\n        JSON.stringify(res.getBody());\n        bru.cookies.jar().getCookie(\"u\", \"n\");\n        const jar = bru.cookies.jar();\n        jar.getCookies(\"u\");\n        test(\"t\", function() { expect(1).to.equal(1); });\n        bru.sendRequest({url: \"u\", headers: {a: \"1\"}, data: \"x\"}, (err, r) => { r.data; });\n        const rs = res;\n        rs.getBody();\n";
    let (_d, c) = with_pre_request(code);
    let lines: Vec<String> = exec(&exported(&c)["item"][0])
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        lines,
        [
            "pm.request.url = \"u\";",
            "pm.request.method = \"POST\";",
            "pm.request.headers.upsert({ key: \"h\", value: \"v\" });",
            "pm.request.body.update({ mode: \"raw\", raw: JSON.stringify({a: 1}) });",
            "(function() { const _headers = {h: \"v\"}; for (const key in _headers) { pm.request.headers.upsert({ key: key, value: _headers[key] }); } })();",
            "pm.response.text();",
            "pm.cookies.jar().get(\"u\", \"n\");",
            "const jar = pm.cookies.jar();",
            "jar.getAll(\"u\");",
            "pm.test(\"t\", function() { pm.expect(1).to.equal(1); });",
            "pm.sendRequest({url: \"u\", header: {a: \"1\"}, body: { mode: \"raw\", raw: \"x\" }}, function(err, r) { r.json(); });",
            "pm.response.json();",
        ]
    );
    assert_eq!(
        notes(&c),
        pairs(&[(
            "a / pre-request script",
            "script translated; 14 calls rewritten for Postman"
        )])
    );
}
