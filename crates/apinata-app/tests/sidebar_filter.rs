use crate::support;

use apinata_app::command::{Command, TreeOp};
use apinata_core::ItemPath;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Focusable, TestAppContext};
use support::*;

fn type_filter(cx: &mut TestAppContext, h: &SidebarHarness, text: &str) {
    let filter = h.view.read_with(cx, |s, _| s.filter().clone());
    cx.update_window(h.win, |_, w, cx| filter.update(cx, |i, cx| i.focus(w, cx)))
        .unwrap();
    cx.update_window(h.win, |_, w, cx| w.input(text, cx))
        .unwrap();
}

#[gpui_kit::test]
fn typing_narrows_the_tree_and_clearing_restores_it(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    type_filter(cx, &h, "LIST");
    assert!(shown(cx, h.win, row("")));
    assert!(shown(cx, h.win, row("users")));
    assert!(shown(cx, h.win, row("users/list.yml")));
    assert!(!shown(cx, h.win, row("health.yml")));
    assert!(!shown(cx, h.win, row("ws.yml")));
    cx.update_window(h.win, |_, w, cx| {
        (0..4).for_each(|_| w.press("backspace", cx))
    })
    .unwrap();
    assert!(shown(cx, h.win, row("health.yml")));
    // The filter's forced expansion was not recorded.
    assert!(!shown(cx, h.win, row("users/list.yml")));
}

#[gpui_kit::test]
fn the_filter_applies_to_later_snapshots(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    type_filter(cx, &h, "health");
    assert!(!shown(cx, h.win, row("status.yml")));
    let create = TreeOp::CreateRequest {
        parent: ItemPath::root(),
        name: "healthz".into(),
    };
    h.ws.update(cx, |w, cx| w.send(&h.root, Command::Tree(create), cx));
    assert!(shown(cx, h.win, row("healthz.yml")));
    assert!(!shown(cx, h.win, row("status.yml")));
}

#[gpui_kit::test]
fn secondary_f_focuses_the_filter(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    focus_tree(cx, &h);
    cx.simulate_keystrokes(h.win, &secondary("f"));
    let filter = h.view.read_with(cx, |s, _| s.filter().clone());
    let focused = cx
        .update_window(h.win, |_, w, cx| filter.focus_handle(cx).is_focused(w))
        .unwrap();
    assert!(focused);
}

#[gpui_kit::test]
fn a_filter_with_no_match_says_so(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    assert!(!shown(cx, h.win, "sidebar-no-match"));
    type_filter(cx, &h, "zzz");
    assert!(!shown(cx, h.win, row("")), "the collection row is hidden");
    assert!(shown(cx, h.win, "sidebar-no-match"));
}

#[gpui_kit::test]
fn secondary_f_selects_the_filter_so_typing_replaces_it(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    type_filter(cx, &h, "LIST");
    focus_tree(cx, &h);
    cx.simulate_keystrokes(h.win, &secondary("f"));
    cx.update_window(h.win, |_, w, cx| w.input("health", cx))
        .unwrap();
    let filter = h.view.read_with(cx, |s, _| s.filter().clone());
    assert_eq!(filter.read_with(cx, |i, _| i.value().to_string()), "health");
    assert!(shown(cx, h.win, row("health.yml")));
}

// Escape clears a typed filter in place; a second Escape leaves the empty filter for the tree.
#[gpui_kit::test]
fn escape_clears_the_filter_then_returns_focus_to_the_tree(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    type_filter(cx, &h, "abc");
    assert!(!shown(cx, h.win, row("health.yml")));
    let filter = h.view.read_with(cx, |s, _| s.filter().clone());
    let in_filter = |cx: &mut TestAppContext| {
        cx.update_window(h.win, |_, w, cx| filter.focus_handle(cx).is_focused(w))
            .unwrap()
    };
    cx.simulate_keystrokes(h.win, "escape");
    cx.run_until_parked();
    assert_eq!(filter.read_with(cx, |i, _| i.value().to_string()), "");
    assert!(shown(cx, h.win, row("health.yml")));
    assert!(shown(cx, h.win, row("ws.yml")));
    assert!(in_filter(cx), "the first Escape keeps focus in the filter");
    cx.simulate_keystrokes(h.win, "escape");
    cx.run_until_parked();
    assert!(!in_filter(cx));
    frame(cx, h.win);
    assert!(
        ring_on(cx, h.win, "sidebar-tree"),
        "the second Escape moves focus to the tree"
    );
}

// Down in the filter moves into the tree on its first visible row, whatever was selected before.
#[gpui_kit::test]
fn down_in_the_filter_moves_to_the_first_visible_row(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let tree = h.view.read_with(cx, |s, _| s.tree().clone());
    focus_tree(cx, &h);
    cx.simulate_keystrokes(h.win, "down");
    cx.run_until_parked();
    assert_ne!(tree.read_with(cx, |t, _| t.selected_index()), Some(0));
    type_filter(cx, &h, "LIST");
    cx.simulate_keystrokes(h.win, "down");
    cx.run_until_parked();
    let filter = h.view.read_with(cx, |s, _| s.filter().clone());
    let in_filter = cx
        .update_window(h.win, |_, w, cx| filter.focus_handle(cx).is_focused(w))
        .unwrap();
    assert!(!in_filter, "Down leaves the filter");
    assert_eq!(filter.read_with(cx, |i, _| i.value().to_string()), "LIST");
    assert_eq!(tree.read_with(cx, |t, _| t.selected_index()), Some(0));
    frame(cx, h.win);
    assert!(ring_on(cx, h.win, "sidebar-tree"), "the tree has focus");
}
