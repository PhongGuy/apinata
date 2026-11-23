use crate::support;
use crate::support::sse_server::{Script, Step, event};
use crate::support::*;
use apinata_app::response_pane::{ResponsePane, ResponseTab};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Bounds, ElementId, Pixels, TestAppContext, WindowOptions};
use std::time::Duration;

// An event-stream that sends one event, stays open long enough to look at, then ends cleanly.
fn open_stream(
    cx: &mut TestAppContext,
) -> (
    gpui_kit::AnyWindowHandle,
    gpui_kit::Entity<ResponsePane>,
    gpui_kit::Entity<apinata_app::send_model::SendModel>,
    std::path::PathBuf,
    support::sse_server::SlowServer,
    tempfile::TempDir,
) {
    http_ready(cx);
    let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nx-trace: abc\r\n\
                transfer-encoding: chunked\r\n\r\n"
        .to_owned();
    let steps = vec![
        event("one"),
        Step::Pause(Duration::from_secs(2)),
        Step::Finish,
    ];
    let server = sse(cx, Script::raw_head(head, steps));
    let col = sse_collection(&server);
    let (_state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    support::init(cx);
    let m = model.clone();
    let (win, pane) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(m, w, cx))
            })
        })
        .unwrap();
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| m.live().is_some_and(|l| l.events().len() == 1))
    });
    (win, pane, model, root, server, col)
}

fn label(
    cx: &mut TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    id: impl Into<ElementId>,
) -> String {
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id)
            .and_then(|e| e.label().map(str::to_owned))
            .unwrap_or_default()
    })
    .unwrap()
}

fn bounds(
    cx: &mut TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    id: impl Into<ElementId>,
) -> Bounds<Pixels> {
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).bounds()
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_tab_strip_is_the_same_while_streaming_and_after_close(cx: &mut TestAppContext) {
    let (_win, pane, model, _root, _server, _col) = open_stream(cx);
    assert!(model.read_with(cx, |m, _| m.is_streaming()));
    let live = pane.read_with(cx, |p, cx| p.shown_tabs(cx));
    finished(cx, &model);
    let done = pane.read_with(cx, |p, cx| p.shown_tabs(cx));
    assert_eq!(live, done);
    assert!(live.contains(&ResponseTab::Headers), "{live:?}");
    assert!(live.contains(&ResponseTab::Events), "{live:?}");
}

#[gpui_kit::test]
fn the_headers_show_while_the_stream_is_live(cx: &mut TestAppContext) {
    let (win, pane, model, _root, _server, _col) = open_stream(cx);
    pane.update(cx, |p, cx| p.set_tab(ResponseTab::Headers, cx));
    assert!(model.read_with(cx, |m, _| m.is_streaming()));
    let mut names = Vec::new();
    for ix in (0..8usize).take_while(|&ix| present(cx, win, ("header-name", ix))) {
        names.push(ix);
    }
    let names: Vec<String> = names
        .into_iter()
        .map(|ix| label(cx, win, ("header-name", ix)))
        .collect();
    assert!(names.iter().any(|n| n == "x-trace"), "{names:?}");
    model.update(cx, |m, cx| m.cancel(cx));
    finished(cx, &model);
}

#[gpui_kit::test]
fn the_timeline_says_it_fills_in_when_the_stream_ends(cx: &mut TestAppContext) {
    let (win, pane, model, _root, _server, _col) = open_stream(cx);
    pane.update(cx, |p, cx| p.set_tab(ResponseTab::Timeline, cx));
    assert!(model.read_with(cx, |m, _| m.is_streaming()));
    assert_eq!(
        label(cx, win, "response-stream-pending"),
        "Fills in when the stream ends"
    );
    finished(cx, &model);
    assert!(!present(cx, win, "response-stream-pending"));
    assert!(present(cx, win, "response-timeline"));
}

#[gpui_kit::test]
fn the_status_line_keeps_its_shape_when_the_stream_ends(cx: &mut TestAppContext) {
    let (win, _pane, model, _root, _server, _col) = open_stream(cx);
    assert!(model.read_with(cx, |m, _| m.is_streaming()));
    assert_eq!(label(cx, win, "response-status"), "200 OK");
    assert_eq!(label(cx, win, "response-stream-state"), "streaming");
    let pill = bounds(cx, win, "response-status");
    let chip = bounds(cx, win, "response-stream-state");
    let slot = bounds(cx, win, "response-timing-pending");
    let line = bounds(cx, win, "response-status-line");
    finished(cx, &model);
    assert_eq!(label(cx, win, "response-status"), "200 OK");
    assert_eq!(label(cx, win, "response-stream-end"), "closed");
    assert_eq!(bounds(cx, win, "response-status").size, pill.size);
    assert_eq!(bounds(cx, win, "response-stream-end"), chip);
    assert_eq!(
        bounds(cx, win, "response-timing").size.width,
        slot.size.width
    );
    let after = bounds(cx, win, "response-status-line");
    assert_eq!(after.size.height, line.size.height);
    assert_eq!(after.origin.y, line.origin.y);
}

// A first send has no previous runs, yet its History button lands when the stream ends; the
// live line holds that place, so the button does not move in.
#[gpui_kit::test]
fn the_history_button_holds_its_place_on_a_first_send(cx: &mut TestAppContext) {
    http_ready(cx);
    let steps = vec![
        event("one"),
        Step::Pause(Duration::from_secs(2)),
        Step::Finish,
    ];
    let server = sse(cx, Script::sse(steps));
    let col = sse_collection(&server);
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("send", cx))
        .unwrap();
    let ed = editor_of(cx, &s, tab);
    let model = ed.read_with(cx, |e, _| e.send_model().clone());
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| m.live().is_some_and(|l| l.events().len() == 1))
    });
    frame(cx, s.win);
    assert!(
        present(cx, s.win, "response-history"),
        "no History while streaming"
    );
    let live = bounds(cx, s.win, "response-history");
    finished(cx, &model);
    wait_real(cx, |cx| present(cx, s.win, "response-stream-end"));
    frame(cx, s.win);
    assert_eq!(bounds(cx, s.win, "response-history"), live);
}
