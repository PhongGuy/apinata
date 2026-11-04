//! `ui::checkbox`: the kit control's behaviour with a neutral look.

use crate::support;

use std::cell::RefCell;
use std::rc::Rc;

use apinata_app::ui::{checkbox, checkbox_look};
use gpui_kit::component::{ActiveTheme as _, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, FocusHandle, InteractiveElement, IntoElement,
    ParentElement, Render, Styled, TestAppContext, Window, WindowOptions, div,
};
use support::*;

struct Host {
    focus: FocusHandle,
    checked: bool,
    disabled: bool,
    tab_stop: bool,
    seen: Rc<RefCell<Vec<bool>>>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let seen = self.seen.clone();
        div().id("host").size_full().track_focus(&self.focus).child(
            checkbox("box", self.checked)
                .label("Remember")
                .disabled(self.disabled)
                .tab_stop(self.tab_stop)
                .on_click(move |on, _, _| seen.borrow_mut().push(*on)),
        )
    }
}

fn host(
    cx: &mut TestAppContext,
    checked: bool,
    disabled: bool,
    tab_stop: bool,
) -> (AnyWindowHandle, Rc<RefCell<Vec<bool>>>) {
    support::init(cx);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, move |_, cx| {
                cx.new(|cx| Host {
                    focus: cx.focus_handle(),
                    checked,
                    disabled,
                    tab_stop,
                    seen: sink,
                })
            })
        })
        .unwrap();
    cx.update_window(win, |_, w, cx| {
        let focus = view.read(cx).focus.clone();
        focus.focus(w, cx);
    })
    .unwrap();
    frame(cx, win);
    (win, seen)
}

#[gpui_kit::test]
fn a_click_asks_for_the_flipped_value_and_leaves_focus(cx: &mut TestAppContext) {
    let (win, seen) = host(cx, false, false, true);
    cx.update_window(win, |_, w, cx| {
        w.click("box", cx);
        w.render_frame(cx);
        let b = w.find("box");
        assert_eq!(b.checked(), Some(false), "controlled: the owner decides");
        assert_eq!(b.label(), Some("Remember"));
        assert_ne!(b.focused(), Some(true), "a click does not take focus");
    })
    .unwrap();
    assert_eq!(*seen.borrow(), [true]);
}

#[gpui_kit::test]
fn space_toggles_the_focused_box(cx: &mut TestAppContext) {
    let (win, seen) = host(cx, true, false, true);
    cx.update_window(win, |_, w, cx| {
        w.focus_next(cx);
        w.render_frame(cx);
        assert_eq!(w.find("box").focused(), Some(true), "Tab reaches the box");
        w.press("space", cx);
    })
    .unwrap();
    assert_eq!(*seen.borrow(), [false]);
}

#[gpui_kit::test]
fn a_disabled_box_ignores_clicks(cx: &mut TestAppContext) {
    let (win, seen) = host(cx, true, true, true);
    cx.update_window(win, |_, w, cx| w.click("box", cx))
        .unwrap();
    assert!(seen.borrow().is_empty());
}

#[gpui_kit::test]
fn a_box_without_a_tab_stop_is_skipped(cx: &mut TestAppContext) {
    let (win, _) = host(cx, false, false, false);
    cx.update_window(win, |_, w, cx| {
        w.focus_next(cx);
        w.render_frame(cx);
        assert_ne!(w.find("box").focused(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_tab_focused_box_draws_the_focus_ring(cx: &mut TestAppContext) {
    use apinata_app::focus_mode::{self, Modality};
    let (win, _) = host(cx, false, false, true);
    cx.update(|cx| focus_mode::set(Modality::Keyboard, cx));
    let ring = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.within("box").try_find("checkbox-focus").is_some()
        })
        .unwrap()
    };
    let bounds = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find("box").bounds()
        })
        .unwrap()
    };
    assert!(!ring(cx), "no ring before focus");
    let before = bounds(cx);
    cx.update_window(win, |_, w, cx| w.focus_next(cx)).unwrap();
    assert!(ring(cx), "Tab focus draws the ring");
    assert_eq!(bounds(cx), before, "focusing never changes layout");
    cx.update(|cx| {
        let t = cx.theme();
        assert!(
            t.focus_ring && t.ring == t.primary,
            "keyboard: the accent ring"
        );
    });
}

#[gpui_kit::test]
fn the_box_is_neutral_and_a_disabled_tick_is_muted(cx: &mut TestAppContext) {
    support::init(cx);
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        cx.update(|cx| apinata_app::theme::set_mode(mode, None, cx));
        cx.update(|cx| {
            let t = cx.theme();
            let on = checkbox_look(true, false, cx);
            assert_eq!(
                (on.border, on.fill, on.tick, on.opacity),
                (t.input, t.secondary, Some(t.foreground), 1.0),
                "{mode:?}"
            );
            assert!(
                on.fill != t.primary && on.tick != Some(t.primary),
                "{mode:?}"
            );
            assert_eq!(checkbox_look(false, false, cx).tick, None, "{mode:?}");
            let locked = checkbox_look(true, true, cx);
            assert_eq!(locked.tick, Some(t.muted_foreground), "{mode:?}");
            assert!(locked.opacity < 1.0, "{mode:?}");
            assert_ne!(locked, on, "{mode:?} disabled-checked reads as disabled");
        });
    }
}
