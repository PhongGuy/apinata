//! Launch-level global environments reach every send, and their overrides stay secret.

use crate::common;

use apinata_core::GlobalEnvironments;
use common::{ROOT, TestServer};
use serde_json::json;
use std::sync::Arc;
use wiremock::matchers::{header, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn launch_globals_reach_a_send_and_their_overrides_are_redacted() {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .and(header("x-key", "s3cret-global"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&s)
        .await;
    let gdir = tempfile::TempDir::new().expect("tempdir");
    std::fs::write(
        gdir.path().join("shared.yml"),
        format!(
            "name: shared\nvariables:\n  - name: gbase\n    value: {}\n",
            s.uri()
        ),
    )
    .expect("global");
    let globals = Arc::new(GlobalEnvironments::open(gdir.path()).expect("globals"));
    let req = "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{{gbase}}/ok\"\n  headers:\n    - name: X-Key\n      value: \"{{key}}\"\n";
    let server = TestServer::start_with(&[("opencollection.yml", ROOT), ("r.yml", req)], |o| {
        o.global_env(globals, "shared")
            .global_env_var("key", "s3cret-global")
    })
    .await;
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    assert_eq!(result["status"], 200, "{result}");
    assert!(!result.to_string().contains("s3cret-global"), "{result}");
}
