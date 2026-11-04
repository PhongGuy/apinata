//! `export::openapi`: document shape, walk order, left-out requests, duplicates, operation ids.

use crate::export_support;

use apinata_core::export::openapi;
use export_support::*;
use serde_json::{Value, json};

#[test]
fn an_empty_collection_is_a_bare_3_1_document() {
    let (_d, c) = open(&[("opencollection.yml", ROOT)]);
    let out = openapi(&c);
    assert!(
        out.json.starts_with("{\n  \"openapi\": \"3.1.0\",\n"),
        "{}",
        out.json
    );
    assert!(out.json.ends_with("}\n") && out.notes.is_empty());
    let v = openapi_doc(&c);
    assert_eq!(keys(&v), ["openapi", "info", "paths"]);
    assert_eq!(
        v,
        json!({"openapi": "3.1.0", "info": {"title": "c", "version": "1.0.0"}, "paths": {}})
    );
    let root = format!("{ROOT}docs:\n  content: About c\n  type: text/markdown\n");
    let (_d, c) = open(&[("opencollection.yml", &root)]);
    let v = openapi_doc(&c);
    assert_eq!(
        v["info"],
        json!({"title": "c", "version": "1.0.0", "description": "About c"})
    );
    assert_eq!(keys(&v["info"]), ["title", "version", "description"]);
}

fn default_response() -> Value {
    json!({"default": {"description": "Default response"}})
}

fn folder(name: &str, rest: &str) -> String {
    format!("info:\n  name: {name}\n  type: folder\n  seq: 1\n{rest}")
}

fn get(name: &str, seq: u32, url: &str) -> String {
    http_at(name, seq, "GET", url, INHERIT)
}

#[test]
fn requests_become_operations_tagged_with_their_top_level_folder() {
    let docs = "docs:\n  content: People\n  type: text/markdown\n";
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("Users", docs)),
        (
            "users/get.yml",
            &(get("Get user", 1, "https://x.test/users") + "docs: Fetch one.\n"),
        ),
        ("users/admin/folder.yml", &folder("Admin", "")),
        (
            "users/admin/list.yml",
            &http("List", "get", "https://x.test/admins/:id", INHERIT),
        ),
        ("empty/folder.yml", &folder("Empty", "")),
        ("ping.yml", &http("ping", "HEAD", "https://x.test", INHERIT)),
        ("g.yml", "info:\n  name: g\n  type: grpc\n  seq: 3\n"),
        ("odd.yml", &get("odd", 2, "/relative")),
        (
            "purge.yml",
            &http_at("purge", 3, "PURGE", "https://x.test/p", INHERIT),
        ),
    ]);
    let v = openapi_doc(&c);
    let op = &v["paths"]["/users"]["get"];
    assert_eq!(
        keys(op),
        ["tags", "summary", "description", "operationId", "responses"]
    );
    assert_eq!(
        *op,
        json!({"tags": ["Users"], "summary": "Get user", "description": "Fetch one.",
        "operationId": "usersGetUser", "responses": default_response()})
    );
    assert_eq!(
        v["paths"]["/admins/{id}"]["get"]["operationId"],
        "usersAdminList"
    );
    assert_eq!(v["paths"]["/admins/{id}"]["get"]["tags"], json!(["Users"]));
    assert_eq!(
        v["paths"]["/"]["head"],
        json!({"summary": "ping", "operationId": "ping", "responses": default_response()})
    );
    // Sub-folders come before requests, so the nested folder's path is seen first.
    assert_eq!(keys(&v["paths"]), ["/admins/{id}", "/users", "/"]);
    assert_eq!(
        v["tags"],
        json!([{"name": "Users", "description": "People"}])
    );
    assert_eq!(
        openapi_notes(&c),
        pairs(&[
            (
                "odd / url",
                "the URL has no origin OpenAPI can express; the request was left out"
            ),
            (
                "purge / method",
                "OpenAPI 3.1 has no field for the PURGE method; the request was left out"
            ),
            (
                "g",
                "only HTTP requests are exported; this grpc item was left out"
            ),
        ])
    );
}

#[test]
fn a_later_request_with_the_same_method_and_path_template_is_dropped() {
    let files = vec![
        ("a.yml".to_owned(), get("first", 1, "{{base}}/users/:id")),
        (
            "b.yml".to_owned(),
            get("second", 2, "https://o.test/users/{{userId}}"),
        ),
        (
            "c.yml".to_owned(),
            http_at("third", 3, "POST", "{{base}}/users/:id", INHERIT),
        ),
        ("d.yml".to_owned(), get("fourth", 4, "{{base}}/users/:id/")),
    ];
    let (_d, c) = open_with_root(ROOT, &files);
    let v = openapi_doc(&c);
    assert_eq!(
        operations(&v),
        pairs(&[
            ("/users/{id}", "get"),
            ("/users/{id}", "post"),
            ("/users/{id}/", "get")
        ])
    );
    assert_eq!(v["paths"]["/users/{id}"]["get"]["summary"], "first");
    assert_eq!(
        openapi_notes(&c),
        pairs(&[(
            "second",
            "GET /users/{userId} has the same method and path as first; this request was left out"
        )])
    );
}

#[test]
fn operation_ids_are_unique_lower_camel_case_slugs() {
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        ("a/folder.yml", &folder("My API", "")),
        ("a/r.yml", &get("get-user v2", 1, "https://x.test/1")),
        ("a/s.yml", &get("b", 2, "https://x.test/2")),
        ("c.yml", &get("_", 1, "https://x.test/3")),
        ("d.yml", &get("__", 2, "https://x.test/4")),
        ("e.yml", &get("my api b", 3, "https://x.test/5")),
        ("f.yml", &get("operation 2", 4, "https://x.test/6")),
    ]);
    let v = openapi_doc(&c);
    let ids: Vec<&str> = (1..=6)
        .map(|i| {
            v["paths"][format!("/{i}")]["get"]["operationId"]
                .as_str()
                .unwrap()
        })
        .collect();
    assert_eq!(
        ids,
        [
            "myApiGetUserV2",
            "myApiB",
            "operation",
            "operation2",
            "myApiB2",
            "operation22"
        ]
    );
}

#[test]
fn equivalent_templated_paths_share_the_first_seen_key() {
    let params = |name: &str, value: &str| {
        format!(
            "  params:\n    - name: {name}\n      value: \"{value}\"\n      type: path\n{INHERIT}"
        )
    };
    let files = vec![
        (
            "a.yml".to_owned(),
            http_at("first", 1, "GET", "{{base}}/users/:id", &params("id", "7")),
        ),
        (
            "b.yml".to_owned(),
            http_at(
                "second",
                2,
                "POST",
                "{{base}}/users/:userId",
                &params("userId", "42"),
            ),
        ),
        (
            "c.yml".to_owned(),
            get("third", 3, "{{base}}/users/{{uid}}"),
        ),
    ];
    let (_d, c) = open_with_root(ROOT, &files);
    let v = openapi_doc(&c);
    assert_eq!(
        operations(&v),
        pairs(&[("/users/{id}", "get"), ("/users/{id}", "post")])
    );
    assert_eq!(
        v["paths"]["/users/{id}"]["post"]["parameters"],
        json!([{"name": "id", "in": "path", "required": true,
            "schema": {"type": "string"}, "example": "42"}])
    );
    assert_eq!(
        openapi_notes(&c),
        pairs(&[(
            "third",
            "GET /users/{uid} has the same method and path as first; this request was left out"
        )])
    );
}
