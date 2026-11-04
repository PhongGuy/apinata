//! `apinata run` signs OAuth 1.0 through core.

use crate::common;

use common::{ROOT, check, column, run_json, write_tree};
use wiremock::matchers::{header_regex, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_sends_a_plaintext_signature() {
    let s = MockServer::start().await;
    let signed = r#"^OAuth oauth_consumer_key="ck", oauth_nonce="[A-Za-z0-9]{32}", oauth_signature="cs%26ts", oauth_signature_method="PLAINTEXT", oauth_timestamp="\d+", oauth_token="tk", oauth_version="1\.0"$"#;
    Mock::given(path("/o"))
        .and(header_regex("authorization", signed))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let request = format!(
        "info:\n  name: O\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{}/o\"\n  auth:\n    type: oauth1\n    consumerKey: ck\n    consumerSecret: cs\n    accessToken: tk\n    accessTokenSecret: ts\n    signatureMethod: PLAINTEXT\nruntime:\n  assertions:\n{}",
        s.uri(),
        check("res.status", "eq", "200")
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("o.yml", &request)]);
    let (code, report) = run_json(dir.path(), &["--no-scripts"]);
    assert_eq!(
        (code, column(&report, "outcome")),
        (0, vec!["passed".to_owned()])
    );
}
