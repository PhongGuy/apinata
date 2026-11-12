//! `export::postman`: shape, item order, requests, URLs and notes.

use crate::export_support;

use apinata_core::export::postman;
use export_support::*;
use serde_json::{Value, json};

const SCHEMA: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";

#[test]
fn an_empty_collection_is_info_item_and_variable_two_space_indented() {
    let (_d, c) = open(&[("opencollection.yml", ROOT)]);
    let out = postman(&c);
    assert!(
        out.json
            .starts_with("{\n  \"info\": {\n    \"name\": \"c\",\n"),
        "{}",
        out.json
    );
    assert!(out.json.ends_with("}\n"));
    let v: Value = serde_json::from_str(&out.json).unwrap();
    assert_eq!(keys(&v), ["info", "item", "variable"]);
    assert_eq!(
        v,
        json!({"info": {"name": "c", "schema": SCHEMA}, "item": [], "variable": []})
    );
    assert!(out.notes.is_empty());
}

#[test]
fn collection_and_folder_docs_become_descriptions() {
    let root = format!("{ROOT}docs:\n  content: About c\n  type: text/markdown\n");
    let folder = "info:\n  name: f\n  type: folder\n  seq: 1\ndocs:\n  content: About f\n  type: text/markdown\n";
    let (_d, c) = open(&[("opencollection.yml", &root), ("f/folder.yml", folder)]);
    let v = exported(&c);
    assert_eq!(keys(&v["info"]), ["name", "description", "schema"]);
    assert_eq!(v["info"]["description"], "About c");
    assert_eq!(
        v["item"][0],
        json!({"name": "f", "description": "About f", "item": []})
    );
    assert_eq!(keys(&v["item"][0]), ["name", "description", "item"]);
}

#[test]
fn folders_come_first_then_requests_by_seq_and_opaque_items_are_left_out() {
    let req = |name: &str, seq: u32| {
        format!(
            "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: http://x.test/{name}\n  auth: inherit\n"
        )
    };
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("late", 2)),
        ("b.yml", &req("early", 1)),
        ("g.yml", "info:\n  name: g\n  type: grpc\n  seq: 3\n"),
        (
            "f/folder.yml",
            "info:\n  name: f\n  type: folder\n  seq: 1\n",
        ),
        ("f/r.yml", &req("inner", 1)),
    ]);
    let v = exported(&c);
    assert_eq!(names(&v), ["f", "early", "late"]);
    assert_eq!(v["item"][0]["item"][0]["name"], "inner");
    assert_eq!(
        notes(&c),
        pairs(&[(
            "g",
            "only HTTP requests are exported; this gRPC item was left out"
        )])
    );
}

#[test]
fn folders_sort_by_display_name_not_directory_name() {
    // Directory order is a, b, c; display names sort Alpha, Mid, Zeta; `seq` 2 puts Mid second.
    let folder = |name: &str, seq: &str| format!("info:\n  name: {name}\n  type: folder\n{seq}");
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        ("a/folder.yml", &folder("Zeta", "")),
        ("b/folder.yml", &folder("Alpha", "")),
        ("c/folder.yml", &folder("Mid", "  seq: 2\n")),
    ]);
    assert_eq!(names(&exported(&c)), ["Alpha", "Mid", "Zeta"]);
}

#[test]
fn a_request_carries_method_headers_docs_and_url_in_order() {
    let under = format!(
        "  headers:\n    - name: Content-Type\n      value: application/json\n    - name: X-Off\n      value: \"0\"\n      disabled: true\n  params:\n    - name: page\n      value: \"2\"\n      type: query\n    - name: debug\n      value: \"1\"\n      type: query\n      disabled: true\n{INHERIT}"
    );
    let file = http(
        "r",
        "POST",
        "https://api.example.com/v1/users?page=2",
        &under,
    ) + "docs: Creates a user.\n";
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("r.yml", &file)]);
    let v = exported(&c);
    let item = &v["item"][0];
    assert_eq!(keys(item), ["name", "protocolProfileBehavior", "request"]);
    assert_eq!(
        keys(&item["request"]),
        ["method", "header", "description", "url"]
    );
    assert_eq!(
        keys(&item["request"]["url"]),
        ["raw", "protocol", "host", "path", "query", "variable"]
    );
    assert_eq!(
        keys(&item["request"]["header"][0]),
        ["key", "value", "description", "disabled", "type"]
    );
    assert_eq!(
        item["request"],
        json!({
            "method": "POST",
            "header": [
                {"key": "Content-Type", "value": "application/json", "description": "", "disabled": false, "type": "default"},
                {"key": "X-Off", "value": "0", "description": "", "disabled": true, "type": "default"}
            ],
            "description": "Creates a user.",
            "url": {"raw": "https://api.example.com/v1/users?page=2", "protocol": "https",
                "host": ["api", "example", "com"], "path": ["v1", "users"],
                "query": [{"key": "page", "value": "2"}, {"key": "debug", "value": "1", "disabled": true}],
                "variable": []}
        })
    );
}

#[test]
fn urls_keep_raw_verbatim_and_split_host_and_path() {
    let path = format!(
        "  params:\n    - name: id\n      value: \"7\"\n      type: path\n      description: the user\n{INHERIT}"
    );
    let query =
        format!("  params:\n    - name: q\n      value: \"1\"\n      type: query\n{INHERIT}");
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &http("a", "GET", "{{host}}/users/:id", &path)),
        (
            "b.yml",
            &http("b", "GET", "https://x.test:8080//a\\\\b?q=1", &query),
        ),
        ("c.yml", &http("c", "GET", "{{host}}/search?q=1", INHERIT)),
    ]);
    let v = exported(&c);
    let url = |i: usize| v["item"][i]["request"]["url"].clone();
    assert_eq!(
        url(0),
        json!({"raw": "{{host}}/users/:id", "protocol": "", "host": ["{{host}}"], "path": ["users", ":id"], "query": [], "variable": [{"key": "id", "value": "7", "description": "the user"}]})
    );
    assert_eq!(
        url(1),
        json!({"raw": "https://x.test:8080//a\\b?q=1", "protocol": "https", "host": ["x", "test:8080"], "path": ["", "a\\b"], "query": [{"key": "q", "value": "1"}], "variable": []})
    );
    assert_eq!(
        url(2),
        json!({"raw": "{{host}}/search?q=1", "protocol": "", "host": ["{{host}}"], "path": ["search"], "query": [{"key": "q", "value": "1"}], "variable": []})
    );
}

#[test]
fn what_has_no_place_in_the_file_is_noted_in_tree_order() {
    let root = format!("{ROOT}request:\n  headers:\n    - name: X-C\n      value: \"1\"\n");
    let folder = "info:\n  name: f\n  type: folder\n  seq: 1\nrequest:\n  headers:\n    - name: X-F\n      value: \"1\"\n  variables:\n    - name: v\n      value: \"1\"\n";
    let runtime = "runtime:\n  variables:\n    - name: rv\n      value: \"2\"\n  actions:\n    - type: set-variable\n      phase: after-response\n      selector:\n        expression: res.body.id\n        method: jsonq\n      variable:\n        name: id\n        scope: runtime\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n";
    let request = http("r", "GET", "http://x.test/r", INHERIT) + runtime;
    let (_d, c) = open(&[
        ("opencollection.yml", &root),
        ("f/folder.yml", folder),
        ("f/r.yml", &request),
        ("f/g.yml", "info:\n  name: g\n  type: websocket\n  seq: 2\n"),
    ]);
    let copied = "headers were copied into 1 request, because Postman has no collection or folder headers; importing the file back gives that request its own copy";
    let vars = "variables set here have no place in the exported file";
    assert_eq!(
        notes(&c),
        pairs(&[
            ("collection / headers", copied),
            ("f / headers", copied),
            ("f / variables", vars),
            ("f / r / variables", vars),
            (
                "f / r / post-response variables",
                "post-response variables have no place in the exported file"
            ),
            (
                "f / r / assertions",
                "assertions have no place in the exported file"
            ),
            (
                "f / g",
                "only HTTP requests are exported; this WebSocket item was left out"
            ),
        ])
    );
}
