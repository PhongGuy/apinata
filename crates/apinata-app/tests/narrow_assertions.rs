//! A 480 px window with the sidebar open: the assertion table's operator column gives way
//! first so Expression and Value stay readable, and a wrapped Send lines up with the URL row.

use crate::support;

use apinata_app::request_editor::Section;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext, px, size};
use support::*;

const A: &str = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: http://x\nruntime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n";

// The request open on its Assertions tab in a window `width` px wide with the sidebar shown.
fn narrow(cx: &mut TestAppContext, width: f32) -> Started {
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", A)]);
    let s = started(cx, &col);
    std::mem::forget(col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.click(("section", Section::Assertions as usize), cx);
    })
    .unwrap();
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(width), px(800.)));
    cx.run_until_parked();
    frame(cx, s.win);
    if !s.shell.read_with(cx, |sh, _| sh.sidebar_visible()) {
        press_in_shell(cx, &s, &key("b"));
    }
    cx.run_until_parked();
    for _ in 0..4 {
        frame(cx, s.win);
    }
    assert!(s.shell.read_with(cx, |sh, _| sh.sidebar_visible()));
    s
}

#[gpui_kit::test]
fn at_480_the_operator_gives_way_and_expression_and_value_stay_readable(cx: &mut TestAppContext) {
    let s = narrow(cx, 480.);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let expr = w.find(("as-expr", 0usize)).bounds();
        let op = w.find(("as-op", 0usize)).bounds();
        let value = w.find(("as-value", 0usize)).bounds();
        let row = w.find(("as-row", 0usize)).bounds();
        // About six characters of the table's text each.
        for (name, b) in [("expression", expr), ("value", value)] {
            assert!(
                b.size.width >= px(40.),
                "{name} {b:?} op {op:?} row {row:?}"
            );
            assert!(b.right() <= row.right(), "{name} {b:?} row {row:?}");
        }
        assert!(op.size.width >= px(40.), "op {op:?}");
        assert!(op.right() <= value.left(), "op {op:?} value {value:?}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn at_480_a_wrapped_send_starts_at_the_url_rows_left_edge(cx: &mut TestAppContext) {
    let s = narrow(cx, 480.);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let method = w.find("method").bounds();
        let send = w.find("send-slot").bounds();
        assert!(
            send.top() > method.bottom(),
            "method {method:?} send {send:?}"
        );
        assert_eq!(
            send.left(),
            method.left(),
            "method {method:?} send {send:?}"
        );
    })
    .unwrap();
}
