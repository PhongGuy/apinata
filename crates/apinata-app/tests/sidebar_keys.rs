//! The keyboard opens requests from the sidebar: Enter keeps a tab, Space previews, and
//! Enter in the filter opens the first match.

use crate::support;

use apinata_app::sidebar::SidebarEvent;
use apinata_core::ItemPath;
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

fn press(cx: &mut TestAppContext, h: &SidebarHarness, key: &str) {
    cx.update_window(h.win, |_, w, cx| w.press(key, cx))
        .unwrap();
    cx.run_until_parked();
}

fn opened(events: &[SidebarEvent]) -> Vec<(String, bool)> {
    events
        .iter()
        .filter_map(|e| match e {
            SidebarEvent::OpenItem {
                path, permanent, ..
            } => Some((path.as_str().to_owned(), *permanent)),
            _ => None,
        })
        .collect()
}

fn type_filter(cx: &mut TestAppContext, h: &SidebarHarness, text: &str) {
    let filter = h.view.read_with(cx, |s, _| s.filter().clone());
    cx.update_window(h.win, |_, w, cx| filter.update(cx, |i, cx| i.focus(w, cx)))
        .unwrap();
    cx.update_window(h.win, |_, w, cx| w.input(text, cx))
        .unwrap();
    frame(cx, h.win);
}

#[gpui_kit::test]
fn enter_keeps_and_space_previews_the_selected_request(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let seen = sidebar_events(cx, &h.view);
    select(cx, &h, "health.yml");
    press(cx, &h, "enter");
    select(cx, &h, "status.yml");
    press(cx, &h, "space");
    assert_eq!(
        opened(&seen.borrow()),
        [
            ("health.yml".to_owned(), true),
            ("status.yml".to_owned(), false)
        ]
    );
}

#[gpui_kit::test]
fn enter_on_a_folder_toggles_it(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let seen = sidebar_events(cx, &h.view);
    assert!(!shown(cx, h.win, row("users/list.yml")));
    select(cx, &h, "users");
    press(cx, &h, "enter");
    assert!(shown(cx, h.win, row("users/list.yml")), "Enter expands");
    press(cx, &h, "enter");
    assert!(!shown(cx, h.win, row("users/list.yml")), "Enter collapses");
    assert!(opened(&seen.borrow()).is_empty());
}

#[gpui_kit::test]
fn enter_in_the_filter_opens_the_first_match_and_moves_to_the_tree(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let seen = sidebar_events(cx, &h.view);
    type_filter(cx, &h, "stat");
    press(cx, &h, &secondary("enter"));
    assert!(
        opened(&seen.borrow()).is_empty(),
        "ctrl+enter in the filter opens nothing"
    );
    press(cx, &h, "enter");
    assert_eq!(opened(&seen.borrow()), [("status.yml".to_owned(), true)]);
    // Focus moved to the tree with the match selected, so Space previews it.
    press(cx, &h, "space");
    assert_eq!(
        opened(&seen.borrow()),
        [
            ("status.yml".to_owned(), true),
            ("status.yml".to_owned(), false)
        ]
    );
}

// Escape on a sidebar button empties the filter first; the next Escape hands the keys to
// the tree, where Space previews the selected row.
#[gpui_kit::test]
fn escape_on_a_sidebar_button_clears_the_filter_then_returns_to_the_tree(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    // The tree's stand-in hands focus on only in an active window, as the app's is.
    cx.update_window(h.win, |_, w, _| w.activate_window())
        .unwrap();
    let seen = sidebar_events(cx, &h.view);
    type_filter(cx, &h, "stat");
    let tree = h.view.read_with(cx, |s, _| s.tree().clone());
    tree.update(cx, |t, cx| {
        let ix = t.index_of(&row("status.yml"));
        t.set_selected_index(ix, cx)
    });
    tab_to(cx, h.win, "sidebar-history");
    press(cx, &h, "escape");
    let filter = h.view.read_with(cx, |s, _| s.filter().clone());
    assert_eq!(filter.read_with(cx, |i, _| i.value().to_string()), "");
    assert!(holds_focus(cx, h.win, "sidebar-history"), "focus stays");
    press(cx, &h, "escape");
    assert!(!holds_focus(cx, h.win, "sidebar-history"));
    press(cx, &h, "space");
    assert_eq!(opened(&seen.borrow()), [("status.yml".to_owned(), false)]);
}

#[gpui_kit::test]
fn space_while_renaming_types_a_space(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let seen = sidebar_events(cx, &h.view);
    select(cx, &h, "health.yml");
    let (view, root) = (h.view.clone(), h.root.clone());
    cx.update_window(h.win, |_, w, cx| {
        view.update(cx, |s, cx| s.start_rename(&root, &ip("health.yml"), w, cx))
    })
    .unwrap();
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| {
        w.input("My", cx);
        w.press("space", cx);
        w.input("check", cx);
        w.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(opened(&seen.borrow()).is_empty());
    assert!(
        h.root.join("My check.yml").exists(),
        "the space reached the name"
    );
}

#[gpui_kit::test]
fn enter_on_a_folder_while_filtering_clears_the_filter_and_opens_it(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    type_filter(cx, &h, "list");
    select(cx, &h, "users");
    press(cx, &h, "enter");
    frame(cx, h.win);
    let filter = h.view.read_with(cx, |s, _| s.filter().clone());
    assert_eq!(filter.read_with(cx, |i, _| i.value().to_string()), "");
    assert!(shown(cx, h.win, row("health.yml")), "the filter is gone");
    assert!(shown(cx, h.win, row("users/list.yml")), "users stays open");
    let selected = h.view.read_with(cx, |s, cx| s.selected(cx));
    assert_eq!(selected, Some((h.root.clone(), ip("users"))));
}

#[gpui_kit::test]
fn enter_on_a_nested_folder_while_filtering_opens_its_ancestors(cx: &mut TestAppContext) {
    let mut files = sidebar_files();
    files.push(("a/folder.yml", folder("A")));
    files.push(("a/b/folder.yml", folder("B")));
    files.push(("a/b/deep.yml", request("Deep", "http://x/deep")));
    let h = open_sidebar_with(cx, files);
    let collection = apinata_app::sidebar::model::node_id(0, &ItemPath::root());
    let tree = h.view.read_with(cx, |s, _| s.tree().clone());
    tree.update(cx, |t, cx| {
        let ix = t.index_of(&collection);
        t.set_selected_index(ix, cx)
    });
    focus_tree(cx, &h);
    frame(cx, h.win);
    press(cx, &h, "enter");
    assert!(!shown(cx, h.win, row("a")), "the collection is collapsed");
    type_filter(cx, &h, "deep");
    select(cx, &h, "a/b");
    press(cx, &h, "enter");
    let filter = h.view.read_with(cx, |s, _| s.filter().clone());
    assert_eq!(filter.read_with(cx, |i, _| i.value().to_string()), "");
    assert!(shown(cx, h.win, row("a/b")), "a/b stays in view");
    assert!(shown(cx, h.win, row("a/b/deep.yml")), "a/b stays open");
    let selected = h.view.read_with(cx, |s, cx| s.selected(cx));
    assert_eq!(selected, Some((h.root.clone(), ip("a/b"))));
}

#[gpui_kit::test]
fn left_collapses_a_folder_and_its_chevron(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let drawn =
        |cx: &mut TestAppContext| h.view.read_with(cx, |s, _| s.row_expanded(&row("users")));
    select(cx, &h, "users");
    press(cx, &h, "right");
    assert!(shown(cx, h.win, row("users/list.yml")), "Right expands");
    assert_eq!(drawn(cx), Some(true), "the chevron and icon draw open");
    press(cx, &h, "left");
    assert!(!shown(cx, h.win, row("users/list.yml")), "Left collapses");
    assert_eq!(drawn(cx), Some(false), "the chevron and icon draw closed");
}
