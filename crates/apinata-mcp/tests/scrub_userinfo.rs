//! `scrub_example_userinfo`: a dry run by default; with `write`, saved examples lose their
//! URL userinfo while the request's own URL stays.

use crate::common;

use common::{ROOT, TestServer};
use serde_json::json;
use std::fs;

const LEAKY: &str = "info:
  name: leaky
  type: http
  seq: 1

http:
  method: GET
  url: https://u:pa55word@x.test/

examples:
  - name: one
    request:
      url: https://u:pa55word@x.test/a
      method: GET
    response:
      status: 200
";

#[tokio::test]
async fn a_dry_run_reports_and_a_write_saves() {
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("leaky.yml", LEAKY)]).await;
    let change = json!({"item": "leaky.yml", "example": "one", "url": "https://x.test/a"});
    let dry = server.ok("scrub_example_userinfo", json!({})).await;
    assert_eq!(dry, json!({"written": false, "changes": [change]}));
    assert!(!dry.to_string().contains("pa55word"));
    assert_eq!(
        fs::read_to_string(server.root().join("leaky.yml")).unwrap(),
        LEAKY
    );
    let wet = server
        .ok("scrub_example_userinfo", json!({"write": true}))
        .await;
    assert_eq!(wet, json!({"written": true, "changes": [change]}));
    assert_eq!(
        fs::read_to_string(server.root().join("leaky.yml")).unwrap(),
        LEAKY.replace("u:pa55word@x.test/a", "x.test/a")
    );
    assert_eq!(
        server
            .ok("scrub_example_userinfo", json!({"write": true}))
            .await,
        json!({"written": false, "changes": []})
    );
}
