//! A run applies post-response variables before assertions and carries them to later requests.

use apinata_core::{
    CancelHandle, Collection, ContextOptions, ExecOptions, Executor, Outcome, RunOptions,
    VarContext, VarResult, run,
};
use std::fs;
use std::sync::Arc;
use wiremock::matchers::{header, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ROOT_HEAD: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

fn action(name: &str, expression: &str, extra: &str) -> String {
    format!(
        "    - type: set-variable\n      phase: after-response\n      selector:\n        expression: \"{expression}\"\n        method: jsonq\n      variable:\n        name: \"{name}\"\n        scope: runtime\n{extra}"
    )
}

#[tokio::test]
async fn a_run_chains_a_captured_token_into_its_own_assertions_and_the_next_request() {
    let server = MockServer::start().await;
    Mock::given(path("/login"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw("{\"token\":\"abc\"}", "application/json"),
        )
        .mount(&server)
        .await;
    Mock::given(path("/me"))
        .and(header("authorization", "Bearer abc"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let login = format!(
        "info:\n  name: login\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: \"{}/login\"\nruntime:\n  actions:\n{}  assertions:\n    - expression: res.body.token\n      operator: eq\n      value: \"{{{{token}}}}\"\n",
        server.uri(),
        action("token", "res.body.token", "")
    );
    let me = format!(
        "info:\n  name: me\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: \"{}/me\"\n  headers:\n    - name: Authorization\n      value: \"Bearer {{{{token}}}}\"\nruntime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n",
        server.uri()
    );
    let broken = "info:\n  name: broken\n  type: http\n  seq: 3\nhttp:\n  method: GET\n  url: \"{{nope}}\"\n";
    let dir = tempfile::TempDir::new().expect("tempdir");
    for (rel, text) in [
        ("opencollection.yml", ROOT_HEAD),
        ("login.yml", &login),
        ("me.yml", &me),
        ("broken.yml", broken),
    ] {
        fs::write(dir.path().join(rel), text).expect("write");
    }
    let c = Collection::open(dir.path()).expect("opens");
    let plan = c.plan(dir.path()).expect("plan");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let runtime = ctx.runtime_vars().clone();
    let exec = Arc::new(Executor::new(ExecOptions::new()).expect("client"));
    let report = run(
        plan,
        ctx,
        exec,
        RunOptions::new(),
        CancelHandle::new(),
        Box::new(|_| {}),
    )
    .await;
    let outcomes: Vec<Outcome> = report.results.iter().map(|r| r.outcome).collect();
    assert_eq!(
        outcomes,
        [Outcome::Passed, Outcome::Passed, Outcome::Errored]
    );
    let first = &report.results[0].post_response_vars;
    assert_eq!(first.len(), 1);
    assert_eq!(
        (first[0].name(), first[0].result()),
        ("token", &VarResult::Set)
    );
    assert!(report.results[1].post_response_vars.is_empty());
    assert!(report.results[2].post_response_vars.is_empty());
    assert_eq!(runtime.entries()[0].value(), Some("abc"));
}
