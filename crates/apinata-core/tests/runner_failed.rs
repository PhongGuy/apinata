//! Re-running only what failed: `RequestResult::is_failure`, `RunReport::failed_items`,
//! `RunPlan::retain_items`, and folding the re-run back in with `RunReport::merge_rerun`.

use crate::common;

use apinata_core::{
    Assertion, AssertionOutcome, CancelHandle, Collection, ContextOptions, ExecOptions, Executor,
    ItemPath, Outcome, RequestResult, ResultsReport, RunOptions, RunReport, ScriptPhase,
    TestOutcome, VarContext, Verdict, rerun_slots, run,
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

#[test]
fn rerun_slots_match_each_rerun_entry_by_path_and_data_row_in_order() {
    let (a, b, c, d) = (item("a.yml"), item("b.yml"), item("c.yml"), item("d.yml"));
    let earlier = [(&a, None), (&b, None), (&b, None), (&c, None)];
    let rerun = [(&b, None), (&b, None), (&d, None)];
    assert_eq!(rerun_slots(&earlier, &rerun), [Some(1), Some(2), None]);

    let earlier = [(&a, Some(0)), (&b, Some(0)), (&a, Some(1)), (&b, Some(1))];
    let rerun = [(&b, Some(1)), (&b, Some(0))];
    assert_eq!(rerun_slots(&earlier, &rerun), [Some(3), Some(1)]);
}

#[test]
fn only_a_result_that_reached_a_verdict_is_settled() {
    assert!(result(Outcome::Passed, vec![]).is_settled());
    assert!(result(Outcome::Failed, vec![]).is_settled());
    assert!(result(Outcome::Errored, vec![]).is_settled());
    assert!(!result(Outcome::Skipped, vec![]).is_settled());
    assert!(!result(Outcome::Cancelled, vec![]).is_settled());
}

#[tokio::test]
async fn a_merged_rerun_replaces_the_failed_result_and_the_report_covers_the_whole_run() {
    let server = MockServer::start().await;
    Mock::given(path("/flaky"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    for p in ["/flaky", "/ok"] {
        Mock::given(path(p))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
    }
    let base = server.uri();
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("a", 1, &format!("{base}/ok"), 200)),
        ("b.yml", &req("b", 2, &format!("{base}/flaky"), 200)),
        ("c.yml", &req("c", 3, &format!("{base}/ok"), 200)),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let mut report = run_plan(&c, c.plan(dir.path()).expect("plan")).await;
    assert_eq!((report.summary.passed, report.summary.failed), (2, 1));

    let failed = report.failed_items();
    let narrowed = c.plan(dir.path()).expect("plan").retain_items(&failed);
    let rerun = run_plan(&c, narrowed).await;
    assert_eq!(report.merge_rerun(rerun), [1]);

    let names: Vec<&str> = report.results.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["a", "b", "c"]);
    assert_eq!(report.results[1].outcome, Outcome::Passed);
    assert_eq!(report.summary.requests, 3);
    assert_eq!((report.summary.passed, report.summary.failed), (3, 0));
    assert!(report.failed_items().is_empty());

    let export = ResultsReport::new(Vec::new(), report.results.iter().map(Arc::as_ref));
    let json: serde_json::Value = serde_json::from_str(&export.to_json()).expect("json");
    assert_eq!(json["results"].as_array().map(Vec::len), Some(3));
    assert_eq!(json["summary"]["passed"], 3);
    assert_eq!(json["summary"]["failed"], 0);
}

#[tokio::test]
async fn a_rerun_result_that_was_never_sent_keeps_the_earlier_one() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("a", 1, &dead_url(), 200)),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let mut report = run_plan(&c, c.plan(dir.path()).expect("plan")).await;
    let cancel = CancelHandle::new();
    cancel.cancel();
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let exec = Arc::new(Executor::new(ExecOptions::new()).expect("client"));
    let plan = c.plan(dir.path()).expect("plan");
    let rerun = run(plan, ctx, exec, RunOptions::new(), cancel, Box::new(|_| {})).await;
    assert!(!rerun.results[0].is_settled());

    assert!(report.merge_rerun(rerun).is_empty());
    assert_eq!(report.results[0].outcome, Outcome::Errored);
    assert_eq!(report.summary.errored, 1);
}
