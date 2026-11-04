//! Runner sends, script sends and console lines reach the log when the send ends.

use crate::script_support;
use crate::session_log_support;

use apinata_core::{
    CancelHandle, Collection, ConsoleLevel, ContextOptions, ExecOptions, ExecutionMode, Executor,
    ItemPath, Outcome, RequestFile, ResolvedRequest, RunError, ScriptPhase, SendSource, SessionLog,
    VarContext, send_one,
};
use script_support::{ROOT, on, script, scripted, send_full, write_tree};
use session_log_support::{entries, lines, only};
use std::sync::Arc;
use std::time::Duration;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn server() -> MockServer {
    let s = MockServer::start().await;
    for route in ["/main", "/s", "/t", "/fast"] {
        Mock::given(path(route))
            .respond_with(ResponseTemplate::new(200))
            .mount(&s)
            .await;
    }
    Mock::given(path("/slow"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(5)))
        .mount(&s)
        .await;
    s
}

fn plain(name: &str, url: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n"
    )
}

fn sources(log: &SessionLog) -> Vec<SendSource> {
    entries(log).iter().map(|e| e.source).collect()
}

async fn sent(dir: &std::path::Path, opts: apinata_core::RunOptions) -> SessionLog {
    let log = SessionLog::new();
    send_full(
        dir,
        "r.yml",
        opts,
        ContextOptions::new(),
        ExecOptions::new().session_log(log.clone()),
    )
    .await;
    log
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_source_follows_the_mode_and_the_nesting() {
    let s = server().await;
    let main = format!("{}/main", s.uri());
    let bare = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &plain("R", &main))]);
    assert_eq!(
        sources(&sent(bare.path(), on().execution_mode(ExecutionMode::Standalone)).await),
        [SendSource::Request]
    );
    assert_eq!(
        sources(&sent(bare.path(), on()).await),
        [SendSource::Runner]
    );
    assert_eq!(
        sources(&sent(bare.path(), on().execution_mode(ExecutionMode::Cli)).await),
        [SendSource::Runner]
    );
    let code = format!(
        "await bru.sendRequest({{ method: 'get', url: '{}/s' }});\nawait bru.runRequest('t');",
        s.uri()
    );
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("t.yml", &plain("T", &format!("{}/t", s.uri()))),
        (
            "r.yml",
            &scripted("R", &main, &script("before-request", &code), ""),
        ),
    ]);
    let log = sent(dir.path(), on()).await;
    assert_eq!(
        sources(&log),
        [SendSource::Script, SendSource::Script, SendSource::Runner]
    );
    assert!(entries(&log)[1].url.ends_with("/t"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn console_lines_carry_the_request_name_phase_and_level() {
    let s = server().await;
    let scripts = script("before-request", "console.log('pre')")
        + &script("after-response", "console.warn('post')");
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted("Get user", &format!("{}/main", s.uri()), &scripts, ""),
        ),
    ]);
    let log = sent(dir.path(), on()).await;
    let got: Vec<_> = lines(&log)
        .iter()
        .map(|l| (l.request.clone(), l.phase.unwrap(), l.level, l.text.clone()))
        .collect();
    assert_eq!(
        got,
        [
            (
                "Get user".to_owned(),
                ScriptPhase::PreRequest,
                ConsoleLevel::Log,
                "pre".to_owned()
            ),
            (
                "Get user".to_owned(),
                ScriptPhase::PostResponse,
                ConsoleLevel::Warn,
                "post".to_owned()
            ),
        ]
    );
    assert_eq!(lines(&log)[0].at, lines(&log)[1].at);
    assert_eq!(entries(&log).len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_guard_refusal_publishes_console_lines_and_no_entry() {
    let s = server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/main", s.uri()),
                &script("before-request", "console.log('ran')"),
                "",
            ),
        ),
    ]);
    let refuse = Arc::new(|_: &ResolvedRequest| Some("not here".to_owned()));
    let log = SessionLog::new();
    let (result, _) = send_full(
        dir.path(),
        "r.yml",
        on().guard(refuse),
        ContextOptions::new(),
        ExecOptions::new().session_log(log.clone()),
    )
    .await;
    assert!(
        matches!(result.error, Some(RunError::Refused { .. })),
        "{:?}",
        result.error
    );
    assert!(entries(&log).is_empty());
    assert_eq!(
        lines(&log)
            .iter()
            .map(|l| l.text.clone())
            .collect::<Vec<_>>(),
        ["ran"]
    );
}

/// The send at `r.yml` under `dir` on an executor recording into `log`.
fn parts(dir: &std::path::Path, log: &SessionLog) -> (VarContext, Arc<Executor>, RequestFile) {
    let c = Collection::open(dir).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let exec =
        Arc::new(Executor::new(ExecOptions::new().session_log(log.clone())).expect("client"));
    (
        ctx,
        exec,
        RequestFile::load(dir.join("r.yml")).expect("loads"),
    )
}

async fn received(s: &MockServer, n: usize) {
    while s.received_requests().await.expect("recorded").len() < n {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancelled_send_publishes_its_console_and_a_cancelled_entry() {
    let s = server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/slow", s.uri()),
                &script("before-request", "console.log('before')"),
                "",
            ),
        ),
    ]);
    let log = SessionLog::new();
    let (ctx, exec, request) = parts(dir.path(), &log);
    let cancel = CancelHandle::new();
    let opts = on();
    let item = ItemPath::new("r.yml").expect("path");
    let (result, ()) = tokio::join!(
        send_one(item, &request, &ctx, &exec, &opts, &cancel),
        async {
            received(&s, 1).await;
            cancel.cancel();
        }
    );
    assert_eq!(result.outcome, Outcome::Cancelled);
    assert_eq!(only(&log).error.as_deref(), Some("cancelled"));
    assert_eq!(
        lines(&log)
            .iter()
            .map(|l| l.text.clone())
            .collect::<Vec<_>>(),
        ["before"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dropped_send_publishes_what_it_staged() {
    let s = server().await;
    let code = format!(
        "await bru.sendRequest({{ method: 'get', url: '{0}/fast' }});\nawait bru.sendRequest({{ method: 'get', url: '{0}/slow' }});",
        s.uri()
    );
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/main", s.uri()),
                &script("before-request", &code),
                "",
            ),
        ),
    ]);
    let log = SessionLog::new();
    let (ctx, exec, request) = parts(dir.path(), &log);
    let item = ItemPath::new("r.yml").expect("path");
    let (opts, cancel) = (on(), CancelHandle::new());
    tokio::select! {
        _ = send_one(item, &request, &ctx, &exec, &opts, &cancel) => panic!("the slow send finished"),
        () = received(&s, 2) => {}
    }
    assert!(
        entries(&log)
            .iter()
            .any(|e| e.source == SendSource::Script && e.url.ends_with("/fast")),
        "{:?}",
        entries(&log)
    );
}
