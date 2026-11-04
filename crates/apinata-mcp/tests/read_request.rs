//! `read_request`: a request file as a `RequestDoc`, and the errors that name the path.

use crate::common;

use common::{ROOT, TestServer};
use serde_json::json;

const GET_USER: &str = r#"info:
  name: Get user
  type: http
  seq: 2
  tags:
    - smoke
http:
  method: GET
  url: "{{base}}/users/:id?verbose=1"
  headers:
    - name: Accept
      value: application/json
    - name: X-Debug
      value: "1"
      disabled: true
  params:
    - name: verbose
      value: "1"
      type: query
    - name: id
      value: "7"
      type: path
  body:
    type: json
    data: '{"a": 1}'
  auth:
    type: bearer
    token: "{{token}}"
runtime:
  variables:
    - name: userId
      value: "7"
    - name: limit
      value:
        type: number
        data: "10"
  assertions:
    - expression: res.status
      operator: eq
      value: "200"
settings:
  encodeUrl: true
  timeout: 5000
docs: Fetches one user.
"#;

const UPLOAD: &str = r#"info:
  name: Upload
  type: http
http:
  method: POST
  url: http://localhost/upload
  body:
    type: multipart-form
    data:
      - name: note
        type: text
        value: hello
      - name: files
        type: file
        value:
          - ./a.png
        contentType: image/png
        disabled: true
  auth:
    type: apikey
    key: X-Key
    value: "{{apiKey}}"
    placement: query
"#;

const FORM: &str = r#"info:
  name: Form
  type: http
http:
  method: POST
  url: http://localhost/form
  body:
    type: form-urlencoded
    data:
      - name: q
        value: shoes
  auth: inherit
settings:
  timeout: inherit
"#;

async fn shop() -> TestServer {
    TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", "info:\n  name: Users\n  type: folder\n"),
        ("users/get.yml", GET_USER),
        ("upload.yml", UPLOAD),
        ("form.yml", FORM),
        (
            "query.yml",
            "info:\n  name: Query\n  type: grpc\n  seq: 3\n",
        ),
    ])
    .await
}

#[tokio::test]
async fn read_request_returns_every_section() {
    let server = shop().await;
    let doc = server
        .ok("read_request", json!({"path": "users/get.yml"}))
        .await;
    assert_eq!(doc["name"], "Get user");
    assert_eq!(doc["method"], "GET");
    assert_eq!(doc["url"], "{{base}}/users/:id?verbose=1");
    assert_eq!(doc["seq"], 2);
    assert_eq!(doc["tags"], json!(["smoke"]));
    assert_eq!(doc["docs"], "Fetches one user.");
    assert_eq!(
        doc["headers"],
        json!([
            {"name": "Accept", "value": "application/json", "enabled": true},
            {"name": "X-Debug", "value": "1", "enabled": false}
        ])
    );
    assert_eq!(
        doc["params"],
        json!([
            {"name": "verbose", "value": "1", "kind": "query", "enabled": true},
            {"name": "id", "value": "7", "kind": "path", "enabled": true}
        ])
    );
    assert_eq!(doc["body"], json!({"type": "json", "text": "{\"a\": 1}"}));
    assert_eq!(doc["auth"], json!({"type": "bearer", "token": "{{token}}"}));
    assert_eq!(
        doc["vars"],
        json!([
            {"name": "userId", "value": "7", "enabled": true},
            {"name": "limit", "value": "10", "type": "number", "enabled": true}
        ])
    );
    assert_eq!(
        doc["assertions"],
        json!([{"expression": "res.status", "operator": "eq", "value": "200", "enabled": true}])
    );
    assert_eq!(doc["settings"]["encode_url"], true);
    assert_eq!(
        doc["settings"]["timeout"],
        json!({"type": "millis", "ms": 5000})
    );
}

#[tokio::test]
async fn read_request_refuses_folders_other_items_and_unknown_paths_by_name() {
    let server = shop().await;
    let other = server
        .err("read_request", json!({"path": "query.yml"}))
        .await;
    assert!(
        other.contains("query.yml") && other.contains("grpc"),
        "{other}"
    );
    let missing = server
        .err("read_request", json!({"path": "users/nope.yml"}))
        .await;
    assert!(missing.contains("users/nope.yml"), "{missing}");
    let folder = server.err("read_request", json!({"path": "users"})).await;
    assert!(folder.contains("users"), "{folder}");
    let escaping = server
        .err("read_request", json!({"path": "/etc/passwd"}))
        .await;
    assert!(escaping.contains("/etc/passwd"), "{escaping}");
}

#[tokio::test]
async fn read_request_without_a_path_is_a_tool_error_naming_the_field() {
    // rmcp 3.5.1 turns argument deserialisation failures into isError results.
    let server = shop().await;
    let missing = server.err("read_request", json!({})).await;
    assert!(missing.contains("`path`"), "{missing}");
}

#[tokio::test]
async fn read_request_maps_multipart_form_and_api_key_shapes() {
    let server = shop().await;
    let upload = server
        .ok("read_request", json!({"path": "upload.yml"}))
        .await;
    assert_eq!(
        upload["body"],
        json!({"type": "multipart", "fields": [
            {"name": "note", "value": {"type": "text", "text": "hello"},
             "content_type": "", "enabled": true},
            {"name": "files", "value": {"type": "files", "paths": ["./a.png"]},
             "content_type": "image/png", "enabled": false}
        ]})
    );
    assert_eq!(
        upload["auth"],
        json!({"type": "apikey", "key": "X-Key", "value": "{{apiKey}}", "placement": "query"})
    );
    assert_eq!(upload["seq"], json!(null));
    let form = server.ok("read_request", json!({"path": "form.yml"})).await;
    assert_eq!(
        form["body"],
        json!({"type": "form_urlencoded", "fields": [
            {"name": "q", "value": "shoes", "enabled": true}
        ]})
    );
    assert_eq!(form["auth"], json!({"type": "inherit"}));
    assert_eq!(form["settings"]["timeout"], json!({"type": "inherit"}));
}

#[tokio::test]
async fn a_url_query_without_a_params_list_reads_as_params_like_the_app() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        (
            "q.yml",
            "info:\n  name: q\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://x/u/:id?a=1&b\"\n",
        ),
    ])
    .await;
    let doc = server.ok("read_request", json!({"path": "q.yml"})).await;
    assert_eq!(
        doc["params"],
        json!([
            {"name": "a", "value": "1", "kind": "query", "enabled": true},
            {"name": "b", "value": "", "kind": "query", "enabled": true},
            {"name": "id", "value": "", "kind": "path", "enabled": true}
        ])
    );
}

/// A request whose URL sends `z=6` while its params block holds `z: 3`.
const MISMATCHED: &str = "info:\n  name: m\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://localhost/m?z=6\"\n  params:\n    - name: z\n      value: \"3\"\n      type: query\n";

#[tokio::test]
async fn a_url_query_that_disagrees_with_params_reads_as_a_query_mismatch() {
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("m.yml", MISMATCHED)]).await;
    let doc = server.ok("read_request", json!({"path": "m.yml"})).await;
    assert_eq!(
        doc["query_mismatch"],
        json!({"names": ["z"], "note": "the URL's query is sent"})
    );
}

#[tokio::test]
async fn an_agreeing_request_or_one_without_params_has_no_query_mismatch() {
    let agreeing = MISMATCHED.replace("?z=6", "?z=3");
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("agree.yml", &agreeing),
        (
            "bare.yml",
            &common::request("b", 2, "GET", "http://localhost/b?z=6"),
        ),
    ])
    .await;
    for path in ["agree.yml", "bare.yml"] {
        let doc = server.ok("read_request", json!({"path": path})).await;
        assert!(doc.get("query_mismatch").is_none(), "{path}: {doc}");
    }
}

#[tokio::test]
async fn read_requests_output_schema_names_query_mismatch() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let tools = server.tools().await;
    let tool = tools
        .iter()
        .find(|t| t.name == "read_request")
        .expect("read_request");
    let schema = serde_json::to_string(&tool.output_schema).expect("schema");
    assert!(schema.contains("query_mismatch"), "{schema}");
    assert!(
        tool.description
            .as_deref()
            .unwrap_or_default()
            .contains("query_mismatch")
    );
}
