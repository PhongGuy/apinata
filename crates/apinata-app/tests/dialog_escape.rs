//! Escape closes a dialog after a click on any of its controls: a click never takes the key
//! away from the dialog.

use crate::support;

use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, ElementId, TestAppContext};
use support::*;

fn dialog_open(cx: &mut TestAppContext, win: AnyWindowHandle) -> bool {
    cx.update_window(win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}

fn open_prefs(cx: &mut TestAppContext, s: &Started) {
    cx.update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap();
    frame(cx, s.win);
}

// One pointer click: on `id`, or on segment `ix` inside the segmented control `id`. The
// `Shown` forms first wheel Preferences until the target sits inside its rows.
enum Click {
    On(ElementId),
    Segment(&'static str, usize),
    OnShown(&'static str),
    SegmentShown(&'static str, usize),
}

fn click(cx: &mut TestAppContext, s: &Started, at: &Click) {
    match at {
        Click::OnShown(id) | Click::SegmentShown(id, _) => prefs_reveal(cx, s, id),
        Click::On(_) | Click::Segment(..) => {}
    }
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        match at {
            Click::On(id) => w.click(id.clone(), cx),
            Click::OnShown(id) => w.click(*id, cx),
            Click::Segment(track, ix) | Click::SegmentShown(track, ix) => {
                w.within(*track).click(*ix, cx)
            }
        }
    })
    .unwrap();
    frame(cx, s.win);
}

// Opens Preferences, clicks through `clicks`, and checks that one Escape closes it, both as a
// full key press and as the key-down alone that gpui-pilot sends.
fn escape_closes_prefs_after(cx: &mut TestAppContext, clicks: &[Click]) {
    for key_down_only in [false, true] {
        let col = two_requests();
        let s = started(cx, &col);
        open_prefs(cx, &s);
        for at in clicks {
            click(cx, &s, at);
        }
        assert!(dialog_open(cx, s.win), "the clicks leave Preferences open");
        if key_down_only {
            cx.simulate_keystrokes(s.win, "escape");
            frame(cx, s.win);
        } else {
            press_key(cx, s.win, "escape");
        }
        assert!(!dialog_open(cx, s.win), "Escape closes after the clicks");
    }
}

#[gpui_kit::test]
fn escape_closes_preferences_after_a_theme_click(cx: &mut TestAppContext) {
    for ix in 0..3 {
        escape_closes_prefs_after(cx, &[Click::Segment("theme", ix)]);
    }
}

#[gpui_kit::test]
fn escape_closes_preferences_after_the_other_appearance_segments(cx: &mut TestAppContext) {
    escape_closes_prefs_after(cx, &[Click::Segment("orientation", 1)]);
    escape_closes_prefs_after(cx, &[Click::Segment("title-bar-mode", 1)]);
}

#[gpui_kit::test]
fn escape_closes_preferences_after_the_network_and_script_segments(cx: &mut TestAppContext) {
    let network = || Click::On(("prefs-nav-entry", 2usize).into());
    escape_closes_prefs_after(cx, &[network(), Click::Segment("http-version", 1)]);
    // Manual shows the proxy fields; Off then removes the field that took focus.
    escape_closes_prefs_after(
        cx,
        &[
            network(),
            Click::Segment("global-proxy-mode", 2),
            Click::OnShown("global-proxy-host"),
            Click::SegmentShown("global-proxy-protocol", 1),
            Click::SegmentShown("global-proxy-mode", 0),
        ],
    );
    let scripts = Click::On(("prefs-nav-entry", 3usize).into());
    escape_closes_prefs_after(cx, &[scripts, Click::Segment("run-scripts", 1)]);
}

// The other kinds of control in Preferences: a checkbox, a section link, a text field and
// a reset button.
#[gpui_kit::test]
fn escape_closes_preferences_after_a_click_on_other_controls(cx: &mut TestAppContext) {
    escape_closes_prefs_after(cx, &[Click::On("autosave".into())]);
    escape_closes_prefs_after(cx, &[Click::On(("prefs-nav-entry", 1usize).into())]);
    escape_closes_prefs_after(cx, &[Click::On("zoom".into())]);
    escape_closes_prefs_after(cx, &[Click::On("zoom-reset".into())]);
}

// The clicks must not cost the search its first Escape: with text in it, Escape clears it.
#[gpui_kit::test]
fn escape_after_a_segment_click_still_clears_a_search_first(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    replace_input(cx, s.win, None, "prefs-search", "theme");
    frame(cx, s.win);
    click(cx, &s, &Click::Segment("theme", 2));
    press_key(cx, s.win, "escape");
    assert!(dialog_open(cx, s.win), "the first Escape only clears");
    press_key(cx, s.win, "escape");
    assert!(!dialog_open(cx, s.win), "the second Escape closes");
}

#[gpui_kit::test]
fn escape_closes_the_search_dialog_after_a_toggle_click(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.simulate_keystrokes(s.win, &key("shift-f"));
    frame(cx, s.win);
    assert!(dialog_open(cx, s.win));
    click(cx, &s, &Click::On("search-case".into()));
    click(cx, &s, &Click::On("search-replace-toggle-button".into()));
    press_key(cx, s.win, "escape");
    assert!(!dialog_open(cx, s.win));
}

// The key-down alone, as gpui-pilot sends it, closes it too.
#[gpui_kit::test]
fn escape_down_closes_the_search_dialog_after_a_toggle_click(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.simulate_keystrokes(s.win, &key("shift-f"));
    frame(cx, s.win);
    assert!(dialog_open(cx, s.win));
    click(cx, &s, &Click::On("search-case".into()));
    click(cx, &s, &Click::On("search-replace-toggle-button".into()));
    cx.simulate_keystrokes(s.win, "escape");
    frame(cx, s.win);
    assert!(!dialog_open(cx, s.win));
}

// The toggled checkbox is an edit, so Escape reaching the jar opens its close prompt.
#[gpui_kit::test]
fn escape_reaches_the_cookie_jar_after_a_form_checkbox_click(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| {
        apinata_app::cookie_jar::open_cookie_jar(w, cx);
    })
    .unwrap();
    frame(cx, s.win);
    click(cx, &s, &Click::On("jar-add".into()));
    click(cx, &s, &Click::On("jar-form-secure".into()));
    press_key(cx, s.win, "escape");
    assert!(confirm_open(cx), "one Escape reaches the jar and asks");
    answer_confirm(cx, "confirm-discard");
    assert!(
        !dialog_open(cx, s.win),
        "Don't save closes the jar, form and all"
    );
}
