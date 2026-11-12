//! The cookie dialog asks before a domain delete or a clear, naming what goes and how many.

use crate::support;

use apinata_app::cookie_jar::open_cookie_jar;
use apinata_app::executor::SharedExecutor;
use apinata_core::{CookieInfo, ExecOptions, Executor};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, IntoElement, Render, TestAppContext, Window,
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

/// The dialog over a jar holding `a1` and `a2` under a.test and `b1` under b.test.
fn jar(cx: &mut TestAppContext) -> (AnyWindowHandle, Arc<Executor>) {
    support::init(cx);
    let exec = Arc::new(Executor::new(ExecOptions::new()).unwrap());
    for (domain, name) in [("a.test", "a1"), ("a.test", "a2"), ("b.test", "b1")] {
        exec.cookies()
            .insert(CookieInfo::new(domain, name, "1"))
            .unwrap();
    }
    cx.update(|cx| cx.set_global(SharedExecutor(exec.clone())));
    let (win, _) = cx
        .update(|cx| gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Blank)))
        .unwrap();
    cx.update_window(win, |_, w, cx| open_cookie_jar(w, cx))
        .unwrap();
    frame(cx, win);
    (win, exec)
}

fn names(exec: &Executor) -> Vec<String> {
    exec.cookies().list().into_iter().map(|c| c.name).collect()
}

fn click(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<gpui_kit::ElementId>) {
    cx.update_window(win, |_, w, cx| w.click(id, cx)).unwrap();
    cx.run_until_parked();
}

/// The open confirm dialog's question.
fn question(cx: &mut TestAppContext) -> String {
    let win = confirm_window(cx).expect("a confirm dialog is open");
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find("confirm-dialog")
            .label()
            .unwrap_or_default()
            .to_owned()
    })
    .unwrap()
}

#[gpui_kit::test]
fn delete_domain_asks_with_the_domain_and_count_and_cancel_keeps_them(cx: &mut TestAppContext) {
    let (win, exec) = jar(cx);
    click(cx, win, ("jar-domain-delete", 0usize));
    assert_eq!(question(cx), "Delete 2 cookies for a.test?");
    assert_eq!(
        names(&exec),
        ["a1", "a2", "b1"],
        "nothing goes before the answer"
    );
    answer_confirm(cx, "confirm-cancel");
    frame(cx, win);
    assert!(!confirm_open(cx));
    assert_eq!(names(&exec), ["a1", "a2", "b1"]);
}

#[gpui_kit::test]
fn confirming_delete_domain_removes_only_that_domain(cx: &mut TestAppContext) {
    let (win, exec) = jar(cx);
    click(cx, win, ("jar-domain-delete", 1usize));
    assert_eq!(question(cx), "Delete 1 cookie for b.test?");
    answer_confirm(cx, "confirm-delete");
    frame(cx, win);
    assert_eq!(names(&exec), ["a1", "a2"]);
}

#[gpui_kit::test]
fn clear_all_asks_and_escape_keeps_every_cookie(cx: &mut TestAppContext) {
    let (win, exec) = jar(cx);
    click(cx, win, "jar-clear");
    assert_eq!(question(cx), "Delete all 3 cookies?");
    let prompt = confirm_window(cx).unwrap();
    cx.simulate_keystrokes(prompt, "escape");
    cx.run_until_parked();
    assert!(!confirm_open(cx));
    assert_eq!(names(&exec), ["a1", "a2", "b1"]);
    assert!(
        cx.update_window(win, |_, w, cx| w.has_active_dialog(cx))
            .unwrap(),
        "Escape closes only the question, not the cookie dialog"
    );
}

#[gpui_kit::test]
fn confirming_clear_all_empties_the_jar(cx: &mut TestAppContext) {
    let (win, exec) = jar(cx);
    click(cx, win, "jar-clear");
    answer_confirm(cx, "confirm-delete");
    frame(cx, win);
    assert!(names(&exec).is_empty());
    assert!(present(cx, win, "jar-empty"));
}

#[gpui_kit::test]
fn clear_shown_asks_with_the_search_and_deletes_only_the_matches(cx: &mut TestAppContext) {
    let (win, exec) = jar(cx);
    replace_input(cx, win, None, "jar-search", "a.t");
    frame(cx, win);
    click(cx, win, "jar-clear");
    assert_eq!(question(cx), "Delete 2 cookies matching \"a.t\"?");
    assert_eq!(names(&exec), ["a1", "a2", "b1"]);
    answer_confirm(cx, "confirm-delete");
    frame(cx, win);
    assert_eq!(names(&exec), ["b1"]);
}
