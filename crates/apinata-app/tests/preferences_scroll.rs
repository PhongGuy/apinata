//! The Preferences rows' scroll area: a bar clear of the text and a fade over more content.

use crate::support;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Bounds, Pixels, ScrollDelta, TestAppContext, point, px};
use support::*;

// The kit's vertical scrollbar track is 16 px wide whatever the zoom (gpui-base WIDTH).
const BAR: Pixels = px(16.);

fn open_prefs(cx: &mut TestAppContext, s: &Started) {
    cx.update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap();
    frame(cx, s.win);
    frame(cx, s.win);
}

fn bounds(cx: &mut TestAppContext, s: &Started, id: &'static str) -> Bounds<Pixels> {
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).bounds()
    })
    .unwrap()
}

fn wheel(cx: &mut TestAppContext, s: &Started, dy: f32) {
    for _ in 0..40 {
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.scroll(
                "preferences-rows",
                ScrollDelta::Pixels(point(px(0.), px(dy))),
                cx,
            )
        })
        .unwrap();
    }
    frame(cx, s.win);
    frame(cx, s.win);
}

#[gpui_kit::test]
fn the_sections_stop_short_of_the_scrollbar(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    let scroll = bounds(cx, &s, "preferences-rows");
    for id in ["prefs-appearance", "prefs-network", "pref-ca-file"] {
        let section = bounds(cx, &s, id);
        assert!(
            section.right() <= scroll.right() - BAR,
            "{id} {section:?} runs under the bar of {scroll:?}"
        );
    }
}

#[gpui_kit::test]
fn a_fade_shows_at_the_bottom_while_more_rows_are_below(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    assert!(present(cx, s.win, "prefs-scroll-fade"), "more rows below");
    let scroll = bounds(cx, &s, "preferences-rows");
    let fade = bounds(cx, &s, "prefs-scroll-fade");
    assert_eq!(fade.bottom(), scroll.bottom(), "{fade:?} in {scroll:?}");
    assert!(fade.size.height > px(0.) && fade.size.height < scroll.size.height / 2.);
    wheel(cx, &s, -400.);
    assert!(
        !present(cx, s.win, "prefs-scroll-fade"),
        "scrolled to the end"
    );
    wheel(cx, &s, 400.);
    assert!(present(cx, s.win, "prefs-scroll-fade"), "back at the top");
}

#[gpui_kit::test]
fn no_fade_shows_when_the_rows_fit(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    replace_input(cx, s.win, None, "prefs-search", "tls");
    frame(cx, s.win);
    frame(cx, s.win);
    assert!(present(cx, s.win, "pref-verify-tls"));
    assert!(!present(cx, s.win, "prefs-scroll-fade"));
}

#[gpui_kit::test]
fn clearing_a_search_that_left_a_short_section_brings_the_fade_back(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    assert!(present(cx, s.win, "prefs-scroll-fade"), "more rows below");
    replace_input(cx, s.win, None, "prefs-search", "tls");
    frame(cx, s.win);
    frame(cx, s.win);
    assert!(
        !present(cx, s.win, "prefs-scroll-fade"),
        "one short section fits"
    );
    replace_input(cx, s.win, None, "prefs-search", "");
    frame(cx, s.win);
    frame(cx, s.win);
    assert!(present(cx, s.win, "prefs-scroll-fade"), "search cleared");
}
