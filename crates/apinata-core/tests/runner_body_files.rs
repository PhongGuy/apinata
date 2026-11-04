//! A run resolves a body file per request and reports byte progress as it goes.

use crate::common::body_server::{Reply, Tail, body_server, pattern};
use crate::common::write_tree;
use apinata_core::{
    CancelHandle, Collection, ContextOptions, ExecOptions, Executor, ItemPath, Outcome, RunEvent,
    RunOptions, VarContext, run,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

fn get(name: &str, seq: u32, url: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{url}\"\n"
    )
}

/// Runs the files with `opts`; returns the event lines and the outcomes.
async fn go(files: &[(&str, &str)], opts: RunOptions) -> (Vec<String>, Vec<Outcome>) {
    let dir = write_tree(files);
    let c = Collection::open(dir.path()).unwrap();
    let plan = c.plan(dir.path()).unwrap();
    let ctx = VarContext::new(&c, ContextOptions::new()).unwrap();
    let exec = Arc::new(Executor::new(ExecOptions::new()).unwrap());
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&log);
    let report = run(
        plan,
        ctx,
        exec,
        opts,
        CancelHandle::new(),
        Box::new(move |e| {
            let line = match e {
                RunEvent::ResponseProgress {
                    index,
                    received,
                    total,
                } => format!("progress {index} {received} {total:?}"),
                RunEvent::ResponseEnded { index } => format!("ended {index}"),
                RunEvent::RequestFinished { index, .. } => format!("finish {index}"),
                _ => return,
            };
            sink.lock().unwrap().push(line);
        }),
    )
    .await;
    let lines = log.lock().unwrap().clone();
    (lines, report.results.iter().map(|r| r.outcome).collect())
}

fn into(dir: &Path) -> impl Fn(usize, &ItemPath) -> Option<PathBuf> + Send + Sync + 'static {
    let dir = dir.to_path_buf();
    move |index, _| Some(dir.join(format!("{index}.bin")))
}

#[tokio::test]
async fn two_requests_get_two_files() {
    let s = body_server(vec![
        ("/a", Reply::plain(pattern(1000))),
        ("/b", Reply::plain(pattern(2000))),
    ])
    .await;
    let out = tempfile::tempdir().unwrap();
    let (a, b) = (get("a", 1, &s.url("/a")), get("b", 2, &s.url("/b")));
    let (_, outcomes) = go(
        &[("opencollection.yml", ROOT), ("a.yml", &a), ("b.yml", &b)],
        RunOptions::new().body_files(into(out.path())),
    )
    .await;
    assert_eq!(outcomes, [Outcome::Passed, Outcome::Passed]);
    assert_eq!(
        std::fs::read(out.path().join("0.bin")).unwrap(),
        pattern(1000)
    );
    assert_eq!(
        std::fs::read(out.path().join("1.bin")).unwrap(),
        pattern(2000)
    );
}

#[tokio::test]
async fn a_resolver_returning_none_writes_nothing() {
    let s = body_server(vec![("/a", Reply::plain(pattern(1000)))]).await;
    let a = get("a", 1, &s.url("/a"));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    let (_, outcomes) = go(
        &[("opencollection.yml", ROOT), ("a.yml", &a)],
        RunOptions::new().body_files(move |i, p| {
            log.lock().unwrap().push((i, p.to_string()));
            None
        }),
    )
    .await;
    assert_eq!(outcomes, [Outcome::Passed]);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn progress_arrives_before_the_response_ends() {
    let s = body_server(vec![("/a", Reply::plain(pattern(300_000)))]).await;
    let out = tempfile::tempdir().unwrap();
    let a = get("a", 1, &s.url("/a"));
    let (lines, _) = go(
        &[("opencollection.yml", ROOT), ("a.yml", &a)],
        RunOptions::new().body_files(into(out.path())),
    )
    .await;
    let first = lines
        .iter()
        .position(|l| l.starts_with("progress 0 "))
        .unwrap();
    let ended = lines.iter().position(|l| l == "ended 0").unwrap();
    assert!(first < ended, "{lines:?}");
    assert!(
        lines.iter().any(|l| l == "progress 0 300000 Some(300000)"),
        "{lines:?}"
    );
}

#[tokio::test]
async fn a_failed_request_leaves_no_file() {
    let s = body_server(vec![("/a", Reply::short(pattern(1000), 5000, Tail::Close))]).await;
    let out = tempfile::tempdir().unwrap();
    let a = get("a", 1, &s.url("/a"));
    let (_, outcomes) = go(
        &[("opencollection.yml", ROOT), ("a.yml", &a)],
        RunOptions::new().body_files(into(out.path())),
    )
    .await;
    assert_eq!(outcomes, [Outcome::Errored]);
    assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 0);
}
