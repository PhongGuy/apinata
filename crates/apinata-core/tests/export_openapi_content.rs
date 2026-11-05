//! `export::openapi` parameters, request bodies and schema inference.

use crate::export_support;

use apinata_core::export::openapi;
use export_support::*;
use serde_json::{Value, json};

fn param(name: &str, at: &str, required: bool, rest: Value) -> Value {
    let mut p = json!({"name": name, "in": at});
    if let Some(d) = rest.get("description") {
        p["description"] = d.clone();
    }
    p["required"] = required.into();
    p["schema"] = json!({"type": "string"});
    if let Some(e) = rest.get("example") {
        p["example"] = e.clone();
    }
    p
}

#[test]
fn path_query_and_header_parameters_follow_the_rows() {
    let under = "  params:\n    - name: id\n      value: \"7\"\n      type: path\n    - name: page\n      value: \"2\"\n      type: query\n      description: Page number\n    - name: page\n      value: \"3\"\n      type: query\n    - name: debug\n      value: \"\"\n      type: query\n      disabled: true\n  headers:\n    - name: X-Trace\n      value: abc\n    - name: authorization\n      value: Bearer tok-secret\n    - name: Content-Type\n      value: application/json\n    - name: ACCEPT\n      value: \"*/*\"\n    - name: X-Off\n      value: \"1\"\n      disabled: true\n  auth: inherit\n";
    let url = "https://x.test/users/:id/{{section}}?page=2&page=3&debug=";
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &http("r", "GET", url, under)),
    ]);
    let v = openapi_doc(&c);
    let op = &v["paths"]["/users/{id}/{section}"]["get"];
    assert_eq!(
        keys(op),
        ["summary", "operationId", "parameters", "responses"]
    );
    assert_eq!(
        op["parameters"],
        json!([
            param("id", "path", true, json!({"example": "7"})),
            param("section", "path", true, json!({})),
            param(
                "page",
                "query",
                true,
                json!({"description": "Page number", "example": "2"})
            ),
            param("debug", "query", false, json!({})),
            param("X-Trace", "header", true, json!({"example": "abc"})),
            param("X-Off", "header", false, json!({"example": "1"})),
        ])
    );
    assert_eq!(
        keys(&op["parameters"][2]),
        ["name", "in", "description", "required", "schema", "example"]
    );
    assert!(!openapi(&c).json.contains("tok-secret"));
}

const MULTIPART: &str = "  body:\n    type: multipart-form\n    data:\n      - name: a\n        type: text\n        value: \"1\"\n      - name: up\n        type: file\n        value: [/data/up.bin]\n      - name: off\n        type: text\n        value: x\n        disabled: true\n";

#[test]
fn bodies_map_by_mode_with_the_content_type_header_winning() {
    let ctype = |t: &str| format!("  headers:\n    - name: content-type\n      value: \"{t}\"\n");
    let json_body = "  body:\n    type: json\n    data: '{\"a\": 1, \"b\": [1.5, \"x\"], \"c\": {\"d\": null, \"e\": true}, \"f\": [], \"g\": 2.0}'\n";
    let file_body = "  body:\n    type: file\n    data:\n      - filePath: /data/b.png\n        selected: true\n";
    let cases: Vec<(String, Option<Value>)> = vec![
        (json_body.to_owned(), Some(json!({"application/json": {
            "schema": {"type": "object", "properties": {
                "a": {"type": "integer"},
                "b": {"type": "array", "items": {"type": "number"}},
                "c": {"type": "object", "properties": {"d": {"type": "null"}, "e": {"type": "boolean"}}},
                "f": {"type": "array", "items": {}},
                "g": {"type": "integer"}}},
            "example": {"a": 1, "b": [1.5, "x"], "c": {"d": null, "e": true}, "f": [], "g": 2.0}}}))),
        ("  body:\n    type: json\n    data: '{\"id\": {{id}}}'\n".to_owned(),
            Some(json!({"application/json": {"schema": {"type": "string"}, "example": "{\"id\": {{id}}}"}}))),
        ("  body:\n    type: xml\n    data: <a/>\n".to_owned(),
            Some(json!({"application/xml": {"schema": {"type": "string"}, "example": "<a/>"}}))),
        ("  body:\n    type: text\n    data: hi\n".to_owned(),
            Some(json!({"text/plain": {"schema": {"type": "string"}, "example": "hi"}}))),
        ("  body:\n    type: form-urlencoded\n    data:\n      - name: a\n        value: \"1\"\n      - name: b\n        value: x\n        disabled: true\n".to_owned(),
            Some(json!({"application/x-www-form-urlencoded": {"schema": {"type": "object", "properties": {"a": {"type": "string"}}}, "example": {"a": "1"}}}))),
        (MULTIPART.to_owned(), Some(json!({"multipart/form-data": {"schema": {"type": "object", "properties": {
            "a": {"type": "string"}, "up": {"type": "string", "contentMediaType": "application/octet-stream"}}},
            "example": {"a": "1"}}}))),
        (file_body.to_owned(), Some(json!({"application/octet-stream": {"schema": {"type": "string", "contentMediaType": "application/octet-stream"}}}))),
        (ctype("image/png") + file_body, Some(json!({"image/png": {"schema": {"type": "string", "contentMediaType": "image/png"}}}))),
        (ctype("application/vnd.api+json; charset=utf-8") + "  body:\n    type: json\n    data: '[3]'\n",
            Some(json!({"application/vnd.api+json": {"schema": {"type": "array", "items": {"type": "integer"}}, "example": [3]}}))),
        ("  body:\n    type: graphql\n".to_owned(), None),
        (String::new(), None),
    ];
    let files: Vec<(String, String)> = cases
        .iter()
        .enumerate()
        .map(|(i, (body, _))| {
            (
                format!("r{i}.yml"),
                http_at(
                    &format!("r{i}"),
                    i as u32 + 1,
                    "POST",
                    &format!("https://x.test/{i}"),
                    &format!("{body}{INHERIT}"),
                ),
            )
        })
        .collect();
    let (_d, c) = open_with_root(ROOT, &files);
    let v = openapi_doc(&c);
    // JSON schemas are hoisted; each is compared with its `$ref` inlined.
    let inline = |mut content: Value| {
        for media in content.as_object_mut().expect("content").values_mut() {
            if let Some(name) = media["schema"]["$ref"].as_str() {
                let name = name.trim_start_matches("#/components/schemas/");
                media["schema"] = v["components"]["schemas"][name].clone();
            }
        }
        content
    };
    for (i, (_, want)) in cases.iter().enumerate() {
        let got = v["paths"][format!("/{i}")]["post"]
            .get("requestBody")
            .map(|b| inline(b["content"].clone()));
        assert_eq!(got.as_ref(), want.as_ref(), "case {i}");
    }
    assert_eq!(
        keys(&v["paths"]["/0"]["post"]),
        ["summary", "operationId", "requestBody", "responses"]
    );
    assert_eq!(keys(&v["paths"]["/0"]["post"]["requestBody"]), ["content"]);
    assert_eq!(
        openapi_notes(&c),
        pairs(&[
            (
                "r1 / body",
                "the JSON body does not parse; it was exported as text"
            ),
            ("r9 / body", "the graphql body cannot be exported"),
        ])
    );
}

#[test]
fn header_parameters_dedupe_ignoring_case_and_the_first_row_wins() {
    let under = "  headers:\n    - name: X-Trace\n      value: a\n    - name: x-trace\n      value: b\n    - name: X-TRACE\n      value: c\n  auth: inherit\n";
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &http("r", "GET", "https://x.test/r", under)),
    ]);
    let v = openapi_doc(&c);
    assert_eq!(
        v["paths"]["/r"]["get"]["parameters"],
        json!([param("X-Trace", "header", true, json!({"example": "a"}))])
    );
}

#[test]
fn repeated_form_field_names_keep_the_first_row() {
    let form = "  body:\n    type: form-urlencoded\n    data:\n      - name: a\n        value: \"1\"\n      - name: a\n        value: \"2\"\n";
    let multipart = "  body:\n    type: multipart-form\n    data:\n      - name: a\n        type: text\n        value: \"1\"\n      - name: a\n        type: file\n        value: [/data/a.bin]\n      - name: up\n        type: file\n        value: [/data/up.bin]\n      - name: up\n        type: text\n        value: x\n";
    let empty = "  body:\n    type: multipart-form\n    data:\n      - name: off\n        type: text\n        value: x\n        disabled: true\n";
    let files: Vec<(String, String)> = [form, multipart, empty]
        .iter()
        .enumerate()
        .map(|(i, body)| {
            (
                format!("r{i}.yml"),
                http_at(
                    &format!("r{i}"),
                    i as u32 + 1,
                    "POST",
                    &format!("https://x.test/{i}"),
                    &format!("{body}{INHERIT}"),
                ),
            )
        })
        .collect();
    let (_d, c) = open_with_root(ROOT, &files);
    let v = openapi_doc(&c);
    let content = |i: usize| v["paths"][format!("/{i}")]["post"]["requestBody"]["content"].clone();
    assert_eq!(
        content(0),
        json!({"application/x-www-form-urlencoded": {"schema": {"type": "object", "properties": {"a": {"type": "string"}}}, "example": {"a": "1"}}})
    );
    let binary = json!({"type": "string", "contentMediaType": "application/octet-stream"});
    assert_eq!(
        content(1),
        json!({"multipart/form-data": {"schema": {"type": "object", "properties": {"a": {"type": "string"}, "up": binary}}, "example": {"a": "1"}}})
    );
    assert_eq!(
        content(2),
        json!({"multipart/form-data": {"schema": {"type": "object", "properties": {}}}})
    );
}

#[test]
fn a_graphql_request_is_an_operation_without_a_request_body() {
    let g = "info:\n  name: g\n  type: graphql\n  seq: 1\ngraphql:\n  url: \"https://x.test/graphql\"\n  body:\n    query: \"{ me }\"\n";
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("g.yml", g)]);
    let v = openapi_doc(&c);
    let op = &v["paths"]["/graphql"]["post"];
    assert_eq!(op["summary"], "g");
    assert!(op.get("requestBody").is_none(), "{op}");
    assert_eq!(
        openapi_notes(&c),
        pairs(&[(
            "g / body",
            "the GraphQL query and variables are not described; the operation has no request body"
        )])
    );
    assert_valid_openapi(&apinata_core::export::openapi(&c).json);
}

#[test]
fn a_literal_credential_header_parameter_gets_a_note() {
    let under = "  headers:\n    - name: X-Api-Key\n      value: abc123\n    - name: Cookie\n      value: \"{{session}}\"\n    - name: Authorization\n      value: Bearer tok\n    - name: X-Trace\n      value: t1\n  auth: inherit\n";
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &http("r", "GET", "https://x.test/r", under)),
    ]);
    assert_eq!(
        openapi_notes(&c),
        pairs(&[(
            "r / headers",
            "header X-Api-Key holds a literal credential, written into the document as a parameter example"
        )])
    );
}

const QUERY_CREDENTIALS: &str = "  params:\n    - name: api_key\n      value: k-1\n      type: query\n    - name: Access-Token\n      value: t-2\n      type: query\n    - name: token\n      value: \"{{tok}}\"\n      type: query\n    - name: page\n      value: \"2\"\n      type: query\n    - name: secret\n      value: \" \"\n      type: query\n  auth: inherit\n";

#[test]
fn a_literal_credential_query_parameter_gets_a_note() {
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &http("r", "GET", "https://x.test/r", QUERY_CREDENTIALS),
        ),
    ]);
    let tail = "holds a literal credential, written into the document as a parameter example";
    assert_eq!(
        openapi_notes(&c),
        pairs(&[
            ("r / params", &format!("query parameter api_key {tail}")),
            (
                "r / params",
                &format!("query parameter Access-Token {tail}")
            ),
        ])
    );
}

#[test]
fn a_credential_only_in_the_url_query_gets_no_note_since_the_query_is_not_exported() {
    let under =
        "  params:\n    - name: page\n      value: \"2\"\n      type: query\n  auth: inherit\n";
    let url = "https://x.test/r?page=2&api_key=k-1";
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &http("r", "GET", url, under)),
    ]);
    assert_eq!(openapi_notes(&c), pairs(&[]));
}

#[test]
fn a_credential_in_both_the_url_and_params_is_noted_once() {
    let url = "https://x.test/r?api_key=k-1";
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &http("r", "GET", url, "  auth: inherit\n")),
    ]);
    assert_eq!(openapi_notes(&c).len(), 1);
}
