//! The live Events view in a short window with the console dock open.

use crate::support::sse_server::{Script, event};
use crate::support::*;
use apinata_app::keymap::ToggleConsole;
use apinata_app::response_pane::ResponseTab;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext, px, size};

#[gpui_kit::test]
fn events_rows_stay_usable_at_a_short_window_with_the_dock_open(cx: &mut TestAppContext) {
    http_ready(cx);
    let events = (0..6).map(|i| event(&format!("event {i}"))).collect();
    let server = sse(cx, Script::sse(events));
    let col = sse_collection(&server);
    let s = started(cx, &col);
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1000.), px(500.)));
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
    });
    let pane = ed.read_with(cx, |e, _| e.response().clone());
    pane.update(cx, |p, cx| p.set_tab(ResponseTab::Events, cx));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let list = w.find("response-events").bounds();
        let dock = w.find("console-dock").bounds();
        let rows = (0..6usize)
            .filter_map(|ix| w.try_find(("event-row", ix)).map(|r| r.bounds()))
            .collect::<Vec<_>>();
        // Two full rows are visible inside the list, and the list ends above the dock.
        let full = rows
            .iter()
            .filter(|r| r.top() >= list.top() && r.bottom() <= list.bottom())
            .count();
        assert!(full >= 2, "{full} full rows in {list:?}: {rows:?}");
        assert!(list.bottom() <= dock.top(), "{list:?} {dock:?}");
    })
    .unwrap();
}
