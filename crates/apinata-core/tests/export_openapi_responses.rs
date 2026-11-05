//! `export::openapi` responses from saved examples.

use crate::export_support;

use export_support::*;
use serde_json::json;

const EXAMPLES: &str = "examples:
  - name: ok
    response:
      status: 200
      statusText: OK
      headers:
        - name: Content-Type
          value: application/json; charset=utf-8
        - name: X-Hidden
          value: header-value
      body:
        type: json
        data: '{\"id\": 7}'
  - name: ok
    response:
      status: 200
      body:
        type: json
        data: not json
  - name: missing
    response:
      status: 404
      body:
        type: text
        data: nope
  - name: page
    response:
      status: 404
      statusText: Not Found
      body:
        type: html
        data: <p>no</p>
  - name: png
    response:
      status: 200
      body:
        type: binary
        data: iVBORw0K
  - name: bare
  - name: ok
    response:
      status: 200
      body:
        type: xml
        data: <ok/>
";

#[test]
fn examples_group_by_status_with_a_media_type_per_body() {
    let file = http("r", "GET", "https://x.test/r", INHERIT) + EXAMPLES;
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("r.yml", &file)]);
    let v = openapi_doc(&c);
    let responses = &v["paths"]["/r"]["get"]["responses"];
    assert_eq!(keys(responses), ["200", "404"]);
    assert_eq!(
        *responses,
        json!({
            "200": {"description": "OK",
            "headers": {"X-Hidden": {"schema": {"type": "string"}, "example": "header-value"}},
            "content": {
                "application/json; charset=utf-8": {"schema": {"$ref": "#/components/schemas/r_response_200"},
                    "examples": {"ok": {"value": {"id": 7}}}},
                "application/json": {"examples": {"ok 2": {"value": "not json"}}},
                "application/octet-stream": {"examples": {"png": {}}},
                "application/xml": {"examples": {"ok 3": {"value": "<ok/>"}}}}},
            "404": {"description": "Example", "content": {
                "text/plain": {"examples": {"missing": {"value": "nope"}}},
                "text/html": {"examples": {"page": {"value": "<p>no</p>"}}}}}
        })
    );
    assert_eq!(
        keys(&responses["200"]),
        ["description", "headers", "content"]
    );
    assert_eq!(
        openapi_notes(&c),
        pairs(&[(
            "r / examples",
            "the binary body of example png was left out"
        )])
    );
}

#[test]
fn examples_without_bodies_give_a_description_only() {
    let file = http("r", "GET", "https://x.test/r", INHERIT)
        + "examples:\n  - name: gone\n    response:\n      status: 204\n      statusText: No Content\n  - name: bare\n";
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("r.yml", &file)]);
    let v = openapi_doc(&c);
    assert_eq!(
        v["paths"]["/r"]["get"]["responses"],
        json!({"204": {"description": "No Content"}, "200": {"description": "Example"}})
    );
    assert!(openapi_notes(&c).is_empty());
}

#[test]
fn a_blank_content_type_falls_back_to_the_body_kind() {
    let file = http("r", "GET", "https://x.test/r", INHERIT)
        + "examples:\n  - name: ok\n    response:\n      status: 200\n      headers:\n        - name: Content-Type\n          value: \" \"\n      body:\n        type: json\n        data: '{\"id\": 7}'\n";
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("r.yml", &file)]);
    let v = openapi_doc(&c);
    assert_eq!(
        v["paths"]["/r"]["get"]["responses"]["200"]["content"],
        json!({"application/json": {"schema": {"$ref": "#/components/schemas/r_response_200"},
            "examples": {"ok": {"value": {"id": 7}}}}})
    );
}
