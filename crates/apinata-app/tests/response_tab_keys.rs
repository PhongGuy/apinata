//! The response tab strip is reached by keyboard: Tab lands once on the chosen tab, the
//! arrows move focus, and Enter or Space opens the focused tab.

use crate::support;

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

// The strip tab holding keyboard focus, after a frame.
fn focused(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    pane: &Entity<ResponsePane>,
) -> Option<ResponseTab> {
    use gpui_kit::test::TestWindowExt as _;
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        pane.read(cx).focused_tab(w, cx)
    })
    .unwrap()
}

fn press(cx: &mut TestAppContext, win: AnyWindowHandle, keys: &str) {
    cx.simulate_keystrokes(win, keys);
    cx.run_until_parked();
}

// A whole key press, down and up: the kit activates the focused tab on release.
fn activate(cx: &mut TestAppContext, win: AnyWindowHandle, key: &str) {
    use gpui_kit::test::TestWindowExt as _;
    cx.update_window(win, |_, w, cx| w.press(key, cx)).unwrap();
    cx.run_until_parked();
}

// Moves focus into the strip with one Tab press, where it lands on the chosen tab.
fn enter_strip(cx: &mut TestAppContext, win: AnyWindowHandle, pane: &Entity<ResponsePane>) {
    cx.update_window(win, |_, w, cx| w.focus_next(cx)).unwrap();
    assert_eq!(focused(cx, win, pane), Some(ResponseTab::Body));
}

#[gpui_kit::test]
fn tab_lands_once_on_the_chosen_response_tab_and_enter_opens_the_focused_one(
    cx: &mut TestAppContext,
) {
    let (win, pane, _s) = pane(cx);
    enter_strip(cx, win, &pane);
    press(cx, win, "right");
    assert_eq!(focused(cx, win, &pane), Some(ResponseTab::Headers));
    assert_eq!(pane.read_with(cx, |p, _| p.tab()), ResponseTab::Body);
    activate(cx, win, "enter");
    assert_eq!(pane.read_with(cx, |p, _| p.tab()), ResponseTab::Headers);
    press(cx, win, "right");
    assert_eq!(focused(cx, win, &pane), Some(ResponseTab::Cookies));
    activate(cx, win, "space");
    assert_eq!(pane.read_with(cx, |p, _| p.tab()), ResponseTab::Cookies);
    press(cx, win, "tab");
    assert_eq!(focused(cx, win, &pane), None, "one Tab leaves the strip");
    press(cx, win, "shift-tab");
    assert_eq!(
        focused(cx, win, &pane),
        Some(ResponseTab::Cookies),
        "Shift+Tab returns to the chosen tab"
    );
}

// Enter and Space with a modifier held leave the tab unopened.
#[gpui_kit::test]
fn modified_enter_or_space_does_not_open_a_response_tab(cx: &mut TestAppContext) {
    let (win, pane, _s) = pane(cx);
    enter_strip(cx, win, &pane);
    press(cx, win, "right");
    assert_eq!(focused(cx, win, &pane), Some(ResponseTab::Headers));
    for keys in ["shift-enter", "alt-space", "shift-space"] {
        activate(cx, win, keys);
        assert_eq!(
            pane.read_with(cx, |p, _| p.tab()),
            ResponseTab::Body,
            "{keys} leaves Body open"
        );
    }
}

#[gpui_kit::test]
fn arrows_home_and_end_move_focus_between_response_tabs_without_opening(cx: &mut TestAppContext) {
    let (win, pane, _s) = pane(cx);
    enter_strip(cx, win, &pane);
    press(cx, win, "right");
    assert_eq!(focused(cx, win, &pane), Some(ResponseTab::Headers));
    press(cx, win, "left");
    assert_eq!(focused(cx, win, &pane), Some(ResponseTab::Body));
    press(cx, win, "left");
    let last = focused(cx, win, &pane);
    assert!(
        last.is_some_and(|t| t != ResponseTab::Body),
        "left from the first wraps to the last: {last:?}"
    );
    press(cx, win, "home");
    assert_eq!(focused(cx, win, &pane), Some(ResponseTab::Body));
    press(cx, win, "end");
    assert_eq!(focused(cx, win, &pane), last);
    assert_eq!(
        pane.read_with(cx, |p, _| p.tab()),
        ResponseTab::Body,
        "moving does not open"
    );
}
