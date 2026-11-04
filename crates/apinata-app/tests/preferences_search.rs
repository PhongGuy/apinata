//! The Preferences search box: filtering, hidden sections and the empty state.

use crate::support;

use gpui_kit::component::WindowExt;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext};
use support::*;

const SECTIONS: [&str; 6] = [
    "prefs-general",
    "prefs-appearance",
    "prefs-network",
    "prefs-scripts",
    "prefs-updates",
    "prefs-history",
];

fn open_prefs(cx: &mut TestAppContext, s: &Started) {
    cx.update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap();
    frame(cx, s.win);
}

fn search(cx: &mut TestAppContext, s: &Started, text: &str) {
    replace_input(cx, s.win, None, "prefs-search", text);
    frame(cx, s.win);
}

fn sections_shown(cx: &mut TestAppContext, s: &Started) -> Vec<&'static str> {
    SECTIONS
        .into_iter()
        .filter(|id| present(cx, s.win, *id))
        .collect()
}

#[gpui_kit::test]
fn the_search_box_sits_beside_the_title(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let title = w.find("prefs-title").bounds();
        let search = w.find("prefs-search").bounds();
        assert!(search.left() > title.right(), "{title:?} then {search:?}");
        assert!(
            search.top() < title.bottom() && title.top() < search.bottom(),
            "same row: {title:?} {search:?}"
        );
    })
    .unwrap();
    assert_eq!(sections_shown(cx, &s), SECTIONS);
    assert!(!present(cx, s.win, "prefs-no-match"));
}

#[gpui_kit::test]
fn typing_filters_rows_by_label_and_hides_empty_sections(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    search(cx, &s, "tls");
    assert_eq!(sections_shown(cx, &s), ["prefs-network"]);
    assert!(present(cx, s.win, "pref-verify-tls"));
    assert!(!present(cx, s.win, "pref-body-cap"));
    assert!(present(cx, s.win, "prefs-network-heading"));
    assert!(!present(cx, s.win, "prefs-no-match"));
}

#[gpui_kit::test]
fn the_filter_ignores_case_and_matches_section_names(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    search(cx, &s, "NETWORK");
    assert_eq!(sections_shown(cx, &s), ["prefs-network"]);
    assert!(present(cx, s.win, "pref-verify-tls"));
    assert!(present(cx, s.win, "pref-body-cap"));
    search(cx, &s, "Zoom");
    assert_eq!(sections_shown(cx, &s), ["prefs-appearance"]);
    assert!(present(cx, s.win, "pref-zoom"));
    assert!(!present(cx, s.win, "pref-theme"));
    search(cx, &s, "responses");
    assert_eq!(sections_shown(cx, &s), ["prefs-history"]);
}

#[gpui_kit::test]
fn nothing_matching_shows_the_empty_state_until_cleared(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    search(cx, &s, "qqqq");
    assert!(sections_shown(cx, &s).is_empty());
    assert!(present(cx, s.win, "prefs-no-match"));
    search(cx, &s, "");
    assert_eq!(sections_shown(cx, &s), SECTIONS);
    assert!(!present(cx, s.win, "prefs-no-match"));
}

#[gpui_kit::test]
fn a_filtered_input_keeps_its_value(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let view = cx
        .update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap()
        .unwrap();
    frame(cx, s.win);
    search(cx, &s, "theme");
    assert!(!present(cx, s.win, "pref-zoom"));
    search(cx, &s, "");
    let zoom = view.read_with(cx, |v, cx| v.field_texts(cx)).0;
    assert_eq!(zoom, "100");
    let query = view.read_with(cx, |v, cx| v.search().read(cx).value().to_string());
    assert_eq!(query, "");
}

fn press(cx: &mut TestAppContext, s: &Started, keys: &str) {
    cx.simulate_keystrokes(s.win, keys);
    cx.run_until_parked();
    frame(cx, s.win);
}

fn dialog_open(cx: &mut TestAppContext, s: &Started) -> bool {
    cx.update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}

fn opened(
    cx: &mut TestAppContext,
    s: &Started,
) -> gpui_kit::Entity<apinata_app::preferences::PreferencesView> {
    let view = cx
        .update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap()
        .unwrap();
    frame(cx, s.win);
    view
}

fn search_focused(
    cx: &mut TestAppContext,
    s: &Started,
    view: &gpui_kit::Entity<apinata_app::preferences::PreferencesView>,
) -> bool {
    let search = view.read_with(cx, |v, _| v.search().clone());
    cx.update_window(s.win, |_, w, cx| {
        gpui_kit::Focusable::focus_handle(search.read(cx), cx).is_focused(w)
    })
    .unwrap()
}

fn query(
    cx: &mut TestAppContext,
    view: &gpui_kit::Entity<apinata_app::preferences::PreferencesView>,
) -> String {
    view.read_with(cx, |v, cx| v.search().read(cx).value().to_string())
}

#[gpui_kit::test]
fn ctrl_f_focuses_the_search_right_after_opening(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let view = opened(cx, &s);
    assert!(!search_focused(cx, &s, &view));
    press(cx, &s, &secondary("f"));
    assert!(search_focused(cx, &s, &view));
    assert!(dialog_open(cx, &s));
}

#[gpui_kit::test]
fn ctrl_f_focuses_the_search_from_another_input(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let view = opened(cx, &s);
    cx.update_window(s.win, |_, w, cx| w.click("body-cap", cx))
        .unwrap();
    assert!(!search_focused(cx, &s, &view));
    press(cx, &s, &secondary("f"));
    assert!(search_focused(cx, &s, &view));
    press(cx, &s, "z o o m");
    assert_eq!(query(cx, &view), "zoom");
    assert!(present(cx, s.win, "pref-zoom"));
    assert!(!present(cx, s.win, "pref-theme"));
}

#[gpui_kit::test]
fn escape_clears_the_search_then_closes(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let view = opened(cx, &s);
    search(cx, &s, "tls");
    press(cx, &s, "escape");
    assert!(dialog_open(cx, &s), "the first Escape only clears");
    assert_eq!(query(cx, &view), "");
    assert_eq!(sections_shown(cx, &s), SECTIONS);
    press(cx, &s, "escape");
    assert!(!dialog_open(cx, &s), "the second Escape closes");
}

#[gpui_kit::test]
fn escape_in_another_input_clears_the_search_first(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let view = opened(cx, &s);
    search(cx, &s, "zoom");
    cx.update_window(s.win, |_, w, cx| w.click("zoom", cx))
        .unwrap();
    press(cx, &s, "escape");
    assert!(dialog_open(cx, &s));
    assert_eq!(query(cx, &view), "");
    press(cx, &s, "escape");
    assert!(!dialog_open(cx, &s));
}

#[gpui_kit::test]
fn escape_with_an_empty_search_closes_at_once(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    opened(cx, &s);
    press(cx, &s, "escape");
    assert!(!dialog_open(cx, &s));
}

#[gpui_kit::test]
fn the_close_button_closes_even_with_a_search(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    opened(cx, &s);
    search(cx, &s, "tls");
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.click("close", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!dialog_open(cx, &s));
}

// The search box names its focus key the way the sidebar filter does: a key hint inside it.
#[gpui_kit::test]
fn the_search_box_shows_its_focus_key(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    assert!(
        present(cx, s.win, "prefs-search-kbd"),
        "the focus-search chord is shown"
    );
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let search = w.find("prefs-search").bounds();
        let hint = w.find("prefs-search-kbd").bounds();
        assert!(
            search.contains(&hint.center()),
            "{hint:?} inside {search:?}"
        );
    })
    .unwrap();
}

// A narrow dialog drops the shortcut hint from the search box, so the shrunk field still
// shows its placeholder instead of clipping it under the hint.
#[gpui_kit::test]
fn a_narrow_dialog_drops_the_search_shortcut_hint(cx: &mut TestAppContext) {
    use gpui_kit::{px, size};
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    assert!(
        present(cx, s.win, "prefs-search-kbd"),
        "wide: the hint shows"
    );
    gpui_kit::VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(320.), px(700.)));
    frame(cx, s.win);
    frame(cx, s.win);
    assert!(present(cx, s.win, "prefs-search"));
    assert!(!present(cx, s.win, "prefs-search-kbd"), "narrow: no hint");
}

// At 320 px the search field keeps its whole placeholder and stays inside the dialog.
#[gpui_kit::test]
fn the_search_placeholder_fits_at_320_px(cx: &mut TestAppContext) {
    use gpui_kit::{px, size};
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    gpui_kit::VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(320.), px(700.)));
    frame(cx, s.win);
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let field = w.find("prefs-search").bounds();
        let body = w.find("preferences").bounds();
        assert!(
            field.left() >= body.left() - px(0.5) && field.right() <= body.right() + px(0.5),
            "field {field:?} in dialog {body:?}"
        );
        // The kit input's medium text is `text_sm`, 0.875 rem.
        let placeholder = "Search settings";
        let size = gpui_kit::rems(0.875).to_pixels(w.rem_size());
        let run = w.text_style().to_run(placeholder.len());
        let text = w
            .text_system()
            .shape_line(placeholder.into(), size, &[run], None)
            .width;
        assert!(
            text <= field.size.width,
            "placeholder {text:?} in {field:?}"
        );
    })
    .unwrap();
}
