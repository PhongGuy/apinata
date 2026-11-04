//! Data iterations, the delay between requests and how a stop ends every iteration.

use crate::common;

use apinata_core::{
    CancelHandle, Collection, ContextOptions, ExecOptions, ExecutionMode, Executor, IterationData,
    Outcome, ResolveError, RunError, RunEvent, RunOptions, RunReport, RuntimeVars, ScriptOptions,
    VarContext, run,
};
use common::write_tree;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nrequest:\n  variables:\n    - name: id\n      value: from-collection\n";
const FAIL: &str = "runtime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"500\"\n";

fn req(name: &str, seq: u32, url: &str, extra: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{url}\"\n{extra}"
    )
}

async fn ok_server() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    s
}

async fn served(s: &MockServer) -> Vec<String> {
    let got = s.received_requests().await.expect("recorded");
    got.iter().map(|r| r.url.path().to_owned()).collect()
}

fn csv(text: &str) -> IterationData {
    IterationData::from_csv(text).expect("csv")
}

/// Runs the collection at `dir`; `on_finish` sees each finished index and the cancel handle.
async fn run_with(
    dir: &Path,
    opts: RunOptions,
    ctx: ContextOptions,
    on_finish: impl Fn(usize, &CancelHandle) + Send + 'static,
) -> (RunReport, Vec<String>) {
    let c = Collection::open(dir).expect("opens");
    let plan = c.plan(dir).expect("plan");
    let ctx = VarContext::new(&c, ctx).expect("context");
    let exec = Arc::new(Executor::new(ExecOptions::new()).expect("client"));
    let cancel = CancelHandle::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let (sink, handle) = (Arc::clone(&events), cancel.clone());
    let on_event = Box::new(move |event: RunEvent| {
        let line = match event {
            RunEvent::Started { total } => format!("started {total}"),
            RunEvent::IterationStarted { index, total } => format!("iteration {index} of {total}"),
            RunEvent::RequestStarted { index, .. } => format!("start {index}"),
            RunEvent::RequestFinished { index, result } => {
                on_finish(index, &handle);
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

#[tokio::test]
async fn each_row_runs_the_whole_plan_with_its_values() {
    let s = ok_server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &req("a", 1, &format!("{}/a/{{{{id}}}}", s.uri()), ""),
        ),
        (
            "b.yml",
            &req("b", 2, &format!("{}/b/{{{{id}}}}", s.uri()), ""),
        ),
    ]);
    let opts = RunOptions::new().iterations(csv("id\n1\n2\n"));
    let (report, events) = run_with(dir.path(), opts, ContextOptions::new(), |_, _| {}).await;
    assert_eq!(served(&s).await, ["/a/1", "/b/1", "/a/2", "/b/2"]);
    assert_eq!(
        events,
        [
            "started 2",
            "iteration 0 of 2",
            "start 0",
            "finish 0 Passed",
            "start 1",
            "finish 1 Passed",
            "iteration 1 of 2",
            "start 0",
            "finish 0 Passed",
            "start 1",
            "finish 1 Passed",
            "finished 4",
        ]
    );
    let marks: Vec<Option<usize>> = report.results.iter().map(|r| r.iteration).collect();
    assert_eq!(marks, [Some(0), Some(0), Some(1), Some(1)]);
}

#[tokio::test]
async fn without_data_there_is_one_pass_and_no_iteration_marks() {
    let s = ok_server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &req("a", 1, &format!("{}/a/{{{{id}}}}", s.uri()), ""),
        ),
    ]);
    let (report, events) = run_with(
        dir.path(),
        RunOptions::new(),
        ContextOptions::new(),
        |_, _| {},
    )
    .await;
    assert_eq!(served(&s).await, ["/a/from-collection"]);
    assert_eq!(
        events,
        ["started 1", "start 0", "finish 0 Passed", "finished 1"]
    );
    assert_eq!(report.results[0].iteration, None);
}

#[tokio::test]
async fn data_sits_above_request_vars_and_below_runtime_vars() {
    let s = ok_server().await;
    let own = "runtime:\n  variables:\n    - name: id\n      value: from-request\n    - name: other\n      value: from-request\n";
    let url = format!("{}/x/{{{{id}}}}/{{{{other}}}}/{{{{late}}}}", s.uri());
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("a", 1, &url, own)),
    ]);
    let runtime = RuntimeVars::new();
    runtime.set_text("late", "from-runtime");
    let opts = RunOptions::new().iterations(csv("id,late\n1,from-data\n"));
    let ctx = ContextOptions::new().runtime_vars(runtime);
    run_with(dir.path(), opts, ctx, |_, _| {}).await;
    assert_eq!(served(&s).await, ["/x/1/from-request/from-runtime"]);
}

#[tokio::test]
async fn a_data_column_shadowing_a_secret_is_not_sensitive() {
    let s = ok_server().await;
    let read = "runtime:\n  scripts:\n    - type: before-request\n      code: bru.interpolate('{{tok}}')\n";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("a", 1, &format!("{}/a", s.uri()), read)),
    ]);
    let scripts = || RunOptions::new().scripts(ScriptOptions::new().enabled(true));
    let secret = || ContextOptions::new().sensitive_env_var("tok", "s3cret");
    let (plain, _) = run_with(dir.path(), scripts(), secret(), |_, _| {}).await;
    assert_eq!(plain.results[0].script_sensitive_values(), ["s3cret"]);
    let opts = scripts().iterations(csv("tok\nopen-value\n"));
    let (shadowed, _) = run_with(dir.path(), opts, secret(), |_, _| {}).await;
    assert!(
        shadowed.results[0].error.is_none(),
        "{:?}",
        shadowed.results[0].error
    );
    assert!(shadowed.results[0].script_sensitive_values().is_empty());
}

#[tokio::test]
async fn bail_stops_every_later_iteration() {
    let s = ok_server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("a", 1, &format!("{}/a", s.uri()), FAIL)),
        ("b.yml", &req("b", 2, &format!("{}/b", s.uri()), "")),
    ]);
    let opts = RunOptions::new().bail(true).iterations(csv("id\n1\n2\n"));
    let (report, _) = run_with(dir.path(), opts, ContextOptions::new(), |_, _| {}).await;
    let outcomes: Vec<Outcome> = report.results.iter().map(|r| r.outcome).collect();
    assert_eq!(outcomes, [Outcome::Failed, Outcome::Skipped]);
    assert_eq!(served(&s).await, ["/a"]);
}

#[tokio::test]
async fn a_script_stop_ends_every_iteration() {
    let s = ok_server().await;
    let stop = "runtime:\n  scripts:\n    - type: after-response\n      code: bru.runner.stopExecution()\n";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("a", 1, &format!("{}/a", s.uri()), stop)),
        ("b.yml", &req("b", 2, &format!("{}/b", s.uri()), "")),
    ]);
    let opts = RunOptions::new()
        .scripts(ScriptOptions::new().enabled(true))
        .iterations(csv("id\n1\n2\n"));
    let (report, _) = run_with(dir.path(), opts, ContextOptions::new(), |_, _| {}).await;
    assert_eq!(report.results.len(), 2, "iteration 2 never starts");
    assert_eq!(report.results[1].outcome, Outcome::Skipped);
    assert_eq!(served(&s).await, ["/a"]);
}

#[tokio::test]
async fn the_delay_spaces_sent_requests_across_iterations_but_not_after_the_last() {
    let s = ok_server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("a", 1, &format!("{}/a", s.uri()), "")),
    ]);
    let t = Instant::now();
    let opts = RunOptions::new()
        .delay(Duration::from_millis(400))
        .iterations(csv("id\n1\n2\n"));
    run_with(dir.path(), opts, ContextOptions::new(), |_, _| {}).await;
    assert!(
        t.elapsed() >= Duration::from_millis(400),
        "{:?}",
        t.elapsed()
    );
    let t = Instant::now();
    let opts = RunOptions::new().delay(Duration::from_secs(5));
    run_with(dir.path(), opts, ContextOptions::new(), |_, _| {}).await;
    assert!(
        t.elapsed() < Duration::from_secs(3),
        "no pause before the first or after the last"
    );
}

#[tokio::test]
async fn cancel_during_a_delay_ends_the_run_at_once() {
    let s = ok_server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("a", 1, &format!("{}/a", s.uri()), "")),
        ("b.yml", &req("b", 2, &format!("{}/b", s.uri()), "")),
    ]);
    let t = Instant::now();
    let opts = RunOptions::new().delay(Duration::from_secs(30));
    // The cancel arrives from another task while the 30 s pause is under way, so only a
    // pause that watches the handle lets the run return early.
    let on_finish = |index: usize, cancel: &CancelHandle| {
        if index == 0 {
            let cancel = cancel.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(200)).await;
                cancel.cancel();
            });
        }
    };
    let (report, _) = run_with(dir.path(), opts, ContextOptions::new(), on_finish).await;
    assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
    let outcomes: Vec<Outcome> = report.results.iter().map(|r| r.outcome).collect();
    assert_eq!(outcomes, [Outcome::Passed, Outcome::Skipped]);
    assert_eq!(served(&s).await, ["/a"]);
}

fn one_request(s: &MockServer, extra: &str) -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &req(
                "a",
                1,
                &format!("{}/a/{{{{iteration}}}}/of/{{{{iterationCount}}}}", s.uri()),
                extra,
            ),
        ),
    ])
}

#[tokio::test]
async fn a_count_without_data_runs_that_many_passes_and_exposes_the_index() {
    let s = ok_server().await;
    let dir = one_request(&s, "");
    let opts = RunOptions::new().iteration_count(3);
    let (report, events) = run_with(dir.path(), opts, ContextOptions::new(), |_, _| {}).await;
    assert_eq!(served(&s).await, ["/a/0/of/3", "/a/1/of/3", "/a/2/of/3"]);
    let marks: Vec<Option<usize>> = report.results.iter().map(|r| r.iteration).collect();
    assert_eq!(marks, [Some(0), Some(1), Some(2)]);
    assert!(
        events.contains(&"iteration 2 of 3".to_owned()),
        "{events:?}"
    );
}

#[tokio::test]
async fn a_count_below_the_row_count_caps_the_rows() {
    let s = ok_server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &req(
                "a",
                1,
                &format!("{}/a/{{{{id}}}}/{{{{iterationCount}}}}", s.uri()),
                "",
            ),
        ),
    ]);
    let opts = RunOptions::new()
        .iterations(csv("id\n1\n2\n3\n"))
        .iteration_count(2);
    let (report, _) = run_with(dir.path(), opts, ContextOptions::new(), |_, _| {}).await;
    assert_eq!(served(&s).await, ["/a/1/2", "/a/2/2"]);
    assert_eq!(report.results.len(), 2);
}

#[tokio::test]
async fn a_count_above_the_row_count_never_repeats_a_row() {
    let s = ok_server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &req("a", 1, &format!("{}/a/{{{{id}}}}", s.uri()), ""),
        ),
    ]);
    let opts = RunOptions::new()
        .iterations(csv("id\n1\n2\n"))
        .iteration_count(5);
    let (report, _) = run_with(dir.path(), opts, ContextOptions::new(), |_, _| {}).await;
    assert_eq!(served(&s).await, ["/a/1", "/a/2"]);
    assert_eq!(report.results.len(), 2);
}

#[tokio::test]
async fn data_alone_exposes_the_index_and_the_row_count() {
    let s = ok_server().await;
    let dir = one_request(&s, "");
    let opts = RunOptions::new().iterations(csv("id\n1\n2\n"));
    run_with(dir.path(), opts, ContextOptions::new(), |_, _| {}).await;
    assert_eq!(served(&s).await, ["/a/0/of/2", "/a/1/of/2"]);
}

#[tokio::test]
async fn a_data_column_named_iteration_wins_over_the_index() {
    let s = ok_server().await;
    let dir = one_request(&s, "");
    let opts = RunOptions::new().iterations(csv("iteration\nfirst\nsecond\n"));
    run_with(dir.path(), opts, ContextOptions::new(), |_, _| {}).await;
    assert_eq!(served(&s).await, ["/a/first/of/2", "/a/second/of/2"]);
}

#[tokio::test]
async fn a_script_reads_the_index() {
    let s = ok_server().await;
    let read = "runtime:\n  scripts:\n    - type: before-request\n      code: bru.setVar('seen', bru.interpolate('{{iteration}}/{{iterationCount}}'))\n";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &req("a", 1, &format!("{}/a/{{{{seen}}}}", s.uri()), read),
        ),
    ]);
    let opts = RunOptions::new()
        .scripts(ScriptOptions::new().enabled(true))
        .iteration_count(2);
    run_with(dir.path(), opts, ContextOptions::new(), |_, _| {}).await;
    assert_eq!(served(&s).await, ["/a/0/2", "/a/1/2"]);
}

#[tokio::test]
async fn a_count_of_zero_is_raised_to_one_pass() {
    let s = ok_server().await;
    let dir = one_request(&s, "");
    let opts = RunOptions::new().iteration_count(0);
    let (report, _) = run_with(dir.path(), opts, ContextOptions::new(), |_, _| {}).await;
    assert_eq!(served(&s).await, ["/a/0/of/1"]);
    assert_eq!(report.results[0].iteration, Some(0));
}

#[tokio::test]
async fn a_plain_run_is_iteration_zero_of_one_without_an_iteration_mark() {
    let s = ok_server().await;
    let dir = one_request(&s, "");
    let (report, events) = run_with(
        dir.path(),
        RunOptions::new(),
        ContextOptions::new(),
        |_, _| {},
    )
    .await;
    assert_eq!(served(&s).await, ["/a/0/of/1"]);
    assert_eq!(report.results[0].iteration, None);
    assert!(
        !events.iter().any(|e| e.starts_with("iteration")),
        "{events:?}"
    );
}

#[tokio::test]
async fn a_standalone_send_leaves_the_iteration_variables_undefined() {
    let s = ok_server().await;
    let dir = one_request(&s, "");
    let (report, _) = run_with(
        dir.path(),
        RunOptions::new().execution_mode(ExecutionMode::Standalone),
        ContextOptions::new(),
        |_, _| {},
    )
    .await;
    assert!(
        matches!(
            &report.results[0].error,
            Some(RunError::Resolve(ResolveError::UndefinedVariable { name, .. })) if name == "iteration"
        ),
        "{:?}",
        report.results[0].error
    );
    assert!(served(&s).await.is_empty(), "nothing is sent");
}
