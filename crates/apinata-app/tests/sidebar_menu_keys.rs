//! The Menu key and Shift+F10 open the selected row's context menu from the keyboard, with
//! the right-click menu's entries, the first one highlighted.

use crate::support;

use apinata_core::RequestKind;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

fn select(cx: &mut TestAppContext, h: &SidebarHarness, path: &str) {
    let tree = h.view.read_with(cx, |s, _| s.tree().clone());
    tree.update(cx, |t, cx| {
        let ix = t.index_of(&row(path));
        assert!(ix.is_some(), "{path} is shown");
        t.set_selected_index(ix, cx)
    });
    focus_tree(cx, h);
    frame(cx, h.win);
}

fn keys(cx: &mut TestAppContext, h: &SidebarHarness, keys: &str) {
    cx.simulate_keystrokes(h.win, keys);
    cx.run_until_parked();
    frame(cx, h.win);
}

fn labels(cx: &mut TestAppContext, h: &SidebarHarness) -> Vec<Option<String>> {
    cx.update_window(h.win, |_, w, _| {
        if w.try_find("popup-menu").is_none() {
            return Vec::new();
        }
        menu_labels(w)
    })
    .unwrap()
}

// The entries the row's right-click menu lists, read before the keyboard is tried.
fn right_click_labels(
    cx: &mut TestAppContext,
    h: &SidebarHarness,
    path: &str,
) -> Vec<Option<String>> {
    cx.update_window(h.win, |_, w, cx| w.right_click(row(path), cx))
        .unwrap();
    frame(cx, h.win);
    let shown = labels(cx, h);
    keys(cx, h, "escape");
    assert!(labels(cx, h).is_empty(), "the right-click menu closed");
    assert!(!shown.is_empty(), "{path} has a menu");
    shown
}

fn tree_focused(cx: &mut TestAppContext, h: &SidebarHarness) -> bool {
    let view = h.view.clone();
    cx.update_window(h.win, |_, w, cx| view.read(cx).tree_holds_focus(w, cx))
        .unwrap()
}

#[gpui_kit::test]
fn shift_f10_opens_a_folder_rows_menu_with_its_right_click_entries(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let clicked = right_click_labels(cx, &h, "users");
    select(cx, &h, "users");
    assert!(labels(cx, &h).is_empty(), "no menu before the key");
    keys(cx, &h, "shift-f10");
    assert_eq!(labels(cx, &h), clicked);
}

#[gpui_kit::test]
fn the_menu_key_opens_a_request_rows_menu_with_its_right_click_entries(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let clicked = right_click_labels(cx, &h, "status.yml");
    select(cx, &h, "status.yml");
    keys(cx, &h, "menu");
    assert_eq!(labels(cx, &h), clicked);
}

#[gpui_kit::test]
fn enter_runs_the_first_entry_of_the_key_opened_menu(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    select(cx, &h, "status.yml");
    keys(cx, &h, "shift-f10");
    keys(cx, &h, "enter");
    assert!(labels(cx, &h).is_empty(), "the menu closed");
    assert!(
        h.root.join("Status copy.yml").exists(),
        "the first entry, Duplicate, ran"
    );
}

#[gpui_kit::test]
fn down_then_enter_runs_the_second_entry(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    select(cx, &h, "users");
    keys(cx, &h, "menu");
    keys(cx, &h, "down");
    keys(cx, &h, "enter");
    let tabs = tab_names(cx, &h.ws);
    let [draft] = tabs.as_slice() else {
        panic!("one draft opened: {tabs:?}")
    };
    let view = view_of(cx, &h.ws, &h.root, draft);
    assert_eq!(view.kind, RequestKind::Graphql, "New GraphQL request ran");
}

#[gpui_kit::test]
fn escape_closes_the_key_opened_menu_and_returns_to_the_tree(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    select(cx, &h, "health.yml");
    keys(cx, &h, "shift-f10");
    assert!(!labels(cx, &h).is_empty(), "the menu opened");
    assert!(!tree_focused(cx, &h), "the menu holds focus");
    keys(cx, &h, "escape");
    assert!(labels(cx, &h).is_empty(), "the menu closed");
    assert!(tree_focused(cx, &h), "focus is back on the tree");
    let selected = h.view.read_with(cx, |s, cx| s.selected(cx));
    assert_eq!(selected.map(|(_, p)| p), Some(ip("health.yml")));
}
