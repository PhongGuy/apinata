//! Request bodies from examples or generated samples.

use crate::openapi_support;

use apinata_core::import::NoteKind;
use apinata_core::{BodySpec, KeyValue};
use openapi_support::{import, request};
use serde_json::{Value, json};

fn body(content: Value) -> (BodySpec, Vec<(NoteKind, String)>) {
    let doc = json!({"openapi": "3.1.0", "servers": [{"url": "https://x.test"}],
        "components": {
            "schemas": {"Pet": {"type": "object", "properties": {"id": {"type": "integer"}, "name": {"type": "string"}}}},
            "examples": {"Rex": {"value": {"name": "rex"}}},
            "requestBodies": {"PetBody": {"content": {"application/json": {"schema": {"$ref": "#/components/schemas/Pet"}}}}}
        },
        "paths": {"/b": {"post": {"summary": "B", "requestBody": content}}}});
    let c = import(&doc).collection;
    let r = request(&c, "B");
    let notes = r.notes.iter().map(|n| (n.kind, n.detail.clone())).collect();
    (r.request.body.clone(), notes)
}

fn json_body(content: Value) -> Value {
    match body(content).0 {
        BodySpec::Json(text) => serde_json::from_str(&text).expect("JSON text"),
        other => panic!("not JSON: {other:?}"),
    }
}

fn dropped(notes: &[(NoteKind, String)], part: &str) -> bool {
    notes
        .iter()
        .any(|(k, d)| *k == NoteKind::Dropped && d.contains(part))
}

#[test]
fn json_bodies_prefer_examples_then_samples() {
    let pet = || json!({"$ref": "#/components/schemas/Pet"});
    assert_eq!(
        json_body(json!({"content": {"application/json": {"schema": pet()}}})),
        json!({"id": 0, "name": ""})
    );
    assert_eq!(
        json_body(
            json!({"content": {"application/json": {"schema": pet(), "example": {"id": 9}}}})
        ),
        json!({"id": 9})
    );
    let examples = json!({"first": {"$ref": "#/components/examples/Rex"}, "second": {"value": {"name": "max"}}});
    assert_eq!(
        json_body(
            json!({"content": {"application/json": {"schema": pet(), "examples": examples}}})
        ),
        json!({"name": "rex"})
    );
    let own = json!({"type": "object", "example": {"own": true}});
    assert_eq!(
        json_body(
            json!({"content": {"application/json": {"schema": own, "example": {"media": true}}}})
        ),
        json!({"own": true})
    );
    assert_eq!(
        json_body(json!({"$ref": "#/components/requestBodies/PetBody"})),
        json!({"id": 0, "name": ""})
    );
    let (pretty, _) = body(json!({"content": {"application/json": {"example": {"a": 1}}}}));
    assert_eq!(pretty, BodySpec::Json("{\n  \"a\": 1\n}".into()));
}

#[test]
fn media_type_parameters_are_ignored() {
    let content = json!({"content": {"Application/Vnd.Api+JSON; charset=utf-8": {"example": {"a": 1}}, "text/plain": {}}});
    assert_eq!(json_body(content.clone()), json!({"a": 1}));
    let notes = body(content).1;
    assert!(dropped(&notes, "`text/plain`"), "{notes:?}");
}

#[test]
fn an_external_example_is_never_fetched() {
    let examples = json!({"remote": {"externalValue": "https://x.test/pet.json"}, "local": {"value": {"ok": 1}}});
    let (spec, notes) = body(json!({"content": {"application/json": {"examples": examples}}}));
    assert_eq!(spec, BodySpec::Json("{\n  \"ok\": 1\n}".into()));
    assert!(dropped(&notes, "`https://x.test/pet.json`"), "{notes:?}");
}

#[test]
fn text_and_other_media_types() {
    let text = json!({"content": {"text/plain": {"schema": {"type": "string", "example": 42}}}});
    assert_eq!(body(text).0, BodySpec::Text("42".into()));
    assert_eq!(
        body(json!({"content": {"application/octet-stream": {}}})).0,
        BodySpec::Text(String::new())
    );
    for media in ["image/png", "application/sparql-query"] {
        let (spec, notes) = body(json!({"content": {media: {"schema": {"example": "SELECT 1"}}}}));
        assert_eq!(spec, BodySpec::None, "{media}");
        assert!(dropped(&notes, &format!("`{media}`")), "{media}: {notes:?}");
    }
}

#[test]
fn form_fields_come_from_the_example_or_the_properties() {
    let schema = json!({"type": "object", "allOf": [{"properties": {"extra": {"type": "string", "default": "d"}}}],
        "properties": {"name": {"type": "string", "example": "rex", "description": "pet name"}, "age": {"type": "integer"}}});
    let (spec, _) =
        body(json!({"content": {"application/x-www-form-urlencoded": {"schema": schema.clone()}}}));
    let mut name = KeyValue::new("name", "rex");
    name.description = Some("pet name".into());
    let want = vec![name, KeyValue::new("age", ""), KeyValue::new("extra", "d")];
    assert_eq!(spec, BodySpec::FormUrlEncoded(want));
    let media = json!({"schema": schema, "example": {"n": 1, "t": "x"}});
    let (spec, _) = body(json!({"content": {"application/x-www-form-urlencoded": media}}));
    assert_eq!(
        spec,
        BodySpec::FormUrlEncoded(vec![KeyValue::new("n", "1"), KeyValue::new("t", "x")])
    );
}

#[test]
fn a_json_example_over_the_size_bound_falls_back_to_the_sample() {
    let schema = json!({"type": "object", "properties": {"id": {"type": "integer"}}});
    let big = json!({"blob": "y".repeat(70 << 10)});
    let (spec, notes) =
        body(json!({"content": {"application/json": {"schema": schema, "example": big}}}));
    assert_eq!(spec, BodySpec::Json("{\n  \"id\": 0\n}".into()));
    let cut = notes
        .iter()
        .filter(|(k, d)| *k == NoteKind::Converted && d.contains("the sample was cut at 64 KiB"))
        .count();
    assert_eq!(cut, 1, "{notes:?}");
}

#[test]
fn an_oversized_example_passed_over_for_a_later_one_is_not_a_cut_sample() {
    let big = json!({"blob": "y".repeat(70 << 10)});
    let schema = json!({"type": "object", "example": big});
    let media = json!({"schema": schema, "example": {"id": 1}});
    let (spec, notes) = body(json!({"content": {"application/json": media}}));
    assert_eq!(spec, BodySpec::Json("{\n  \"id\": 1\n}".into()));
    let cut = notes.iter().any(|(_, d)| d.contains("64 KiB"));
    assert!(!cut, "{notes:?}");
}
