//! The request bar in a narrow window: the method and URL stay on one line and the buttons
//! wrap first, and the main window cannot shrink below the size where the bar still fits.

use crate::support;

use apinata_app::app::window_options;
use apinata_app::session::{MIN_HEIGHT, MIN_WIDTH, Session};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext, px, size};
use support::*;

fn resize(cx: &mut TestAppContext, s: &Started, w: f32, h: f32) {
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(w), px(h)));
    cx.run_until_parked();
    frame(cx, s.win);
}

#[gpui_kit::test]
fn a_narrow_window_keeps_the_url_beside_the_method(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    resize(cx, &s, 640., 700.);
    if !s.shell.read_with(cx, |sh, _| sh.sidebar_visible()) {
        press_in_shell(cx, &s, &key("b"));
        frame(cx, s.win);
    }
    assert!(s.shell.read_with(cx, |sh, _| sh.sidebar_visible()));
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let method = w.find("method").bounds();
        let url = w.find("url").bounds();
        let send = w.find("send-slot").bounds();
        let pane = w.find("request-column").bounds();
        assert_eq!(url.top(), method.top(), "method {method:?} url {url:?}");
        assert!(url.left() > method.right(), "method {method:?} url {url:?}");
        assert!(url.right() <= pane.right(), "url {url:?} pane {pane:?}");
        assert!(send.right() <= pane.right(), "send {send:?} pane {pane:?}");
    })
    .unwrap();
}

#[test]
fn the_main_window_has_the_session_minimum_size() {
    let opts = window_options(&Session::default(), &[]);
    assert_eq!(
        opts.window_min_size,
        Some(size(px(MIN_WIDTH), px(MIN_HEIGHT)))
    );
}

#[gpui_kit::test]
fn the_request_bar_and_sections_fit_the_minimum_window(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    resize(cx, &s, MIN_WIDTH, MIN_HEIGHT);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let pane = w.find("request-column").bounds();
        let method = w.find("method").bounds();
        let url = w.find("url").bounds();
        let send = w.find("send-slot").bounds();
        let tabs = w.find("section-strip").bounds();
        // Too narrow for a usable URL beside the method boxes, it takes its own line.
        assert!(
            url.top() >= method.bottom(),
            "method {method:?} url {url:?}"
        );
        assert!(url.size.width >= method.size.width, "url {url:?}");
        for (name, b) in [
            ("method", method),
            ("url", url),
            ("send", send),
            ("sections", tabs),
        ] {
            assert!(
                b.left() >= pane.left() && b.right() <= pane.right(),
                "{name} {b:?} pane {pane:?}"
            );
        }
        assert!(
            send.top() >= url.bottom(),
            "send wraps below: {send:?} {url:?}"
        );
    })
    .unwrap();
}
