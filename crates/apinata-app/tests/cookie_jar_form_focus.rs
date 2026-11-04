//! The cookie jar's form takes focus as it opens, frame by frame: Add lands in Domain and Edit
//! in Value, whether pressed by key or clicked.

use crate::support;

use apinata_app::cookie_jar::open_cookie_jar;
use apinata_app::executor::SharedExecutor;
use apinata_core::{CookieInfo, ExecOptions, Executor};
use gpui_kit::{AppContext as _, TestAppContext};
use std::sync::Arc;
use support::*;

// The started app with its window active and the jar open over one cookie.
fn jar(cx: &mut TestAppContext) -> (Started, tempfile::TempDir) {
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
    let exec = Arc::new(Executor::new(ExecOptions::new()).unwrap());
    exec.cookies()
        .insert(CookieInfo::new("a.test", "k", "v"))
        .unwrap();
    cx.update(|cx| cx.set_global(SharedExecutor(exec)));
    cx.update_window(s.win, |_, w, cx| open_cookie_jar(w, cx))
        .unwrap();
    frame(cx, s.win);
    (s, col)
}

// The drawn frame rings `id`, a typed key lands in it, and the search query stays empty.
fn typing_reaches(cx: &mut TestAppContext, s: &Started, id: &'static str) {
    assert!(ring_on(cx, s.win, id), "{id} shows its ring");
    platform_press(cx, s.win, "x");
    next_frame(cx, s.win);
    assert!(
        drawn_text(cx, s.win, id).contains('x'),
        "typing reaches {id}"
    );
    assert_eq!(
        drawn_text(cx, s.win, "jar-search"),
        "",
        "the search is untouched"
    );
}

#[gpui_kit::test]
fn enter_on_edit_focuses_the_value_field(cx: &mut TestAppContext) {
    let (s, _col) = jar(cx);
    platform_enter_on(cx, s.win, ("jar-edit", 0usize));
    typing_reaches(cx, &s, "jar-form-value");
}

#[gpui_kit::test]
fn space_on_edit_focuses_the_value_field(cx: &mut TestAppContext) {
    let (s, _col) = jar(cx);
    tab_to(cx, s.win, ("jar-edit", 0usize));
    platform_press(cx, s.win, "space");
    next_frame(cx, s.win);
    typing_reaches(cx, &s, "jar-form-value");
}

#[gpui_kit::test]
fn clicking_edit_focuses_the_value_field(cx: &mut TestAppContext) {
    let (s, _col) = jar(cx);
    platform_click(cx, s.win, ("jar-edit", 0usize));
    next_frame(cx, s.win);
    typing_reaches(cx, &s, "jar-form-value");
}

#[gpui_kit::test]
fn clicking_add_from_the_search_focuses_the_domain_field(cx: &mut TestAppContext) {
    let (s, _col) = jar(cx);
    click_with(cx, s.win, "jar-search", Default::default());
    assert!(
        ring_on(cx, s.win, "jar-search"),
        "focus starts in the search"
    );
    platform_click(cx, s.win, "jar-add");
    next_frame(cx, s.win);
    typing_reaches(cx, &s, "jar-form-domain");
}

#[gpui_kit::test]
fn add_cookie_by_key_focuses_the_domain_field(cx: &mut TestAppContext) {
    let (s, _col) = jar(cx);
    platform_enter_on(cx, s.win, "jar-add");
    typing_reaches(cx, &s, "jar-form-domain");
}
