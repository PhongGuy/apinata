//! Re-running only what failed: `RequestResult::is_failure`, `RunReport::failed_items` and
//! `RunPlan::retain_items`.

use crate::common;

use apinata_core::{
    Assertion, AssertionOutcome, CancelHandle, Collection, ContextOptions, ExecOptions, Executor,
    ItemPath, Outcome, RequestResult, RunOptions, RunReport, ScriptPhase, TestOutcome, VarContext,
    Verdict, run,
};
use common::write_tree;
use std::net::TcpListener;
use std::path::Path;
use std::sync::Arc;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

fn item(rel: &str) -> ItemPath {
    ItemPath::new(rel).expect("a valid item path")
}

fn result(outcome: Outcome, verdicts: Vec<Verdict>) -> RequestResult {
    let assertions = verdicts
        .into_iter()
        .map(|v| AssertionOutcome::new(Assertion::new("res.status", "eq", Some("200".into())), v))
        .collect();
    RequestResult::from_parts(item("a.yml"), "a", outcome, None, assertions)
}

fn req(name: &str, seq: u32, url: &str, want: u16) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{url}\"\nruntime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"{want}\"\n"
    )
}

/// A 127.0.0.1 port nothing listens on: bound to learn a free port, then released.
fn dead_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);
    format!("http://127.0.0.1:{port}/x")
}

async fn run_plan(c: &Collection, plan: apinata_core::RunPlan) -> RunReport {
    let ctx = VarContext::new(c, ContextOptions::new()).expect("context");
    let exec = Arc::new(Executor::new(ExecOptions::new()).expect("client"));
    run(
        plan,
        ctx,
        exec,
        RunOptions::new(),
        CancelHandle::new(),
        Box::new(|_| {}),
    )
    .await
}

#[test]
fn an_errored_result_is_a_failure() {
    assert!(result(Outcome::Errored, vec![]).is_failure());
}

#[test]
fn a_failed_assertion_is_a_failure() {
    let r = result(Outcome::Failed, vec![Verdict::fail("500", None)]);
    assert!(r.is_failure());
}

#[test]
fn a_failed_script_test_is_a_failure() {
    let mut r = result(Outcome::Passed, vec![Verdict::Pass]);
    r.tests.push(TestOutcome::new(
        ScriptPhase::Tests,
        "t",
        Verdict::fail("nope", None),
    ));
    assert!(r.is_failure());
}

#[test]
fn a_passed_result_is_not_a_failure() {
    assert!(!result(Outcome::Passed, vec![Verdict::Pass]).is_failure());
}

#[test]
fn a_passed_result_with_only_skipped_checks_is_not_a_failure() {
    let r = result(Outcome::Passed, vec![Verdict::skipped("not supported")]);
    assert!(!r.is_failure());
}

#[test]
fn never_sent_and_cancelled_results_are_not_failures() {
    assert!(!result(Outcome::Skipped, vec![]).is_failure());
    assert!(!result(Outcome::Cancelled, vec![]).is_failure());
}

#[tokio::test]
async fn only_the_failed_items_are_sent_again_in_plan_order() {
    let server = MockServer::start().await;
    for (p, status, times) in [("/a", 200, 1), ("/boom", 500, 2), ("/d", 200, 1)] {
        Mock::given(path(p))
            .respond_with(ResponseTemplate::new(status))
            .expect(times)
            .mount(&server)
            .await;
    }
    let base = server.uri();
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("a", 1, &format!("{base}/a"), 200)),
        ("b.yml", &req("b", 2, &format!("{base}/boom"), 200)),
        ("c.yml", &req("c", 3, &dead_url(), 200)),
        ("d.yml", &req("d", 4, &format!("{base}/d"), 200)),
    ]);
    let c = Collection::open(dir.path()).expect("opens");

    let first = run_plan(&c, c.plan(dir.path()).expect("plan")).await;
    let failed = first.failed_items();
    assert_eq!(failed, [item("b.yml"), item("c.yml")]);

    let narrowed = c
        .plan(Path::new(dir.path()))
        .expect("plan")
        .retain_items(&failed);
    let kept: Vec<&str> = narrowed.items().map(ItemPath::as_str).collect();
    assert_eq!(kept, ["b.yml", "c.yml"]);

    let second = run_plan(&c, narrowed).await;
    let names: Vec<&str> = second.results.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["b", "c"]);
    assert_eq!(second.failed_items(), failed);
    // The mocks' `.expect` counts fail on drop if a passed request was sent a second time.
    server.verify().await;
}

#[tokio::test]
async fn a_path_that_failed_in_several_results_is_listed_once() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("a", 1, &dead_url(), 200)),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let plan = c.plan(dir.path()).expect("plan");
    let mut report = run_plan(&c, plan).await;
    let again = Arc::clone(&report.results[0]);
    report.results.push(again);
    assert_eq!(report.failed_items(), [item("a.yml")]);
}

#[tokio::test]
async fn retain_items_with_no_match_leaves_an_empty_plan() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("a", 1, "http://127.0.0.1:1/x", 200)),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let plan = c.plan(dir.path()).expect("plan").retain_items(&[]);
    assert_eq!(plan.items().count(), 0);
}
