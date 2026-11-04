//! The inferred JSON Schema test: inference rules, layout, and a run through the script engine.

use crate::script_support;

use apinata_core::{Outcome, schema_test_script};
use script_support::*;
use serde_json::{Value, json};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const NAME: &str = "response matches schema";

/// The schema passed to `ajv.compile(...)` in the generated block.
fn schema_of(script: &str) -> Value {
    let start = script.find("ajv.compile(").expect("compile call") + "ajv.compile(".len();
    let end = script.find(");\n  expect").expect("end of compile");
    serde_json::from_str(&script[start..end]).expect("schema is JSON")
}

fn infer(body: &str) -> Value {
    schema_of(&schema_test_script("", body).expect("valid JSON"))
}

#[test]
fn nested_objects_and_arrays_are_described_with_required_keys() {
    let schema = infer(r#"{"id": 7, "user": {"name": "a", "tags": ["x"]}, "ok": true}"#);
    assert_eq!(
        schema,
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "integer"},
                "user": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"},
                        "tags": {"type": "array", "items": {"type": "string"}}
                    },
                    "required": ["name", "tags"]
                },
                "ok": {"type": "boolean"}
            },
            "required": ["id", "user", "ok"]
        })
    );
    let keys: Vec<_> = schema["properties"].as_object().unwrap().keys().collect();
    assert_eq!(keys, ["id", "user", "ok"], "sample key order is kept");
}

#[test]
fn integral_numbers_are_integers_and_any_fraction_makes_number() {
    assert_eq!(infer("3"), json!({"type": "integer"}));
    assert_eq!(infer("3.5"), json!({"type": "number"}));
    assert_eq!(infer("3.0"), json!({"type": "number"}));
    assert_eq!(
        infer("[1, 2.5]"),
        json!({"type": "array", "items": {"type": "number"}})
    );
}

#[test]
fn differing_item_types_merge_into_a_type_array() {
    assert_eq!(
        infer(r#"[{"a": "x"}, {"a": null}]"#),
        json!({
            "type": "array",
            "items": {
                "type": "object",
                "properties": {"a": {"type": ["string", "null"]}},
                "required": ["a"]
            }
        })
    );
}

#[test]
fn a_key_missing_from_some_items_is_not_required() {
    assert_eq!(
        infer(r#"[{"a": 1, "b": 2}, {"a": 3}]"#),
        json!({
            "type": "array",
            "items": {
                "type": "object",
                "properties": {"a": {"type": "integer"}, "b": {"type": "integer"}},
                "required": ["a"]
            }
        })
    );
}

#[test]
fn empty_containers_and_scalar_roots() {
    assert_eq!(infer("[]"), json!({"type": "array"}));
    assert_eq!(
        infer(r#"{"o": {}}"#),
        json!({
            "type": "object",
            "properties": {"o": {"type": "object", "properties": {}}},
            "required": ["o"]
        })
    );
    assert_eq!(infer("{}"), json!({"type": "object", "properties": {}}));
    assert_eq!(infer("null"), json!({"type": "null"}));
    assert_eq!(infer(r#""hi""#), json!({"type": "string"}));
    assert_eq!(infer("false"), json!({"type": "boolean"}));
}

#[test]
fn nesting_deeper_than_32_levels_is_any() {
    let body = format!("{}1{}", "[".repeat(40), "]".repeat(40));
    let mut schema = infer(&body);
    let mut depth = 0;
    while let Some(items) = schema.get("items").cloned() {
        schema = items;
        depth += 1;
    }
    assert_eq!(schema, json!({}), "the cut level accepts anything");
    assert_eq!(depth, 32);
}

#[test]
fn invalid_json_gives_none() {
    assert_eq!(schema_test_script("", "not json"), None);
    assert_eq!(schema_test_script("x", ""), None);
    assert_eq!(schema_test_script("", "{\"a\":"), None);
}

#[test]
fn a_repeated_name_gets_a_numeric_suffix() {
    let first = schema_test_script("", "{}").unwrap();
    assert!(first.contains(&format!("test(\"{NAME}\", ")), "{first}");
    let second = schema_test_script(&first, "{}").unwrap();
    assert!(
        second.contains(&format!("test(\"{NAME} (2)\", ")),
        "{second}"
    );
    let third = schema_test_script(&second, "{}").unwrap();
    assert!(third.contains(&format!("test(\"{NAME} (3)\", ")), "{third}");
}

#[test]
fn the_block_is_separated_by_one_blank_line_and_ends_with_a_newline() {
    let block = schema_test_script("", r#"{"a": 1}"#).unwrap();
    assert!(block.starts_with(&format!("test(\"{NAME}\", function () {{\n")));
    assert!(block.ends_with("});\n"), "{block:?}");
    assert!(block.contains("  const validate = ajv.compile({\n    \"type\": \"object\","));

    let after_bare = schema_test_script("a();", "{}").unwrap();
    assert!(after_bare.starts_with("a();\n\ntest("), "{after_bare:?}");
    let after_newline = schema_test_script("a();\n", "{}").unwrap();
    assert!(
        after_newline.starts_with("a();\n\ntest("),
        "{after_newline:?}"
    );
    let after_blank = schema_test_script("a();\n\n", "{}").unwrap();
    assert!(after_blank.starts_with("a();\n\ntest("), "{after_blank:?}");
}

async fn run_script(body: &str, code: &str) -> apinata_core::RequestResult {
    let s = MockServer::start().await;
    Mock::given(path("/r"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&s)
        .await;
    let text = scripted("R", &format!("{}/r", s.uri()), &script("tests", code), "");
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    send(dir.path(), "r.yml", on()).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_generated_block_passes_on_the_sample_and_fails_without_a_required_key() {
    let sample = r#"{"id": 7, "user": {"name": "a", "tags": ["x", "y"]}, "n": null}"#;
    let code = schema_test_script("", sample).expect("valid JSON");

    let ok = run_script(sample, &code).await;
    assert_eq!(ok.outcome, Outcome::Passed, "{:?}", tests(&ok));
    assert_eq!(tests(&ok), [format!("tests {NAME}: pass")]);

    let cut = r#"{"id": 7, "user": {"tags": ["x", "y"]}, "n": null}"#;
    let bad = run_script(cut, &code).await;
    assert_ne!(bad.outcome, Outcome::Passed);
    let reported = tests(&bad);
    assert_eq!(reported.len(), 1, "{reported:?}");
    assert!(
        reported[0].starts_with(&format!("tests {NAME}: fail")),
        "{reported:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_proto_key_is_left_out_of_the_schema_and_the_block_still_passes() {
    for sample in [
        r#"{"__proto__":{"z":1},"k":1}"#,
        r#"[{"__proto__":{"z":1},"k":1}]"#,
    ] {
        let code = schema_test_script("", sample).expect("valid JSON");
        assert!(!code.contains("__proto__"), "{code}");
        let ok = run_script(sample, &code).await;
        assert_eq!(ok.outcome, Outcome::Passed, "{sample}: {:?}", tests(&ok));
        assert_eq!(tests(&ok), [format!("tests {NAME}: pass")]);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn line_and_paragraph_separators_are_escaped_in_the_schema_text() {
    let sample = r#"{"a b c":1}"#;
    let code = schema_test_script("", sample).expect("valid JSON");
    assert!(
        !code.contains('\u{2028}') && !code.contains('\u{2029}'),
        "{code:?}"
    );
    assert!(
        code.contains("\\u2028") && code.contains("\\u2029"),
        "{code}"
    );
    let ok = run_script(sample, &code).await;
    assert_eq!(ok.outcome, Outcome::Passed, "{:?}", tests(&ok));
}
