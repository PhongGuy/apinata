//! `export_openapi` and `export_postman`: the core exports the CLI writes, returned as tool
//! text with secret values absent and the text cut to a byte budget.

use crate::common;

use apinata_core::{Collection, export};
use common::{ROOT, TestServer};
use serde_json::json;

const KEY: &str = "mcp-exp-Key-5521-SECRET";
const KEYCHAIN: &str = "mcp-exp-Pass-8830-SECRET";
const DEV: &str = "name: dev\nvariables:\n  - name: host\n    value: https://api.example.test\n  - name: apikey\n    secret: true\n  - name: pw\n    secret: true\n";
const REQUEST: &str = "info:\n  name: list\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{{host}}/users?key={{apikey}}\"\n  headers:\n    - name: X-Key\n      value: \"{{apikey}}\"\n  auth:\n    type: basic\n    username: bob\n    password: \"{{pw}}\"\n";

async fn shop() -> TestServer {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("users/list.yml", REQUEST),
    ])
    .await;
    server.set_secret("dev", "apikey", KEY);
    server.set_secret("dev", "pw", KEYCHAIN);
    server
}

fn notes(core: &[apinata_core::export::ExportNote]) -> Vec<String> {
    core.iter()
        .map(|n| format!("{}: {}", n.source, n.detail))
        .collect()
}

#[tokio::test]
async fn openapi_json_is_the_cores_text() {
    let server = shop().await;
    let got = server
        .ok("export_openapi", json!({"max_bytes": 1048576}))
        .await;
    let core = export::openapi(&Collection::open(server.root()).expect("open"));
    assert_eq!(got["format"], "json");
    assert_eq!(got["text"], core.json.as_str());
    assert_eq!(got["total_bytes"], core.json.len());
    assert_eq!(got["truncated"], false);
    assert_eq!(
        serde_json::to_value(notes(&core.notes)).unwrap(),
        got["notes"]
    );
}

#[tokio::test]
async fn openapi_yaml_is_the_cores_yaml() {
    let server = shop().await;
    let got = server
        .ok(
            "export_openapi",
            json!({"format": "yaml", "max_bytes": 1048576}),
        )
        .await;
    let core = export::openapi(&Collection::open(server.root()).expect("open"));
    assert_eq!(got["format"], "yaml");
    assert_eq!(got["text"], core.yaml().expect("yaml").as_str());
}

#[tokio::test]
async fn postman_is_the_cores_text() {
    let server = shop().await;
    let got = server
        .ok("export_postman", json!({"max_bytes": 1048576}))
        .await;
    let core = export::postman(&Collection::open(server.root()).expect("open"));
    assert_eq!(got["text"], core.json.as_str());
    assert_eq!(got["truncated"], false);
}

#[tokio::test]
async fn secret_values_never_appear() {
    let server = shop().await;
    for (tool, args) in [
        ("export_openapi", json!({"max_bytes": 1048576})),
        (
            "export_openapi",
            json!({"format": "yaml", "max_bytes": 1048576}),
        ),
        ("export_postman", json!({"max_bytes": 1048576})),
    ] {
        let got = server.ok(tool, args).await;
        let text = got.to_string();
        assert!(got["text"].as_str().is_some_and(|t| t.contains("users")));
        for leak in [KEY, KEYCHAIN] {
            assert!(!text.contains(leak), "{tool}: {leak} leaked");
        }
    }
}

#[tokio::test]
async fn text_is_cut_to_max_bytes_with_the_flag() {
    let server = shop().await;
    let got = server.ok("export_postman", json!({"max_bytes": 40})).await;
    let text = got["text"].as_str().expect("text");
    assert!(text.len() <= 40 + 16, "{}", text.len());
    assert_eq!(got["truncated"], true);
    assert!(got["total_bytes"].as_u64().unwrap() > 40);
}

#[tokio::test]
async fn default_budget_applies_and_unknown_format_is_refused() {
    let server = shop().await;
    let got = server.ok("export_openapi", json!({})).await;
    assert_eq!(
        got["truncated"],
        got["total_bytes"].as_u64().unwrap() > 32 * 1024
    );
    let message = server
        .err("export_openapi", json!({"format": "toml"}))
        .await;
    assert!(message.contains("toml"), "{message}");
}
