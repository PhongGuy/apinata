//! Bounds on a subscription: event and time limits, the body cap and the non-streaming bound.

use crate::common::ROOT;
use crate::common::ws_server::{
    QUERY, VARS, WsScript, graphql_request, record, recording, sent, sub, tick, ws_exec, ws_server,
};
use apinata_core::HttpVersionPolicy::Auto;
use apinata_core::{
    CancelHandle, ExecOptions, Executor, StreamBound, StreamEnd, StreamLimits, StreamOptions,
};
use serde_json::json;
use std::time::{Duration, Instant};

#[tokio::test]
async fn an_event_limit_ends_politely() {
    let s = ws_server(WsScript::new(vec![tick(1), tick(2), tick(3)])).await;
    let (opts, _) = recording(StreamLimits::new().max_events(Some(2)));
    let res = ws_exec(Auto)
        .send_streaming(&sub(&s.http("/")), &CancelHandle::new(), opts)
        .await
        .unwrap();
    assert_eq!(
        res.stream_end(),
        Some(&StreamEnd::Bound(StreamBound::Events))
    );
    assert_eq!(res.events().len(), 2);
    let lines = s.seen.until(|l| l.starts_with("close: ")).await;
    assert_eq!(
        sent(&lines).last(),
        Some(&json!({"id": "1", "type": "complete"}))
    );
    assert!(lines.contains(&"close: 1000".to_owned()), "{lines:?}");
}

#[tokio::test]
async fn a_time_limit_ends_politely() {
    let s = ws_server(WsScript::new(vec![tick(1)])).await;
    let (opts, _) = recording(StreamLimits::new().max_duration(Some(Duration::from_millis(300))));
    let started = Instant::now();
    let res = ws_exec(Auto)
        .send_streaming(&sub(&s.http("/")), &CancelHandle::new(), opts)
        .await
        .unwrap();
    assert_eq!(res.stream_end(), Some(&StreamEnd::Bound(StreamBound::Time)));
    assert_eq!(res.events().len(), 1);
    assert!(started.elapsed() < Duration::from_secs(5));
    let lines = s.seen.until(|l| l.starts_with("close: ")).await;
    assert!(lines.contains(&"close: 1000".to_owned()), "{lines:?}");
}

#[tokio::test]
async fn the_body_cap_ends_the_transcript() {
    // One record is 39 bytes: `event: next\n` (12), `data: ` (6), the payload (19), `\n\n` (2).
    let s = ws_server(WsScript::new(vec![tick(1), tick(2)])).await;
    let exec = Executor::new(ExecOptions::new().max_body_bytes(Some(40))).unwrap();
    let res = exec
        .send_streaming(
            &sub(&s.http("/")),
            &CancelHandle::new(),
            StreamOptions::new(),
        )
        .await
        .unwrap();
    assert_eq!(res.stream_end(), Some(&StreamEnd::BodyCap));
    assert_eq!(res.body().len(), 40);
    assert!(res.truncated());
    assert_eq!(res.events(), vec![record("next", r#"{"data":{"tick":1}}"#)]);
    let lines = s.seen.until(|l| l.starts_with("close: ")).await;
    assert!(lines.contains(&"close: 1000".to_owned()), "{lines:?}");
}

#[tokio::test]
async fn an_unstreamed_send_returns_at_the_request_timeout() {
    let s = ws_server(WsScript::new(vec![tick(1)])).await;
    let req = graphql_request(
        ROOT,
        &s.http("/"),
        QUERY,
        VARS,
        "settings:\n  timeout: 400\n",
    );
    let started = Instant::now();
    let res = ws_exec(Auto).send(&req).await.unwrap();
    assert_eq!(res.status(), 101);
    assert_eq!(res.stream_end(), Some(&StreamEnd::Bound(StreamBound::Time)));
    assert_eq!(res.events().len(), 1);
    assert!(started.elapsed() < Duration::from_secs(5));
}
