//! A cancel during the pause between attempts keeps the attempt before it, and the session log
//! numbers the attempts.

use crate::script_support::{ROOT, on, write_tree};
use crate::session_log_support::entries;

use apinata_core::{
    CancelHandle, Collection, ContextOptions, ExecOptions, Executor, GlobalProxy, NetworkSettings,
    Outcome, PacSource, RequestResult, RunError, RunEvent, SessionLog, VarContext, Warning, run,
};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn get(url: &str) -> String {
    format!("info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n")
}

/// Runs `r.yml` with a 30 s retry pause, cancelling once the run reports the pause is starting.
async fn cancelled_in_the_pause(url: &str, exec: ExecOptions) -> RequestResult {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &get(url))]);
    let c = Collection::open(dir.path()).expect("opens");
    let plan = c.plan(dir.path()).expect("plan");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let exec = Arc::new(Executor::new(exec).expect("client"));
    let cancel = CancelHandle::new();
    let opts = on().retries(2).retry_delay(Duration::from_secs(30));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let on_event = Box::new(move |e: RunEvent| {
        if let RunEvent::Retrying { .. } = e {
            let _ = tx.send(e);
        }
    });
    let task = tokio::spawn(run(plan, ctx, exec, opts, cancel.clone(), on_event));
    // The event precedes the pause and the earlier attempt is fully read, so the cancel lands in it.
    let event = tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("a retry is announced")
        .expect("channel open");
    assert!(matches!(event, RunEvent::Retrying { delay, .. } if delay == Duration::from_secs(30)));
    cancel.cancel();
    let report = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("run ends at once")
        .expect("joins");
    assert_eq!(report.results.len(), 1);
    (*report.results[0]).clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancel_after_a_502_keeps_the_response_warnings_and_attempts() {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(502))
        .mount(&s)
        .await;
    // A PAC script that throws falls back to a direct send and leaves a warning.
    let dir = tempfile::TempDir::new().expect("tempdir");
    let file = dir.path().join("proxy.pac");
    std::fs::write(
        &file,
        "function FindProxyForURL(u, h) { throw new Error('x'); }",
    )
    .expect("pac");
    let source = PacSource::parse(file.to_str().expect("utf-8"), None).expect("source");
    let exec =
        ExecOptions::new().network(NetworkSettings::new().proxy(GlobalProxy::Pac { source }));
    let r = cancelled_in_the_pause(&s.uri(), exec).await;
    assert_eq!(r.outcome, Outcome::Cancelled, "{r:?}");
    assert_eq!(r.attempts(), 1);
    assert_eq!(r.response.as_ref().map(|x| x.status()), Some(502));
    assert!(
        r.post_response_vars.is_empty() && r.assertions.is_empty(),
        "{r:?}"
    );
    assert!(
        r.warnings
            .iter()
            .any(|w| matches!(w, Warning::ProxyAutoConfig(_))),
        "{r:?}"
    );
    assert!(matches!(
        r.error,
        Some(RunError::Exec(apinata_core::ExecError::Cancelled))
    ));
    assert!(r.last_attempt_error.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancel_after_a_refused_connection_keeps_that_failure() {
    // A listener that drops each connection it accepts: the send fails in transit.
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let url = format!("http://127.0.0.1:{}", l.local_addr().expect("addr").port());
    std::thread::spawn(move || {
        while let Ok((conn, _)) = l.accept() {
            drop(conn);
        }
    });
    let r = cancelled_in_the_pause(&url, ExecOptions::new()).await;
    assert_eq!(r.outcome, Outcome::Cancelled, "{r:?}");
    assert_eq!(r.attempts(), 1);
    assert!(r.response.is_none());
    assert!(
        matches!(r.last_attempt_error, Some(RunError::Exec(ref e)) if !matches!(e, apinata_core::ExecError::Cancelled)),
        "{r:?}"
    );
}

/// A server answering `fail` times with `status`, then 200, and the log of a run against it.
async fn logged_run(fail: u64, retries: u32) -> Vec<Option<(u32, u32)>> {
    let s = MockServer::start().await;
    if fail > 0 {
        Mock::given(any())
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(fail)
            .mount(&s)
            .await;
    }
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &get(&s.uri()))]);
    let log = SessionLog::new();
    let c = Collection::open(dir.path()).expect("opens");
    let plan = c.plan(dir.path()).expect("plan");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let exec =
        Arc::new(Executor::new(ExecOptions::new().session_log(log.clone())).expect("client"));
    run(
        plan,
        ctx,
        exec,
        on().retries(retries),
        CancelHandle::new(),
        Box::new(|_| {}),
    )
    .await;
    entries(&log).iter().map(|e| e.attempt).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retried_sends_are_numbered_in_the_session_log() {
    assert_eq!(
        logged_run(2, 2).await,
        [Some((1, 3)), Some((2, 3)), Some((3, 3))]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_send_without_retries_has_no_attempt_number() {
    assert_eq!(logged_run(0, 0).await, [None]);
}

/// The `(index, attempt, of, delay)` of each `Retrying` event of a run against a server that
/// answers 503 `fail` times and then 200.
async fn retrying_events(fail: u64, retries: u32) -> Vec<(usize, u32, u32, Duration)> {
    let s = MockServer::start().await;
    if fail > 0 {
        Mock::given(any())
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(fail)
            .mount(&s)
            .await;
    }
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &get(&s.uri()))]);
    let c = Collection::open(dir.path()).expect("opens");
    let plan = c.plan(dir.path()).expect("plan");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let exec = Arc::new(Executor::new(ExecOptions::new()).expect("client"));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let on_event = Box::new(move |e: RunEvent| {
        if let RunEvent::Retrying {
            index,
            attempt,
            of,
            delay,
            ..
        } = e
        {
            sink.lock().expect("lock").push((index, attempt, of, delay));
        }
    });
    let opts = on().retries(retries).retry_delay(Duration::ZERO);
    run(plan, ctx, exec, opts, CancelHandle::new(), on_event).await;
    seen.lock().expect("lock").clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_retry_is_announced_once_before_its_pause() {
    assert_eq!(
        retrying_events(2, 2).await,
        [(0, 2, 3, Duration::ZERO), (0, 3, 3, Duration::ZERO)]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_run_without_retries_announces_none() {
    assert!(retrying_events(0, 0).await.is_empty());
    assert!(retrying_events(0, 2).await.is_empty());
}
