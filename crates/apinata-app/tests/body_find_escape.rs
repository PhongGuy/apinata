//! Escape inside the response body closes an open find bar and keeps focus in the body; with
//! the bar closed the key passes on to the ancestors.

use crate::support;

use std::sync::Arc;

use apinata_app::body_view::{BodyView, Shown};
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, Entity, Focusable as _, InteractiveElement,
    IntoElement, KeyBinding, ParentElement, Render, Styled, TestAppContext, Window, WindowOptions,
    actions, div,
};
use support::*;

actions!(escape_probe, [Outer]);

struct Host {
    body: Entity<BodyView>,
    outer: usize,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .key_context("EscapeHost")
            .on_action(cx.listener(|h, _: &Outer, _, _| h.outer += 1))
            .child(self.body.clone())
    }
}

fn long_json() -> String {
    let items: Vec<String> = (0..30_000).map(|i| format!("{{\"id\":{i}}}")).collect();
    format!("[{}]", items.join(","))
}

// A body of the wanted mode under a host that counts a bare Escape reaching it, with the
// real keymap and focus on the body.
fn open(
    cx: &mut TestAppContext,
    text: String,
) -> (AnyWindowHandle, Entity<Host>, Entity<BodyView>) {
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    cx.update(|cx| cx.bind_keys([KeyBinding::new("escape", Outer, Some("EscapeHost"))]));
    let (win, host) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| Host {
                    body: cx.new(|cx| BodyView::new(w, cx)),
                    outer: 0,
                })
            })
        })
        .unwrap();
    let view = host.read_with(cx, |h, _| h.body.clone());
    let r = Arc::new(RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(
            Response::new(200)
                .with_header("content-type", "application/json")
                .with_text_body(text),
        ),
        Vec::new(),
    ));
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(r), w, cx))
    })
    .unwrap();
    frame(cx, win);
    frame(cx, win);
    (win, host, view)
}

fn focus_body(cx: &mut TestAppContext, win: AnyWindowHandle, view: &Entity<BodyView>) {
    cx.update_window(win, |_, w, cx| {
        let handle = if view.read(cx).shown() == Shown::Editor {
            view.read(cx).editor().read(cx).focus_handle(cx)
        } else {
            view.read(cx).body_focus().clone()
        };
        handle.focus(w, cx);
    })
    .unwrap();
    frame(cx, win);
}

fn open_find_and_focus_body(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    view: &Entity<BodyView>,
) {
    cx.update_window(win, |_, w, cx| view.update(cx, |v, cx| v.open_find(w, cx)))
        .unwrap();
    frame(cx, win);
    assert!(view.read_with(cx, |v, _| v.find_open()));
    focus_body(cx, win, view);
}

fn body_has_focus(cx: &mut TestAppContext, win: AnyWindowHandle, view: &Entity<BodyView>) -> bool {
    cx.update_window(win, |_, w, cx| {
        let v = view.read(cx);
        let handle = if v.shown() == Shown::Editor {
            v.editor().read(cx).focus_handle(cx)
        } else {
            v.body_focus().clone()
        };
        handle.is_focused(w)
    })
    .unwrap()
}

fn escape_closes_and_keeps_focus(cx: &mut TestAppContext, text: String, editor: bool) {
    let (win, host, view) = open(cx, text);
    assert_eq!(
        view.read_with(cx, |v, _| v.shown()) == Shown::Editor,
        editor
    );
    open_find_and_focus_body(cx, win, &view);
    cx.simulate_keystrokes(win, "escape");
    frame(cx, win);
    assert!(!view.read_with(cx, |v, _| v.find_open()));
    assert!(body_has_focus(cx, win, &view));
    assert_eq!(host.read_with(cx, |h, _| h.outer), 0);
}

#[gpui_kit::test]
fn escape_in_the_rows_body_closes_the_find_bar(cx: &mut TestAppContext) {
    escape_closes_and_keeps_focus(cx, long_json(), false);
}

#[gpui_kit::test]
fn escape_in_the_editor_body_closes_the_find_bar(cx: &mut TestAppContext) {
    escape_closes_and_keeps_focus(cx, "{\"a\":1}".into(), true);
}

#[gpui_kit::test]
fn escape_with_the_bar_closed_reaches_the_ancestors(cx: &mut TestAppContext) {
    for text in [long_json(), "{\"a\":1}".to_string()] {
        let (win, host, view) = open(cx, text);
        focus_body(cx, win, &view);
        cx.simulate_keystrokes(win, "escape");
        frame(cx, win);
        assert_eq!(host.read_with(cx, |h, _| h.outer), 1);
    }
}
