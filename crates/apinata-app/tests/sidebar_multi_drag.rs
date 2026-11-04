use crate::support;

use gpui_kit::AppContext as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{TestAppContext, point};
use support::*;

fn drag_onto(cx: &mut TestAppContext, h: &SidebarHarness, from: &str, onto: &str, fraction: f32) {
    cx.update_window(h.win, |_, w, cx| {
        w.render_frame(cx);
        let start = w.find(row(from)).bounds().center();
        let b = w.find(row(onto)).bounds();
        w.drag(
            start,
            point(b.center().x, b.origin.y + b.size.height * fraction),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
}

fn keys(cx: &mut TestAppContext, h: &SidebarHarness) -> Vec<String> {
    h.view.read_with(cx, |s, _| {
        s.multi_keys()
            .into_iter()
            .map(|(_, p)| p.as_str().to_owned())
            .collect()
    })
}

fn select(cx: &mut TestAppContext, h: &SidebarHarness, first: &str, second: &str) {
    cx.update_window(h.win, |_, w, cx| w.click(row(first), cx))
        .unwrap();
    secondary_click(cx, h.win, row(second));
}

#[gpui_kit::test]
fn dragging_a_member_drops_the_whole_set_inside_the_folder(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, multi_files());
    select(cx, &h, "a.yml", "b.yml");
    drag_onto(cx, &h, "b.yml", "f", 0.5);
    for name in ["a.yml", "b.yml"] {
        assert!(h.root.join("f").join(name).exists(), "{name} moved into f");
        assert!(!h.root.join(name).exists());
    }
    assert!(h.root.join("c.yml").exists());
    assert_eq!(h.view.read_with(cx, |s, _| s.hover()), None);
}

#[gpui_kit::test]
fn dragging_a_row_outside_the_set_clears_it_and_moves_that_row_alone(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, multi_files());
    select(cx, &h, "a.yml", "b.yml");
    drag_onto(cx, &h, "c.yml", "f", 0.5);
    assert!(h.root.join("f/c.yml").exists());
    assert!(h.root.join("a.yml").exists() && h.root.join("b.yml").exists());
    assert!(keys(cx, &h).is_empty());
}

#[gpui_kit::test]
fn before_and_after_zones_move_nothing_for_a_set(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, multi_files());
    select(cx, &h, "a.yml", "b.yml");
    drag_onto(cx, &h, "b.yml", "c.yml", 0.1);
    drag_onto(cx, &h, "b.yml", "c.yml", 0.9);
    for name in ["a.yml", "b.yml", "c.yml"] {
        assert!(h.root.join(name).exists(), "{name} stayed");
    }
    assert_eq!(
        keys(cx, &h),
        ["a.yml", "b.yml"],
        "a rejected drop keeps the set"
    );
}

#[gpui_kit::test]
fn a_set_that_contains_the_target_is_rejected_whole(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, multi_files());
    cx.update_window(h.win, |_, w, cx| w.click(row("f"), cx))
        .unwrap(); // expands and selects f
    secondary_click(cx, h.win, row("a.yml"));
    drag_onto(cx, &h, "a.yml", "f", 0.5);
    assert!(h.root.join("a.yml").exists() && !h.root.join("f/a.yml").exists());
}

#[gpui_kit::test]
fn members_already_inside_the_target_are_skipped_silently(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, multi_files());
    cx.update_window(h.win, |_, w, cx| w.click(row("f"), cx))
        .unwrap(); // expands f
    select(cx, &h, "a.yml", "f/x.yml");
    drag_onto(cx, &h, "a.yml", "f", 0.5);
    assert!(h.root.join("f/a.yml").exists() && !h.root.join("a.yml").exists());
    assert!(h.root.join("f/x.yml").exists());
}
