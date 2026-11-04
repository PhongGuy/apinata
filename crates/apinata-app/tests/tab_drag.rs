//! Dragging a tab onto another tab of the same collection reorders it and saves the order.

use crate::support;

use apinata_app::tab_area::{DraggedTab, drop_side};
use apinata_app::tabs::{Side, TabId};
use gpui_kit::AppContext as _;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{Bounds, TestAppContext, point, px, size};
use support::*;

fn three(cx: &mut TestAppContext) -> (Area, [TabId; 3]) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/a")),
        ("b.yml", &request("b", "http://x/b")),
        ("c.yml", &request("c", "http://x/c")),
    ]);
    let a = open_area_of(cx, col);
    let ids = ["a.yml", "b.yml", "c.yml"].map(|n| open_tab(cx, &a.ws, &a.root, n));
    cx.run_until_parked();
    (a, ids)
}

fn saved(cx: &mut TestAppContext, a: &Area) -> Vec<String> {
    let session = a.ws.read_with(cx, |w, _| w.session().clone());
    session_value(cx, &session).collections[&a.root]
        .tabs
        .clone()
}

fn drag_tab(cx: &mut TestAppContext, a: &Area, from: usize, onto: usize, right: bool) {
    cx.update_window(a.win, |_, w, cx| {
        w.render_frame(cx);
        let start = w.find(("tab", from)).bounds().center();
        let b = w.find(("tab", onto)).bounds();
        let x = if right {
            b.right() - px(4.)
        } else {
            b.left() + px(4.)
        };
        w.drag(start, point(x, b.center().y), cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[test]
fn the_pointer_half_picks_the_side() {
    let b = Bounds::new(point(px(100.), px(0.)), size(px(80.), px(20.)));
    assert_eq!(drop_side(px(110.), b), Side::Before);
    assert_eq!(drop_side(px(170.), b), Side::After);
}

#[gpui_kit::test]
fn dropping_on_the_right_half_puts_the_tab_after_and_saves_the_order(cx: &mut TestAppContext) {
    let (a, _) = three(cx);
    drag_tab(cx, &a, 0, 1, true);
    assert_eq!(tab_names(cx, &a.ws), ["b.yml", "a.yml", "c.yml"]);
    assert_eq!(saved(cx, &a), ["b.yml", "a.yml", "c.yml"]);
}

#[gpui_kit::test]
fn dropping_on_the_left_half_puts_the_tab_before(cx: &mut TestAppContext) {
    let (a, _) = three(cx);
    drag_tab(cx, &a, 2, 0, false);
    assert_eq!(tab_names(cx, &a.ws), ["c.yml", "a.yml", "b.yml"]);
}

#[gpui_kit::test]
fn a_drag_does_not_change_the_active_tab(cx: &mut TestAppContext) {
    let (a, ids) = three(cx);
    drag_tab(cx, &a, 0, 2, true);
    let active = a.ws.read_with(cx, |w, _| w.tabs().active().map(|e| e.id));
    assert_eq!(active, Some(ids[2]));
}

#[gpui_kit::test]
fn dropping_a_tab_on_itself_changes_nothing(cx: &mut TestAppContext) {
    let (a, _) = three(cx);
    let before = saved(cx, &a);
    drag_tab(cx, &a, 1, 1, true);
    assert_eq!(tab_names(cx, &a.ws), ["a.yml", "b.yml", "c.yml"]);
    assert_eq!(saved(cx, &a), before);
}

#[gpui_kit::test]
fn a_tab_of_another_collection_is_not_a_drop_target(cx: &mut TestAppContext) {
    let (a, _) = three(cx);
    let other = write_tree(&[
        ("opencollection.yml", ROOT),
        ("z.yml", &request("z", "http://x/z")),
    ]);
    let root2 = other.path().to_path_buf();
    a.ws.update(cx, |w, cx| w.open(root2.clone(), cx));
    cx.run_until_parked();
    open_tab(cx, &a.ws, &root2, "z.yml");
    cx.run_until_parked();
    let before = tab_names(cx, &a.ws);
    drag_tab(cx, &a, 0, 3, false);
    assert_eq!(tab_names(cx, &a.ws), before);
    drag_tab(cx, &a, 3, 0, true);
    assert_eq!(tab_names(cx, &a.ws), before);
}

#[gpui_kit::test]
fn the_indicator_shows_only_over_a_valid_target(cx: &mut TestAppContext) {
    let (a, ids) = three(cx);
    let payload = |id| DraggedTab::new(id, a.root.clone(), "x".into());
    let shown = |cx: &mut TestAppContext| {
        cx.update_window(a.win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find("tab-drop-indicator").is_some()
        })
        .unwrap()
    };
    assert!(!shown(cx));
    a.view.update(cx, |v, cx| {
        v.hover_drop(&payload(ids[0]), ids[1], Some(Side::After), cx)
    });
    assert!(shown(cx), "over another tab");
    a.view.update(cx, |v, cx| {
        v.hover_drop(&payload(ids[0]), ids[1], None, cx);
        v.hover_drop(&payload(ids[0]), ids[0], Some(Side::After), cx)
    });
    assert!(!shown(cx), "not over itself");
    a.view.update(cx, |v, cx| {
        v.hover_drop(&payload(ids[0]), ids[1], Some(Side::Before), cx)
    });
    a.view
        .update(cx, |v, cx| v.hover_drop(&payload(ids[0]), ids[1], None, cx));
    assert!(!shown(cx), "cleared once the pointer leaves");
}
