//! A Preferences field with an error keeps its label beside the field, not the error.

use crate::support;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

#[gpui_kit::test]
fn a_refused_field_keeps_its_label_on_the_input_line(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    prefs_click(cx, &s, "history-limit");
    cx.update_window(s.win, |_, w, cx| {
        w.press(&key("a"), cx);
        w.input("x", cx);
        w.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(present(cx, s.win, "history-limit-error"));
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let label = w.within("pref-history-limit").find("form-label").bounds();
        let input = w.find("history-limit").bounds();
        let error = w.find("history-limit-error").bounds();
        // The label may wrap in its column; its first line band is the input's line.
        assert!(
            (f32::from(label.top()) - f32::from(input.top())).abs() < 0.5,
            "{label:?} {input:?}"
        );
        assert!(label.right() <= input.left(), "{label:?} {input:?}");
        assert!(error.top() >= input.bottom());
        assert!(!error.intersects(&label), "{error:?} {label:?}");
    })
    .unwrap();
}
