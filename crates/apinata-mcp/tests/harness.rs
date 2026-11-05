//! The in-process harness itself: handshake, tool listing, and the two failure shapes.

use crate::common;

use common::{ROOT, TestServer};
use serde_json::json;

#[tokio::test]
async fn lists_get_collection_as_a_read_only_tool_with_an_output_schema() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let tools = server.tools().await;
    let tool = tools
        .iter()
        .find(|t| t.name == "get_collection")
        .expect("listed");
    let annotations = tool.annotations.as_ref().expect("annotations");
    assert_eq!(annotations.read_only_hint, Some(true));
    assert!(tool.output_schema.is_some());
}

#[tokio::test]
async fn get_collection_answers_over_the_wire() {
    let server = TestServer::start_with(&[("opencollection.yml", ROOT)], |o| o.env("dev")).await;
    let info = server.ok("get_collection", json!({})).await;
    assert_eq!(info["name"], "shop");
    assert_eq!(info["default_environment"], "dev");
    assert_eq!(info["environments"], json!([]));
}

#[tokio::test]
async fn an_unknown_tool_is_a_protocol_error_not_a_tool_result() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let outcome = server.try_call("no_such_tool", json!({})).await;
    assert!(outcome.is_err(), "{outcome:?}");
}

#[tokio::test]
async fn the_server_lists_the_read_and_write_tools_of_the_spec() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let mut names: Vec<String> = server
        .tools()
        .await
        .iter()
        .map(|t| t.name.to_string())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "check_collection",
            "clear_cookies",
            "clear_runtime_variables",
            "clone_folder",
            "clone_request",
            "create_environment",
            "create_folder",
            "create_request",
            "delete_environment",
            "delete_item",
            "duplicate_environment",
            "export_har",
            "export_openapi",
            "export_postman",
            "generate_code",
            "get_collection",
            "git_commit",
            "git_diff",
            "git_status",
            "graphql_schema",
            "import_collection",
            "import_request",
            "list_cookies",
            "list_items",
            "list_runtime_variables",
            "list_trusted_hosts",
            "mock_hits",
            "mock_start",
            "mock_stop",
            "move_item",
            "oauth2_token",
            "read_environment",
            "read_folder",
            "read_request",
            "rename_environment",
            "rename_item",
            "replace",
            "restore_item",
            "restore_pruned",
            "run_folder",
            "save_example",
            "scrub_example_userinfo",
            "search",
            "send_request",
            "set_runtime_variable",
            "update_environment",
            "update_folder",
            "update_request",
        ]
    );
}

#[tokio::test]
async fn send_and_run_are_open_world_and_the_listings_read_only() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let tools = server.tools().await;
    let hints = |name: &str| {
        let tool = tools.iter().find(|t| t.name == name).expect("listed");
        let a = tool.annotations.as_ref().expect("annotations");
        (a.read_only_hint, a.open_world_hint)
    };
    assert_eq!(hints("send_request"), (Some(false), Some(true)));
    assert_eq!(hints("run_folder"), (Some(false), Some(true)));
    assert_eq!(hints("list_trusted_hosts").0, Some(true));
    assert_eq!(hints("list_cookies").0, Some(true));
    assert_eq!(hints("mock_hits").0, Some(true));
    assert_eq!(hints("generate_code"), (Some(true), Some(false)));
    assert_eq!(hints("graphql_schema"), (Some(false), Some(true)));
    assert_eq!(hints("clear_cookies"), (None, Some(false)));
}
