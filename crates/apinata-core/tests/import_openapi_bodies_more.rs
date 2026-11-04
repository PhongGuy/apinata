//! XML, multipart and Swagger 2.0 bodies.

use crate::openapi_support;

use apinata_core::import::{ImportNote, NoteKind};
use apinata_core::{BodySpec, KeyValue, MultipartField};
use openapi_support::{import, request};
use serde_json::{Value, json};

fn xml_body(media: Value) -> BodySpec {
    let doc = json!({"openapi": "3.0.3", "servers": [{"url": "https://x.test"}],
        "paths": {"/x": {"post": {"summary": "X", "requestBody": {"content": {"application/xml": media}}}}}});
    request(&import(&doc).collection, "X").request.body.clone()
}

fn pet() -> Value {
    json!({"type": "object", "xml": {"name": "Pet"}, "properties": {
        "id": {"type": "integer", "xml": {"attribute": true}},
        "name": {"type": "string", "example": "doggie"},
        "category": {"type": "object", "properties": {"label": {"type": "string"}}}
    }})
}

#[test]
fn xml_bodies_follow_the_schema_shape() {
    let empty = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Pet id=\"\">\n  <name></name>\n  <category>\n    <label></label>\n  </category>\n</Pet>";
    assert_eq!(
        xml_body(json!({"schema": pet()})),
        BodySpec::Xml(empty.into())
    );
    let filled = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Pet id=\"7\">\n  <name>R&amp;R &lt;3</name>\n  <category>\n    <label></label>\n  </category>\n</Pet>";
    let media = json!({"schema": pet(), "example": {"id": 7, "name": "R&R <3"}});
    assert_eq!(xml_body(media), BodySpec::Xml(filled.into()));
    let raw = json!({"schema": {"type": "string", "example": "<raw/>"}});
    assert_eq!(xml_body(raw), BodySpec::Xml("<raw/>".into()));
}

#[test]
fn multipart_binary_fields_need_a_file() {
    let schema = json!({"type": "object", "properties": {"file": {"type": "string", "format": "binary"}, "note": {"type": "string", "example": "hi"}}});
    let doc = json!({"openapi": "3.0.3", "servers": [{"url": "https://x.test"}],
        "paths": {"/m": {"post": {"summary": "M", "requestBody": {"content": {"multipart/form-data": {"schema": schema}}}}}}});
    let c = import(&doc).collection;
    let r = request(&c, "M");
    let want = vec![
        MultipartField::files("file", Vec::new()),
        MultipartField::text("note", "hi"),
    ];
    assert_eq!(r.request.body, BodySpec::Multipart(want));
    let needs_file = r
        .notes
        .iter()
        .any(|n| n.kind == NoteKind::NeedsInput && n.detail.contains("`file`"));
    assert!(needs_file, "{:?}", r.notes);
}

#[test]
fn array_and_object_values_are_json_text() {
    let media = json!({"example": {"list": ["a", "b"], "obj": {"k": 1}}});
    let doc = json!({"openapi": "3.0.3", "servers": [{"url": "https://x.test"}],
        "paths": {"/m": {"post": {"summary": "M", "requestBody": {"content": {"multipart/form-data": media}}},
            "get": {"summary": "Q", "parameters": [{"name": "ids", "in": "query", "example": ["a", "b"]}]}}}});
    let c = import(&doc).collection;
    let want = vec![
        MultipartField::text("list", r#"["a","b"]"#),
        MultipartField::text("obj", r#"{"k":1}"#),
    ];
    assert_eq!(request(&c, "M").request.body, BodySpec::Multipart(want));
    assert_eq!(request(&c, "Q").request.params[0].value, r#"["a","b"]"#);
}

fn swagger(op: Value, root_consumes: Option<Value>) -> (BodySpec, Vec<NoteKind>) {
    let mut doc = json!({"swagger": "2.0", "host": "x.test", "paths": {"/s": {"post": op}},
        "definitions": {"Pet": {"type": "object", "properties": {"name": {"type": "string", "example": "rex"}}}}});
    if let Some(consumes) = root_consumes {
        doc["consumes"] = consumes;
    }
    let c = import(&doc).collection;
    let r = request(&c, "S");
    (
        r.request.body.clone(),
        r.notes.iter().map(|n| n.kind).collect(),
    )
}

#[test]
fn swagger_2_body_parameters_use_consumes() {
    let body = json!({"name": "pet", "in": "body", "schema": {"$ref": "#/definitions/Pet"}});
    let (spec, notes) = swagger(json!({"summary": "S", "parameters": [body.clone()]}), None);
    assert_eq!(spec, BodySpec::Json("{\n  \"name\": \"rex\"\n}".into()));
    assert!(notes.is_empty(), "{notes:?}");
    let op = json!({"summary": "S", "parameters": [body.clone()]});
    let (spec, _) = swagger(op, Some(json!(["text/plain"])));
    assert_eq!(spec, BodySpec::Text(String::new()));
    let op = json!({"summary": "S", "consumes": ["application/xml"], "parameters": [body]});
    let (spec, _) = swagger(op, Some(json!(["text/plain"])));
    assert!(matches!(spec, BodySpec::Xml(_)), "{spec:?}");
}

#[test]
fn swagger_2_form_data_becomes_urlencoded_or_multipart() {
    let fields = json!([{"name": "a", "in": "formData", "type": "string", "default": "x"}, {"name": "b", "in": "formData", "type": "integer"}]);
    let (spec, _) = swagger(json!({"summary": "S", "parameters": fields.clone()}), None);
    assert_eq!(
        spec,
        BodySpec::FormUrlEncoded(vec![KeyValue::new("a", "x"), KeyValue::new("b", "")])
    );
    let with_file = json!([{"name": "a", "in": "formData", "type": "string"}, {"name": "upload", "in": "formData", "type": "file"}]);
    let (spec, notes) = swagger(json!({"summary": "S", "parameters": with_file}), None);
    let want = vec![
        MultipartField::text("a", ""),
        MultipartField::files("upload", Vec::new()),
    ];
    assert_eq!(spec, BodySpec::Multipart(want));
    assert!(notes.contains(&NoteKind::NeedsInput), "{notes:?}");
    let op = json!({"summary": "S", "consumes": ["multipart/form-data"], "parameters": fields});
    let (spec, _) = swagger(op, None);
    assert!(matches!(spec, BodySpec::Multipart(_)), "{spec:?}");
}

#[test]
fn swagger_2_operation_body_overrides_the_path_body() {
    let doc = json!({"swagger": "2.0", "host": "x.test", "paths": {"/s": {
        "parameters": [{"name": "old", "in": "body", "schema": {"type": "object", "properties": {"old": {"type": "string"}}}}],
        "post": {"summary": "S", "parameters": [{"name": "pet", "in": "body", "schema": {"$ref": "#/definitions/Pet"}}]}}},
        "definitions": {"Pet": {"type": "object", "properties": {"name": {"type": "string", "example": "rex"}}}}});
    let c = import(&doc).collection;
    let body = request(&c, "S").request.body.clone();
    assert_eq!(body, BodySpec::Json("{\n  \"name\": \"rex\"\n}".into()));
}

#[test]
fn a_broken_swagger_2_parameter_ref_is_noted_once() {
    let doc = json!({"swagger": "2.0", "host": "x.test", "paths": {"/s": {
        "parameters": [{"$ref": "#/parameters/Missing"}],
        "post": {"summary": "S", "parameters": [{"$ref": "#/parameters/Gone"}]}}}});
    let c = import(&doc).collection;
    let r = request(&c, "S");
    let at = |source: &str| {
        let broken = |n: &&ImportNote| n.source == source && n.detail.contains("points at nothing");
        r.notes.iter().filter(broken).count()
    };
    assert_eq!(at("paths./s.parameters[0]"), 1, "{:?}", r.notes);
    assert_eq!(at("paths./s.post.parameters[0]"), 1, "{:?}", r.notes);
}
