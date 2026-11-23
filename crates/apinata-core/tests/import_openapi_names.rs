//! Request names, URLs and document edges.

use crate::openapi_support;

use apinata_core::import::{DocumentError, GroupBy, NoteKind};
use openapi_support::{error, import, import_bytes, notes, requests, tree};
use serde_json::json;

fn names(doc: &serde_json::Value) -> Vec<String> {
    requests(&import(doc).collection)
        .into_iter()
        .map(|(p, _)| p)
        .collect()
}

#[test]
fn names_come_from_summary_operation_id_or_description_and_are_normalized() {
    let doc = json!({"openapi": "3.0.0", "paths": {"/a": {
        "get": {"summary": "  Get\n  the   thing...  ", "operationId": "ignored"},
        "put": {"summary": "", "operationId": "putA"},
        "post": {"summary": " . ", "description": "ignored"},
        "patch": {"summary": 5, "operationId": "patchA"},
        "delete": {"description": "Removes\tit."},
        "head": {}
    }}});
    assert_eq!(
        names(&doc),
        [
            "Get the thing",
            "putA",
            "post /a",
            "patchA",
            "Removes it",
            "head /a"
        ]
    );
}

#[test]
fn duplicate_names_get_the_method_then_a_counter() {
    let doc = json!({"openapi": "3.0.0", "paths": {
        "/a": {"get": {"summary": "Same"}, "post": {"summary": "Same"}},
        "/b": {"get": {"summary": "Same"}, "post": {"summary": "Same"}}
    }});
    let c = import(&doc).collection;
    let got: Vec<String> = requests(&c).into_iter().map(|(p, _)| p).collect();
    assert_eq!(got, ["Same", "Same (POST)", "Same (GET)", "Same (1)"]);
    let renamed: Vec<&str> = notes(&c)
        .iter()
        .filter(|n| n.kind == NoteKind::Renamed)
        .map(|n| n.source.as_str())
        .collect();
    assert_eq!(renamed, ["paths./a.post", "paths./b.get", "paths./b.post"]);
}

#[test]
fn urls_join_base_url_and_path_with_single_slashes() {
    let doc = json!({"openapi": "3.1.0", "paths": {"//double//slash/": {"get": {"summary": "d"}}, "/": {"get": {"summary": "r"}}}});
    let urls: Vec<String> = requests(&import(&doc).collection)
        .iter()
        .map(|(_, r)| r.request.url.clone())
        .collect();
    assert_eq!(urls, ["{{baseUrl}}/double/slash/", "{{baseUrl}}/"]);
}

#[test]
fn vendor_extensions_add_no_requests() {
    let doc = json!({"openapi": "3.0.0", "paths": {"/a": {"get": {"summary": "g", "x-bruno-variants": [{"summary": "v"}]}, "x-extra": {"get": {}}}}});
    assert_eq!(names(&doc), ["g"]);
}

#[test]
fn a_document_without_paths_imports_empty() {
    let hooks = import(&json!({"openapi": "3.1.0", "info": {"title": "hooks"}, "webhooks": {"newPet": {"post": {}}}})).collection;
    assert!(hooks.items.is_empty());
    assert!(hooks.unwritten.iter().any(|n| n.kind == NoteKind::Dropped
        && n.source == "webhooks"
        && n.detail == "webhooks are not imported"));
    let untitled = import(&json!({"swagger": "2.0"})).collection;
    assert_eq!(
        (untitled.name.as_str(), untitled.items.len()),
        ("Untitled Collection", 0)
    );
}

#[test]
fn unsupported_versions_and_non_documents_fail() {
    assert_eq!(
        error(&json!({"openapi": "4.0.0", "paths": {}})),
        DocumentError::Unsupported {
            found: "openapi 4.0.0".into()
        }
    );
    assert!(matches!(
        error(&json!({"info": {"title": "x"}})),
        DocumentError::Invalid { .. }
    ));
    assert!(matches!(
        error(&json!({"openapi": "3.0.0", "paths": []})),
        DocumentError::Invalid { .. }
    ));
}

#[test]
fn yaml_documents_with_numeric_versions_and_merge_keys_import() {
    let yaml = b"openapi: 3.0\ninfo: {title: y}\npaths:\n  /a:\n    get: &op\n      summary: one\n    put:\n      <<: *op\n      summary: two\n";
    let bundles = import_bytes(yaml, GroupBy::Tags).expect("imports");
    let c = &bundles[0].collection;
    assert_eq!(
        tree(c),
        ["R 1 one GET {{baseUrl}}/a", "R 2 two PUT {{baseUrl}}/a"]
    );
    assert!(
        c.unwritten
            .iter()
            .any(|n| n.kind == NoteKind::Converted && n.source == "paths./a.put.<<")
    );
}

fn environment_names(doc: &serde_json::Value) -> Vec<String> {
    import(doc)
        .environments
        .into_iter()
        .map(|e| e.name)
        .collect()
}

#[test]
fn an_unnamed_server_names_its_environment_after_its_host() {
    let doc = json!({"openapi": "3.0.0", "paths": {}, "servers": [
        {"url": "http://127.0.0.1:18083/v1"},
        {"url": "https://api.example.test/v2", "description": "Live"},
        {"url": "https://{region}.example.test", "variables": {"region": {"default": "eu"}}},
        {"url": "/relative"}
    ]});
    assert_eq!(
        environment_names(&doc),
        ["127.0.0.1", "Live", "eu.example.test", "Environment 4"]
    );
}

#[test]
fn servers_sharing_a_host_add_their_port() {
    let doc = json!({"openapi": "3.0.0", "paths": {}, "servers": [
        {"url": "http://localhost:8080"},
        {"url": "http://user@localhost:9090/v1"},
        {"url": "https://other.test"}
    ]});
    assert_eq!(
        environment_names(&doc),
        ["localhost:8080", "localhost:9090", "other.test"]
    );
}

#[test]
fn swagger_environments_take_the_host_and_repeats_count_up() {
    let one = json!({"swagger": "2.0", "paths": {}, "host": "petstore.test:8080"});
    assert_eq!(environment_names(&one), ["petstore.test"]);
    let two =
        json!({"swagger": "2.0", "paths": {}, "host": "p.test", "schemes": ["http", "https"]});
    assert_eq!(environment_names(&two), ["p.test", "p.test (2)"]);
    let relative = json!({"swagger": "2.0", "paths": {}, "basePath": "/v2"});
    assert_eq!(environment_names(&relative), ["Environment"]);
}
