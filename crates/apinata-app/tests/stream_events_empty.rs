//! The Events tab with nothing to list, and the width of its rows.

use crate::support::sse_server::{Script, event};
use crate::support::*;
use apinata_app::response_pane::ResponseTab;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, TestAppContext};
use std::time::Duration;

const TRANSPORT: &str = "graphql-transport-ws";

#[gpui_kit::test]
fn a_finished_stream_without_events_says_so_only_once_it_ends(cx: &mut TestAppContext) {
    http_ready(cx);
    let port = ws_peer(cx, TRANSPORT, vec![], Duration::from_millis(10));
    let col = subscription_collection(port);
    let (win, pane, model, root) = pane_for(cx, &col);
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.is_streaming()));
    pane.update(cx, |p, cx| p.set_tab(ResponseTab::Events, cx));
    // While live, the waiting row speaks for the empty list.
    assert!(present(cx, win, "response-live-waiting"));
    assert!(!present(cx, win, "response-events-empty"));
    model.update(cx, |m, cx| m.stop(cx));
    finished(cx, &model);
    frame(cx, win);
    assert!(present(cx, win, "response-events-empty"));
    assert!(!present(cx, win, "response-live-waiting"));
    let empty = cx
        .update_window(win, |_, w, _| w.try_find("response-events-empty"))
        .unwrap()
        .expect("the empty line shows");
    assert_eq!(empty.label(), Some("No events"));
}

#[gpui_kit::test]
fn event_rows_and_their_separators_span_the_list(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = sse(cx, Script::sse(vec![event("a"), event("b")]));
    let col = sse_collection(&server);
    let (win, pane, model, root) = pane_for(cx, &col);
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| m.live().is_some_and(|l| l.events().len() == 2))
    });
    pane.update(cx, |p, cx| p.set_tab(ResponseTab::Events, cx));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let list = w.find("response-events").bounds();
        let row = w.find(("event-row", 0usize)).bounds();
        // The row carries the separator, so a row cut to its content cuts the line short.
        assert_eq!(row.left(), list.left(), "{row:?} in {list:?}");
        assert_eq!(row.size.width, list.size.width, "{row:?} in {list:?}");
    })
    .unwrap();
    model.update(cx, |m, cx| m.cancel(cx));
    finished(cx, &model);
}
