//! A GraphQL subscription sent from a tab: live over WebSocket, Stop, both protocols, and its
//! Events tab once the run is over.

use crate::support::ws_server::{COMPLETE, tick};
use crate::support::*;
use apinata_app::response_pane::ResponseTab;
use apinata_core::{Outcome, RequestResult, Response, StreamEnd};
use gpui_kit::TestAppContext;
use std::sync::Arc;
use std::time::Duration;

const TRANSPORT: &str = "graphql-transport-ws";
const LEGACY: &str = "graphql-ws";

fn ticks(legacy: bool, n: u64) -> Vec<String> {
    (1..=n).map(|i| tick(legacy, i)).collect()
}

fn nexts(rows: &[apinata_app::live_stream::EventRow]) -> usize {
    rows.iter().filter(|r| r.name == "next").count()
}

fn live_then_stop(cx: &mut TestAppContext, protocol: &'static str, legacy: bool) {
    http_ready(cx);
    let port = ws_peer(cx, protocol, ticks(legacy, 100), Duration::from_millis(20));
    let col = subscription_collection(port);
    let (_win, pane, model, root) = pane_for(cx, &col);
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| m.live().is_some_and(|l| l.events().len() >= 2))
    });
    model.read_with(cx, |m, _| {
        let live = m.live().expect("live");
        assert_eq!(live.status(), 101);
        assert!(live.is_event_stream(), "a subscription streams as events");
    });
    assert!(nexts(&pane.read_with(cx, |p, cx| p.event_rows(cx))) >= 2);
    model.update(cx, |m, cx| m.stop(cx));
    let result = finished(cx, &model);
    assert_eq!(result.outcome, Outcome::Passed);
    let res = result.response.as_ref().expect("a response");
    assert_eq!(res.status(), 101);
    assert_eq!(res.stream_end(), Some(&StreamEnd::Stopped));
    assert!(
        res.events()
            .iter()
            .filter(|e| e.event.as_deref() == Some("next"))
            .count()
            >= 2
    );
}

#[gpui_kit::test]
fn a_subscription_streams_live_and_stop_keeps_its_events(cx: &mut TestAppContext) {
    live_then_stop(cx, TRANSPORT, false);
}

#[gpui_kit::test]
fn the_legacy_protocol_streams_live_too(cx: &mut TestAppContext) {
    live_then_stop(cx, LEGACY, true);
}

#[gpui_kit::test]
fn a_completed_subscription_keeps_its_events_tab(cx: &mut TestAppContext) {
    http_ready(cx);
    let mut frames = ticks(false, 2);
    frames.push(COMPLETE.to_owned());
    let port = ws_peer(cx, TRANSPORT, frames, Duration::from_millis(10));
    let col = subscription_collection(port);
    let (win, pane, model, root) = pane_for(cx, &col);
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    let result = finished(cx, &model);
    assert_eq!(result.outcome, Outcome::Passed);
    assert_eq!(
        result.response.as_ref().and_then(|r| r.stream_end()),
        Some(&StreamEnd::Closed)
    );
    assert!(
        pane.read_with(cx, |p, cx| p.shown_tabs(cx))
            .contains(&ResponseTab::Events),
        "the 101 transcript still offers Events"
    );
    wait_real(cx, |cx| {
        pane.read_with(cx, |p, cx| nexts(&p.event_rows(cx)) == 2)
    });
    assert_eq!(
        pane.read_with(cx, |p, cx| p.stream_status(cx)).as_deref(),
        Some("closed")
    );
    pane.update(cx, |p, cx| p.set_tab(ResponseTab::Events, cx));
    assert!(present(cx, win, "response-events"));
}

#[gpui_kit::test]
fn a_kept_subscription_transcript_shows_its_events(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = two_requests();
    let (_win, pane, model, _root) = pane_for(cx, &col);
    let response = Response::new(101)
        .with_text_body("event: next\ndata: {\"data\":{\"tick\":1}}\n\n")
        .with_stream_end(StreamEnd::Closed);
    let result =
        RequestResult::from_parts(ip("a.yml"), "A", Outcome::Passed, Some(response), vec![]);
    model.update(cx, |m, cx| m.show(Arc::new(result), cx));
    assert!(
        pane.read_with(cx, |p, cx| p.shown_tabs(cx))
            .contains(&ResponseTab::Events)
    );
    wait_real(cx, |cx| {
        pane.read_with(cx, |p, cx| nexts(&p.event_rows(cx)) == 1)
    });
}

#[gpui_kit::test]
fn a_subscription_restored_from_the_last_response_store_shows_its_events(cx: &mut TestAppContext) {
    http_ready(cx);
    let mut frames = ticks(false, 2);
    frames.push(COMPLETE.to_owned());
    let port = ws_peer(cx, TRANSPORT, frames, Duration::from_millis(10));
    let col = subscription_collection(port);
    let (_win, pane, model, root) = pane_for(cx, &col);
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    let result = finished(cx, &model);
    let dir = tempfile::tempdir().expect("store dir");
    let store = apinata_app::last_response::ResponseStore::new(dir.path().to_path_buf());
    store
        .save(&root, &ip("a.yml"), &result, std::time::SystemTime::now())
        .expect("save");
    let kept = store.load(&root, &ip("a.yml")).expect("kept");
    assert_eq!(
        kept.result.response.as_ref().map(Response::status),
        Some(101)
    );
    model.update(cx, |m, cx| m.show(Arc::new(kept.result), cx));
    assert!(
        pane.read_with(cx, |p, cx| p.shown_tabs(cx))
            .contains(&ResponseTab::Events),
        "a restored subscription still offers Events"
    );
    wait_real(cx, |cx| {
        pane.read_with(cx, |p, cx| nexts(&p.event_rows(cx)) == 2)
    });
}

#[gpui_kit::test]
fn an_ordinary_101_without_a_stream_end_offers_no_events(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = two_requests();
    let (_win, pane, model, _root) = pane_for(cx, &col);
    let response = Response::new(101).with_text_body("event: next\ndata: {}\n\n");
    let result =
        RequestResult::from_parts(ip("a.yml"), "A", Outcome::Passed, Some(response), vec![]);
    model.update(cx, |m, cx| m.show(Arc::new(result), cx));
    assert!(
        !pane
            .read_with(cx, |p, cx| p.shown_tabs(cx))
            .contains(&ResponseTab::Events)
    );
}
