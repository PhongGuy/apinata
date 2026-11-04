//! The Preferences rows for the app zoom and the response font size.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::settings::Settings;
use gpui_kit::component::WindowExt;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext};
use support::*;

fn settings(cx: &mut TestAppContext) -> Settings {
    cx.update(|cx| cx.global::<AppState>().settings.read(cx).get().clone())
}

fn open_prefs(cx: &mut TestAppContext, s: &Started) {
    cx.update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap();
    frame(cx, s.win);
}

fn enter(cx: &mut TestAppContext, s: &Started, id: &'static str, text: &str) {
    replace_input(cx, s.win, None, id, text);
    cx.update_window(s.win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
}

fn click(cx: &mut TestAppContext, s: &Started, id: &'static str) {
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.click(id, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_zoom_row_takes_whole_percent_and_resets(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    assert!(present(cx, s.win, "pref-zoom"));
    enter(cx, &s, "zoom", "150");
    assert_eq!(settings(cx).zoom, 1.5);
    enter(cx, &s, "zoom", "120%");
    assert_eq!(settings(cx).zoom, 1.2);
    enter(cx, &s, "zoom", "250");
    assert!(present(cx, s.win, "zoom-error"));
    assert_eq!(settings(cx).zoom, 1.2);
    click(cx, &s, "zoom-reset");
    assert_eq!(settings(cx).zoom, 1.0);
    assert!(!present(cx, s.win, "zoom-error"));
}

#[gpui_kit::test]
fn the_response_row_takes_whole_px_and_resets(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    assert!(present(cx, s.win, "pref-response-font"));
    enter(cx, &s, "response-font-size", "14");
    assert_eq!(
        settings(cx).response_font_offset,
        3.0,
        "14 minus the rounded 11.375"
    );
    enter(cx, &s, "response-font-size", "40");
    assert!(present(cx, s.win, "response-font-size-error"));
    assert_eq!(settings(cx).response_font_offset, 3.0);
    click(cx, &s, "response-font-reset");
    assert_eq!(settings(cx).response_font_offset, 0.0);
}

#[gpui_kit::test]
fn a_reset_refills_the_focused_fields(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let view = cx
        .update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap()
        .unwrap();
    frame(cx, s.win);
    let texts = |cx: &mut TestAppContext| view.read_with(cx, |v, cx| v.field_texts(cx));
    enter(cx, &s, "zoom", "150");
    enter(cx, &s, "response-font-size", "20");
    assert_eq!(texts(cx), ("150".into(), "20".into()));
    click(cx, &s, "zoom-reset");
    assert_eq!(
        texts(cx),
        ("100".into(), "20".into()),
        "the size field shows the setting, which the zoom never moves"
    );
    enter(cx, &s, "response-font-size", "17");
    click(cx, &s, "response-font-reset");
    assert_eq!(texts(cx), ("100".into(), "11".into()));
}

fn keys(cx: &mut TestAppContext, s: &Started, k: &str) {
    cx.simulate_keystrokes(s.win, &secondary(k));
    cx.run_until_parked();
}

fn dialog_open(cx: &mut TestAppContext, s: &Started) -> bool {
    cx.update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}

#[gpui_kit::test]
fn zoom_keys_zoom_the_app_right_after_preferences_opens(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let view = cx
        .update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap()
        .unwrap();
    frame(cx, s.win);
    keys(cx, &s, "=");
    assert_eq!(settings(cx).zoom, 1.1);
    keys(cx, &s, "+");
    assert_eq!(settings(cx).zoom, 1.2);
    keys(cx, &s, "-");
    assert_eq!(settings(cx).zoom, 1.1);
    frame(cx, s.win);
    let shown = view.read_with(cx, |v, cx| v.field_texts(cx)).0;
    assert_eq!(shown, "110", "the Zoom (%) field follows the key");
    keys(cx, &s, "0");
    assert_eq!(settings(cx).zoom, 1.0);
    assert!(dialog_open(cx, &s), "zooming leaves Preferences open");
}

#[gpui_kit::test]
fn zoom_keys_work_from_an_input_inside_preferences(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    click(cx, &s, "body-cap");
    keys(cx, &s, "=");
    keys(cx, &s, "=");
    assert_eq!(settings(cx).zoom, 1.2);
    keys(cx, &s, "0");
    assert_eq!(settings(cx).zoom, 1.0);
    assert!(dialog_open(cx, &s));
}

// Every caller presses Escape from Zoom, so the press first proves Zoom holds focus.
fn escape(cx: &mut TestAppContext, s: &Started) {
    let on_zoom = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find("zoom").and_then(|e| e.focused())
        })
        .unwrap();
    assert_eq!(on_zoom, Some(true), "Zoom holds focus before Escape");
    cx.simulate_keystrokes(s.win, "escape");
    cx.run_until_parked();
    frame(cx, s.win);
}

#[gpui_kit::test]
fn escape_closes_preferences_from_the_zoom_field(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    click(cx, &s, "zoom");
    escape(cx, &s);
    assert!(!dialog_open(cx, &s));
}

#[gpui_kit::test]
fn escape_closes_preferences_with_a_typed_zoom_and_keeps_it(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    replace_input(cx, s.win, None, "zoom", "150");
    escape(cx, &s);
    assert!(!dialog_open(cx, &s));
    assert_eq!(settings(cx).zoom, 1.5, "closing commits what was typed");
}

// A refused value: `on_close` commits every field and warns on refusals; the dialog still closes.
fn escape_with_typed_zoom(cx: &mut TestAppContext, typed: &str) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    replace_input(cx, s.win, None, "zoom", typed);
    escape(cx, &s);
    assert!(!dialog_open(cx, &s), "Escape closes with {typed:?} typed");
    assert_eq!(settings(cx).zoom, 1.0, "{typed:?} is refused");
}

#[gpui_kit::test]
fn escape_closes_preferences_with_a_non_number_zoom(cx: &mut TestAppContext) {
    escape_with_typed_zoom(cx, "abc");
}

#[gpui_kit::test]
fn escape_closes_preferences_with_an_out_of_range_zoom(cx: &mut TestAppContext) {
    escape_with_typed_zoom(cx, "15");
}

// QA reached Zoom by keyboard; Tab focus with keyboard modality is the other entry path.
#[gpui_kit::test]
fn escape_closes_preferences_when_zoom_was_reached_by_tab(cx: &mut TestAppContext) {
    use apinata_app::focus_mode::{self, Modality};
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    cx.update(|cx| focus_mode::set(Modality::Keyboard, cx));
    tab_to(cx, s.win, "zoom");
    escape(cx, &s);
    assert!(!dialog_open(cx, &s));
}

#[gpui_kit::test]
fn escape_closes_preferences_after_a_zoom_is_entered(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    enter(cx, &s, "zoom", "150");
    frame(cx, s.win);
    escape(cx, &s);
    assert!(!dialog_open(cx, &s));
}

// Own repro: Enter refuses a value and shows the row's error while Zoom keeps focus.
#[gpui_kit::test]
fn escape_closes_preferences_while_the_zoom_error_shows(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    enter(cx, &s, "zoom", "250");
    frame(cx, s.win);
    assert!(present(cx, s.win, "zoom-error"));
    escape(cx, &s);
    assert!(!dialog_open(cx, &s));
    assert_eq!(settings(cx).zoom, 1.0);
}

// Own repro: a zoom key refills the focused Zoom field before Escape.
#[gpui_kit::test]
fn escape_closes_preferences_after_a_zoom_key_refills_the_field(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    click(cx, &s, "zoom");
    keys(cx, &s, "=");
    frame(cx, s.win);
    escape(cx, &s);
    assert!(!dialog_open(cx, &s));
    assert_eq!(settings(cx).zoom, 1.1);
}

#[gpui_kit::test]
fn enter_on_the_focused_zoom_reset_resets_the_zoom(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    enter(cx, &s, "zoom", "150");
    assert_eq!(settings(cx).zoom, 1.5);
    enter_on(cx, s.win, "zoom-reset-button");
    assert_eq!(settings(cx).zoom, 1.0, "Enter pressed Reset");
}
