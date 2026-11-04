//! `apinata run` against a local event stream: limits, the stderr note and streamEnd.

use crate::common::sse_server::{Script, Step, event, slow_sse_server};
use crate::common::{ROOT, apinata, check, get, write_tree};
use std::time::{Duration, Instant};

fn ticks(n: usize, gap_ms: u64) -> Vec<Step> {
    (0..n)
        .flat_map(|i| {
            [
                event(&format!("e{i}")),
                Step::Pause(Duration::from_millis(gap_ms)),
            ]
        })
        .collect()
}

fn run_with(url: &str, assertions: &str, extra: &[&str]) -> (i32, serde_json::Value, String) {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &get("A", 1, url, assertions)),
    ]);
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .args(["--format", "json"])
        .args(extra)
        .output()
        .unwrap();
    let report = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout)));
    (
        out.status.code().unwrap(),
        report,
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_event_limit_ends_the_stream_normally_with_a_note() {
    let s = slow_sse_server(Script::sse(ticks(10, 20))).await;
    let (code, report, stderr) = run_with(
        &s.url("/"),
        &check("res.status", "eq", "200"),
        &["--stream-max-events", "3"],
    );
    assert_eq!(code, 0, "{report:#}");
    assert_eq!(report["results"][0]["streamEnd"], "event_limit");
    assert!(
        stderr.contains("stream ended at the limit (3 events)"),
        "{stderr}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_time_limit_ends_an_endless_stream() {
    let s = slow_sse_server(Script::sse(ticks(100, 100))).await;
    let started = Instant::now();
    let (code, report, stderr) = run_with(
        &s.url("/"),
        "",
        &["--stream-max-time", "1", "--stream-max-events", "0"],
    );
    assert_eq!(code, 0);
    assert_eq!(report["results"][0]["streamEnd"], "time_limit");
    assert!(
        stderr.contains("stream ended at the limit (1 s)"),
        "{stderr}"
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_assertion_on_an_event_that_never_came_fails() {
    let s = slow_sse_server(Script::sse(ticks(10, 20))).await;
    let (code, report, _) = run_with(
        &s.url("/"),
        &check("res.body", "contains", "e9"),
        &["--stream-max-events", "2"],
    );
    assert_eq!(code, 1, "{report:#}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stream_the_server_closes_has_no_note_and_an_ordinary_response_no_key() {
    let mut steps = ticks(2, 20);
    steps.push(Step::Finish);
    let s = slow_sse_server(Script::sse(steps)).await;
    let (code, report, stderr) = run_with(&s.url("/"), "", &[]);
    assert_eq!(code, 0);
    assert_eq!(report["results"][0]["streamEnd"], "closed");
    assert!(!stderr.contains("stream ended"), "{stderr}");

    let plain = slow_sse_server(Script::with_type(
        "text/plain",
        vec![Step::Chunk("hi".into()), Step::Finish],
    ))
    .await;
    let (_, report, _) = run_with(&plain.url("/"), "", &[]);
    assert!(report["results"][0].get("streamEnd").is_none());
}

/// A plain chunked body of `chunks` 64 KiB pieces that stays open afterwards when `finish` is false.
async fn plain_body(chunks: usize, finish: bool) -> crate::common::sse_server::SlowServer {
    let piece = "z".repeat(64 * 1024);
    let mut steps: Vec<Step> = (0..chunks).map(|_| Step::Chunk(piece.clone())).collect();
    if finish {
        steps.push(Step::Chunk("END".into()));
        steps.push(Step::Finish);
    }
    slow_sse_server(Script::with_type("text/plain", steps)).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_body_cut_at_the_default_cap_prints_one_note() {
    let s = plain_body(200, true).await;
    let (code, _, stderr) = run_with(&s.url("/"), "", &[]);
    assert_eq!(code, 0);
    assert_eq!(
        stderr
            .matches("body truncated at 10 MiB (raise with --max-body-bytes)")
            .count(),
        1,
        "{stderr}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zero_lifts_the_body_cap_and_a_small_cap_is_named_in_the_note() {
    let s = plain_body(200, true).await;
    let (_, report, stderr) = run_with(
        &s.url("/"),
        &check("res.body", "contains", "END"),
        &["--max-body-bytes", "0"],
    );
    assert!(!stderr.contains("truncated"), "{stderr}");
    assert_eq!(report["summary"]["failed"], 0, "{report:#}");
    assert_eq!(report["summary"]["skipped_assertions"], 0, "{report:#}");

    let (_, _, stderr) = run_with(&s.url("/"), "", &["--max-body-bytes", "1000"]);
    assert!(stderr.contains("body truncated at 1000 bytes"), "{stderr}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_endless_plain_body_still_ends_at_the_cap() {
    let s = plain_body(100, false).await;
    let started = Instant::now();
    let (code, _, stderr) = run_with(&s.url("/"), "", &["--max-body-bytes", "1048576"]);
    assert_eq!(code, 0);
    assert!(stderr.contains("body truncated at 1 MiB"), "{stderr}");
    assert!(started.elapsed() < Duration::from_secs(10));
}
