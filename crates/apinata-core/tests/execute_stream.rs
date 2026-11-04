//! Streaming sends: live chunks, the end reasons, Stop, limits, idle timeout and the opt-in rule.

use crate::common::resolved_one;
use crate::common::sse_server::{Script, SlowServer, Step, event, slow_sse_server};
use crate::ntlm_support::{Options, PASSWORD, USER, ntlm_server};
use apinata_core::{
    CancelHandle, ExecError, ExecOptions, Executor, StreamBound, StreamEnd, StreamEvent,
    StreamLimits, StreamOptions,
};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MS: fn(u64) -> Duration = Duration::from_millis;

type Seen = Arc<Mutex<Vec<StreamEvent>>>;

fn recording(limits: StreamLimits) -> (StreamOptions, Seen) {
    let seen: Seen = Arc::default();
    let sink = Arc::clone(&seen);
    let opts = StreamOptions::new()
        .limits(limits)
        .sink(move |e| sink.lock().unwrap().push(e));
    (opts, seen)
}

fn started(seen: &Seen) -> usize {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|e| matches!(e, StreamEvent::Started { .. }))
        .count()
}

fn chunks(seen: &Seen) -> Vec<u8> {
    seen.lock()
        .unwrap()
        .iter()
        .flat_map(|e| match e {
            StreamEvent::Chunk(b) => b.clone(),
            _ => Vec::new(),
        })
        .collect()
}

fn req(server: &SlowServer, timeout_ms: Option<u64>) -> apinata_core::ResolvedRequest {
    let top = timeout_ms.map_or(String::new(), |t| format!("settings:\n  timeout: {t}\n"));
    resolved_one("GET", &server.url("/"), "", &top)
}

fn exec() -> Executor {
    Executor::new(ExecOptions::new()).unwrap()
}

fn ticks(n: usize, gap: u64) -> Vec<Step> {
    (0..n)
        .flat_map(|i| [event(&format!("e{i}")), Step::Pause(MS(gap))])
        .collect()
}

#[tokio::test]
async fn chunks_arrive_live_in_order_and_a_finished_stream_is_closed() {
    let mut steps = ticks(3, 80);
    steps.push(Step::Finish);
    let s = slow_sse_server(Script::sse(steps)).await;
    let (opts, seen) = recording(StreamLimits::new());
    let res = exec()
        .send_streaming(&req(&s, None), &CancelHandle::new(), opts)
        .await
        .unwrap();
    assert_eq!(res.stream_end(), Some(&StreamEnd::Closed));
    assert_eq!(started(&seen), 1);
    assert!(matches!(
        seen.lock().unwrap()[0],
        StreamEvent::Started {
            status: 200,
            event_stream: true,
            ..
        }
    ));
    assert_eq!(chunks(&seen), res.body());
    assert_eq!(res.events().len(), 3);
}

#[tokio::test]
async fn a_plain_slow_body_is_reported_but_is_not_a_stream() {
    let s = slow_sse_server(Script::with_type(
        "text/plain",
        vec![
            Step::Chunk("ab".into()),
            Step::Pause(MS(80)),
            Step::Chunk("cd".into()),
            Step::Finish,
        ],
    ))
    .await;
    let (opts, seen) = recording(StreamLimits::new());
    let res = exec()
        .send_streaming(&req(&s, None), &CancelHandle::new(), opts)
        .await
        .unwrap();
    assert_eq!(res.text(), "abcd");
    assert_eq!(res.stream_end(), None);
    assert!(matches!(
        seen.lock().unwrap()[0],
        StreamEvent::Started {
            event_stream: false,
            ..
        }
    ));
    assert_eq!(chunks(&seen), b"abcd");
}

#[tokio::test]
async fn stop_after_headers_keeps_what_arrived() {
    let mut steps = vec![event("first"), Step::Pause(Duration::from_secs(30))];
    steps.push(event("never"));
    let s = slow_sse_server(Script::sse(steps)).await;
    let cancel = CancelHandle::new();
    let stopper = cancel.clone();
    let opts = StreamOptions::new().sink(move |e| {
        if matches!(e, StreamEvent::Chunk(_)) {
            stopper.stop();
        }
    });
    let started = Instant::now();
    let res = exec()
        .send_streaming(&req(&s, None), &cancel, opts)
        .await
        .unwrap();
    assert_eq!(res.stream_end(), Some(&StreamEnd::Stopped));
    assert_eq!(
        res.events(),
        vec![apinata_core::sse::SseEvent::new("first")]
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn stop_before_headers_is_a_no_op_and_cancel_still_cancels() {
    let mut steps = ticks(2, 50);
    steps.push(Step::Finish);
    let s = slow_sse_server(Script::sse(steps).head_delay(MS(500))).await;
    let cancel = CancelHandle::new();
    let early = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(MS(100)).await;
        early.stop();
    });
    let res = exec()
        .send_streaming(&req(&s, None), &cancel, StreamOptions::new())
        .await
        .unwrap();
    assert_eq!(res.stream_end(), Some(&StreamEnd::Closed));
    assert_eq!(res.events().len(), 2);

    let s = slow_sse_server(Script::sse(vec![]).head_delay(Duration::from_secs(5))).await;
    let cancel = CancelHandle::new();
    let c = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(MS(100)).await;
        c.cancel();
    });
    let err = exec()
        .send_streaming(&req(&s, None), &cancel, StreamOptions::new())
        .await
        .unwrap_err();
    assert!(matches!(err, ExecError::Cancelled), "{err:?}");
}

#[tokio::test]
async fn silence_past_the_timeout_ends_an_event_stream_but_keeps_the_response() {
    let s = slow_sse_server(Script::sse(vec![
        event("a"),
        Step::Pause(Duration::from_secs(20)),
    ]))
    .await;
    let started = Instant::now();
    let res = exec()
        .send_streaming(
            &req(&s, Some(1500)),
            &CancelHandle::new(),
            StreamOptions::new(),
        )
        .await
        .unwrap();
    assert_eq!(res.stream_end(), Some(&StreamEnd::IdleTimeout));
    assert_eq!(res.events().len(), 1);
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[tokio::test]
async fn the_event_and_time_limits_end_a_stream_as_a_bound() {
    let s = slow_sse_server(Script::sse(ticks(10, 40))).await;
    let limits = StreamLimits::new().max_events(Some(3));
    let res = exec()
        .send_streaming(
            &req(&s, None),
            &CancelHandle::new(),
            StreamOptions::new().limits(limits),
        )
        .await
        .unwrap();
    assert_eq!(
        res.stream_end(),
        Some(&StreamEnd::Bound(StreamBound::Events))
    );
    assert_eq!(res.events().len(), 3);

    let s = slow_sse_server(Script::sse(ticks(50, 100))).await;
    let limits = StreamLimits::new().max_duration(Some(MS(400)));
    let started = Instant::now();
    let res = exec()
        .send_streaming(
            &req(&s, None),
            &CancelHandle::new(),
            StreamOptions::new().limits(limits),
        )
        .await
        .unwrap();
    assert_eq!(res.stream_end(), Some(&StreamEnd::Bound(StreamBound::Time)));
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn the_body_cap_ends_an_event_stream_as_truncated_and_leaves_a_plain_body_unmarked() {
    let big = "x".repeat(200);
    let s = slow_sse_server(Script::sse(vec![event(&big), event(&big), event(&big)])).await;
    let limits = StreamLimits::new().max_bytes(Some(300));
    let res = exec()
        .send_streaming(
            &req(&s, None),
            &CancelHandle::new(),
            StreamOptions::new().limits(limits),
        )
        .await
        .unwrap();
    assert_eq!(res.stream_end(), Some(&StreamEnd::BodyCap));
    assert!(res.truncated());
    assert_eq!(res.body().len(), 300);

    // The executor's own cap wins over max_bytes, and a capped ordinary body keeps no stream end.
    let capped = Executor::new(ExecOptions::new().max_body_bytes(Some(50))).unwrap();
    let s = slow_sse_server(Script::with_type(
        "text/plain",
        vec![Step::Chunk("y".repeat(200)), Step::Finish],
    ))
    .await;
    let res = capped
        .send_streaming(&req(&s, None), &CancelHandle::new(), StreamOptions::new())
        .await
        .unwrap();
    assert!(res.truncated());
    assert_eq!(res.stream_end(), None);
}

#[tokio::test]
async fn a_reset_mid_body_keeps_an_event_stream_and_fails_a_plain_body() {
    let s = slow_sse_server(Script::sse(vec![
        event("a"),
        Step::Pause(MS(50)),
        Step::Abort,
    ]))
    .await;
    let res = exec()
        .send_streaming(&req(&s, None), &CancelHandle::new(), StreamOptions::new())
        .await
        .unwrap();
    assert!(
        matches!(res.stream_end(), Some(StreamEnd::Error(_))),
        "{:?}",
        res.stream_end()
    );
    assert_eq!(res.events().len(), 1);

    let s = slow_sse_server(Script::with_type(
        "text/plain",
        vec![Step::Chunk("ab".into()), Step::Pause(MS(50)), Step::Abort],
    ))
    .await;
    let err = exec()
        .send_streaming(&req(&s, None), &CancelHandle::new(), StreamOptions::new())
        .await
        .unwrap_err();
    assert!(matches!(err, ExecError::Transport { .. }), "{err:?}");
}

#[tokio::test]
async fn a_slow_plain_body_still_meets_the_total_deadline_in_streaming_mode() {
    let steps: Vec<Step> = (0..50)
        .flat_map(|_| [Step::Chunk("x".into()), Step::Pause(MS(100))])
        .collect();
    let s = slow_sse_server(Script::with_type("text/plain", steps)).await;
    let (opts, _seen) = recording(StreamLimits::new());
    let err = exec()
        .send_streaming(&req(&s, Some(400)), &CancelHandle::new(), opts)
        .await
        .unwrap_err();
    assert!(matches!(err, ExecError::Timeout { .. }), "{err:?}");
}

#[tokio::test]
async fn a_followed_redirect_never_reports_its_hop_and_a_plain_send_is_unchanged() {
    let last = slow_sse_server(Script::sse(vec![event("a"), Step::Finish])).await;
    let head = format!(
        "HTTP/1.1 302 Found\r\nlocation: {}\r\ncontent-type: text/event-stream\r\ncontent-length: 0\r\n\r\n",
        last.url("/")
    );
    let first = slow_sse_server(Script::raw_head(head, vec![])).await;
    let (opts, seen) = recording(StreamLimits::new());
    let res = exec()
        .send_streaming(&req(&first, None), &CancelHandle::new(), opts)
        .await
        .unwrap();
    assert_eq!(res.timeline().len(), 2);
    assert_eq!(started(&seen), 1);

    // Without StreamOptions an event-stream is just a body under the total deadline.
    let s = slow_sse_server(Script::sse(ticks(20, 100))).await;
    let err = exec().send(&req(&s, Some(400))).await.unwrap_err();
    assert!(matches!(err, ExecError::Timeout { .. }), "{err:?}");
}

#[tokio::test]
async fn an_ntlm_final_leg_is_reported_once_through_the_same_read_path() {
    let n = ntlm_server(Options::default()).await;
    let auth = format!(
        "  auth:\n    type: ntlm\n    username: {USER}\n    password: {PASSWORD}\n    domain: CORP\n"
    );
    let req = resolved_one(
        "GET",
        &format!("{}/ok", n.origin()),
        &auth,
        "settings:\n  timeout: 5000\n",
    );
    let (opts, seen) = recording(StreamLimits::new());
    let res = exec()
        .send_streaming(&req, &CancelHandle::new(), opts)
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    assert_eq!(started(&seen), 1);
    assert_eq!(chunks(&seen), res.body());
    assert_eq!(res.stream_end(), None);
}

/// A request that inherits a default timeout of zero, the only route to `Some(0)`.
fn inherits_zero(server: &SlowServer) -> apinata_core::ResolvedRequest {
    let text = format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{}\"\n",
        server.url("/")
    );
    let root = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";
    let (_dir, c, ctx) = crate::common::setup(&[("opencollection.yml", root), ("r.yml", &text)]);
    let ctx = ctx.with_default_timeout(Some(Duration::ZERO));
    crate::common::at(&c, &ctx, "r.yml").expect("resolves")
}

#[tokio::test]
async fn a_zero_timeout_means_no_limit_for_streams_and_plain_bodies() {
    let s = slow_sse_server(Script::sse(vec![
        event("a"),
        Step::Pause(MS(300)),
        event("b"),
        Step::Finish,
    ]))
    .await;
    let res = exec()
        .send_streaming(
            &inherits_zero(&s),
            &CancelHandle::new(),
            StreamOptions::new(),
        )
        .await
        .expect("a zero timeout is no idle timer");
    assert_eq!(res.stream_end(), Some(&StreamEnd::Closed));
    assert_eq!(res.events().len(), 2);

    let s = slow_sse_server(Script::sse(vec![
        event("a"),
        Step::Pause(MS(300)),
        event("b"),
        Step::Finish,
    ]))
    .await;
    let res = exec().send(&inherits_zero(&s)).await.expect("no deadline");
    assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn an_endless_plain_body_with_no_limit_set_ends_at_the_default_cap() {
    // 12.5 MiB in 64 KiB chunks, then the connection idles open: only a cap can end this read.
    let chunk = "z".repeat(64 * 1024);
    let steps = (0..200).map(|_| Step::Chunk(chunk.clone())).collect();
    let s = slow_sse_server(Script::with_type("text/plain", steps)).await;
    let res = tokio::time::timeout(
        Duration::from_secs(30),
        exec().send_streaming(&req(&s, None), &CancelHandle::new(), StreamOptions::new()),
    )
    .await
    .expect("the read must end at the cap, not run on")
    .unwrap();
    assert!(res.truncated());
    assert_eq!(res.body().len(), 10 * 1024 * 1024);
    assert_eq!(res.stream_end(), None);
}
