//! `apinata run` chains a login token into the next request through a post-response variable.

use crate::common;

use common::{ROOT, apinata, column, run_json, write_tree};
use serde_json::json;
use wiremock::matchers::{header, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn action(name: &str, expression: &str) -> String {
    format!(
        "    - type: set-variable\n      phase: after-response\n      selector:\n        expression: {expression}\n        method: jsonq\n      variable:\n        name: {name}\n        scope: runtime\n"
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_login_token_chains_into_the_next_request() {
    let s = MockServer::start().await;
    Mock::given(path("/login"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw("{\"token\":\"abc\"}", "application/json"),
        )
        .mount(&s)
        .await;
    Mock::given(path("/me"))
        .and(header("authorization", "Bearer abc"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let login = format!(
        "info:\n  name: login\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: \"{}/login\"\nruntime:\n  actions:\n{}{}",
        s.uri(),
        action("token", "res.body.token"),
        action("len", "res.body.token.length")
    );
    let me = format!(
        "info:\n  name: me\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: \"{}/me\"\n  headers:\n    - name: Authorization\n      value: \"Bearer {{{{token}}}}\"\nruntime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n",
        s.uri()
    );
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("login.yml", &login),
        ("me.yml", &me),
    ]);
    // Scripts off, so the JavaScript expression shows the skip path.
    let (code, report) = run_json(dir.path(), &["--no-scripts"]);
    assert_eq!(code, 0);
    assert_eq!(column(&report, "outcome"), ["passed", "passed"]);
    assert_eq!(
        report["results"][0]["post_response_vars"],
        json!([
            {"name": "token", "result": "set"},
            {"name": "len", "result": "skipped", "reason": "`res.body.token.length` needs a JavaScript runtime"}
        ])
    );
    assert_eq!(report["results"][1]["post_response_vars"], json!([]));
    let out = (apinata().arg("run").arg(dir.path()).arg("--no-scripts"))
        .output()
        .expect("runs");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("   ! post-response variable `len` not set: `res.body.token.length` needs a JavaScript runtime"),
        "{text}"
    );
    assert!(!text.contains("abc"), "{text}");
}
