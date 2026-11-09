//! A request column that scrolls says so, and the section strip yields a vertical wheel to
//! the column: it moves sideways only on horizontal intent.

use crate::support;

use apinata_app::request_editor::Section;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, InputEvent as _, Modifiers, Pixels, ScrollDelta, ScrollEvent,
    TestAppContext, point, px, size,
};
use support::*;

fn settle(cx: &mut TestAppContext, win: AnyWindowHandle) {
    frame(cx, win);
    frame(cx, win);
}

fn cue(cx: &mut TestAppContext, win: AnyWindowHandle) -> bool {
    settle(cx, win);
    present(cx, win, "request-column-fade")
}

// The first section tab's left edge and the strip's top: where the strip is scrolled to and
// where the column is scrolled to.
fn place(cx: &mut TestAppContext, win: AnyWindowHandle) -> (Pixels, Pixels) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let first = w.find(("section", Section::Params as usize)).bounds();
        (first.left(), w.find("section-strip").bounds().top())
    })
    .unwrap()
}

fn wheel(cx: &mut TestAppContext, win: AnyWindowHandle, delta: (f32, f32), modifiers: Modifiers) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.hover("section-strip", cx);
        let position = w.find("section-strip").bounds().center();
        w.dispatch_event(
            ScrollEvent {
                position,
                delta: ScrollDelta::Pixels(point(px(delta.0), px(delta.1))),
                modifiers,
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
    })
    .unwrap();
    settle(cx, win);
}

#[gpui_kit::test]
fn the_cue_shows_only_while_the_column_overflows(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.simulate_window_resize(win, size(px(900.), px(900.)));
    assert!(!cue(cx, win), "a tall window fits the column");
    cx.simulate_window_resize(win, size(px(360.), px(260.)));
    assert!(cue(cx, win), "a short window overflows the column");
}

#[gpui_kit::test]
fn the_cue_goes_when_the_column_is_scrolled_to_its_end(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.simulate_window_resize(win, size(px(360.), px(260.)));
    assert!(cue(cx, win));
    cx.update_window(win, |_, w, cx| {
        let down = ScrollDelta::Pixels(point(px(0.), px(-5000.)));
        w.scroll("request-column", down, cx)
    })
    .unwrap();
    assert!(!cue(cx, win), "nothing is left below");
}

#[gpui_kit::test]
fn a_vertical_wheel_over_the_strip_scrolls_the_column_not_the_strip(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.simulate_window_resize(win, size(px(360.), px(260.)));
    settle(cx, win);
    let (left, top) = place(cx, win);
    wheel(cx, win, (0., -20.), Modifiers::default());
    let (now_left, now_top) = place(cx, win);
    assert_eq!(now_left, left, "the strip stayed put");
    assert!(now_top < top, "the column scrolled");
}

#[gpui_kit::test]
fn a_horizontal_wheel_or_shift_wheel_scrolls_the_strip_not_the_column(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.simulate_window_resize(win, size(px(360.), px(260.)));
    settle(cx, win);
    let (left, top) = place(cx, win);
    wheel(cx, win, (-20., 0.), Modifiers::default());
    let (l1, t1) = place(cx, win);
    assert!(l1 < left, "a horizontal wheel moved the strip");
    assert_eq!(t1, top, "and left the column");
    let shift = Modifiers {
        shift: true,
        ..Default::default()
    };
    wheel(cx, win, (0., -20.), shift);
    let (l2, t2) = place(cx, win);
    assert!(l2 < l1, "Shift+wheel moved the strip");
    assert_eq!(t2, top, "and left the column");
}
