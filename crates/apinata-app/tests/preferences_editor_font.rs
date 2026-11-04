//! The Preferences row for the code editors' font size.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::body_view::BodyView;
use apinata_app::settings::Settings;
use gpui_kit::component::Theme;
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

fn editor_px(cx: &mut TestAppContext) -> f32 {
    cx.update(|cx| f32::from(Theme::global(cx).mono_font_size))
}

fn response_px(cx: &mut TestAppContext) -> f32 {
    cx.update(|cx| f32::from(BodyView::font_size(cx)))
}

#[gpui_kit::test]
fn the_editor_row_sets_the_theme_size_live_and_leaves_the_response_alone(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    assert!(present(cx, s.win, "pref-editor-font"));
    let response = response_px(cx);
    enter(cx, &s, "editor-font-size", "18");
    assert_eq!(
        settings(cx).editor_font_offset,
        7.0,
        "18 minus the rounded 11.375"
    );
    assert_eq!(editor_px(cx).round(), 18.0);
    assert_eq!(
        response_px(cx),
        response,
        "the response size is independent"
    );
    assert_eq!(settings(cx).response_font_offset, 0.0);
}

#[gpui_kit::test]
fn the_response_row_leaves_the_editor_size_alone(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    let editor = editor_px(cx);
    enter(cx, &s, "response-font-size", "20");
    assert_eq!(editor_px(cx), editor);
    assert_eq!(response_px(cx).round(), 20.0);
}

#[gpui_kit::test]
fn a_refused_editor_size_keeps_the_old_one_and_reset_restores_the_default(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    enter(cx, &s, "editor-font-size", "16");
    let kept = editor_px(cx);
    enter(cx, &s, "editor-font-size", "5");
    assert!(present(cx, s.win, "editor-font-size-error"));
    assert_eq!(editor_px(cx), kept);
    assert_eq!(settings(cx).editor_font_offset, 5.0);
    click(cx, &s, "editor-font-reset");
    assert_eq!(settings(cx).editor_font_offset, 0.0);
    assert!(!present(cx, s.win, "editor-font-size-error"));
    assert_eq!(editor_px(cx), 11.375);
}

#[gpui_kit::test]
fn the_offset_stays_relative_when_the_zoom_changes(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    enter(cx, &s, "editor-font-size", "18");
    let at_one = editor_px(cx);
    enter(cx, &s, "zoom", "150");
    cx.run_until_parked();
    let at_one_and_a_half = editor_px(cx);
    assert!(
        (at_one_and_a_half - at_one - 11.375 * 0.5).abs() < 0.01,
        "{at_one} then {at_one_and_a_half}"
    );
    assert_eq!(settings(cx).editor_font_offset, 7.0);
}

#[gpui_kit::test]
fn the_editor_row_is_found_by_search(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    replace_input(cx, s.win, None, "prefs-search", "editor font");
    frame(cx, s.win);
    assert!(present(cx, s.win, "pref-editor-font"));
}

/// Opens Preferences and returns a reader of the editor and response size fields' texts.
fn open_sizes(
    cx: &mut TestAppContext,
    s: &Started,
) -> impl Fn(&mut TestAppContext) -> (String, String) + use<> {
    let view = cx
        .update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap()
        .unwrap();
    frame(cx, s.win);
    move |cx: &mut TestAppContext| {
        view.read_with(cx, |v, cx| {
            let text = |id| v.input_text(id, cx).unwrap();
            (text("editor-font-size"), text("response-font-size"))
        })
    }
}

#[gpui_kit::test]
fn at_110_percent_zoom_the_size_fields_show_the_default_setting(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let sizes = open_sizes(cx, &s);
    enter(cx, &s, "zoom", "110");
    frame(cx, s.win);
    assert_eq!(sizes(cx), ("11".into(), "11".into()));
}

#[gpui_kit::test]
fn a_size_typed_at_110_percent_is_stored_as_the_size_at_100_percent(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let sizes = open_sizes(cx, &s);
    enter(cx, &s, "zoom", "110");
    enter(cx, &s, "editor-font-size", "14");
    enter(cx, &s, "response-font-size", "14");
    frame(cx, s.win);
    assert_eq!(settings(cx).editor_font_offset, 3.0);
    assert_eq!(settings(cx).response_font_offset, 3.0);
    assert_eq!(sizes(cx), ("14".into(), "14".into()));
}

#[gpui_kit::test]
fn changing_the_zoom_never_changes_the_size_fields(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let sizes = open_sizes(cx, &s);
    enter(cx, &s, "editor-font-size", "14");
    let before = sizes(cx);
    for zoom in ["110", "150", "200", "50"] {
        enter(cx, &s, "zoom", zoom);
        frame(cx, s.win);
        assert_eq!(sizes(cx), before, "at {zoom}%");
    }
}
