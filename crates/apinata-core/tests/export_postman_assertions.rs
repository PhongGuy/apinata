//! `export::postman`: assertions as Postman tests, and which notes count as lost.

use crate::export_support;

use apinata_core::export::{ExportNoteKind, postman};
use export_support::*;
use serde_json::{Value, json};

/// A request `a` whose `runtime.assertions` holds one `expression operator value` row each.
fn with_assertions(rows: &[(&str, &str, &str)]) -> String {
    let mut text = http("a", "GET", "http://x.test/a", INHERIT) + "runtime:\n  assertions:\n";
    for (expression, operator, value) in rows {
        text.push_str(&format!(
            "    - expression: \"{expression}\"\n      operator: {operator}\n      value: '{value}'\n"
        ));
    }
    text
}

fn test_lines(v: &Value) -> Value {
    v["item"][0]["event"][0]["script"]["exec"].clone()
}

#[test]
fn each_supported_assertion_becomes_a_postman_test() {
    let cases = [
        (
            ("res.status", "eq", "200"),
            r#"pm.test("res.status eq 200", () => pm.expect(pm.response.code).to.equal(200));"#,
        ),
        (
            ("res.status", "neq", "500"),
            r#"pm.test("res.status neq 500", () => pm.expect(pm.response.code).to.not.equal(500));"#,
        ),
        (
            ("res.responseTime", "lt", "300"),
            r#"pm.test("res.responseTime lt 300", () => pm.expect(pm.response.responseTime).to.be.below(300));"#,
        ),
        (
            ("res.responseTime", "lte", "300"),
            r#"pm.test("res.responseTime lte 300", () => pm.expect(pm.response.responseTime).to.be.at.most(300));"#,
        ),
        (
            ("res.body.count", "gt", "1.5"),
            r#"pm.test("res.body.count gt 1.5", () => pm.expect(pm.response.json().count).to.be.above(1.5));"#,
        ),
        (
            ("res.body.count", "gte", "2"),
            r#"pm.test("res.body.count gte 2", () => pm.expect(pm.response.json().count).to.be.at.least(2));"#,
        ),
        (
            ("res.headers.etag", "contains", "abc"),
            r#"pm.test("res.headers.etag contains abc", () => pm.expect(pm.response.headers.get("etag")).to.include("abc"));"#,
        ),
        (
            ("res.headers['x-id']", "notContains", "\"a b\""),
            r#"pm.test("res.headers['x-id'] notContains \"a b\"", () => pm.expect(pm.response.headers.get("x-id")).to.not.include("a b"));"#,
        ),
        (
            ("res.statusText", "eq", "OK"),
            r#"pm.test("res.statusText eq OK", () => pm.expect(pm.response.status).to.equal("OK"));"#,
        ),
        (
            ("res.body.items[0]['the id']", "eq", "true"),
            r#"pm.test("res.body.items[0]['the id'] eq true", () => pm.expect(pm.response.json().items[0]['the id']).to.equal(true));"#,
        ),
        (
            ("res.body.id", "isDefined", ""),
            r#"pm.test("res.body.id isDefined", () => pm.expect(pm.response.json().id).to.not.be.undefined);"#,
        ),
        (
            ("res.body.gone", "isUndefined", ""),
            r#"pm.test("res.body.gone isUndefined", () => pm.expect(pm.response.json().gone).to.be.undefined);"#,
        ),
        (
            ("res.body.next", "isNull", ""),
            r#"pm.test("res.body.next isNull", () => pm.expect(pm.response.json().next).to.be.null);"#,
        ),
    ];
    for ((expression, operator, value), want) in cases {
        let request = with_assertions(&[(expression, operator, value)]);
        let (_d, c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request)]);
        let v = exported(&c);
        assert_eq!(
            test_lines(&v),
            json!(["// Assertions", want]),
            "{expression} {operator} {value}"
        );
        let out = postman(&c);
        assert_eq!(out.notes.len(), 1, "{:?}", out.notes);
        assert_eq!(out.notes[0].source, "a / assertions");
        assert_eq!(out.notes[0].detail, "assertions turned into Postman tests");
        assert_eq!(out.notes[0].kind, ExportNoteKind::Changed);
    }
}

#[test]
fn an_assertion_without_a_postman_equivalent_is_still_reported() {
    let request = with_assertions(&[
        ("res.status", "eq", "200"),
        ("res.body", "isJson", ""),
        ("res.body.id", "eq", "{{id}}"),
        ("res.body.name", "matches", "^a"),
        ("res.getBody()", "eq", "1"),
    ]);
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request)]);
    let v = exported(&c);
    assert_eq!(
        test_lines(&v),
        json!([
            "// Assertions",
            r#"pm.test("res.status eq 200", () => pm.expect(pm.response.code).to.equal(200));"#
        ])
    );
    let out = postman(&c);
    let notes: Vec<_> = out
        .notes
        .iter()
        .map(|n| (n.source.as_str(), n.detail.as_str(), n.kind))
        .collect();
    assert_eq!(
        notes,
        [
            (
                "a / assertions",
                "assertions turned into Postman tests",
                ExportNoteKind::Changed
            ),
            (
                "a / assertions",
                "these assertions could not be turned into Postman tests and were left out: res.body isJson; res.body.id eq {{id}}; res.body.name matches ^a; res.getBody() eq 1",
                ExportNoteKind::Lost
            ),
        ]
    );
}

#[test]
fn a_disabled_assertion_is_left_out_and_reported() {
    let request = http("a", "GET", "http://x.test/a", INHERIT)
        + "runtime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n      disabled: true\n";
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request)]);
    let v = exported(&c);
    assert_eq!(v["item"][0].get("event"), None);
    assert_eq!(
        notes(&c),
        pairs(&[(
            "a / assertions",
            "these assertions could not be turned into Postman tests and were left out: res.status eq 200"
        )])
    );
}

#[test]
fn assertions_follow_the_scripts_in_the_test_event() {
    let request = with_assertions(&[("res.status", "eq", "200")])
        + "  scripts:\n    - type: tests\n      code: \"test('t', () => {});\"\n";
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request)]);
    let v = exported(&c);
    assert_eq!(
        test_lines(&v),
        json!([
            "// Tests",
            "pm.test('t', () => {});",
            "",
            "// Assertions",
            r#"pm.test("res.status eq 200", () => pm.expect(pm.response.code).to.equal(200));"#
        ])
    );
}

#[test]
fn a_translated_script_is_a_change_and_what_is_dropped_is_lost() {
    let request = http("a", "GET", "http://x.test/a", INHERIT)
        + "runtime:\n  variables:\n    - name: v\n      value: \"1\"\n  scripts:\n    - type: before-request\n      code: \"bru.setVar('a', 1);\"\n";
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request)]);
    let kinds: Vec<_> = postman(&c)
        .notes
        .iter()
        .map(|n| (n.source.clone(), n.kind))
        .collect();
    assert_eq!(
        kinds,
        [
            ("a / variables".to_owned(), ExportNoteKind::Lost),
            ("a / pre-request script".to_owned(), ExportNoteKind::Changed),
        ]
    );
}

#[test]
fn the_tags_note_reads_the_same_for_every_request() {
    let tagged = |name: &str, tags: &str| {
        http(name, "GET", "http://x.test/a", INHERIT).replacen(
            "  seq: 1\n",
            &format!("  seq: 1\n  tags: [{tags}]\n"),
            1,
        )
    };
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &tagged("a", "smoke")),
        ("b.yml", &tagged("b", "smoke, data")),
    ]);
    let tags = "tags have no place in Postman and were left out";
    assert_eq!(notes(&c), pairs(&[("a / tags", tags), ("b / tags", tags)]));
}
