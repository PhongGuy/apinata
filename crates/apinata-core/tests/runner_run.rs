//! The core run loop: event order, outcomes, bail, cancellation and the plan snapshot.

use crate::common;

use apinata_core::{
    CancelHandle, Collection, ContextOptions, ExecError, ExecOptions, Executor, Outcome,
    RequestResult, RunError, RunEvent, RunOptions, RunReport, Summary, VarContext, run,
};
use common::write_tree;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

/// A request to `url` with one `res.status eq <want>` assertion.
fn req(name: &str, seq: u32, url: &str, want: u16) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{url}\"\nruntime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"{want}\"\n    - expression: res.url\n      operator: isDefined\n"
    )
}

async fn server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(path("/slow"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
        .mount(&server)
        .await;
    server
}

/// Runs the whole collection in `dir`, returning the report and the events as short strings.
async fn run_all(dir: &Path, opts: RunOptions, cancel: CancelHandle) -> (RunReport, Vec<String>) {
    let c = Collection::open(dir).expect("opens");
    let plan = c.plan(dir).expect("plan");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let exec = Arc::new(Executor::new(ExecOptions::new()).expect("client"));
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let on_event = Box::new(move |event: RunEvent| {
        let line = match event {
            RunEvent::Started { total } => format!("started {total}"),
            RunEvent::RequestStarted { index, path } => format!("start {index} {}", path.as_str()),
            RunEvent::RequestFinished { index, result } => {
                format!("finish {index} {:?}", result.outcome)
            }
            RunEvent::Finished { summary } => format!("finished {}", summary.requests),
            other => panic!("unexpected event {other:?}"),
        };
        sink.lock().expect("lock").push(line);
    });
    let report = run(plan, ctx, exec, opts, cancel, on_event).await;
    let events = events.lock().expect("lock").clone();
    (report, events)
}

fn outcomes(report: &RunReport) -> Vec<Outcome> {
    report.results.iter().map(|r| r.outcome).collect()
}

#[tokio::test]
async fn events_arrive_in_order_and_the_report_counts_outcomes() {
    let server = server().await;
    let ok = format!("{}/ok", server.uri());
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("a", 1, &ok, 200)),
        ("b.yml", &req("b", 2, &ok, 500)),
        ("c.yml", &req("c", 3, "{{nope}}", 200)),
    ]);
    let (report, events) = run_all(dir.path(), RunOptions::new(), CancelHandle::new()).await;
    assert_eq!(
        events,
        [
            "started 3",
            "start 0 a.yml",
            "finish 0 Passed",
            "start 1 b.yml",
            "finish 1 Failed",
            "start 2 c.yml",
            "finish 2 Errored",
            "finished 3",
        ]
    );
    assert_eq!(
        outcomes(&report),
        [Outcome::Passed, Outcome::Failed, Outcome::Errored]
    );
    let mut want = Summary::default();
    (
        want.requests,
        want.passed,
        want.failed,
        want.errored,
        want.skipped_assertions,
    ) = (3, 1, 1, 1, 2);
    assert_eq!(report.summary, want);
    assert_eq!(report.summary.unverified, 0);
    assert_eq!(report.summary.no_tests, 0);
    let a = &report.results[0];
    assert_eq!((a.path.as_str(), a.name.as_str()), ("a.yml", "a"));
    assert_eq!(a.response.as_ref().map(|r| r.status()), Some(200));
    assert_eq!(a.assertions.len(), 2);
    assert_eq!(
        report.results[2].error.as_ref().map(ToString::to_string),
        Some("variable `nope` is not defined".to_owned())
    );
}

#[tokio::test]
async fn bail_skips_everything_after_the_first_failure_without_starting_it() {
    let server = server().await;
    let ok = format!("{}/ok", server.uri());
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("a", 1, &ok, 500)),
        ("b.yml", &req("b", 2, &ok, 200)),
    ]);
    let (report, events) = run_all(
        dir.path(),
        RunOptions::new().bail(true),
        CancelHandle::new(),
    )
    .await;
    assert_eq!(
        events,
        [
            "started 2",
            "start 0 a.yml",
            "finish 0 Failed",
            "finish 1 Skipped",
            "finished 2"
        ]
    );
    assert_eq!(outcomes(&report), [Outcome::Failed, Outcome::Skipped]);
    assert!(report.results[1].response.is_none());
    assert_eq!(server.received_requests().await.expect("recorded").len(), 1);
}

#[tokio::test]
async fn a_plan_is_a_snapshot_that_edits_on_disk_do_not_reach() {
    let server = server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("a", 1, &format!("{}/ok", server.uri()), 200)),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let plan = c.plan(dir.path()).expect("plan");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    std::fs::write(
        dir.path().join("a.yml"),
        req("a", 1, &format!("{}/elsewhere", server.uri()), 200),
    )
    .expect("rewrite");
    drop(c);
    let exec = Arc::new(Executor::new(ExecOptions::new()).expect("client"));
    // Spawning proves the run future is Send and 'static.
    let report = tokio::spawn(run(
        plan,
        ctx,
        exec,
        RunOptions::new(),
        CancelHandle::new(),
        Box::new(|_| {}),
    ))
    .await
    .expect("joins");
    assert_eq!(outcomes(&report), [Outcome::Passed]);
    let seen = server.received_requests().await.expect("recorded");
    assert_eq!(seen[0].url.path(), "/ok");
}

#[tokio::test]
async fn cancelling_marks_the_request_in_flight_and_skips_the_rest() {
    let server = server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &req("a", 1, &format!("{}/slow", server.uri()), 200),
        ),
        ("b.yml", &req("b", 2, &format!("{}/ok", server.uri()), 200)),
    ]);
    let cancel = CancelHandle::new();
    // Cancel once the slow request has reached the server, so it is provably in flight.
    let trigger = async {
        while server
            .received_requests()
            .await
            .expect("recorded")
            .is_empty()
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        cancel.cancel();
    };
    let ((report, events), ()) = tokio::join!(
        run_all(dir.path(), RunOptions::new(), cancel.clone()),
        trigger
    );
    assert_eq!(
        events,
        [
            "started 2",
            "start 0 a.yml",
            "finish 0 Cancelled",
            "finish 1 Skipped",
            "finished 2"
        ]
    );
    assert_eq!((report.summary.cancelled, report.summary.skipped), (1, 1));
    // The request in flight keeps the executor's error; the skipped one has none.
    let kept = &report.results[0].error;
    assert!(
        matches!(kept, Some(RunError::Exec(ExecError::Cancelled))),
        "{kept:?}"
    );
    assert!(report.results[1].error.is_none());
    assert_eq!(server.received_requests().await.expect("recorded").len(), 1);
}

#[tokio::test]
async fn a_handle_cancelled_before_the_run_skips_every_request_and_sends_nothing() {
    let server = server().await;
    let ok = format!("{}/ok", server.uri());
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("a", 1, &ok, 200)),
        ("b.yml", &req("b", 2, &ok, 200)),
    ]);
    let cancel = CancelHandle::new();
    cancel.cancel();
    let (report, events) = run_all(dir.path(), RunOptions::new(), cancel).await;
    assert_eq!(
        events,
        [
            "started 2",
            "finish 0 Skipped",
            "finish 1 Skipped",
            "finished 2"
        ]
    );
    let mut want = Summary::default();
    (want.requests, want.skipped) = (2, 2);
    assert_eq!(report.summary, want);
    assert!(
        report
            .results
            .iter()
            .all(|r| r.error.is_none() && r.response.is_none())
    );
    assert!(
        server
            .received_requests()
            .await
            .expect("recorded")
            .is_empty()
    );
}

#[tokio::test]
async fn a_finished_event_shares_its_result_with_the_report() {
    let server = server().await;
    let ok = format!("{}/ok", server.uri());
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("a", 1, &ok, 200)),
        ("b.yml", &req("b", 2, &ok, 200)),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let plan = c.plan(dir.path()).expect("plan");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let exec = Arc::new(Executor::new(ExecOptions::new()).expect("client"));
    let seen: Arc<Mutex<Vec<Arc<RequestResult>>>> = Arc::default();
    let sink = Arc::clone(&seen);
    let on_event = Box::new(move |event: RunEvent| {
        if let RunEvent::RequestFinished { result, .. } = event {
            sink.lock().expect("lock").push(result);
        }
    });
    let report = run(
        plan,
        ctx,
        exec,
        RunOptions::new(),
        CancelHandle::new(),
        on_event,
    )
    .await;
    let seen = seen.lock().expect("lock");
    assert_eq!((seen.len(), report.results.len()), (2, 2));
    // The event and the report hold one allocation, so a large response is never copied.
    for (event, kept) in seen.iter().zip(&report.results) {
        assert!(
            Arc::ptr_eq(event, kept),
            "{} was copied",
            kept.path.as_str()
        );
    }
}

#[tokio::test]
async fn a_guard_refuses_a_request_before_it_is_sent_and_the_others_still_run() {
    let server = server().await;
    let ok = format!("{}/ok", server.uri());
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("a", 1, &ok, 200)),
        ("b.yml", &req("b", 2, &format!("{ok}?refuse"), 200)),
        ("c.yml", &req("c", 3, &ok, 200)),
    ]);
    let opts = RunOptions::new().guard(Arc::new(|r: &apinata_core::ResolvedRequest| {
        r.url()
            .ends_with("refuse")
            .then(|| "not this one".to_owned())
    }));
    assert!(format!("{opts:?}").contains("guard: true"), "{opts:?}");
    let (report, _) = run_all(dir.path(), opts, CancelHandle::new()).await;
    assert_eq!(
        outcomes(&report),
        [Outcome::Passed, Outcome::Errored, Outcome::Passed]
    );
    let refused = &report.results[1];
    assert!(matches!(refused.error, Some(RunError::Refused { .. })));
    assert_eq!(
        refused.error.as_ref().map(ToString::to_string).as_deref(),
        Some("not this one")
    );
    assert!(refused.response.is_none());
    assert!(refused.post_response_vars.is_empty());
    let sent = server.received_requests().await.expect("recorded");
    assert_eq!(sent.len(), 2);
    assert!(sent.iter().all(|r| r.url.query().is_none()));
}

#[tokio::test]
async fn unverified_counts_passed_requests_whose_only_checks_were_skipped() {
    let server = server().await;
    let ok = format!("{}/ok", server.uri());
    let head = format!(
        "info:\n  name: {{n}}\n  type: http\n  seq: {{s}}\nhttp:\n  method: GET\n  url: \"{ok}\"\n"
    );
    let skipped_only = head.replace("{n}", "u").replace("{s}", "1")
        + "runtime:\n  assertions:\n    - expression: res.url\n      operator: isDefined\n";
    let no_checks = head.replace("{n}", "n").replace("{s}", "2");
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &skipped_only),
        ("b.yml", &no_checks),
        ("c.yml", &req("c", 3, &ok, 200)),
    ]);
    let (report, _) = run_all(dir.path(), RunOptions::new(), CancelHandle::new()).await;
    assert_eq!(report.summary.passed, 3);
    assert_eq!(report.summary.unverified, 1);
    assert_eq!(report.summary.no_tests, 1);
    assert_eq!(report.summary.skipped, 0);
}
