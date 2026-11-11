//! The cookie dialog's domain search.

use crate::support;

use apinata_app::cookie_jar::{CookieJarView, domain_matches, open_cookie_jar};
use apinata_app::executor::SharedExecutor;
use apinata_core::{CookieInfo, ExecOptions, Executor};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, Entity, IntoElement, Render, TestAppContext, Window,
    WindowOptions, div,
};
use std::sync::Arc;
use support::*;

struct Blank;
impl Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// The dialog over a jar holding `a` and `b` under two example.test hosts and a cookie
/// named `example` under other.test.
fn searched(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<CookieJarView>) {
    support::init(cx);
    let exec = Arc::new(Executor::new(ExecOptions::new()).unwrap());
    for (domain, name) in [
        ("api.example.test", "a"),
        ("auth.example.test", "b"),
        ("other.test", "example"),
    ] {
        exec.cookies()
            .insert(CookieInfo::new(domain, name, "1"))
            .unwrap();
    }
    cx.update(|cx| cx.set_global(SharedExecutor(exec)));
    let (win, _) = cx
        .update(|cx| gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Blank)))
        .unwrap();
    let view = cx
        .update_window(win, |_, w, cx| open_cookie_jar(w, cx))
        .unwrap();
    frame(cx, win);
    (win, view)
}

fn shown_names(cx: &mut TestAppContext, view: &Entity<CookieJarView>) -> Vec<String> {
    view.read_with(cx, |v, _| {
        v.shown()
            .into_iter()
            .map(|ix| v.cookies()[ix].name.clone())
            .collect()
    })
}

#[test]
fn a_domain_matches_a_case_insensitive_substring() {
    assert!(domain_matches("api.example.test", ""));
    assert!(domain_matches("api.example.test", "EXAMPLE"));
    assert!(domain_matches("API.Example.test", "example.T"));
    assert!(domain_matches("api.example.test", "api.example.test"));
    assert!(!domain_matches("api.example.test", "other"));
    assert!(!domain_matches("api.example.test", "example.test.x"));
    assert!(
        !domain_matches("api.example.test", " example"),
        "the query is not trimmed"
    );
}

#[gpui_kit::test]
fn typing_in_the_search_narrows_the_list_to_matching_domains(cx: &mut TestAppContext) {
    let (win, view) = searched(cx);
    assert!(present(cx, win, "jar-search"));
    assert_eq!(shown_names(cx, &view), ["a", "b", "example"]);
    replace_input(cx, win, None, "jar-search", "EXAMPLE");
    frame(cx, win);
    assert_eq!(
        shown_names(cx, &view),
        ["a", "b"],
        "the cookie named example is not matched by its name"
    );
    assert!(present(cx, win, ("jar-cookie", 0usize)) && present(cx, win, ("jar-cookie", 1usize)));
    assert!(!present(cx, win, ("jar-cookie", 2usize)));
    assert!(present(cx, win, ("jar-domain", 1usize)) && !present(cx, win, ("jar-domain", 2usize)));
    replace_input(cx, win, None, "jar-search", "nothing-here");
    frame(cx, win);
    assert!(shown_names(cx, &view).is_empty());
    assert!(present(cx, win, "jar-no-match") && !present(cx, win, "jar-empty"));
    replace_input(cx, win, None, "jar-search", "");
    frame(cx, win);
    assert_eq!(shown_names(cx, &view), ["a", "b", "example"]);
}

#[gpui_kit::test]
fn a_delete_under_a_search_removes_the_cookie_shown(cx: &mut TestAppContext) {
    let (win, view) = searched(cx);
    replace_input(cx, win, None, "jar-search", "other");
    frame(cx, win);
    assert_eq!(shown_names(cx, &view), ["example"]);
    cx.update_window(win, |_, w, cx| w.click(("jar-delete", 2usize), cx))
        .unwrap();
    frame(cx, win);
    let left: Vec<String> = view.read_with(cx, |v, _| {
        v.cookies().iter().map(|c| c.name.clone()).collect()
    });
    assert_eq!(left, ["a", "b"]);
    assert!(present(cx, win, "jar-no-match"));
}

fn clear_label(cx: &mut TestAppContext, win: AnyWindowHandle) -> Option<String> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find("jar-clear")
            .and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

fn left(cx: &mut TestAppContext, view: &Entity<CookieJarView>) -> Vec<String> {
    view.read_with(cx, |v, _| {
        v.cookies().iter().map(|c| c.name.clone()).collect()
    })
}

#[gpui_kit::test]
fn clear_under_a_search_removes_only_the_shown_domains(cx: &mut TestAppContext) {
    let (win, view) = searched(cx);
    assert_eq!(clear_label(cx, win).as_deref(), Some("Clear all"));
    replace_input(cx, win, None, "jar-search", "example");
    frame(cx, win);
    assert_eq!(clear_label(cx, win).as_deref(), Some("Clear shown"));
    cx.update_window(win, |_, w, cx| w.click("jar-clear", cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(
        left(cx, &view),
        ["example"],
        "other.test is hidden and kept"
    );
    replace_input(cx, win, None, "jar-search", "");
    frame(cx, win);
    assert_eq!(clear_label(cx, win).as_deref(), Some("Clear all"));
    cx.update_window(win, |_, w, cx| w.click("jar-clear", cx))
        .unwrap();
    frame(cx, win);
    assert!(left(cx, &view).is_empty());
}

#[gpui_kit::test]
fn a_search_matching_nothing_offers_no_clear(cx: &mut TestAppContext) {
    let (win, view) = searched(cx);
    replace_input(cx, win, None, "jar-search", "nothing-here");
    frame(cx, win);
    assert!(present(cx, win, "jar-no-match"));
    assert!(!present(cx, win, "jar-clear"));
    assert_eq!(left(cx, &view).len(), 3);
}

#[gpui_kit::test]
fn a_revealed_value_stays_revealed_across_a_search_and_a_delete_above(cx: &mut TestAppContext) {
    let (win, view) = searched(cx);
    let value = |cx: &mut TestAppContext, ix: usize| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find(("jar-value", ix))
                .label()
                .unwrap_or_default()
                .to_owned()
        })
        .unwrap()
    };
    cx.update_window(win, |_, w, cx| w.click(("jar-reveal", 2usize), cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(value(cx, 2), "1");
    replace_input(cx, win, None, "jar-search", "other");
    frame(cx, win);
    assert_eq!(value(cx, 2), "1", "still revealed under the filter");
    replace_input(cx, win, None, "jar-search", "");
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click(("jar-delete", 0usize), cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(shown_names(cx, &view), ["b", "example"]);
    assert_eq!(value(cx, 0), apinata_app::response_pane::NAME_MASK);
    assert_eq!(
        value(cx, 1),
        "1",
        "the reveal follows the cookie, not the row"
    );
}
