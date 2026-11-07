//! Directives read from introspection: the admission rule and the built-ins core adds.

use apinata_core::Schema;
use serde_json::{Value, json};

pub(crate) const BUILT_INS: [&str; 4] = ["skip", "include", "deprecated", "specifiedBy"];

/// An answer with `Query { a: Int }`, an `Int` scalar and a `Color` enum, plus `directives`.
pub(crate) fn intro(directives: Option<Value>) -> Schema {
    let mut schema = json!({
        "queryType": {"name": "Query"},
        "types": [
            {"kind": "OBJECT", "name": "Query", "fields": [
                {"name": "a", "args": [], "type": {"kind": "SCALAR", "name": "Int"}}
            ]},
            {"kind": "SCALAR", "name": "Int"},
            {"kind": "ENUM", "name": "Color", "enumValues": [{"name": "RED"}]}
        ]
    });
    if let Some(d) = directives {
        schema["directives"] = d;
    }
    let text = json!({"data": {"__schema": schema}}).to_string();
    Schema::from_json(&text).unwrap_or_else(|e| panic!("{e}: {text}"))
}

pub(crate) fn names(s: &Schema) -> Vec<&str> {
    s.directives().iter().map(|d| d.name()).collect()
}

fn on(name: &str, locations: Value) -> Value {
    json!({"name": name, "locations": locations, "args": []})
}

#[test]
fn the_built_ins_are_added_when_the_answer_lists_none() {
    let s = intro(None);
    assert_eq!(names(&s), BUILT_INS);
    assert_eq!(intro(Some(json!([]))), s);
    assert_eq!(intro(Some(Value::Null)), s);
    let include = s.directive_named("include").expect("include");
    assert_eq!(
        include.description(),
        Some(
            "Directs the executor to include this field or fragment only when the `if` argument is true."
        )
    );
    assert_eq!(
        include.locations(),
        ["FIELD", "FRAGMENT_SPREAD", "INLINE_FRAGMENT"]
    );
    assert!(!include.is_repeatable());
    let arg = &include.args()[0];
    assert_eq!(
        (arg.name(), arg.ty().display(), arg.description()),
        ("if", "Boolean!", Some("Included when true."))
    );
    let skip = s.directive_named("skip").unwrap();
    assert_eq!(
        skip.description(),
        Some("Directs the executor to skip this field or fragment when the `if` argument is true.")
    );
    assert_eq!(skip.args()[0].description(), Some("Skipped when true."));
    let deprecated = s.directive_named("deprecated").unwrap();
    assert_eq!(
        deprecated.locations(),
        [
            "FIELD_DEFINITION",
            "ARGUMENT_DEFINITION",
            "INPUT_FIELD_DEFINITION",
            "ENUM_VALUE"
        ]
    );
    let reason = &deprecated.args()[0];
    assert_eq!(
        (reason.name(), reason.ty().display(), reason.default_value()),
        ("reason", "String", Some("\"No longer supported\""))
    );
    let specified = s.directive_named("specifiedBy").unwrap();
    assert_eq!(specified.locations(), ["SCALAR"]);
    assert_eq!(specified.args()[0].ty().display(), "String!");
    assert!(s.directive_named("defer").is_none());
}

#[test]
fn own_directives_come_first_then_the_missing_built_ins() {
    let s = intro(Some(json!([
        {"name": "tag", "description": "Tags a field.", "isRepeatable": true,
         "locations": ["FIELD", "QUERY"],
         "args": [{"name": "name", "description": "The tag.", "defaultValue": "\"x\"",
                   "type": {"kind": "NON_NULL", "name": null,
                            "ofType": {"kind": "SCALAR", "name": "String"}}}]},
        on("include", json!(["FIELD"]))
    ])));
    assert_eq!(
        names(&s),
        ["tag", "include", "skip", "deprecated", "specifiedBy"]
    );
    let tag = s.directive_named("tag").unwrap();
    assert_eq!(tag.description(), Some("Tags a field."));
    assert!(tag.is_repeatable());
    assert_eq!(tag.locations(), ["FIELD", "QUERY"]);
    let arg = &tag.args()[0];
    assert_eq!(
        (
            arg.name(),
            arg.ty().display(),
            arg.default_value(),
            arg.description()
        ),
        ("name", "String!", Some("\"x\""), Some("The tag."))
    );
    let include = s.directive_named("include").unwrap();
    assert_eq!(
        (include.description(), include.locations().len()),
        (None, 1)
    );
}

#[test]
fn is_repeatable_absent_or_not_a_bool_is_false() {
    let s = intro(Some(json!([
        on("a", json!(["FIELD"])),
        {"name": "b", "locations": ["FIELD"], "args": [], "isRepeatable": "yes"}
    ])));
    assert!(!s.directive_named("a").unwrap().is_repeatable());
    assert!(!s.directive_named("b").unwrap().is_repeatable());
}

#[test]
fn the_admission_rule_skips_bad_definitions_and_keeps_the_rest() {
    let int = json!({"kind": "SCALAR", "name": "Int"});
    let s = intro(Some(json!([
        on("bad name", json!(["FIELD"])),
        on("loc", json!(["FIELD", "NOWHERE"])),
        on("lower", json!(["field"])),
        on("none", json!([])),
        {"name": "argname", "locations": ["FIELD"], "args": [{"name": "1x", "type": int}]},
        {"name": "argtype", "locations": ["FIELD"],
         "args": [{"name": "x", "type": {"kind": "INPUT_OBJECT", "name": "Missing"}}]},
        {"name": "enumarg", "locations": ["FIELD"],
         "args": [{"name": "c", "type": {"kind": "ENUM", "name": "Color"}}]},
        on("enumarg", json!(["QUERY"])),
        on("twice", json!(["NOWHERE"])),
        on("twice", json!(["FIELD"])),
        {"name": "noargs", "locations": ["FIELD"]}
    ])));
    assert_eq!(
        names(&s),
        [
            "enumarg",
            "twice",
            "noargs",
            "skip",
            "include",
            "deprecated",
            "specifiedBy"
        ]
    );
    assert_eq!(s.directive_named("enumarg").unwrap().locations(), ["FIELD"]);
    assert_eq!(s.directive_named("twice").unwrap().locations(), ["FIELD"]);
    assert_eq!(s.type_named("Query").unwrap().fields().len(), 1);
}

#[test]
fn malformed_directive_data_never_fails_the_load() {
    assert_eq!(names(&intro(Some(json!("not a list")))), BUILT_INS);
    let s = intro(Some(json!([
        42,
        {"locations": ["FIELD"]},
        {"name": "x", "locations": "FIELD"},
        {"name": "y", "locations": ["FIELD", 3]},
        {"name": "z", "locations": ["FIELD"], "args": "no"},
        {"name": "w", "locations": ["FIELD"], "args": [{"name": "a"}]},
        on("ok", json!(["FIELD"]))
    ])));
    assert_eq!(
        names(&s),
        ["ok", "skip", "include", "deprecated", "specifiedBy"]
    );
}

#[test]
fn the_fixture_has_only_the_built_ins() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/graphql/schema.json"
    ))
    .unwrap();
    assert_eq!(names(&Schema::from_json(&text).unwrap()), BUILT_INS);
}
