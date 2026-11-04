use crate::support;

use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, InputEvent as _, MouseButton, MouseDownEvent, MouseMoveEvent, TestAppContext,
    point,
};
use support::*;

fn toasts(cx: &mut TestAppContext, s: &Started) -> usize {
    cx.update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

// Presses on `from` and moves over `onto` at `fraction` of its height, leaving the button down.
fn hold_over(cx: &mut TestAppContext, h: &SidebarHarness, from: &str, onto: &str, fraction: f32) {
    cx.update_window(h.win, |_, w, cx| {
        w.render_frame(cx);
        let start = w.find(row(from)).bounds().center();
        let b = w.find(row(onto)).bounds();
        let end = point(b.center().x, b.origin.y + b.size.height * fraction);
        let at = |position, pressed_button| {
            MouseMoveEvent {
                position,
                pressed_button,
                modifiers: Default::default(),
            }
            .to_platform_input()
        };
        w.dispatch_event(at(start, None), cx);
        let down = MouseDownEvent {
            button: MouseButton::Left,
            position: start,
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        };
        w.dispatch_event(down.to_platform_input(), cx);
        for step in 1..=8 {
            let f = step as f32 / 8.;
            let p = point(
                start.x + (end.x - start.x) * f,
                start.y + (end.y - start.y) * f,
            );
            w.dispatch_event(at(p, Some(MouseButton::Left)), cx);
            w.render_frame(cx);
        }
    })
    .unwrap();
    cx.run_until_parked();
}

fn hover(cx: &mut TestAppContext, h: &SidebarHarness) -> bool {
    h.view.read_with(cx, |s, _| s.hover().is_some())
}

#[gpui_kit::test]
fn a_dragged_set_shows_an_indicator_only_where_it_can_drop(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, multi_files());
    cx.update_window(h.win, |_, w, cx| w.click(row("a.yml"), cx))
        .unwrap();
    secondary_click(cx, h.win, row("b.yml"));
    hold_over(cx, &h, "b.yml", "f", 0.5);
    assert!(hover(cx, &h), "the middle of a folder accepts the set");
    hold_over(cx, &h, "b.yml", "f", 0.1);
    assert!(!hover(cx, &h), "the before zone shows nothing");
    hold_over(cx, &h, "b.yml", "c.yml", 0.5);
    assert!(!hover(cx, &h), "a request row takes no set");
}

#[gpui_kit::test]
fn a_target_inside_the_set_shows_no_indicator(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, multi_files());
    cx.update_window(h.win, |_, w, cx| w.click(row("f"), cx))
        .unwrap();
    secondary_click(cx, h.win, row("a.yml"));
    hold_over(cx, &h, "a.yml", "f", 0.5);
    assert!(!hover(cx, &h), "the set contains the target");
}

#[gpui_kit::test]
fn a_set_moves_in_sidebar_order_whatever_row_is_grabbed(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("F")),
        ("p/folder.yml", &folder("P")),
        ("p/n.yml", &request("FromP", "http://x/p")),
        ("q/folder.yml", &folder("Q")),
        ("q/n.yml", &request("FromQ", "http://x/q")),
    ]);
    let s = started(cx, &col);
    for dir in ["p", "q"] {
        cx.update_window(s.win, |_, w, cx| w.click(row(dir), cx))
            .unwrap();
    }
    cx.update_window(s.win, |_, w, cx| w.click(row("p/n.yml"), cx))
        .unwrap();
    secondary_click(cx, s.win, row("q/n.yml"));
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let start = w.find(row("q/n.yml")).bounds().center();
        let b = w.find(row("f")).bounds();
        w.drag(start, point(b.center().x, b.center().y), cx);
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    let got = std::fs::read_to_string(col.path().join("f/n.yml")).unwrap();
    assert!(
        got.contains("FromP"),
        "p comes first in the sidebar, so it takes the name: {got}"
    );
}

#[gpui_kit::test]
fn a_member_under_a_selected_folder_travels_as_that_folder(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("F")),
        ("f/x.yml", &request("X", "http://x/x")),
        ("t/folder.yml", &folder("T")),
    ]);
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| w.click(row("f"), cx))
        .unwrap();
    secondary_click(cx, s.win, row("f/x.yml"));
    let before = toasts(cx, &s);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let start = w.find(row("f/x.yml")).bounds().center();
        let b = w.find(row("t")).bounds();
        w.drag(start, point(b.center().x, b.center().y), cx);
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    assert!(
        col.path().join("t/f/x.yml").exists(),
        "the folder moved whole"
    );
    assert!(!col.path().join("f").exists());
    assert_eq!(toasts(cx, &s), before, "the child was not moved on its own");
}
