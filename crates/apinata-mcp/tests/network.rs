//! The MCP server honours the collection's proxy and certificates: an untrusted proxy host
//! files an approval naming the proxy, a secret in it files nothing, and launch-time
//! certificates are used. A PAC global proxy reports its fallback warning and files the same
//! approval for the proxy it chooses. No test reaches a non-local host.

use crate::common;

use apinata_core::{ClientCertificate, GlobalProxy, NetworkSettings, PacSource};
use apinata_mcp::McpOptions;
use common::{ROOT, TestServer, request};
use serde_json::json;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn root_with_proxy(host: &str) -> String {
    format!(
        "{ROOT}config:\n  proxy:\n    inherit: false\n    config:\n      protocol: http\n      hostname: \"{host}\"\n      port: \"3128\"\n      auth:\n        username: \"\"\n        password: \"\"\n      bypassProxy: \"\"\n"
    )
}

#[tokio::test]
async fn an_untrusted_proxy_files_an_approval_for_the_proxy_host() {
    let server = TestServer::start(&[
        ("opencollection.yml", &root_with_proxy("proxy.corp.example")),
        ("r.yml", &request("r", 1, "GET", "http://127.0.0.1:9/x")),
    ])
    .await;
    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(message.contains("`proxy.corp.example`"), "{message}");
    let pending = server.inbox().pending().expect("inbox");
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0].host, "proxy.corp.example");
    assert!(
        message.contains("proxy host `proxy.corp.example` (hop 0) is not trusted"),
        "{message}"
    );
    assert_eq!(
        pending[0].proxy.as_deref(),
        Some("http://proxy.corp.example:3128")
    );
}

#[tokio::test]
async fn a_trusted_non_loopback_proxy_sends_without_approval() {
    // 0.0.0.0 is not loopback to the policy, yet Linux dials it locally, so the proxy is a
    // local mock and no name reaches DNS; the target is judged by name and never resolved here.
    let proxy = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("via-proxy"))
        .mount(&proxy)
        .await;
    let port = proxy.address().port();
    let root = root_with_proxy("0.0.0.0").replace("\"3128\"", &format!("\"{port}\""));
    let server = TestServer::start(&[
        ("opencollection.yml", &root),
        ("r.yml", &request("r", 1, "GET", "http://api.example/x")),
    ])
    .await;
    server.trust("0.0.0.0");
    server.trust("api.example");
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    assert!(result.to_string().contains("via-proxy"), "{result}");
    assert!(server.inbox().pending().expect("inbox").is_empty());
    assert_eq!(proxy.received_requests().await.expect("recording").len(), 1);
}

#[tokio::test]
async fn a_secret_in_the_proxy_host_files_nothing() {
    let server = TestServer::start(&[
        (
            "opencollection.yml",
            &root_with_proxy("{{token}}.corp.example"),
        ),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: token\n    secret: true\n",
        ),
        ("r.yml", &request("r", 1, "GET", "http://127.0.0.1:9/x")),
    ])
    .await;
    server.set_secret("dev", "token", "Mixed9Case");
    let message = server
        .err(
            "send_request",
            json!({"path": "r.yml", "environment": "dev"}),
        )
        .await;
    assert!(
        message.contains(
            "a secret cannot be used in a proxy host or port; nothing was sent. Move the proxy \
             host to a non-secret variable."
        ),
        "{message}"
    );
    assert!(!message.to_lowercase().contains("mixed9case"), "{message}");
    assert!(server.inbox().pending().expect("inbox").is_empty());
}

#[tokio::test]
async fn launch_certificates_are_used() {
    let server = TestServer::start_with(
        &[
            ("opencollection.yml", ROOT),
            ("r.yml", &request("r", 1, "GET", "https://127.0.0.1:9/x")),
        ],
        |options| {
            options.client_certificates(vec![ClientCertificate::pkcs12("127.0.0.1", "c.p12")])
        },
    )
    .await;
    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(message.contains("openssl pkcs12"), "{message}");
}

// Launch options whose global proxy is the PAC file at the absolute path `script`.
fn pac_options(script: &std::path::Path) -> impl FnOnce(McpOptions) -> McpOptions {
    let source = PacSource::parse(script.to_str().expect("utf-8"), None).expect("absolute");
    move |options| options.network(NetworkSettings::new().proxy(GlobalProxy::Pac { source }))
}

#[tokio::test]
async fn a_pac_fallback_reaches_the_send_result_warnings() {
    let origin = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("direct"))
        .mount(&origin)
        .await;
    let dir = tempfile::tempdir().expect("tempdir");
    let url = format!("{}/x", origin.uri());
    let server = TestServer::start_with(
        &[
            ("opencollection.yml", ROOT),
            ("r.yml", &request("r", 1, "GET", &url)),
        ],
        pac_options(&dir.path().join("missing.pac")),
    )
    .await;
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    assert_eq!(result["status"], 200, "{result}");
    let warnings = result["warnings"].as_array().expect("warnings");
    assert_eq!(warnings.len(), 1, "{result}");
    let warning = warnings[0].as_str().expect("text");
    assert!(
        warning.starts_with("proxy auto-config could not be fetched: ")
            && warning.ends_with("; connected directly"),
        "{warning}"
    );
}

#[tokio::test]
async fn a_pac_chosen_untrusted_proxy_files_an_approval_naming_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("proxy.pac");
    std::fs::write(
        &script,
        r#"function FindProxyForURL(url, host) { return "PROXY proxy.corp.example:3128"; }"#,
    )
    .expect("write");
    let server = TestServer::start_with(
        &[
            ("opencollection.yml", ROOT),
            ("r.yml", &request("r", 1, "GET", "http://127.0.0.1:9/x")),
        ],
        pac_options(&script),
    )
    .await;
    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(
        message.contains("proxy host `proxy.corp.example` (hop 0) is not trusted"),
        "{message}"
    );
    let pending = server.inbox().pending().expect("inbox");
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0].host, "proxy.corp.example");
    assert_eq!(
        pending[0].proxy.as_deref(),
        Some("http://proxy.corp.example:3128")
    );
}
