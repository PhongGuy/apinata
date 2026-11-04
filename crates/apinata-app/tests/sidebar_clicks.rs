//! Mouse handling in the sidebar: a chevron click on a selected folder, and a mouse-down on
//! the header buttons' gap keeping the tree focused.

use crate::support;

use apinata_app::sidebar::marker_id;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Pixels, Point, TestAppContext, point};
use support::*;

// A left press and release at `at`, as the platform reports them, with no element lookup.
fn press_at(cx: &mut TestAppContext, h: &SidebarHarness, at: Point<Pixels>) {
    use gpui_kit::{InputEvent as _, MouseButton, MouseDownEvent, MouseUpEvent};
    let down = MouseDownEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: Default::default(),
        click_count: 1,
        first_mouse: false,
    };
    let up = MouseUpEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: Default::default(),
        click_count: 1,
    };
    for event in [down.to_platform_input(), up.to_platform_input()] {
        cx.update_window(h.win, |_, w, cx| {
            w.dispatch_event(event, cx);
        })
        .unwrap();
        frame(cx, h.win);
    }
}

fn click(cx: &mut TestAppContext, h: &SidebarHarness, id: impl Into<gpui_kit::ElementId>) {
    let id = id.into();
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| w.click(id, cx)).unwrap();
    frame(cx, h.win);
}

#[gpui_kit::test]
fn a_chevron_click_expands_a_collapsed_folder_that_is_already_selected(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    assert!(
        !shown(cx, h.win, row("users/list.yml")),
        "users starts closed"
    );
    // Selected but still closed, as after an arrow key or a click on a closed row elsewhere.
    let tree = h.view.read_with(cx, |s, _| s.tree().clone());
    tree.update(cx, |t, cx| {
        let ix = t.index_of(&row("users"));
        t.set_selected_index(ix, cx)
    });
    frame(cx, h.win);
    click(cx, &h, marker_id("chevron", &row("users")));
    assert!(
        shown(cx, h.win, row("users/list.yml")),
        "the chevron click expands the selected folder"
    );
    assert_eq!(
        h.view.read_with(cx, |s, _| s.row_expanded(&row("users"))),
        Some(true)
    );
}

#[gpui_kit::test]
fn a_row_click_opens_a_folder_and_the_chevron_toggles_it(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    click(cx, &h, row("users"));
    assert!(
        shown(cx, h.win, row("users/list.yml")),
        "the row click opens it"
    );
    click(cx, &h, marker_id("chevron", &row("users")));
    assert!(
        !shown(cx, h.win, row("users/list.yml")),
        "the chevron toggles it shut"
    );
    click(cx, &h, marker_id("chevron", &row("users")));
    assert!(shown(cx, h.win, row("users/list.yml")), "and open again");
}

#[gpui_kit::test]
fn a_mouse_down_between_the_header_buttons_keeps_the_tree_focused(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let tree = h.view.read_with(cx, |s, _| s.tree().clone());
    focus_tree(cx, &h);
    frame(cx, h.win);
    let selected = |cx: &mut TestAppContext| tree.read_with(cx, |t, _| t.selected_index());
    let before = selected(cx);
    let gap = cx
        .update_window(h.win, |_, w, _| {
            let a = w.find(("new-request", 0usize)).bounds();
            let b = w.find(("new-folder", 0usize)).bounds();
            point((a.right() + b.left()) / 2.0, a.center().y)
        })
        .unwrap();
    press_at(cx, &h, gap);
    cx.update_window(h.win, |_, w, cx| w.press("down", cx))
        .unwrap();
    cx.run_until_parked();
    assert_ne!(selected(cx), before, "Down still moves the selection");
}
