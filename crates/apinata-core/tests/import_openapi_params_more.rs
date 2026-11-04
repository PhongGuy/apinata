//! Operations and parameters that are dropped, and Swagger 2.0 parameters.

use crate::openapi_support;

use apinata_core::import::NoteKind;
use openapi_support::{P, Q, h, headers, import, notes, one, p, params, request, tree};
use serde_json::json;

#[test]
fn only_http_methods_become_requests_and_3_2_additions_are_noted() {
    let doc = json!({"openapi": "3.2.0", "paths": {"/a": {
        "summary": "path summary", "GET": {"summary": "upper"}, "trace": {}, "query": {"summary": "q"},
        "additionalOperations": {"LINK": {}}, "options": "not an object"
    }}});
    let c = import(&doc).collection;
    assert_eq!(
        tree(&c),
        [
            "R 1 upper GET {{baseUrl}}/a",
            "R 2 trace /a TRACE {{baseUrl}}/a"
        ]
    );
    let dropped: Vec<(&str, &str)> = c
        .unwritten
        .iter()
        .filter(|n| n.kind == NoteKind::Dropped)
        .map(|n| (n.source.as_str(), n.detail.as_str()))
        .collect();
    assert_eq!(
        dropped,
        [
            (
                "paths./a.query",
                "the query method (OpenAPI 3.2) is not imported"
            ),
            (
                "paths./a.additionalOperations",
                "additionalOperations (OpenAPI 3.2) is not imported"
            ),
            (
                "paths./a.options",
                "not an operation object, so no request was made"
            ),
        ]
    );
}

#[test]
fn the_3_2_additions_are_matched_in_any_case_like_methods() {
    let doc = json!({"openapi": "3.2.0", "paths": {"/a": {
        "QUERY": {}, "AdditionalOperations": {"LINK": {}}, "get": {"summary": "S"}
    }}});
    let c = import(&doc).collection;
    let dropped: Vec<(&str, &str)> = c
        .unwritten
        .iter()
        .map(|n| (n.source.as_str(), n.detail.as_str()))
        .collect();
    assert_eq!(
        dropped,
        [
            (
                "paths./a.QUERY",
                "the query method (OpenAPI 3.2) is not imported"
            ),
            (
                "paths./a.AdditionalOperations",
                "additionalOperations (OpenAPI 3.2) is not imported"
            ),
        ]
    );
}

#[test]
fn reserved_headers_cookies_and_unknown_locations_are_dropped_with_notes() {
    let c = import(&one(json!([
        {"name": "accept", "in": "header"}, {"name": "Content-Type", "in": "header"}, {"name": "Authorization", "in": "header"},
        {"name": "session", "in": "cookie"}, {"name": "raw", "in": "querystring"}, {"name": "x", "in": "body-ish"},
        {"in": "query"}, "text", {"name": "ok", "in": "header", "example": "1"}
    ]))).collection;
    let r = request(&c, "Q");
    assert!(params(r).is_empty());
    assert_eq!(headers(r), [h("ok", "1", true)]);
    let dropped: Vec<(&str, &str)> = r
        .notes
        .iter()
        .filter(|n| n.kind == NoteKind::Dropped)
        .map(|n| (n.source.as_str(), n.detail.as_str()))
        .collect();
    let sources: Vec<String> = (0..8)
        .map(|i| format!("paths./q.get.parameters[{i}]"))
        .collect();
    assert_eq!(
        dropped
            .iter()
            .map(|(s, _)| s.to_string())
            .collect::<Vec<_>>(),
        sources
    );
    assert!(
        dropped[0].1.contains("accept") && dropped[2].1.contains("Authorization"),
        "{dropped:?}"
    );
    assert!(dropped[3].1.contains("cookie"), "{dropped:?}");
    assert_eq!(
        dropped[4].1,
        "in: querystring (OpenAPI 3.2) is not imported"
    );
    assert!(dropped[5].1.contains("body-ish"), "{dropped:?}");
    assert!(
        dropped[6].1.contains("not a parameter object")
            && dropped[7].1.contains("not a parameter object"),
        "{dropped:?}"
    );
}

#[test]
fn swagger_2_parameters_use_their_collection_format() {
    let doc = json!({"swagger": "2.0", "host": "x.test", "paths": {"/s/{id}": {"get": {"summary": "S", "parameters": [
        {"name": "id", "in": "path", "required": true, "type": "integer", "default": 1},
        {"name": "ids", "in": "query", "type": "array", "items": {"type": "integer"}, "collectionFormat": "pipes", "default": [1, 2]},
        {"name": "csv", "in": "query", "type": "array", "items": {"type": "string"}, "default": ["a", "b"]},
        {"name": "multi", "in": "query", "type": "array", "items": {"type": "string"}, "collectionFormat": "multi", "default": ["m", "n"]},
        {"name": "level", "in": "query", "type": "string", "enum": ["low", "high"]},
        {"name": "X-Key", "in": "header", "type": "string"},
        {"name": "payload", "in": "body", "schema": {"type": "object"}}
    ]}}}});
    let c = import(&doc).collection;
    let r = request(&c, "S");
    assert_eq!(
        params(r),
        [
            p("id", "1", P, true),
            p("ids", "1|2", Q, true),
            p("csv", "a,b", Q, true),
            p("multi", "m", Q, true),
            p("multi", "n", Q, true),
            p("level", "low", Q, false),
            p("level", "high", Q, false),
        ]
    );
    assert_eq!(headers(r), [h("X-Key", "", false)]);
    assert!(r.notes.is_empty(), "{:?}", r.notes);
}

#[test]
fn a_swagger_2_array_without_a_value_is_empty() {
    let array =
        json!({"name": "tags", "in": "query", "type": "array", "items": {"type": "string"}});
    let swagger = json!({"swagger": "2.0", "host": "x.test",
        "paths": {"/s": {"get": {"summary": "S", "parameters": [array]}}}});
    let c = import(&swagger).collection;
    assert_eq!(params(request(&c, "S")), [p("tags", "", Q, false)]);
    let openapi = json!({"openapi": "3.0.3", "paths": {"/s": {"get": {"summary": "S", "parameters": [
        {"name": "tags", "in": "query", "schema": {"type": "array", "items": {"type": "string"}}}]}}}});
    let c = import(&openapi).collection;
    assert_eq!(params(request(&c, "S")), [p("tags", "[]", Q, false)]);
}

#[test]
fn swagger_2_enum_items_join_and_item_values_and_bounds_are_ignored() {
    let enum_items = |name: &str, format: &str, required: bool| {
        json!({"name": name, "in": "query", "type": "array", "required": required,
            "items": {"type": "string", "enum": ["x", "y"], "default": "y"}, "collectionFormat": format})
    };
    let doc = json!({"swagger": "2.0", "host": "x.test", "paths": {"/s": {"get": {"summary": "S", "parameters": [
        enum_items("csv", "csv", false),
        enum_items("pipes", "pipes", true),
        enum_items("multi", "multi", false),
        {"name": "ex", "in": "query", "type": "array", "items": {"type": "string", "example": "e"}},
        {"name": "def", "in": "query", "type": "array", "items": {"type": "string", "default": "d"}},
        {"name": "min", "in": "query", "type": "integer", "minimum": 5}
    ]}}}});
    let c = import(&doc).collection;
    assert_eq!(
        params(request(&c, "S")),
        [
            p("csv", "x,y", Q, false),
            p("pipes", "x|y", Q, true),
            p("multi", "x", Q, false),
            p("multi", "y", Q, true),
            p("ex", "", Q, false),
            p("def", "", Q, false),
            p("min", "", Q, false),
        ]
    );
}

#[test]
fn no_note_names_another_tool() {
    let doc = json!({"openapi": "3.2.0", "webhooks": {}, "servers": [{"url": "/rel"}], "paths": {"/a": {
        "get": {"summary": "S", "servers": [{"url": "https://a.test"}, {"url": "https://b.test"}],
            "parameters": [{"name": "Accept", "in": "header"}, {"name": "c", "in": "cookie"}, {"$ref": "other.yaml#/p"}, {"$ref": "#/nope"}]},
        "put": {"summary": "S"}, "query": {}, "additionalOperations": {}, "head": 1
    }}});
    let c = import(&doc).collection;
    let texts: Vec<String> = notes(&c)
        .iter()
        .map(|n| format!("{} {}", n.source, n.detail))
        .collect();
    assert!(texts.len() >= 10, "{texts:?}");
    for t in texts {
        assert!(!t.to_lowercase().contains("bruno"), "{t}");
    }
}

/// The `(source, kind is Converted, detail)` notes of the one request `Q` built from `parameters`.
fn style_notes(parameters: serde_json::Value) -> (Vec<(String, String, bool)>, Vec<String>) {
    let c = import(&one(parameters)).collection;
    let r = request(&c, "Q");
    let notes = r
        .notes
        .iter()
        .map(|n| {
            (
                n.source.clone(),
                n.detail.clone(),
                n.kind == NoteKind::Converted,
            )
        })
        .collect();
    let names = params(r).into_iter().map(|(n, ..)| n).collect();
    (notes, names)
}

fn array_param(extra: serde_json::Value) -> serde_json::Value {
    let mut param = json!({"name": "tags", "in": "query",
        "schema": {"type": "array", "items": {"type": "string"}}});
    param
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    param
}

const SRC: &str = "paths./q.get.parameters[0]";

#[test]
fn pipe_delimited_arrays_are_noted_as_plain_entries() {
    let (notes, names) = style_notes(json!([array_param(json!({"style": "pipeDelimited"}))]));
    assert_eq!(
        notes,
        [(
            SRC.to_owned(),
            "the query parameter `tags` uses style pipeDelimited with explode false, which is not expressed; it is imported as plain `tags` entries".to_owned(),
            true
        )]
    );
    assert_eq!(names, ["tags"]);
}

#[test]
fn space_delimited_arrays_are_noted_even_with_explode_true() {
    let (notes, _) = style_notes(json!([array_param(
        json!({"style": "spaceDelimited", "explode": true})
    )]));
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(
        notes[0].1,
        "the query parameter `tags` uses style spaceDelimited with explode true, which is not expressed; it is imported as plain `tags` entries"
    );
}

#[test]
fn form_without_explode_on_an_array_is_noted() {
    let (notes, _) = style_notes(json!([array_param(json!({"explode": false}))]));
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(
        notes[0].1,
        "the query parameter `tags` uses style form with explode false, which is not expressed; it is imported as plain `tags` entries"
    );
    assert!(notes[0].2);
}

#[test]
fn form_without_explode_on_a_scalar_is_not_noted() {
    let (notes, _) = style_notes(json!([{"name": "n", "in": "query", "explode": false,
        "schema": {"type": "string"}}]));
    assert!(notes.is_empty(), "{notes:?}");
}

#[test]
fn deep_object_properties_become_bracketed_params() {
    let (notes, names) = style_notes(
        json!([{"name": "filter", "in": "query", "style": "deepObject",
        "explode": true, "schema": {"type": "object", "properties": {"a": {"type": "string"}, "b": {"type": "integer"}}}}]),
    );
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(names, ["filter[a]", "filter[b]"]);
}

#[test]
fn deep_object_without_explode_is_treated_as_exploded() {
    let (notes, names) = style_notes(
        json!([{"name": "filter", "in": "query", "style": "deepObject",
        "schema": {"type": "object", "properties": {"a": {"type": "string"}}}}]),
    );
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(names, ["filter[a]"]);
}

#[test]
fn a_nested_deep_object_property_is_one_noted_entry() {
    let (notes, names) = style_notes(
        json!([{"name": "filter", "in": "query", "style": "deepObject",
        "schema": {"type": "object", "properties": {"a": {"type": "object", "properties": {"x": {"type": "string"}}}}}}]),
    );
    assert_eq!(names, ["filter[a]"]);
    assert_eq!(
        notes.iter().map(|n| n.1.as_str()).collect::<Vec<_>>(),
        [
            "the query parameter `filter` uses style deepObject, whose property `a` is an object or array; nested values are not expanded, and it is imported as one `filter[a]` entry"
        ]
    );
}

#[test]
fn deep_object_without_properties_is_noted() {
    let (notes, names) = style_notes(
        json!([{"name": "filter", "in": "query", "style": "deepObject",
        "schema": {"type": "object"}}]),
    );
    assert_eq!(names, ["filter"]);
    assert_eq!(
        notes[0].1,
        "the query parameter `filter` uses style deepObject, which is not expressed without known properties; it is imported as plain `filter` entries"
    );
}

#[test]
fn deep_object_with_explode_false_is_noted_and_stays_flat() {
    let (notes, names) = style_notes(
        json!([{"name": "filter", "in": "query", "style": "deepObject",
        "explode": false, "schema": {"type": "object", "properties": {"a": {"type": "string"}}}}]),
    );
    assert_eq!(names, ["a"]);
    assert_eq!(
        notes[0].1,
        "the query parameter `filter` uses style deepObject with explode false, which is not expressed; it is imported as one entry per property"
    );
}

#[test]
fn label_and_matrix_paths_and_misplaced_styles_are_noted() {
    let (notes, _) = style_notes(json!([
        {"name": "id", "in": "path", "required": true, "style": "matrix", "schema": {"type": "string"}},
        {"name": "who", "in": "path", "required": true, "style": "label", "schema": {"type": "string"}},
        {"name": "q", "in": "query", "style": "matrix", "schema": {"type": "string"}},
    ]));
    let details: Vec<&str> = notes.iter().map(|n| n.1.as_str()).collect();
    assert_eq!(
        details,
        [
            "the path parameter `id` uses style matrix, which is not expressed; it is imported as plain `id` entries",
            "the path parameter `who` uses style label, which is not expressed; it is imported as plain `who` entries",
            "the query parameter `q` uses style matrix, which is not expressed; it is imported as plain `q` entries",
        ]
    );
    assert_eq!(notes[1].0, "paths./q.get.parameters[1]");
}

#[test]
fn default_styles_add_no_notes() {
    let object = json!({"type": "object", "properties": {"a": {"type": "string"}}});
    let (notes, names) = style_notes(json!([
        array_param(json!({})),
        {"name": "tags2", "in": "query", "style": "form", "explode": true, "schema": {"type": "array", "items": {"type": "string"}}},
        {"name": "o", "in": "query", "schema": object},
        {"name": "id", "in": "path", "required": true, "style": "simple", "schema": {"type": "string"}},
        {"name": "s", "in": "path", "required": true, "explode": true, "schema": {"type": "string"}},
        {"name": "X-List", "in": "header", "style": "simple", "explode": true, "schema": {"type": "array", "items": {"type": "string"}}},
    ]));
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(names, ["tags", "tags2", "a", "id", "s"]);
}

#[test]
fn swagger_2_parameters_are_never_style_noted() {
    let doc = json!({"swagger": "2.0", "host": "x.test", "paths": {"/s": {"get": {"summary": "S", "parameters": [
        {"name": "tags", "in": "query", "type": "array", "items": {"type": "string"}, "style": "pipeDelimited", "explode": false}]}}}});
    let c = import(&doc).collection;
    let styled: Vec<_> = request(&c, "S")
        .notes
        .iter()
        .filter(|n| n.kind == NoteKind::Converted)
        .collect();
    assert!(styled.is_empty(), "{styled:?}");
}
