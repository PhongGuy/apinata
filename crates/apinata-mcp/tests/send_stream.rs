//! send_request and run_folder against a local event stream.

use crate::common::sse_server::{Script, Step, event, slow_sse_server};
use crate::common::{ROOT, TestServer, request};
use serde_json::json;
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

async fn start(url: &str) -> TestServer {
    TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request("r", 1, "GET", url)),
    ])
    .await
}

#[tokio::test]
async fn the_event_limit_ends_the_stream_and_the_result_says_so() {
    let s = slow_sse_server(Script::sse(ticks(10, 20))).await;
    let server = start(&s.url("/")).await;
    let r = server
        .ok(
            "send_request",
            json!({"path": "r.yml", "stream_max_events": 2}),
        )
        .await;
    assert_eq!(r["status"], 200);
    assert_eq!(r["stream_end"], "event_limit");
    assert_eq!(r["events"], 2);
    assert!(r["body"]["text"].as_str().unwrap().contains("data: e1"));
}

#[tokio::test]
async fn out_of_range_values_are_clamped() {
    let s = slow_sse_server(Script::sse(ticks(50, 200))).await;
    let server = start(&s.url("/")).await;
    let started = Instant::now();
    // 0 seconds clamps to 1 second; 0 events clamps to 1 event, so the event limit ends it first.
    let r = server
        .ok(
            "send_request",
            json!({"path": "r.yml", "stream_max_seconds": 0, "stream_max_events": 0}),
        )
        .await;
    assert_eq!(r["events"], 1);
    assert_eq!(r["stream_end"], "event_limit");
    assert!(started.elapsed() < Duration::from_secs(3));

    let r = server
        .ok(
            "send_request",
            json!({"path": "r.yml", "stream_max_seconds": 0}),
        )
        .await;
    assert_eq!(r["stream_end"], "time_limit");
    assert!(started.elapsed() < Duration::from_secs(8));
}

#[tokio::test]
async fn a_closed_stream_and_an_ordinary_response_differ() {
    let s = slow_sse_server(Script::sse(vec![event("a"), Step::Finish])).await;
    let server = start(&s.url("/")).await;
    let r = server.ok("send_request", json!({"path": "r.yml"})).await;
    assert_eq!(r["stream_end"], "closed");
    assert_eq!(r["events"], 1);

    let plain = slow_sse_server(Script::with_type(
        "text/plain",
        vec![Step::Chunk("hi".into()), Step::Finish],
    ))
    .await;
    let server = start(&plain.url("/")).await;
    let r = server.ok("send_request", json!({"path": "r.yml"})).await;
    assert!(r.get("stream_end").is_none() && r.get("events").is_none());
}

#[tokio::test]
async fn run_folder_reports_the_stream_end_per_request() {
    let s = slow_sse_server(Script::sse(ticks(10, 20))).await;
    let server = start(&s.url("/")).await;
    let r = server
        .ok("run_folder", json!({"path": "", "stream_max_events": 2}))
        .await;
    assert_eq!(r["results"][0]["stream_end"], "event_limit");
    assert_eq!(r["results"][0]["events"], 2);
}

#[tokio::test]
async fn unknown_parameters_are_still_refused() {
    let server = start("http://127.0.0.1:9/").await;
    let outcome = server
        .try_call(
            "send_request",
            json!({"path": "r.yml", "stream_max_minutes": 1}),
        )
        .await;
    assert!(
        outcome.as_ref().map_or(true, |r| r.is_error == Some(true)),
        "{outcome:?}"
    );
}
