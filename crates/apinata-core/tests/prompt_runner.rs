//! The runner refuses an unanswered prompt before any script runs or anything is sent.

use crate::script_support;

use apinata_core::{
    CancelHandle, Collection, ContextOptions, ExecOptions, Executor, Outcome, ResolveError,
    RunError, VarContext, run,
};
use script_support::*;
use std::sync::Arc;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::any};

async fn ok_server() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    s
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unanswered_prompt_errors_before_the_pre_request_script() {
    let s = ok_server().await;
    let url = format!("{}/{{{{?code}}}}", s.uri());
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted(
                "r",
                &url,
                &script("before-request", "bru.setVar('ran', 'yes')"),
                "",
            ),
        ),
    ]);
    let (result, ctx) = send_full(
        dir.path(),
        "r.yml",
        on(),
        ContextOptions::new(),
        ExecOptions::new(),
    )
    .await;
    assert_eq!(result.outcome, Outcome::Errored);
    assert!(
        matches!(&result.error, Some(RunError::Resolve(ResolveError::PromptRequired { labels })) if labels == &["code".to_owned()]),
        "{:?}",
        result.error
    );
    assert!(ctx.runtime_vars().is_empty(), "the script did not run");
    assert!(s.received_requests().await.expect("recording").is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_answered_run_sends_and_a_script_set_prompt_still_fails() {
    let s = ok_server().await;
    let url = format!("{}/{{{{?code}}}}", s.uri());
    let late = script("before-request", "req.setHeader('X-Late', '{{?late}}')");
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &scripted("a", &url, "", "")),
        (
            "b.yml",
            &scripted("b", &url, &late, "").replace("seq: 1", "seq: 2"),
        ),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let plan = c.plan(dir.path()).expect("plan");
    let ctx = VarContext::new(&c, ContextOptions::new())
        .expect("context")
        .with_prompt_values([("code", "A1")]);
    let exec = Arc::new(Executor::new(ExecOptions::new()).expect("client"));
    let report = run(plan, ctx, exec, on(), CancelHandle::new(), Box::new(|_| {})).await;
    let outcomes: Vec<Outcome> = report.results.iter().map(|r| r.outcome).collect();
    assert_eq!(outcomes, [Outcome::Passed, Outcome::Errored]);
    assert_eq!(
        report.results[1].error.as_ref().map(|e| e.to_string()),
        Some("no value for prompt variables: late".to_owned())
    );
    let paths: Vec<String> = s
        .received_requests()
        .await
        .expect("recording")
        .iter()
        .map(|r| r.url.path().to_owned())
        .collect();
    assert_eq!(paths, ["/A1"]);
}
