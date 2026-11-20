//! Without a tool `environment` or a launch `--env`, tools use the collection's preset default
//! environment while it exists; a launch without environments ignores the preset.

use crate::common;

use common::TestServer;
use serde_json::json;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: shop\nextensions:\n  bruno:\n    presets:\n      defaultEnvironment: dev\n";
const REQUEST: &str = "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{{base}}\"\n";

fn env(name: &str, base: &str) -> String {
    format!("name: {name}\nvariables:\n  - name: base\n    value: \"{base}\"\n")
}

async fn mock() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path("/dev"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&server)
        .await;
    Mock::given(path("/prod"))
        .respond_with(ResponseTemplate::new(202))
        .mount(&server)
        .await;
    server
}

async fn start(
    mock: &MockServer,
    configure: impl FnOnce(apinata_mcp::McpOptions) -> apinata_mcp::McpOptions,
) -> TestServer {
    let dev = env("dev", &format!("{}/dev", mock.uri()));
    let prod = env("prod", &format!("{}/prod", mock.uri()));
    TestServer::start_with(
        &[
            ("opencollection.yml", ROOT),
            ("environments/dev.yml", &dev),
            ("environments/prod.yml", &prod),
            ("r.yml", REQUEST),
        ],
        configure,
    )
    .await
}

#[tokio::test]
async fn a_send_without_an_environment_uses_the_preset() {
    let mock = mock().await;
    let server = start(&mock, |o| o).await;
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    assert_eq!(result["status"], 201, "{result}");
    let info = server.ok("get_collection", json!({})).await;
    assert_eq!(info["default_environment"], "dev", "{info}");
}

#[tokio::test]
async fn the_tool_environment_wins_over_the_preset() {
    let mock = mock().await;
    let server = start(&mock, |o| o).await;
    let args = json!({"path": "r.yml", "environment": "prod"});
    let result = server.ok("send_request", args).await;
    assert_eq!(result["status"], 202, "{result}");
}

#[tokio::test]
async fn the_launch_env_wins_over_the_preset() {
    let mock = mock().await;
    let server = start(&mock, |o| o.env("prod")).await;
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    assert_eq!(result["status"], 202, "{result}");
}

#[tokio::test]
async fn a_launch_with_no_env_ignores_the_preset() {
    let mock = mock().await;
    let server = start(&mock, |o| o.no_env()).await;
    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(message.contains("base"), "{message}");
    let info = server.ok("get_collection", json!({})).await;
    assert!(info["default_environment"].is_null(), "{info}");
}
