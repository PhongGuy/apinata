//! Preferences segments answer the keyboard: Enter and Space choose, arrows move.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::settings::ThemeChoice;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

fn theme(cx: &mut TestAppContext) -> ThemeChoice {
    cx.update(|cx| cx.global::<AppState>().settings.read(cx).get().theme)
}

// The theme segment holding focus, if any.
fn focused(cx: &mut TestAppContext, s: &Started) -> Option<usize> {
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        (0..3usize)
            .find(|&ix| w.within("theme").try_find(ix).and_then(|e| e.focused()) == Some(true))
    })
    .unwrap()
}

// Key-down only, as gpui-pilot sends keys: the choice must not wait for a key-up.
fn keys(cx: &mut TestAppContext, s: &Started, ks: &str) {
    cx.simulate_keystrokes(s.win, ks);
    frame(cx, s.win);
}

fn prefs_on_theme(cx: &mut TestAppContext) -> (tempfile::TempDir, Started) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap();
    frame(cx, s.win);
    // The first Tab stops on the General section's autosave checkbox.
    keys(cx, &s, "tab tab");
    assert_eq!(
        focused(cx, &s),
        Some(0),
        "Tab reaches the first theme segment"
    );
    assert_eq!(theme(cx), ThemeChoice::System);
    (col, s)
}

#[gpui_kit::test]
fn arrows_move_the_choice_and_the_focus(cx: &mut TestAppContext) {
    let (_col, s) = prefs_on_theme(cx);
    keys(cx, &s, "right");
    assert_eq!((theme(cx), focused(cx, &s)), (ThemeChoice::Light, Some(1)));
    keys(cx, &s, "right");
    assert_eq!((theme(cx), focused(cx, &s)), (ThemeChoice::Dark, Some(2)));
    keys(cx, &s, "right");
    assert_eq!(
        (theme(cx), focused(cx, &s)),
        (ThemeChoice::System, Some(0)),
        "wraps"
    );
    keys(cx, &s, "left");
    assert_eq!(
        (theme(cx), focused(cx, &s)),
        (ThemeChoice::Dark, Some(2)),
        "wraps back"
    );
}

#[gpui_kit::test]
fn enter_and_space_choose_the_focused_segment(cx: &mut TestAppContext) {
    let (_col, s) = prefs_on_theme(cx);
    keys(cx, &s, "tab");
    assert_eq!(focused(cx, &s), Some(1));
    assert_eq!(
        theme(cx),
        ThemeChoice::System,
        "focus alone chooses nothing"
    );
    keys(cx, &s, "enter");
    assert_eq!(theme(cx), ThemeChoice::Light);
    let open = cx
        .update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap();
    assert!(open, "Enter on a segment does not close Preferences");
    keys(cx, &s, "tab");
    assert_eq!(focused(cx, &s), Some(2));
    keys(cx, &s, "space");
    assert_eq!(theme(cx), ThemeChoice::Dark);
}
