//! The live Events view in a short window with the console dock open.

use crate::support::sse_server::{Script, Step, event};
use crate::support::*;
use apinata_app::keymap::ToggleConsole;
use apinata_app::response_pane::ResponseTab;
use apinata_app::style::RESPONSE_FLOOR;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Bounds, Pixels, TestAppContext, VisualTestContext, px, size};

// Streams six events into a `height` px tall window with the dock open and shows the Events
// tab, live or, with `finish`, once the stream has ended. Returns the full rows inside the
// list, the list and dock bounds, the response pane's height, and the request pane's height
// beside what its rows need.
fn events_open(
    cx: &mut TestAppContext,
    height: f32,
    finish: bool,
) -> (
    usize,
    Bounds<Pixels>,
    Bounds<Pixels>,
    Pixels,
    (Pixels, Pixels),
) {
    http_ready(cx);
    let mut events: Vec<Step> = (0..6).map(|i| event(&format!("event {i}"))).collect();
    if finish {
        events.push(Step::Finish);
    }
    let server = sse(cx, Script::sse(events));
    let col = sse_collection(&server);
    let s = started(cx, &col);
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1000.), px(height)));
    frame(cx, s.win);
    VisualTestContext::from_window(s.win, cx).dispatch_action(ToggleConsole);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("send", cx))
        .unwrap();
    let ed = editor_of(cx, &s, tab);
    let model = ed.read_with(cx, |e, _| e.send_model().clone());
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| m.live().is_some_and(|l| l.events().len() == 6))
            || finish && !model.read_with(cx, |m, _| m.is_sending())
    });
    if finish {
        finished(cx, &model);
    }
    let pane = ed.read_with(cx, |e, _| e.response().clone());
    pane.update(cx, |p, cx| p.set_tab(ResponseTab::Events, cx));
    frame(cx, s.win);
    let request = request_pane_and_rows(cx, s.win, &ed);
    let response = ed.read_with(cx, |e, cx| e.split().read(cx).sizes()[1]);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let list = w.find("response-events").bounds();
        let dock = w.find("console-dock").bounds();
        let full = (0..6usize)
            .filter_map(|ix| w.try_find(("event-row", ix)).map(|r| r.bounds()))
            .filter(|r| r.top() >= list.top() && r.bottom() <= list.bottom())
            .count();
        (full, list, dock, response, request)
    })
    .unwrap()
}

// In a short window the request keeps its rows first: the Events list shows at least one full
// row above the dock, and the response keeps its header floor.
#[gpui_kit::test]
fn events_rows_stay_usable_at_a_short_window_with_the_dock_open(cx: &mut TestAppContext) {
    let (full, list, dock, response, (pane, need)) = events_open(cx, 500., false);
    assert!(
        pane >= need - px(0.5),
        "request pane {pane:?}, rows need {need:?}"
    );
    assert!(
        response >= RESPONSE_FLOOR - px(0.5),
        "response {response:?}"
    );
    assert!(full >= 1, "{full} full rows in {list:?}");
    assert!(list.bottom() <= dock.top(), "{list:?} {dock:?}");
}

// Once the window has room for both, two full Events rows show above the dock.
#[gpui_kit::test]
fn two_events_rows_show_in_a_taller_window_with_the_dock_open(cx: &mut TestAppContext) {
    let (full, list, dock, _, _) = events_open(cx, 800., false);
    assert!(full >= 2, "{full} full rows in {list:?}");
    assert!(list.bottom() <= dock.top(), "{list:?} {dock:?}");
}

// The finished Events view has the same header as the live one, so it keeps the same rows.
#[gpui_kit::test]
fn finished_events_rows_stay_usable_at_a_short_window_with_the_dock_open(cx: &mut TestAppContext) {
    let (full, list, dock, response, (pane, need)) = events_open(cx, 500., true);
    assert!(
        pane >= need - px(0.5),
        "request pane {pane:?}, rows need {need:?}"
    );
    assert!(
        response >= RESPONSE_FLOOR - px(0.5),
        "response {response:?}"
    );
    assert!(full >= 1, "{full} full rows in {list:?}");
    assert!(list.bottom() <= dock.top(), "{list:?} {dock:?}");
}
