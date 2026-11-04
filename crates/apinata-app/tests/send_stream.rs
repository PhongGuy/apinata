use crate::support::sse_server::{Script, Step, event};
use crate::support::*;
use apinata_app::send_model::{SendModel, SendState};
use apinata_core::{CancelHandle, Outcome, RequestResult, Response, RunEvent, StreamEnd};
use gpui_kit::{Entity, TestAppContext};
use std::sync::Arc;
use std::time::Duration;

fn started(headers: Vec<(String, String)>, event_stream: bool) -> RunEvent {
    RunEvent::ResponseStarted {
        index: 0,
        status: 200,
        headers,
        event_stream,
    }
}

fn chunk(text: &str) -> RunEvent {
    RunEvent::ResponseChunk {
        index: 0,
        bytes: text.as_bytes().to_vec(),
    }
}

fn finished_event() -> RunEvent {
    let result = RequestResult::from_parts(
        ip("a.yml"),
        "A",
        Outcome::Passed,
        Some(Response::new(200)),
        vec![],
    );
    RunEvent::RequestFinished {
        index: 0,
        result: Arc::new(result),
    }
}

fn streaming(cx: &mut TestAppContext, m: &Entity<SendModel>) -> bool {
    m.read_with(cx, |m, _| m.is_streaming())
}

#[gpui_kit::test]
fn started_and_chunks_make_the_live_state_and_finish_ends_it(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, _root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    let (tx, rx) = async_channel::unbounded();
    model.update(cx, |m, cx| m.follow(rx, cx));
    tx.try_send(started(
        vec![("content-type".into(), "text/event-stream".into())],
        true,
    ))
    .unwrap();
    tx.try_send(chunk("data: a\n\ndata: b")).unwrap();
    cx.run_until_parked();
    model.read_with(cx, |m, _| {
        assert!(matches!(m.state(), SendState::Streaming));
        assert!(m.is_sending(), "a streaming send still counts as sending");
        let live = m.live().expect("live data");
        assert_eq!(
            (live.status(), live.bytes(), live.events().len()),
            (200, 16, 1)
        );
    });
    tx.try_send(chunk("\n\n")).unwrap();
    cx.run_until_parked();
    assert_eq!(
        model.read_with(cx, |m, _| m.live().unwrap().events().len()),
        2
    );
    tx.try_send(finished_event()).unwrap();
    cx.run_until_parked();
    model.read_with(cx, |m, _| {
        assert!(matches!(m.state(), SendState::Done(_)));
        assert!(m.live().is_none());
    });
}

#[gpui_kit::test]
fn a_stale_response_event_after_done_changes_nothing(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, _root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    let (tx, rx) = async_channel::unbounded();
    let handle = CancelHandle::new();
    model.update(cx, |m, cx| m.follow_with(rx, Some(handle), cx));
    tx.try_send(finished_event()).unwrap();
    // A collection run's next request starts after the previous one finished.
    tx.try_send(started(vec![], false)).unwrap();
    tx.try_send(chunk("late")).unwrap();
    cx.run_until_parked();
    model.read_with(cx, |m, _| {
        assert!(matches!(m.state(), SendState::Done(_)));
        assert!(m.live().is_none(), "live is Some only while Streaming");
    });
    tx.try_send(RunEvent::ResponseEnded { index: 0 }).unwrap();
    cx.run_until_parked();
    // A stale end must not turn a later Stop into a cancel.
    model.read_with(cx, |m, _| assert!(!m.body_ended()));
}

#[gpui_kit::test]
fn a_stop_after_the_body_ended_becomes_a_cancel(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, _root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    let (tx, rx) = async_channel::unbounded();
    let handle = CancelHandle::new();
    model.update(cx, |m, cx| m.follow_with(rx, Some(handle.clone()), cx));
    tx.try_send(started(vec![], false)).unwrap();
    cx.run_until_parked();
    // Before the body ends Stop is a stop: the unarmed handle ignores it and nothing is cancelled.
    // The unarmed handle cannot show the stop landed; the real-stream test below proves
    // `CancelHandle::stop` ends a body. Here Stop must leave the send streaming, not cancel it.
    model.update(cx, |m, cx| m.stop(cx));
    assert!(!handle.is_cancelled());
    model.read_with(cx, |m, _| {
        assert!(matches!(m.state(), SendState::Streaming));
        assert!(m.live().is_some(), "a stop keeps what arrived");
    });
    tx.try_send(RunEvent::ResponseEnded { index: 0 }).unwrap();
    cx.run_until_parked();
    // The body is over and post-response work is running, so Stop now cancels.
    model.update(cx, |m, cx| m.stop(cx));
    assert!(handle.is_cancelled());
    // The run reports the cancel like any other: the send ends Cancelled, not Stopped.
    let cancelled = RequestResult::from_parts(ip("a.yml"), "A", Outcome::Cancelled, None, vec![]);
    tx.try_send(RunEvent::RequestFinished {
        index: 0,
        result: Arc::new(cancelled),
    })
    .unwrap();
    cx.run_until_parked();
    model.read_with(cx, |m, _| match m.state() {
        SendState::Done(r) => assert_eq!(r.outcome, Outcome::Cancelled),
        _ => panic!("the send did not end"),
    });
}

#[gpui_kit::test]
fn stop_outside_streaming_does_nothing(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = sse(cx, Script::sse(vec![]).head_delay(Duration::from_secs(5)));
    let col = sse_collection(&server);
    let (_state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    model.update(cx, |m, cx| m.stop(cx));
    assert!(
        !model.read_with(cx, |m, _| m.is_sending()),
        "idle stays idle"
    );
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    wait_real(cx, |_| server.requests() == 1);
    model.update(cx, |m, cx| m.stop(cx));
    cx.run_until_parked();
    assert!(
        model.read_with(cx, |m, _| matches!(m.state(), SendState::Sending)),
        "no headers yet"
    );
    model.update(cx, |m, cx| m.cancel(cx));
    assert_eq!(finished(cx, &model).outcome, Outcome::Cancelled);
}

#[gpui_kit::test]
fn a_real_stream_is_live_and_stop_keeps_what_arrived(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = sse(cx, Script::sse(vec![event("one"), event("two")]));
    let col = sse_collection(&server);
    let (_state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| m.live().is_some_and(|l| l.events().len() == 2))
    });
    model.update(cx, |m, cx| m.stop(cx));
    let result = finished(cx, &model);
    let res = result.response.as_ref().expect("a response");
    assert_eq!(res.stream_end(), Some(&StreamEnd::Stopped));
    assert_eq!(res.events().len(), 2);
    assert_eq!(result.outcome, Outcome::Passed);
}

#[gpui_kit::test]
fn cancel_while_streaming_ends_the_send_as_cancelled(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = sse(cx, Script::sse(vec![event("one")]));
    let col = sse_collection(&server);
    let (_state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    wait_real(cx, |cx| streaming(cx, &model));
    model.update(cx, |m, cx| m.cancel(cx));
    assert_eq!(finished(cx, &model).outcome, Outcome::Cancelled);
}

#[gpui_kit::test]
fn the_single_send_has_no_event_limit(cx: &mut TestAppContext) {
    http_ready(cx);
    // More than the CLI's 10 000-event default, with the connection left open afterwards.
    let steps: Vec<Step> = (0..10_200).map(|i| event(&i.to_string())).collect();
    let server = sse(cx, Script::sse(steps));
    let col = sse_collection(&server);
    let (_state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| {
            // The buffer keeps only the newest EVENT_CAP events; the rest are counted.
            m.live()
                .is_some_and(|l| l.events().len() as u64 + l.dropped_events() >= 10_200)
        })
    });
    assert!(streaming(cx, &model), "no limit ended it");
    model.update(cx, |m, cx| m.cancel(cx));
    finished(cx, &model);
}
