//! `export::openapi`: hoisted schemas, inherited header parameters, example response headers,
//! and a round trip of all three through the OpenAPI importer.

use crate::export_support;
use crate::openapi_support;

use apinata_core::export::openapi;
use export_support::*;
use serde_json::{Value, json};

const JSON_BODY: &str = "  body:\n    type: json\n    data: '{\"name\": \"a\", \"age\": 3}'\n";

fn schema_ref(name: &str) -> Value {
    json!({"$ref": format!("#/components/schemas/{name}")})
}

#[test]
fn a_json_request_body_schema_is_hoisted_into_components() {
    let file = http("Create user!", "POST", "https://x.test/users", JSON_BODY);
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("r.yml", &file)]);
    let v = openapi_doc(&c);
    let media = &v["paths"]["/users"]["post"]["requestBody"]["content"]["application/json"];
    assert_eq!(media["schema"], schema_ref("create_user"));
    assert_eq!(media["example"], json!({"name": "a", "age": 3}));
    assert_eq!(
        v["components"]["schemas"]["create_user"],
        json!({"type": "object", "properties": {"name": {"type": "string"}, "age": {"type": "integer"}}})
    );
}

#[test]
fn component_names_are_sanitised_numbered_on_collision_and_stable() {
    let examples = "examples:\n  - name: ok\n    response:\n      status: 201\n      body:\n        type: json\n        data: '[1]'\n";
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &(http_at("a b", 1, "POST", "https://x.test/a", JSON_BODY) + examples),
        ),
        (
            "b.yml",
            &http_at("A_B", 2, "POST", "https://x.test/b", JSON_BODY),
        ),
        (
            "c.yml",
            &http_at("\u{e9}\u{e9}", 3, "POST", "https://x.test/c", JSON_BODY),
        ),
    ]);
    let v = openapi_doc(&c);
    let schemas = &v["components"]["schemas"];
    assert_eq!(
        keys(schemas),
        ["a_b", "a_b_response_201", "a_b_1", "request"]
    );
    let valid = |k: &str| {
        k.chars()
            .all(|ch| ch.is_ascii_alphanumeric() || "._-".contains(ch))
    };
    assert!(keys(schemas).into_iter().all(valid));
    assert_eq!(
        v["paths"]["/b"]["post"]["requestBody"]["content"]["application/json"]["schema"],
        schema_ref("a_b_1")
    );
    assert_eq!(keys(&v["components"]), ["schemas"]);
    assert_eq!(openapi(&c).json, openapi(&c).json);
}

#[test]
fn a_json_example_response_references_a_schema_inferred_from_it() {
    let examples = "examples:\n  - name: ok\n    response:\n      status: 200\n      body:\n        type: json\n        data: '{\"id\": 7}'\n  - name: other\n    response:\n      status: 200\n      body:\n        type: json\n        data: '{\"id\": 1.5}'\n  - name: broken\n    response:\n      status: 500\n      body:\n        type: json\n        data: not json\n";
    let file = http("get", "GET", "https://x.test/r", INHERIT) + examples;
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("r.yml", &file)]);
    let v = openapi_doc(&c);
    let responses = &v["paths"]["/r"]["get"]["responses"];
    assert_eq!(
        responses["200"]["content"]["application/json"],
        json!({"schema": schema_ref("get_response_200"), "examples": {
            "ok": {"value": {"id": 7}}, "other": {"value": {"id": 1.5}}}})
    );
    assert_eq!(
        responses["500"]["content"]["application/json"],
        json!({"examples": {"broken": {"value": "not json"}}})
    );
    assert_eq!(
        v["components"]["schemas"],
        json!({"get_response_200": {"type": "object", "properties": {"id": {"type": "integer"}}}})
    );
}

fn header(name: &str, required: bool, example: &str) -> Value {
    json!({"name": name, "in": "header", "required": required, "schema": {"type": "string"}, "example": example})
}

const HEADERS_ROOT: &str = "request:\n  headers:\n    - name: X-C\n      value: c\n    - name: X-Both\n      value: c\n    - name: Accept\n      value: \"*/*\"\n    - name: Content-Type\n      value: text/plain\n    - name: Authorization\n      value: Basic abc\n    - name: X-Off\n      value: \"1\"\n      disabled: true\n";

#[test]
fn collection_and_folder_headers_become_header_parameters() {
    let root = format!("{ROOT}{HEADERS_ROOT}");
    let folder = folder_yml(
        "f",
        1,
        "  headers:\n    - name: X-F\n      value: f\n    - name: X-Both\n      value: f\n",
    );
    let own = "  headers:\n    - name: x-both\n      value: r\n    - name: X-F\n      value: off\n      disabled: true\n  auth: inherit\n";
    let (_d, c) = open(&[
        ("opencollection.yml", &root),
        ("f/folder.yml", &folder),
        ("f/r.yml", &http("r", "GET", "https://x.test/r", own)),
        (
            "top.yml",
            &http("top", "GET", "https://x.test/top", INHERIT),
        ),
    ]);
    let v = openapi_doc(&c);
    assert_eq!(
        v["paths"]["/r"]["get"]["parameters"],
        json!([
            header("X-C", true, "c"),
            header("X-F", true, "f"),
            header("x-both", true, "r")
        ])
    );
    assert_eq!(
        v["paths"]["/top"]["get"]["parameters"],
        json!([header("X-C", true, "c"), header("X-Both", true, "c")])
    );
    assert!(
        !openapi_notes(&c)
            .iter()
            .any(|(source, _)| source == "collection / headers")
    );
}

#[test]
fn an_inherited_secret_header_keeps_its_placeholder() {
    let root = format!(
        "{ROOT}request:\n  headers:\n    - name: X-Token\n      value: \"{{{{token}}}}\"\n"
    );
    let env = "name: dev\nvariables:\n  - name: token\n    secret: true\n";
    let (_d, c) = open(&[
        ("opencollection.yml", &root),
        ("environments/dev.yml", env),
        (".env", "token=hunter2-secret\n"),
        ("r.yml", &http("r", "GET", "https://x.test/r", INHERIT)),
    ]);
    let out = openapi(&c);
    assert!(!out.json.contains("hunter2-secret"));
    let v: Value = serde_json::from_str(&out.json).unwrap();
    assert_eq!(
        v["paths"]["/r"]["get"]["parameters"],
        json!([header("X-Token", true, "{{token}}")])
    );
}

#[test]
fn example_response_headers_become_header_objects() {
    let examples = "examples:\n  - name: ok\n    response:\n      status: 200\n      headers:\n        - name: Content-Type\n          value: text/plain\n        - name: X-Rate\n          value: \"10\"\n      body:\n        type: text\n        data: hi\n  - name: again\n    response:\n      status: 200\n      headers:\n        - name: x-rate\n          value: \"9\"\n        - name: ETag\n          value: abc\n  - name: gone\n    response:\n      status: 404\n";
    let file = http("r", "GET", "https://x.test/r", INHERIT) + examples;
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("r.yml", &file)]);
    let v = openapi_doc(&c);
    let responses = &v["paths"]["/r"]["get"]["responses"];
    assert_eq!(
        responses["200"]["headers"],
        json!({
            "X-Rate": {"schema": {"type": "string"}, "example": "10"},
            "ETag": {"schema": {"type": "string"}, "example": "abc"}
        })
    );
    assert_eq!(
        keys(&responses["200"]),
        ["description", "headers", "content"]
    );
    assert_eq!(responses["404"], json!({"description": "Example"}));
}

#[test]
fn an_export_round_trips_requests_headers_and_examples_through_the_importer() {
    use openapi_support::{h, headers, import, request};
    let root = format!("{ROOT}request:\n  headers:\n    - name: X-C\n      value: c\n");
    let folder = folder_yml("Users", 1, "  headers:\n    - name: X-F\n      value: f\n");
    let examples = "examples:\n  - name: created\n    response:\n      status: 201\n      statusText: Created\n      headers:\n        - name: Location\n          value: /users/7\n      body:\n        type: json\n        data: '{\"id\": 7}'\n";
    let create = http("Create user", "POST", "https://x.test/users", JSON_BODY) + examples;
    let (_d, c) = open(&[
        ("opencollection.yml", &root),
        ("users/folder.yml", &folder),
        ("users/create.yml", &create),
    ]);
    let doc: Value = serde_json::from_str(&openapi(&c).json).unwrap();
    let bundle = import(&doc);
    let r = request(&bundle.collection, "Users/Create user");
    assert_eq!(r.request.method, "POST");
    assert_eq!(r.request.url, "{{baseUrl}}/users");
    assert_eq!(headers(r), [h("X-C", "c", true), h("X-F", "f", true)]);
    match &r.request.body {
        apinata_core::BodySpec::Json(text) => assert_eq!(
            serde_json::from_str::<Value>(text).unwrap(),
            json!({"name": "a", "age": 3})
        ),
        other => panic!("not a JSON body: {other:?}"),
    }
    // The saved example comes back with its status, headers and body.
    let [example] = r.request.examples.as_slice() else {
        panic!("one example: {:?}", r.request.examples);
    };
    assert_eq!(example.name, "created");
    let response = example.response.as_ref().expect("a response half");
    assert_eq!(response.status, Some(201));
    assert_eq!(response.status_text.as_deref(), Some("Created"));
    assert_eq!(
        response.headers,
        [
            ("Content-Type".to_owned(), "application/json".to_owned()),
            ("Location".to_owned(), "/users/7".to_owned()),
        ]
    );
    let body = response.body.as_ref().expect("a body");
    assert_eq!(body.kind, apinata_core::ExampleBodyKind::Json);
    assert_eq!(
        serde_json::from_str::<Value>(&body.data).unwrap(),
        json!({"id": 7})
    );
}

#[test]
fn an_inherited_literal_credential_is_noted_once_at_the_layer_holding_it() {
    let root = format!("{ROOT}request:\n  headers:\n    - name: X-Api-Key\n      value: abc123\n");
    let folder = folder_yml("f", 1, "  headers:\n    - name: Api-Key\n      value: k1\n");
    let own = "  headers:\n    - name: X-Api-Key\n      value: own1\n  auth: inherit\n";
    let (_d, c) = open(&[
        ("opencollection.yml", &root),
        ("f/folder.yml", &folder),
        ("f/r3.yml", &http("r3", "GET", "https://x.test/3", INHERIT)),
        (
            "f/r4.yml",
            &http_at("r4", 2, "GET", "https://x.test/4", INHERIT),
        ),
        ("r1.yml", &http_at("r1", 1, "GET", "https://x.test/1", own)),
        (
            "r2.yml",
            &http_at("r2", 2, "GET", "https://x.test/2", INHERIT),
        ),
    ]);
    let tail = "holds a literal credential, written into the document as a parameter example";
    assert_eq!(
        openapi_notes(&c),
        pairs(&[
            ("collection / headers", &format!("header X-Api-Key {tail}")),
            ("f / headers", &format!("header Api-Key {tail}")),
            ("r1 / headers", &format!("header X-Api-Key {tail}")),
        ])
    );
}

#[test]
fn example_response_credentials_are_noted_and_blank_names_skipped() {
    let examples = "examples:\n  - name: ok\n    response:\n      status: 200\n      headers:\n        - name: Set-Cookie\n          value: sid=abc\n        - name: Authorization\n          value: Bearer xyz\n        - name: X-Api-Key\n          value: \"{{k}}\"\n        - name: \" \"\n          value: blank\n        - name: X-Plain\n          value: \"1\"\n";
    let file = http("r", "GET", "https://x.test/r", INHERIT) + examples;
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("r.yml", &file)]);
    let v = openapi_doc(&c);
    assert_eq!(
        keys(&v["paths"]["/r"]["get"]["responses"]["200"]["headers"]),
        ["Set-Cookie", "Authorization", "X-Api-Key", "X-Plain"]
    );
    let tail = "holds a literal credential, written into the document as a header example";
    assert_eq!(
        openapi_notes(&c),
        pairs(&[
            (
                "r / examples",
                &format!("header Set-Cookie of example ok {tail}")
            ),
            (
                "r / examples",
                &format!("header Authorization of example ok {tail}")
            ),
        ])
    );
}
