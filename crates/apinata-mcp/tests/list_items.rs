//! `list_items`: the tree as nested items, in sibling order, with paths other tools accept.

use crate::common;

use common::{ROOT, TestServer, request};
use serde_json::{Value, json};

const FOLDER: &str = "info:\n  name: Users\n  type: folder\n  seq: 1\n";
const GRPC: &str = "info:\n  name: Query\n  type: grpc\n  seq: 3\n";

async fn shop() -> TestServer {
    TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", FOLDER),
        (
            "users/get.yml",
            &request("Get user", 2, "GET", "{{base}}/users/1"),
        ),
        (
            "users/create.yml",
            &request("Create user", 1, "POST", "{{base}}/users"),
        ),
        (
            "health.yml",
            &request("Health", 2, "GET", "http://localhost/health"),
        ),
        ("query.yml", GRPC),
    ])
    .await
}

fn paths(items: &Value) -> Vec<String> {
    items
        .as_array()
        .expect("items array")
        .iter()
        .map(|i| i["path"].as_str().expect("path").to_owned())
        .collect()
}

#[tokio::test]
async fn lists_the_root_with_folders_nested_in_sibling_order() {
    let server = shop().await;
    let list = server.ok("list_items", json!({})).await;
    assert_eq!(paths(&list["items"]), ["users", "health.yml", "query.yml"]);
    let users = &list["items"][0];
    assert_eq!(users["kind"], "folder");
    assert_eq!(users["name"], "Users");
    assert_eq!(
        paths(&users["items"]),
        ["users/create.yml", "users/get.yml"]
    );
    assert_eq!(
        users["items"][1],
        json!({"path": "users/get.yml", "kind": "request", "name": "Get user", "seq": 2,
               "method": "GET", "url": "{{base}}/users/1"})
    );
    assert_eq!(
        list["items"][2],
        json!({"path": "query.yml", "kind": "other", "name": "Query", "seq": 3})
    );
}

#[tokio::test]
async fn lists_one_folder_when_given_its_path() {
    let server = shop().await;
    let list = server.ok("list_items", json!({"path": "users"})).await;
    assert_eq!(paths(&list["items"]), ["users/create.yml", "users/get.yml"]);
}

#[tokio::test]
async fn sees_a_request_written_after_the_server_started() {
    let server = shop().await;
    server.write(
        "users/delete.yml",
        &request("Delete user", 3, "DELETE", "{{base}}/users/1"),
    );
    let list = server.ok("list_items", json!({"path": "users"})).await;
    assert_eq!(
        paths(&list["items"]),
        ["users/create.yml", "users/get.yml", "users/delete.yml"]
    );
}

#[tokio::test]
async fn unknown_and_escaping_paths_are_tool_errors_naming_the_path() {
    let server = shop().await;
    let missing = server.err("list_items", json!({"path": "orders"})).await;
    assert!(missing.contains("orders"), "{missing}");
    let escaping = server
        .err("list_items", json!({"path": "../outside"}))
        .await;
    assert!(escaping.contains("../outside"), "{escaping}");
    // A request is not a folder.
    let file = server
        .err("list_items", json!({"path": "health.yml"}))
        .await;
    assert!(file.contains("health.yml"), "{file}");
}

#[tokio::test]
async fn a_folder_display_name_resolves_and_an_ambiguous_one_lists_its_paths() {
    let folder = |name: &str| format!("info:\n  name: {name}\n  type: folder\n");
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("Posts/folder.yml", &folder("Blog Posts")),
        (
            "Posts/get.yml",
            &request("Get", 1, "GET", "http://localhost/p"),
        ),
        ("a/folder.yml", &folder("Twin")),
        ("b/folder.yml", &folder("Twin")),
    ])
    .await;
    let listed = server.ok("list_items", json!({"path": "Blog Posts"})).await;
    assert_eq!(paths(&listed["items"]), ["Posts/get.yml"]);
    let ambiguous = server.err("list_items", json!({"path": "Twin"})).await;
    assert!(ambiguous.contains("a, b"), "{ambiguous}");
}

#[tokio::test]
async fn a_file_that_fails_to_load_is_listed_as_broken_with_its_error() {
    let conflict =
        "info:\n  name: a\n<<<<<<< HEAD\n  type: http\n=======\n  type: ws\n>>>>>>> other\n";
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        (
            "health.yml",
            &request("Health", 1, "GET", "http://localhost/health"),
        ),
        ("bad.yml", conflict),
    ])
    .await;
    let listed = server.ok("list_items", json!({})).await;
    assert_eq!(paths(&listed["items"]), ["health.yml", "bad.yml"]);
    let broken = &listed["items"][1];
    assert_eq!(broken["kind"], "broken");
    assert_eq!(broken["name"], "bad");
    assert_eq!(
        broken["error"],
        "merge conflict markers at line 3; resolve them in git"
    );
    assert!(listed["items"][0].get("error").is_none());
}

#[tokio::test]
async fn a_broken_folder_file_puts_its_error_on_the_folder_item() {
    let conflict =
        "info:\n  name: f\n<<<<<<< HEAD\n  type: folder\n=======\n  x: 2\n>>>>>>> other\n";
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", conflict),
        ("f/get.yml", &request("Get", 1, "GET", "http://localhost/g")),
    ])
    .await;
    let listed = server.ok("list_items", json!({})).await;
    assert_eq!(listed["items"][0]["kind"], "folder");
    assert_eq!(
        listed["items"][0]["error"],
        "merge conflict markers at line 3; resolve them in git"
    );
    assert_eq!(paths(&listed["items"][0]["items"]), ["f/get.yml"]);
}

#[tokio::test]
async fn without_paging_arguments_the_whole_reply_is_the_legacy_shape() {
    let server = shop().await;
    let list = server.ok("list_items", json!({})).await;
    assert_eq!(
        list,
        json!({"items": [
            {"path": "users", "kind": "folder", "name": "Users", "seq": 1, "items": [
                {"path": "users/create.yml", "kind": "request", "name": "Create user",
                 "seq": 1, "method": "POST", "url": "{{base}}/users"},
                {"path": "users/get.yml", "kind": "request", "name": "Get user",
                 "seq": 2, "method": "GET", "url": "{{base}}/users/1"}
            ]},
            {"path": "health.yml", "kind": "request", "name": "Health", "seq": 2,
             "method": "GET", "url": "http://localhost/health"},
            {"path": "query.yml", "kind": "other", "name": "Query", "seq": 3}
        ]})
    );
}

#[tokio::test]
async fn depth_one_lists_direct_children_with_a_child_count_on_folders() {
    let server = shop().await;
    let list = server.ok("list_items", json!({"depth": 1})).await;
    assert_eq!(paths(&list["items"]), ["users", "health.yml", "query.yml"]);
    let users = &list["items"][0];
    assert_eq!(users["child_count"], 2);
    assert!(users.get("items").is_none(), "{users}");
    assert!(list["items"][1].get("child_count").is_none());
    assert_eq!(list["total"], 3);
    assert_eq!(list["has_more"], false);
}

#[tokio::test]
async fn limit_and_offset_page_the_direct_items_and_report_the_effective_limit() {
    let server = shop().await;
    let first = server.ok("list_items", json!({"limit": 2})).await;
    assert_eq!(paths(&first["items"]), ["users", "health.yml"]);
    assert_eq!(
        (&first["total"], &first["has_more"]),
        (&json!(3), &json!(true))
    );
    assert_eq!(first["limit"], 2);
    let rest = server
        .ok("list_items", json!({"limit": 2, "offset": 2}))
        .await;
    assert_eq!(paths(&rest["items"]), ["query.yml"]);
    assert_eq!(rest["has_more"], false);
    let clamped = server.ok("list_items", json!({"limit": 99999})).await;
    assert_eq!(clamped["limit"], 500);
    let past = server.ok("list_items", json!({"offset": 40})).await;
    assert_eq!(past["items"], json!([]));
    assert_eq!(
        (&past["total"], &past["has_more"]),
        (&json!(3), &json!(false))
    );
}

#[tokio::test]
async fn a_zero_limit_is_a_tool_error_and_a_negative_one_names_the_field() {
    let server = shop().await;
    let zero = server.err("list_items", json!({"limit": 0})).await;
    assert!(zero.contains("limit"), "{zero}");
    for field in ["limit", "offset", "depth"] {
        let err = server.err("list_items", json!({field: -1})).await;
        assert!(err.contains(&format!("`{field}`: invalid value")), "{err}");
        assert!(!err.contains("-1"), "{err}");
    }
}

#[tokio::test]
async fn the_tool_description_tells_an_agent_to_page() {
    let server = shop().await;
    let tools = server.tools().await;
    let tool = tools.iter().find(|t| t.name == "list_items").unwrap();
    let text = tool.description.as_deref().unwrap();
    for word in ["depth", "limit", "offset", "has_more", "child_count"] {
        assert!(text.contains(word), "{word}: {text}");
    }
}
