//! `export::postman`: assertions as Postman tests, and which notes count as lost.

use crate::export_support;

use apinata_core::export::{ExportNoteKind, postman};
use export_support::*;
use serde_json::{Value, json};

/// A request `a` whose `runtime.assertions` holds one `expression operator value` row each.
fn with_assertions(rows: &[(&str, &str, &str)]) -> String {
    let mut text = http("a", "GET", "http://x.test/a", INHERIT) + "runtime:\n  assertions:\n";
    for (expression, operator, value) in rows {
        // JSON strings are YAML double-quoted scalars, so any text survives the trip.
        let (expression, value) = (Value::from(*expression), Value::from(*value));
        text.push_str(&format!(
            "    - expression: {expression}\n      operator: {operator}\n      value: {value}\n"
        ));
    }
    text
}

/// The values core reads for header `name`: the first for a header Node keeps once, else all
/// joined by `sep`, and undefined when there is none.
fn header(name: &str, first: bool, sep: &str) -> String {
    let values = format!(
        "pm.response.headers.all().filter((h) => h.key.toLowerCase() === \"{name}\").map((h) => h.value)"
    );
    if first {
        format!("{values}[0]")
    } else {
        format!("((v) => v.length ? v.join(\"{sep}\") : undefined)({values})")
    }
}

/// The exported test events of a one-request collection with `rows` and the notes' details.
fn translated(rows: &[(&str, &str, &str)]) -> (Value, Vec<String>) {
    let request = with_assertions(rows);
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request)]);
    let notes = postman(&c).notes.into_iter().map(|n| n.detail).collect();
    (test_lines(&exported(&c)), notes)
}

fn test_lines(v: &Value) -> Value {
    v["item"][0]["event"][0]["script"]["exec"].clone()
}

#[test]
fn each_supported_assertion_becomes_a_postman_test() {
    let cases = [
        (
            ("res.status", "eq", "200"),
            r#"pm.test("res.status eq 200", () => pm.expect(pm.response.code).to.equal(200));"#.to_owned(),
        ),
        (
            ("res.status", "neq", "500"),
            r#"pm.test("res.status neq 500", () => pm.expect(pm.response.code).to.not.equal(500));"#.to_owned(),
        ),
        (
            ("res.responseTime", "lt", "300"),
            r#"pm.test("res.responseTime lt 300", () => pm.expect(pm.response.responseTime).to.be.below(300));"#.to_owned(),
        ),
        (
            ("res.responseTime", "lte", "300"),
            r#"pm.test("res.responseTime lte 300", () => pm.expect(pm.response.responseTime).to.be.at.most(300));"#.to_owned(),
        ),
        (
            ("res.body.count", "gt", "1.5"),
            r#"pm.test("res.body.count gt 1.5", () => pm.expect(pm.response.json().count).to.be.above(1.5));"#.to_owned(),
        ),
        (
            ("res.body.count", "gte", "2"),
            r#"pm.test("res.body.count gte 2", () => pm.expect(pm.response.json().count).to.be.at.least(2));"#.to_owned(),
        ),
        (
            ("res.headers.etag", "contains", "abc"),
            format!(
                r#"pm.test("res.headers.etag contains abc", () => pm.expect({}).to.include("abc"));"#,
                header("etag", true, "")
            ),
        ),
        (
            ("res.headers['x-id']", "notContains", "\"a b\""),
            format!(
                r#"pm.test("res.headers['x-id'] notContains \"a b\"", () => pm.expect({}).to.not.include("a b"));"#,
                header("x-id", false, ", ")
            ),
        ),
        (
            ("res.headers.cookie", "eq", "a=1; b=2"),
            format!(
                r#"pm.test("res.headers.cookie eq a=1; b=2", () => pm.expect({}).to.equal("a=1; b=2"));"#,
                header("cookie", false, "; ")
            ),
        ),
        (
            ("res.statusText", "eq", "OK"),
            r#"pm.test("res.statusText eq OK", () => pm.expect(pm.response.status).to.equal("OK"));"#.to_owned(),
        ),
        (
            ("res.body.items[0]['the id']", "eq", "true"),
            r#"pm.test("res.body.items[0]['the id'] eq true", () => pm.expect(pm.response.json().items[0]['the id']).to.equal(true));"#.to_owned(),
        ),
        (
            ("res.body.id", "isDefined", ""),
            r#"pm.test("res.body.id isDefined", () => pm.expect(pm.response.json().id).to.not.be.undefined);"#.to_owned(),
        ),
        (
            ("res.body.id", "neq", "1"),
            r#"pm.test("res.body.id neq 1", () => pm.expect(pm.response.json().id).to.not.equal(1));"#.to_owned(),
        ),
        (
            ("res.body.data.id", "eq", "1"),
            r#"pm.test("res.body.data.id eq 1", () => pm.expect(pm.response.json().data.id).to.equal(1));"#.to_owned(),
        ),
        (
            ("res.body.gone", "isUndefined", ""),
            r#"pm.test("res.body.gone isUndefined", () => pm.expect(pm.response.json().gone).to.be.undefined);"#.to_owned(),
        ),
        (
            ("res.body.next", "isNull", ""),
            r#"pm.test("res.body.next isNull", () => pm.expect(pm.response.json().next).to.be.null);"#.to_owned(),
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
        ("res.headers['Content-Type']", "contains", "json"),
        ("res.headers['set-cookie']", "contains", "a=1"),
        ("res.body.data.id", "neq", "1"),
        ("res.body.data.id", "isDefined", ""),
        ("res.body.id", "eq", "a\\b"),
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
                r#"these assertions could not be turned into Postman tests and were left out: "res.body isJson", "res.body.id eq {{id}}", "res.body.name matches ^a", "res.getBody() eq 1", "res.headers['Content-Type'] contains json", "res.headers['set-cookie'] contains a=1", "res.body.data.id neq 1", "res.body.data.id isDefined", "res.body.id eq a\\b""#,
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
            r#"these assertions could not be turned into Postman tests and were left out: "res.status eq 200""#
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
fn the_tags_note_names_the_tags_and_reads_the_same_for_the_same_tags() {
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
        ("b.yml", &tagged("b", "smoke")),
        ("c.yml", &tagged("c", "smoke, data")),
    ]);
    let tags = "tags have no place in Postman and were left out: smoke";
    let both = "tags have no place in Postman and were left out: smoke, data";
    assert_eq!(
        notes(&c),
        pairs(&[("a / tags", tags), ("b / tags", tags), ("c / tags", both)])
    );
}

#[test]
fn right_sides_keep_their_literal_meaning_and_text_is_escaped() {
    let test = |name: &str, want: &str| {
        format!(
            "pm.test({}, () => pm.expect(pm.response.json().v).{want});",
            Value::from(name)
        )
    };
    let cases = [
        (("eq", r#""200""#), r#"to.equal("200")"#),
        (("eq", "1e3"), "to.equal(1)"),
        (("eq", "0x1F"), "to.equal(0)"),
        (
            ("eq", "9007199254740993"),
            r#"to.equal("9007199254740993")"#,
        ),
        (("eq", "9007199254740991"), "to.equal(9007199254740991)"),
        (("eq", "null"), "to.equal(null)"),
        (("eq", "undefined"), "to.equal(undefined)"),
        (("eq", r#"say "hi""#), r#"to.equal("say \"hi\"")"#),
        (("eq", "a\nb"), r#"to.equal("a\nb")"#),
        (("contains", "'x'"), r#"to.include("x")"#),
    ];
    for ((operator, value), want) in cases {
        let (lines, notes) = translated(&[("res.body.v", operator, value)]);
        let name = format!("res.body.v {operator} {value}");
        assert_eq!(
            lines,
            json!(["// Assertions", test(&name, want)]),
            "{operator} {value:?}"
        );
        assert_eq!(notes, ["assertions turned into Postman tests"]);
    }
    let (lines, _) = translated(&[(r#"res.body["v"]"#, "eq", "1")]);
    assert_eq!(
        lines,
        json!([
            "// Assertions",
            r#"pm.test("res.body[\"v\"] eq 1", () => pm.expect(pm.response.json()['v']).to.equal(1));"#
        ])
    );
}
