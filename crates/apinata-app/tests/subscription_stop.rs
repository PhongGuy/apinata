//! The Send slot and the pane while a subscription runs: Stop replaces Send, a quiet
//! subscription says it waits and offers Stop in the pane.

use crate::support::ws_server::tick;
use crate::support::*;
use apinata_app::response_pane::ResponseTab;
use apinata_core::{Outcome, StreamEnd};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, TestAppContext};
use std::time::Duration;

const TRANSPORT: &str = "graphql-transport-ws";

#[gpui_kit::test]
fn send_turns_into_stop_and_stop_ends_a_subscription(cx: &mut TestAppContext) {
    http_ready(cx);
    let frames = (1..=200).map(|n| tick(false, n)).collect();
    let port = ws_peer(cx, TRANSPORT, frames, Duration::from_millis(20));
    let col = subscription_collection(port);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.update_window(win, |_, w, cx| w.click("send", cx))
        .unwrap();
    let model = ed.read_with(cx, |e, _| e.send_model().clone());
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| m.live().is_some_and(|l| l.events().len() >= 2))
    });
    assert!(present(cx, win, "stop"));
    assert!(!present(cx, win, "send"));
    assert!(!present(cx, win, "cancel"));
    cx.update_window(win, |_, w, cx| w.click("stop-button", cx))
        .unwrap();
    let result = finished(cx, &model);
    assert_eq!(result.outcome, Outcome::Passed);
    assert_eq!(
        result.response.as_ref().unwrap().stream_end(),
        Some(&StreamEnd::Stopped)
    );
    assert!(present(cx, win, "send"));
    assert!(!present(cx, win, "stop"));
}

#[gpui_kit::test]
fn a_quiet_subscription_says_it_waits_and_offers_stop_in_the_pane(cx: &mut TestAppContext) {
    http_ready(cx);
    let port = ws_peer(cx, TRANSPORT, vec![], Duration::from_millis(10));
    let col = subscription_collection(port);
    let (win, pane, model, root) = pane_for(cx, &col);
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.is_streaming()));
    assert!(present(cx, win, "response-live-waiting"));
    assert!(
        present(cx, win, "live-body"),
        "the list stays, so nothing jumps"
    );
    pane.update(cx, |p, cx| p.set_tab(ResponseTab::Events, cx));
    assert!(
        present(cx, win, "response-live-waiting"),
        "on the Events tab too"
    );
    let stop = cx
        .update_window(win, |_, w, _| w.try_find("response-stop"))
        .unwrap()
        .expect("the pane offers Stop");
    assert_eq!(
        stop.label(),
        Some("Stop the stream"),
        "the pane's Stop is named apart from the editor's"
    );
    cx.update_window(win, |_, w, cx| w.click("response-stop", cx))
        .unwrap();
    let result = finished(cx, &model);
    assert_eq!(result.outcome, Outcome::Passed);
    assert_eq!(
        result.response.as_ref().unwrap().stream_end(),
        Some(&StreamEnd::Stopped)
    );
}

#[gpui_kit::test]
fn the_waiting_row_leaves_once_data_arrives(cx: &mut TestAppContext) {
    http_ready(cx);
    let port = ws_peer(
        cx,
        TRANSPORT,
        vec![tick(false, 1)],
        Duration::from_millis(10),
    );
    let col = subscription_collection(port);
    let (win, _pane, model, root) = pane_for(cx, &col);
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| m.live().is_some_and(|l| l.bytes() > 0))
    });
    assert!(!present(cx, win, "response-live-waiting"));
    model.update(cx, |m, cx| m.stop(cx));
    finished(cx, &model);
}
