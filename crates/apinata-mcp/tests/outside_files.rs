//! An MCP send never reads a key or certificate file outside the collection, whatever path the
//! request or collection file names.

use crate::common;

use common::{ROOT, TestServer};
use serde_json::json;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fixture(rel: &str) -> String {
    format!(
        "{}/../apinata-core/tests/fixtures/{rel}",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn get(url: &str, extra: &str) -> String {
    format!("info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\n{extra}")
}

async fn mock() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn jwt_and_oauth1_key_files_outside_the_root_are_never_read() {
    let mock = mock().await;
    let jwt = fixture("jwt/rsa.pem");
    let oauth1 = fixture("oauth1/rsa-pkcs8.pem");
    let auths = [
        format!(
            "  auth:\n    type: jwt\n    algorithm: RS256\n    privateKey:\n      type: file\n      value: \"{jwt}\"\n    payload: '{{\"sub\":\"1\"}}'\n    placement: header\n"
        ),
        format!(
            "  auth:\n    type: oauth1\n    consumerKey: ck\n    signatureMethod: RSA-SHA1\n    privateKey:\n      type: file\n      value: \"{oauth1}\"\n    placement: header\n"
        ),
    ];
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", "")]).await;
    for auth in &auths {
        server.write("r.yml", &get(&mock.uri(), auth));
        let err = server.err("send_request", json!({"path": "r.yml"})).await;
        assert!(err.contains("outside the collection"), "{err}");
        assert!(!err.contains("PRIVATE KEY"), "{err}");
    }
    assert!(mock.received_requests().await.expect("recorded").is_empty());
}

#[tokio::test]
async fn a_client_certificate_outside_the_root_is_never_read() {
    let (cert, key) = (fixture("network/client.pem"), fixture("network/client.key"));
    let root = format!(
        "{ROOT}config:\n  clientCertificates:\n    - domain: \"127.0.0.1\"\n      type: pem\n      certificateFilePath: '{cert}'\n      privateKeyFilePath: '{key}'\n"
    );
    let server = TestServer::start(&[
        ("opencollection.yml", &root),
        ("r.yml", &get("https://127.0.0.1:9/x", "")),
    ])
    .await;
    let err = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(err.contains("outside the collection"), "{err}");
    assert!(!err.contains("BEGIN"), "{err}");
}
