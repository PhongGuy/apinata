//! `export::postman`: scripts as events, collection variables and examples.

use crate::export_support;

use export_support::*;
use serde_json::{Value, json};

const ZERO: &str = "script translated; 0 calls rewritten for Postman";

fn event(listen: &str, exec: Value) -> Value {
    json!({"listen": listen, "script": {"type": "text/javascript", "packages": {}, "requests": {}, "exec": exec}})
}

#[test]
fn collection_scripts_sit_on_top() {
    let root = format!(
        "{ROOT}request:\n  scripts:\n    - type: before-request\n      code: \"bru.setVar('a', 1);\"\n"
    );
    let (_d, c) = open(&[("opencollection.yml", &root)]);
    let v = exported(&c);
    assert_eq!(keys(&v), ["info", "item", "variable", "event"]);
    assert_eq!(
        v["event"],
        json!([event("prerequest", json!(["pm.variables.set('a', 1);"]))])
    );
    assert_eq!(
        notes(&c),
        pairs(&[(
            "collection / pre-request script",
            "script translated; 1 call rewritten for Postman"
        )])
    );
}

#[test]
fn tests_follow_the_post_response_script_in_one_test_event() {
    let all = "runtime:\n  scripts:\n    - type: before-request\n      code: \"const a = 1;\"\n    - type: after-response\n      code: |-\n        const b = 2;\n        const c = 3;\n    - type: tests\n      code: \"test('ok', () => {});\"\n";
    let only_tests =
        "runtime:\n  scripts:\n    - type: tests\n      code: \"test('t', () => {});\"\n";
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &(http("a", "GET", "http://x.test/a", INHERIT) + all),
        ),
        (
            "b.yml",
            &(http("b", "GET", "http://x.test/b", INHERIT) + only_tests),
        ),
        (
            "f/folder.yml",
            "info:\n  name: f\n  type: folder\n  seq: 1\nrequest:\n  scripts:\n    - type: tests\n      code: \"test('f', () => {});\"\n",
        ),
    ]);
    let v = exported(&c);
    assert_eq!(
        v["item"][0]["event"],
        json!([event("test", json!(["// Tests", "test('f', () => {});"]))])
    );
    assert_eq!(keys(&v["item"][0]), ["name", "item", "event"]);
    assert_eq!(
        v["item"][1]["event"],
        json!([
            event("prerequest", json!(["const a = 1;"])),
            event(
                "test",
                json!([
                    "const b = 2;",
                    "const c = 3;",
                    "",
                    "// Tests",
                    "test('ok', () => {});"
                ])
            )
        ])
    );
    assert_eq!(
        v["item"][2]["event"],
        json!([event("test", json!(["// Tests", "test('t', () => {});"]))])
    );
    assert_eq!(
        keys(&v["item"][1]),
        ["name", "protocolProfileBehavior", "request", "event"]
    );
    assert_eq!(
        notes(&c),
        pairs(&[
            ("f / tests script", ZERO),
            ("a / pre-request script", ZERO),
            ("a / post-response script", ZERO),
            ("a / tests script", ZERO),
            ("b / tests script", ZERO),
        ])
    );
}

#[test]
fn declared_and_post_response_collection_variables_export_as_text() {
    let root = format!(
        "{ROOT}request:\n  variables:\n    - name: host\n      value: http://x.test\n    - name: n\n      value:\n        type: number\n        data: \"42\"\n    - name: idle\n      value: \"0\"\n      disabled: true\n  actions:\n    - type: set-variable\n      phase: after-response\n      selector:\n        expression: res.body.id\n        method: jsonq\n      variable:\n        name: id\n        scope: runtime\n    - type: set-variable\n      phase: after-response\n      selector:\n        expression: res.body.host\n        method: jsonq\n      variable:\n        name: host\n        scope: runtime\n"
    );
    let (_d, c) = open(&[
        ("opencollection.yml", &root),
        (
            "a.yml",
            &http("a", "GET", "{{host}}/{{undeclared}}", INHERIT),
        ),
    ]);
    assert_eq!(
        exported(&c)["variable"],
        json!([
        {"key": "host", "value": "http://x.test", "type": "default"},
        {"key": "n", "value": "42", "type": "default"},
        {"key": "idle", "value": "0", "type": "default", "disabled": true},
        {"key": "id", "value": "res.body.id", "type": "default"}])
    );
    assert_eq!(
        notes(&c).iter().map(|n| n.0.as_str()).collect::<Vec<_>>(),
        ["collection / variables / n"]
    );
}

#[test]
fn examples_become_responses() {
    let examples = "examples:\n  - name: created\n    request:\n      url: https://x.test/users/:id\n      method: POST\n      headers:\n        - name: Accept\n          value: application/json\n      params:\n        - name: id\n          value: \"7\"\n          type: path\n      body:\n        type: json\n        data: '{\"a\": 1}'\n    response:\n      status: 201\n      statusText: Created\n      headers:\n        - name: Content-Type\n          value: application/json\n      body:\n        type: json\n        data: '{\"id\": 7}'\n  - name: bare\n";
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &(http("a", "GET", "http://x.test/a", INHERIT) + examples),
        ),
    ]);
    let v = exported(&c);
    assert_eq!(
        keys(&v["item"][0]),
        ["name", "protocolProfileBehavior", "request", "response"]
    );
    assert_eq!(
        v["item"][0]["response"],
        json!([
        {"name": "created",
         "originalRequest": {"method": "POST",
            "header": [{"key": "Accept", "value": "application/json", "description": "", "disabled": false, "type": "default"}],
            "url": {"raw": "https://x.test/users/:id", "protocol": "https", "host": ["x", "test"], "path": ["users", ":id"], "query": [], "variable": [{"key": "id", "value": "7"}]},
            "body": {"mode": "raw", "raw": "{\"a\": 1}", "options": {"raw": {"language": "json"}}}},
         "status": "Created", "code": 201,
         "header": [{"key": "Content-Type", "value": "application/json", "name": "Content-Type", "description": "", "type": "text"}],
         "cookie": [], "body": "{\"id\": 7}", "_postman_previewlanguage": "json"},
        {"name": "bare",
         "originalRequest": {"method": "GET", "header": [], "url": {"raw": "", "protocol": "https", "host": [], "path": []}},
         "status": "OK", "code": 200, "header": [], "cookie": [], "body": ""}])
    );
}
