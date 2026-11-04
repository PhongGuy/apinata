//! Parameters become params and headers.

use crate::openapi_support;

use openapi_support::{P, Q, h, headers, import, one, p, params, request};
use serde_json::json;

#[test]
fn path_item_and_operation_parameters_merge_by_name_and_location() {
    let doc = json!({"openapi": "3.0.0", "servers": [{"url": "https://x.test"}],
    "components": {"parameters": {"Limit": {"name": "limit", "in": "query", "required": true, "schema": {"type": "integer", "default": 20}}}},
    "paths": {"/pets/{id}": {
        "parameters": [
            {"name": "id", "in": "path", "required": true, "schema": {"type": "string"}, "description": "pet id"},
            {"name": "limit", "in": "query", "schema": {"type": "integer"}},
            {"name": "X-Trace", "in": "header", "example": "abc"}
        ],
        "get": {"summary": "Get", "parameters": [{"$ref": "#/components/parameters/Limit"}, {"name": "id", "in": "header", "example": 7}]}
    }}});
    let c = import(&doc).collection;
    let r = request(&c, "Get");
    assert_eq!(params(r), [p("id", "", P, true), p("limit", "20", Q, true)]);
    assert_eq!(r.request.params[0].description.as_deref(), Some("pet id"));
    assert_eq!(headers(r), [h("X-Trace", "abc", true), h("id", "7", true)]);
}

#[test]
fn enabled_query_params_are_in_the_url() {
    let c = import(&one(json!([
        {"name": "a", "in": "query", "example": "ex", "schema": {"default": "d"}},
        {"name": "b", "in": "query", "examples": {"one": {"value": 1.5}, "two": {"value": 2}}},
        {"name": "c", "in": "query", "schema": {"type": "array", "default": ["x", "y"]}},
        {"name": "d", "in": "query", "schema": {"example": true}},
        {"name": "e", "in": "query", "schema": {"type": "array", "items": {"type": "string"}}},
        {"name": "f", "in": "query", "schema": {"examples": ["first"]}},
        {"name": "g", "in": "query", "schema": {"minimum": 3}},
        {"name": "h", "in": "query", "schema": {"type": "string"}},
        {"name": "i", "in": "query", "required": true, "schema": {"type": "string"}},
        {"name": "j", "in": "query", "example": "a&b #c"}
    ])))
    .collection;
    let r = request(&c, "Q");
    assert_eq!(
        params(r),
        [
            p("a", "ex", Q, true),
            p("b", "1.5", Q, true),
            p("c", "[\"x\",\"y\"]", Q, true),
            p("d", "true", Q, true),
            p("e", "[]", Q, false),
            p("f", "first", Q, true),
            p("g", "3", Q, false),
            p("h", "", Q, false),
            p("i", "", Q, true),
            p("j", "a&b #c", Q, true),
        ]
    );
    assert_eq!(
        r.request.url,
        "{{baseUrl}}/q?a=ex&b=1.5&c=[\"x\",\"y\"]&d=true&f=first&i=&j=a%26b%20%23c"
    );
}

#[test]
fn enum_parameters_become_one_entry_per_value() {
    let c = import(&one(json!([
        {"name": "sort", "in": "query", "schema": {"enum": ["asc", "desc"], "default": "desc"}},
        {"name": "kind", "in": "query", "required": true, "schema": {"enum": ["a", "b"]}},
        {"name": "tags", "in": "query", "schema": {"type": "array", "items": {"enum": ["x", "y"]}}},
        {"name": "Mode", "in": "header", "schema": {"enum": [1, 2]}}
    ])))
    .collection;
    let r = request(&c, "Q");
    assert_eq!(
        params(r),
        [
            p("sort", "asc", Q, false),
            p("sort", "desc", Q, true),
            p("kind", "a", Q, true),
            p("kind", "b", Q, false),
            p("tags", "x", Q, false),
            p("tags", "y", Q, false),
        ]
    );
    assert_eq!(headers(r), [h("Mode", "1", false), h("Mode", "2", false)]);
    assert_eq!(r.request.url, "{{baseUrl}}/q?sort=desc&kind=a");
}

#[test]
fn object_schema_parameters_explode_into_their_properties() {
    let c = import(&one(json!([{"name": "filter", "in": "query", "schema": {"type": "object", "required": ["from"],
        "example": {"to": "z"}, "properties": {"from": {"type": "string"}, "to": {"type": "string"}, "n": {"type": "integer", "example": 5}}}}]))).collection;
    assert_eq!(
        params(request(&c, "Q")),
        [
            p("from", "", Q, true),
            p("to", "z", Q, true),
            p("n", "5", Q, true)
        ]
    );
}
