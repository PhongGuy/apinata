//! `apinata run` on a GraphQL subscription: stream flags, -o, legacy servers, report and exit code.

use crate::common::ws_server::{COMPLETE, raw_server, tick, ws_server};
use crate::common::{ROOT, apinata, check, write_tree};
use std::time::{Duration, Instant};

const TRANSPORT: &str = "graphql-transport-ws";

fn sub_item(url: &str, assertions: &str) -> String {
    format!(
        "info:\n  name: S\n  type: graphql\n  seq: 1\ngraphql:\n  method: POST\n  url: \"{url}\"\n  body:\n    query: \"subscription {{ tick }}\"\nruntime:\n  assertions:\n{assertions}"
    )
}

fn ticks(legacy: bool, n: u64) -> Vec<String> {
    (1..=n).map(|i| tick(legacy, i)).collect()
}

/// `apinata run --format json <extra>` on one subscription item: exit code, report, stderr.
fn run_with(url: &str, assertions: &str, extra: &[&str]) -> (i32, serde_json::Value, String) {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("s.yml", &sub_item(url, assertions)),
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
async fn a_subscription_runs_until_the_server_completes() {
    let mut frames = ticks(false, 2);
    frames.push(COMPLETE.to_owned());
    let port = ws_server(TRANSPORT, frames, Duration::from_millis(20)).await;
    let url = format!("http://127.0.0.1:{port}/graphql");
    let (code, report, stderr) = run_with(&url, &check("res.body", "contains", "tick"), &[]);
    assert_eq!(code, 0, "{report:#} {stderr}");
    let row = &report["results"][0];
    assert_eq!(row["status"], 101);
    assert_eq!(row["outcome"], "passed");
    assert_eq!(row["streamEnd"], "closed");
    assert!(!stderr.contains("stream ended"), "{stderr}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_event_limit_ends_a_subscription_with_a_note() {
    let port = ws_server(TRANSPORT, ticks(false, 20), Duration::from_millis(20)).await;
    let url = format!("http://127.0.0.1:{port}/graphql");
    let (code, report, stderr) = run_with(&url, "", &["--stream-max-events", "2"]);
    assert_eq!(code, 0, "{report:#}");
    assert_eq!(report["results"][0]["streamEnd"], "event_limit");
    assert!(
        stderr.contains("stream ended at the limit (2 events)"),
        "{stderr}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_time_limit_ends_an_endless_subscription() {
    let port = ws_server(TRANSPORT, ticks(false, 100), Duration::from_millis(100)).await;
    let url = format!("http://127.0.0.1:{port}/graphql");
    let started = Instant::now();
    let (code, report, stderr) = run_with(
        &url,
        "",
        &["--stream-max-time", "1", "--stream-max-events", "0"],
    );
    assert_eq!(code, 0, "{report:#}");
    assert_eq!(report["results"][0]["streamEnd"], "time_limit");
    assert!(
        stderr.contains("stream ended at the limit (1 s)"),
        "{stderr}"
    );
    assert!(started.elapsed() < Duration::from_secs(6));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_legacy_server_and_output_save_the_transcript() {
    let frames = vec![tick(true, 1), COMPLETE.to_owned()];
    let port = ws_server("graphql-ws", frames, Duration::from_millis(10)).await;
    let url = format!("http://127.0.0.1:{port}/graphql");
    let out_dir = tempfile::tempdir().unwrap();
    let file = out_dir.path().join("sub.txt");
    let (code, report, _) = run_with(&url, "", &["-o", file.to_str().unwrap()]);
    assert_eq!(code, 0, "{report:#}");
    assert_eq!(report["results"][0]["streamEnd"], "closed");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "event: next\ndata: {\"data\":{\"tick\":1}}\n\nevent: complete\ndata:\n\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_handshake_exits_2_and_names_the_ws_url() {
    let port = raw_server("HTTP/1.1 404 Not Found\r\ncontent-length: 4\r\n\r\nnope").await;
    let url = format!("http://127.0.0.1:{port}/graphql");
    let (code, report, _) = run_with(&url, "", &[]);
    assert_eq!(code, 2, "{report:#}");
    let row = &report["results"][0];
    assert_eq!(row["outcome"], "errored");
    assert_eq!(row["status"], 404);
    assert_eq!(
        row["error"],
        format!("WebSocket handshake to ws://127.0.0.1:{port}/graphql failed: 404 Not Found")
    );
    assert_eq!(report["summary"]["errored"], 1);
}
