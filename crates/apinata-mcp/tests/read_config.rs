//! `read_folder` and `read_environment`: shared sections and variables, secrets without values.

use crate::common;

use common::{TestServer, request};
use serde_json::json;

const ROOT_WITH_DEFAULTS: &str = r#"opencollection: "1.0.0"
info:
  name: shop
request:
  headers:
    - name: X-Client
      value: apinata
  auth:
    type: basic
    username: admin
    password: "{{adminPassword}}"
  variables:
    - name: base
      value: http://localhost:3000
docs:
  content: Shop API.
  type: text/markdown
"#;

const USERS_FOLDER: &str = r#"info:
  name: Users
  type: folder
request:
  headers:
    - name: X-Area
      value: users
  auth: inherit
"#;

const DEV: &str = r#"name: dev
variables:
  - name: base
    value: http://localhost:3000
  - name: retries
    value:
      type: number
      data: "3"
  - name: old
    value: x
    disabled: true
  - secret: true
    name: token
  - secret: true
    name: stray
    value: leaked-from-file
"#;

async fn shop() -> TestServer {
    TestServer::start(&[
        ("opencollection.yml", ROOT_WITH_DEFAULTS),
        ("users/folder.yml", USERS_FOLDER),
        (
            "orders/list.yml",
            &request("List", 1, "GET", "{{base}}/orders"),
        ),
        ("environments/dev.yml", DEV),
    ])
    .await
}

#[tokio::test]
async fn read_folder_reads_the_collection_a_folder_and_a_folder_without_folder_yml() {
    let server = shop().await;
    let root = server.ok("read_folder", json!({"path": ""})).await;
    assert_eq!(
        root,
        json!({
            "headers": [{"name": "X-Client", "value": "apinata", "enabled": true}],
            "auth": {"type": "basic", "username": "admin", "password": "{{adminPassword}}"},
            "vars": [{"name": "base", "value": "http://localhost:3000", "enabled": true}],
            "docs": "Shop API."
        })
    );
    let users = server.ok("read_folder", json!({"path": "users"})).await;
    assert_eq!(
        users["headers"],
        json!([{"name": "X-Area", "value": "users", "enabled": true}])
    );
    assert_eq!(users["auth"], json!({"type": "inherit"}));
    let orders = server.ok("read_folder", json!({"path": "orders"})).await;
    assert_eq!(
        orders,
        json!({"headers": [], "auth": {"type": "none"}, "vars": [], "docs": ""})
    );
}

#[tokio::test]
async fn read_folder_reports_a_broken_folder_yml_and_unknown_folders() {
    let server = shop().await;
    server.write("users/folder.yml", "info: [unclosed\n");
    let broken = server.err("read_folder", json!({"path": "users"})).await;
    assert!(broken.contains("folder.yml"), "{broken}");
    let missing = server.err("read_folder", json!({"path": "billing"})).await;
    assert!(missing.contains("billing"), "{missing}");
}

#[tokio::test]
async fn read_environment_lists_variables_and_never_a_secret_value() {
    let server = shop().await;
    let result = server
        .call("read_environment", json!({"name": "dev"}))
        .await;
    // The whole result, text content included, must not carry the stray secret value.
    let wire = serde_json::to_string(&result).expect("serialises");
    assert!(!wire.contains("leaked-from-file"), "{wire}");
    let env = result.structured_content.expect("structured");
    assert_eq!(env["name"], "dev");
    assert_eq!(
        env["variables"],
        json!([
            {"name": "base", "value": "http://localhost:3000", "enabled": true, "secret": false},
            {"name": "retries", "value": "3", "type": "number", "enabled": true, "secret": false},
            {"name": "old", "value": "x", "enabled": false, "secret": false},
            {"name": "token", "enabled": true, "secret": true},
            {"name": "stray", "enabled": true, "secret": true}
        ])
    );
}

#[tokio::test]
async fn read_environment_names_an_unknown_environment() {
    let server = shop().await;
    let missing = server
        .err("read_environment", json!({"name": "prod"}))
        .await;
    assert!(missing.contains("prod"), "{missing}");
}

#[tokio::test]
async fn every_read_tool_is_listed_read_only_with_an_output_schema() {
    const READS: [&str; 14] = [
        "check_collection",
        "generate_code",
        "get_collection",
        "git_diff",
        "git_status",
        "list_cookies",
        "list_items",
        "list_runtime_variables",
        "list_trusted_hosts",
        "mock_hits",
        "read_environment",
        "read_folder",
        "read_request",
        "search",
    ];
    let server = shop().await;
    let tools = server.tools().await;
    for name in READS {
        let tool = tools
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("{name} listed"));
        let annotations = tool.annotations.as_ref().expect("annotations");
        assert_eq!(annotations.read_only_hint, Some(true), "{name}");
        assert!(tool.output_schema.is_some(), "{name}");
    }
    for tool in tools.iter().filter(|t| !READS.contains(&t.name.as_ref())) {
        let annotations = tool.annotations.as_ref().expect("annotations");
        assert_ne!(
            annotations.read_only_hint,
            Some(true),
            "{} writes",
            tool.name
        );
    }
}
