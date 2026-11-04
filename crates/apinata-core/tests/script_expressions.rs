//! JavaScript expressions in post-response variables and assertions run when scripts are on.

use crate::script_support;

use apinata_core::{
    ContextOptions, ExecOptions, Outcome, RequestResult, RunOptions, VarResult, Verdict,
};
use script_support::*;
use std::time::{Duration, Instant};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn action(name: &str, expression: &str) -> String {
    format!(
        "    - type: set-variable\n      phase: after-response\n      selector:\n        expression: \"{expression}\"\n        method: jsonq\n      variable:\n        name: {name}\n        scope: runtime\n"
    )
}

fn assertion(expression: &str, operator: &str, value: &str) -> String {
    format!(
        "    - expression: \"{expression}\"\n      operator: {operator}\n      value: \"{value}\"\n"
    )
}

async fn tree(actions: &str, assertions: &str) -> (MockServer, tempfile::TempDir) {
    let s = MockServer::start().await;
    Mock::given(path("/r"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"items": [1, 2, 3], "name": "abc-x", "n": 2})),
        )
        .mount(&s)
        .await;
    let extra = format!(
        "  variables:\n    - name: prefix\n      value: abc\n  actions:\n{actions}  assertions:\n{assertions}"
    );
    let tests = script(
        "tests",
        "console.log(bru.getVar('count'), bru.getVar('joined'), bru.hasVar('bad'))",
    );
    let text = scripted("R", &format!("{}/r", s.uri()), &tests, &extra);
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    (s, dir)
}

fn var(result: &RequestResult, name: &str) -> VarResult {
    result
        .post_response_vars
        .iter()
        .find(|v| v.name() == name)
        .expect("var")
        .result()
        .clone()
}

fn verdicts(result: &RequestResult) -> Vec<Verdict> {
    result
        .assertions
        .iter()
        .map(|a| a.verdict().clone())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_response_expressions_evaluate_with_scripts_on() {
    let actions = [
        action("count", "res.body.items.length"),
        action("joined", "prefix + '-' + res.body.n"),
        action("bad", "nope.x"),
    ]
    .concat();
    let (_s, dir) = tree(&actions, "").await;
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(var(&result, "count"), VarResult::Set);
    assert_eq!(var(&result, "joined"), VarResult::Set);
    match var(&result, "bad") {
        VarResult::Failed { error } => assert!(error.contains("ReferenceError"), "{error}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(console(&result), ["log:3 abc-2 false"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn assertions_evaluate_javascript_sides() {
    let assertions = [
        assertion("res.status", "eq", "200"),
        assertion("res.body.items.length", "eq", "3"),
        assertion("res.body.name", "eq", "${prefix}-x"),
        assertion("res.body.items.length", "eq", "4"),
        assertion("nope.x", "eq", "1"),
    ]
    .concat();
    let (_s, dir) = tree("", &assertions).await;
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Failed);
    let v = verdicts(&result);
    assert_eq!(v[..3], [Verdict::Pass, Verdict::Pass, Verdict::Pass]);
    assert!(
        matches!(&v[3], Verdict::Fail { actual, .. } if actual == "3"),
        "{:?}",
        v[3]
    );
    assert!(
        matches!(&v[4], Verdict::Fail { detail: Some(d), .. } if d.contains("ReferenceError")),
        "{:?}",
        v[4]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expressions_stay_skipped_with_scripts_off() {
    let (_s, dir) = tree(
        &action("count", "res.body.items.length"),
        &assertion("res.body.items.length", "eq", "3"),
    )
    .await;
    let result = send(dir.path(), "r.yml", RunOptions::new()).await;
    assert!(matches!(var(&result, "count"), VarResult::Skipped { .. }));
    assert!(matches!(verdicts(&result)[0], Verdict::Skipped { .. }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_runaway_expression_stops_at_the_time_limit() {
    let (_s, dir) = tree(&action("spin", "(() => { while (true) {} })()"), "").await;
    let start = Instant::now();
    let result = send(dir.path(), "r.yml", limited(200)).await;
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "{:?}",
        start.elapsed()
    );
    match var(&result, "spin") {
        VarResult::Failed { error } => {
            assert!(error.contains("script exceeded the 0.2 s limit"), "{error}")
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_members_are_unsupported_in_expressions() {
    let (_s, dir) = tree(&action("s", "bru.sleep(1)"), "").await;
    let result = send(dir.path(), "r.yml", on()).await;
    match var(&result, "s") {
        VarResult::Skipped { reason } => assert!(
            reason.contains("bru.sleep is not supported by Apinata"),
            "{reason}"
        ),
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn running_a_request_resetting_a_credential_or_cookies_are_unsupported_in_expressions() {
    for (code, member) in [
        ("bru.runRequest('r')", "bru.runRequest"),
        (
            "bru.resetOauth2Credential('c')",
            "bru.resetOauth2Credential",
        ),
        ("bru.cookies.jar()", "bru.cookies"),
    ] {
        let (_s, dir) = tree(&action("s", code), "").await;
        let result = send(dir.path(), "r.yml", on()).await;
        match var(&result, "s") {
            VarResult::Skipped { reason } => assert!(
                reason.contains(&format!("{member} is not supported by Apinata")),
                "{reason}"
            ),
            other => panic!("{code}: {other:?}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expressions_record_the_secrets_in_scope() {
    let (_s, dir) = tree(&action("t", "tok.length"), "").await;
    let ctx = ContextOptions::new().sensitive_env_var("tok", "s3cr3t");
    let (result, _) = send_full(dir.path(), "r.yml", on(), ctx, ExecOptions::new()).await;
    assert_eq!(var(&result, "t"), VarResult::Set);
    assert!(
        result
            .script_sensitive_values()
            .iter()
            .any(|v| v == "s3cr3t")
    );
}
