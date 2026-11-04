//! The response tab strip is reached by keyboard: each tab is a Tab stop, Enter or Space
//! opens it, and keyboard focus draws an outline.

use crate::support;

use apinata_app::focus_mode::{self, Modality};
use apinata_app::response_pane::{ResponsePane, ResponseTab};
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, px, size};
use support::*;

fn pane(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<ResponsePane>, Sent) {
    let s = send_once(cx, wiremock::ResponseTemplate::new(200), "");
    support::init(cx);
    let model = s.model.clone();
    let (win, pane) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    cx.simulate_window_resize(win, size(px(1400.), px(600.)));
    frame(cx, win);
    (win, pane, s)
}

fn focused(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    pane: &Entity<ResponsePane>,
    tab: ResponseTab,
) -> bool {
    let h = pane.read_with(cx, |p, _| p.tab_focus(tab));
    cx.update_window(win, |_, w, _| h.is_focused(w)).unwrap()
}

fn press(cx: &mut TestAppContext, win: AnyWindowHandle, keys: &str) {
    cx.simulate_keystrokes(win, keys);
    cx.run_until_parked();
}

#[gpui_kit::test]
fn tab_reaches_each_response_tab_and_enter_opens_it(cx: &mut TestAppContext) {
    let (win, pane, _s) = pane(cx);
    let body = pane.read_with(cx, |p, _| p.tab_focus(ResponseTab::Body));
    cx.update_window(win, |_, w, cx| body.focus(w, cx)).unwrap();
    press(cx, win, "tab");
    assert!(
        focused(cx, win, &pane, ResponseTab::Headers),
        "then Headers"
    );
    press(cx, win, "enter");
    assert_eq!(pane.read_with(cx, |p, _| p.tab()), ResponseTab::Headers);
    press(cx, win, "tab");
    assert!(
        focused(cx, win, &pane, ResponseTab::Cookies),
        "then Cookies"
    );
    press(cx, win, "space");
    assert_eq!(pane.read_with(cx, |p, _| p.tab()), ResponseTab::Cookies);
    press(cx, win, "shift-tab");
    assert!(
        focused(cx, win, &pane, ResponseTab::Headers),
        "Shift-Tab goes back"
    );
}

#[gpui_kit::test]
fn keyboard_focus_outlines_the_response_tab(cx: &mut TestAppContext) {
    let (win, pane, _s) = pane(cx);
    let h = pane.read_with(cx, |p, _| p.tab_focus(ResponseTab::Timeline));
    cx.update_window(win, |_, w, cx| h.focus(w, cx)).unwrap();
    cx.update(|cx| focus_mode::set(Modality::Keyboard, cx));
    frame(cx, win);
    let ix = ResponseTab::Timeline as usize;
    assert!(
        present(cx, win, ("response-tab-focus", ix)),
        "keyboard focus draws the outline"
    );
    cx.update(|cx| focus_mode::set(Modality::Pointer, cx));
    frame(cx, win);
    assert!(
        !present(cx, win, ("response-tab-focus", ix)),
        "pointer focus draws none"
    );
}

// Enter and Space with a modifier held leave the tab unopened.
#[gpui_kit::test]
fn modified_enter_or_space_does_not_open_a_response_tab(cx: &mut TestAppContext) {
    let (win, pane, _s) = pane(cx);
    let h = pane.read_with(cx, |p, _| p.tab_focus(ResponseTab::Headers));
    cx.update_window(win, |_, w, cx| h.focus(w, cx)).unwrap();
    for keys in ["shift-enter", "alt-space", "shift-space"] {
        press(cx, win, keys);
        assert_eq!(
            pane.read_with(cx, |p, _| p.tab()),
            ResponseTab::Body,
            "{keys} leaves Body open"
        );
    }
}

#[gpui_kit::test]
fn left_and_right_move_focus_between_response_tabs_and_stop_at_the_ends(cx: &mut TestAppContext) {
    let (win, pane, _s) = pane(cx);
    let body = pane.read_with(cx, |p, _| p.tab_focus(ResponseTab::Body));
    cx.update_window(win, |_, w, cx| body.focus(w, cx)).unwrap();
    press(cx, win, "left");
    assert!(
        focused(cx, win, &pane, ResponseTab::Body),
        "no wrap at the start"
    );
    press(cx, win, "right");
    assert!(
        focused(cx, win, &pane, ResponseTab::Headers),
        "right to Headers"
    );
    assert_eq!(
        pane.read_with(cx, |p, _| p.tab()),
        ResponseTab::Body,
        "moving does not open"
    );
    press(cx, win, "left");
    assert!(
        focused(cx, win, &pane, ResponseTab::Body),
        "left back to Body"
    );
}
