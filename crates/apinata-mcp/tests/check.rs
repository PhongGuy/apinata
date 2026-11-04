//! `check_collection`: read-only, same report as the core function and the CLI.

use crate::common;
use apinata_core::{CheckOptions, check_collection};
use common::{ROOT, TestServer, request};
use serde_json::json;

#[tokio::test]
async fn it_returns_the_core_report_for_the_loaded_collection() {
    let files = [
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1, "GET", "http://{{nope}}/")),
        ("b.yml", &request("b", 1, "GET", "http://x/ok")),
    ];
    let server = TestServer::start(&files).await;
    let got = server.ok("check_collection", json!({})).await;

    let report = check_collection(server.root(), &CheckOptions::default()).unwrap();
    assert_eq!(got, report.to_json_versioned());
    assert_eq!(got["summary"]["errors"], 1);
}

#[tokio::test]
async fn the_env_argument_narrows_the_check_to_that_environment() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: host\n    value: example.test\n",
        ),
        ("environments/prod.yml", "name: prod\n"),
        ("a.yml", &request("a", 1, "GET", "http://{{host}}/")),
    ])
    .await;
    let got = server.ok("check_collection", json!({"env": "dev"})).await;

    let opts = CheckOptions::default().env("dev");
    let report = check_collection(server.root(), &opts).unwrap();
    assert_eq!(got, report.to_json_versioned());
    assert_eq!(got["summary"]["errors"], 0);
    let all = server.ok("check_collection", json!({})).await;
    assert_ne!(got, all);
}

#[tokio::test]
async fn an_unknown_environment_is_a_tool_error() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let err = server.err("check_collection", json!({"env": "nope"})).await;
    assert!(err.contains("nope"), "{err}");
}

#[tokio::test]
async fn it_files_nothing_in_the_approval_inbox() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &request("a", 1, "GET", "http://far.example.invalid/{{x}}"),
        ),
    ])
    .await;
    server.ok("check_collection", json!({})).await;
    assert!(server.inbox().pending().expect("inbox").is_empty());
}

#[tokio::test]
async fn it_is_read_only_and_never_leaks_a_secret() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: token\n    secret: true\n",
        ),
        (".env", "token=canary-mcp-2b9f\n"),
        (
            "a.yml",
            &request("a", 1, "GET", "http://x/{{token}}/{{gone}}"),
        ),
    ])
    .await;
    server.set_secret("dev", "token", "canary-mcp-2b9f");
    let got = server.ok("check_collection", json!({})).await;
    assert!(!got.to_string().contains("canary-mcp-2b9f"));
    let tools = server.tools().await;
    let tool = tools
        .iter()
        .find(|t| t.name == "check_collection")
        .expect("listed");
    let hints = tool.annotations.as_ref().expect("annotations");
    assert_eq!(hints.read_only_hint, Some(true));
    assert_eq!(hints.open_world_hint, Some(false));
}

#[tokio::test]
async fn a_valueless_secret_warning_names_no_command_line_flag() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - secret: true\n    name: tok\n",
        ),
        ("a.yml", &request("a", 1, "GET", "http://x/{{tok}}")),
    ])
    .await;
    let got = server.ok("check_collection", json!({"env": "dev"})).await;
    let text = got.to_string();
    assert!(
        text.contains("tok is a secret in environment dev with no value; set it in the app's environment editor"),
        "{text}"
    );
    assert!(!text.contains("--env-var"), "{text}");
}

#[tokio::test]
async fn a_stored_secret_value_is_not_warned_about() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - secret: true\n    name: tok\n",
        ),
        ("a.yml", &request("a", 1, "GET", "http://x/{{tok}}")),
    ])
    .await;
    server.set_secret("dev", "tok", "s3cret-value");
    let got = server.ok("check_collection", json!({"env": "dev"})).await;
    let text = got.to_string();
    assert!(!text.contains("no value"), "{text}");
    assert!(!text.contains("s3cret-value"), "{text}");
}

#[tokio::test]
async fn a_literal_bad_url_is_an_error_finding_not_an_unresolved_item() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: token\n    secret: true\n",
        ),
        ("a.yml", &request("a", 1, "GET", "")),
        ("b.yml", &request("b", 2, "GET", "   ")),
        ("c.yml", &request("c", 3, "GET", "http://exa mple.com/")),
    ])
    .await;
    let got = server.ok("check_collection", json!({})).await;
    assert_eq!(got["summary"]["errors"], 3, "{got}");
    let text = got.to_string();
    assert!(text.contains("the URL is empty"), "{text}");
    assert!(text.contains("the URL is not valid"), "{text}");
    assert_eq!(got["summary"]["unresolved"], json!([]), "{got}");
}

#[tokio::test]
async fn an_unclosed_placeholder_is_reported_as_a_warning() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1, "GET", "http://x/{{host}")),
    ])
    .await;
    let got = server.ok("check_collection", json!({})).await;
    assert_eq!(got["summary"]["errors"], 0);
    assert_eq!(got["findings"][0]["kind"], "unclosed_placeholder");
    assert_eq!(got["findings"][0]["severity"], "warning");
}

#[tokio::test]
async fn an_unclosed_placeholder_in_a_folder_header_is_reported_against_the_folder_file() {
    let folder = "info:\n  name: sub\n  type: folder\n  seq: 1\nrequest:\n  headers:\n    - name: Authorization\n      value: \"{{token}\"\n";
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("sub/folder.yml", folder),
        ("sub/a.yml", &request("a", 1, "GET", "http://x/")),
    ])
    .await;
    let got = server.ok("check_collection", json!({})).await;
    assert_eq!(got["findings"].as_array().unwrap().len(), 1, "{got}");
    assert_eq!(got["findings"][0]["item"], "sub/folder.yml");
    assert_eq!(got["findings"][0]["kind"], "unclosed_placeholder");
}
