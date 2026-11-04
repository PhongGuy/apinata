use crate::support;

use std::time::UNIX_EPOCH;

use apinata_app::app::AppState;
use apinata_app::last_response::ResponseStore;
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::component::WindowExt as _;
use gpui_kit::{AppContext, TestAppContext};
use support::*;

fn open_prefs(cx: &mut TestAppContext, s: &Started) {
    cx.update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap();
    frame(cx, s.win);
}

// Scrolls the control into view first: the Network rows push later sections below the fold.
fn click(cx: &mut TestAppContext, s: &Started, id: &'static str) {
    prefs_click(cx, s, id);
    cx.run_until_parked();
}

fn remembering(cx: &mut TestAppContext) -> bool {
    cx.update(|cx| {
        cx.global::<AppState>()
            .settings
            .read(cx)
            .get()
            .remember_responses
    })
}

#[gpui_kit::test]
fn the_toggle_switches_remembering(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    assert!(present(cx, s.win, "pref-remember-responses"));
    assert!(remembering(cx));
    click(cx, &s, "remember-responses");
    assert!(!remembering(cx));
    click(cx, &s, "remember-responses");
    assert!(remembering(cx));
}

#[gpui_kit::test]
fn clear_removes_saved_responses_and_toggling_off_keeps_them(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let store = cx.update(|cx| cx.global::<ResponseStore>().clone());
    let kept = RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(Response::new(200)),
        Vec::new(),
    );
    store
        .save(&s.root, &ip("a.yml"), &kept, UNIX_EPOCH)
        .unwrap();
    open_prefs(cx, &s);
    click(cx, &s, "remember-responses");
    assert!(!remembering(cx));
    assert!(
        store.load(&s.root, &ip("a.yml")).is_some(),
        "toggling off deletes nothing"
    );
    assert!(present(cx, s.win, "pref-clear-responses"));
    prefs_click(cx, &s, "clear-responses");
    assert!(store.load(&s.root, &ip("a.yml")).is_none());
    assert!(!store.dir().exists());
}

#[gpui_kit::test]
fn clear_says_it_cleared(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let before = cx
        .update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap();
    open_prefs(cx, &s);
    prefs_click(cx, &s, "clear-responses");
    let after = cx
        .update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap();
    assert_eq!(after, before + 1);
}

#[gpui_kit::test]
fn clear_says_so_inside_preferences(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let prefs = cx
        .update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap()
        .expect("Preferences");
    frame(cx, s.win);
    assert_eq!(prefs.read_with(cx, |p, _| p.status()), None);
    assert!(!present(cx, s.win, "prefs-status"));
    prefs_click(cx, &s, "clear-responses");
    assert_eq!(
        prefs.read_with(cx, |p, _| p.status()).as_deref(),
        Some("Saved responses cleared")
    );
    assert!(present(cx, s.win, "prefs-status"));
}

// Clear's status line shows under the rows; with the Clear row scrolled to the bottom
// edge of the rows, the row must still end above the status once it shows.
#[gpui_kit::test]
fn the_status_line_never_covers_the_rows(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{ScrollDelta, point, px};
    let col = two_requests();
    let s = started(cx, &col);
    gpui_kit::VisualTestContext::from_window(s.win, cx)
        .simulate_resize(gpui_kit::size(px(1000.), px(600.)));
    open_prefs(cx, &s);
    let (row, rows) = cx
        .update_window(s.win, |_, w, _| {
            let rows = w.find("preferences-rows").bounds();
            (w.find("pref-clear-responses").bounds(), rows)
        })
        .unwrap();
    let lift = row.bottom() - rows.bottom() + px(2.);
    assert!(
        lift > px(0.),
        "the row starts below the fold: {row:?} {rows:?}"
    );
    cx.update_window(s.win, |_, w, cx| {
        w.scroll("preferences", ScrollDelta::Pixels(point(px(0.), -lift)), cx)
    })
    .unwrap();
    cx.update_window(s.win, |_, w, cx| w.click("clear-responses", cx))
        .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        let body = w.find("preferences").bounds();
        let status = w.find("prefs-status").bounds();
        let row = w.find("pref-clear-responses").bounds();
        assert!(
            status.bottom() <= body.bottom(),
            "status {status:?} below {body:?}"
        );
        assert!(
            row.bottom() <= status.top(),
            "row {row:?} under status {status:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn enter_on_the_focused_clear_button_clears_saved_responses(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let prefs = cx
        .update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap()
        .expect("Preferences");
    frame(cx, s.win);
    enter_on(cx, s.win, "clear-responses");
    assert_eq!(
        prefs.read_with(cx, |p, _| p.status()).as_deref(),
        Some("Saved responses cleared"),
        "Enter pressed Clear"
    );
}
