//! Collapse all and Expand all: header buttons for every collection, entries on the
//! collection row's menu for that collection, and palette commands. All of them write the
//! same kept set a manual chevron click writes.

use crate::support;

use apinata_app::commands::registry;
use apinata_app::sidebar::menu::{MenuEntry, menu_entries};
use apinata_app::sidebar::model::NodeKind;
use apinata_app::sidebar::{CollapseAll, ExpandAll};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

fn nested() -> Vec<(&'static str, String)> {
    let mut files = sidebar_files();
    files.push(("users/admin/folder.yml", folder("admin")));
    files.push((
        "users/admin/create.yml",
        request("Create admin", "http://x/admin"),
    ));
    files.push(("docs/folder.yml", folder("docs")));
    files.push(("docs/intro.yml", request("Intro", "http://x/docs")));
    files
}

fn kept(cx: &mut TestAppContext, h: &SidebarHarness) -> Vec<String> {
    let mut paths: Vec<String> = h
        .view
        .read_with(cx, |s, _| s.expanded_paths(&h.root))
        .iter()
        .map(|p| p.as_str().to_owned())
        .collect();
    paths.sort();
    paths
}

fn run_expand_all(cx: &mut TestAppContext, h: &SidebarHarness) {
    cx.update_window(h.win, |_, w, cx| {
        h.view.update(cx, |s, cx| s.expand_all(&ExpandAll, w, cx))
    })
    .unwrap();
    frame(cx, h.win);
}

fn run_collapse_all(cx: &mut TestAppContext, h: &SidebarHarness) {
    cx.update_window(h.win, |_, w, cx| {
        h.view
            .update(cx, |s, cx| s.collapse_all(&CollapseAll, w, cx))
    })
    .unwrap();
    frame(cx, h.win);
}

#[gpui_kit::test]
fn expand_all_opens_every_folder_and_collapse_all_closes_them_but_keeps_the_collection(
    cx: &mut TestAppContext,
) {
    let h = open_sidebar_with(cx, nested());
    run_expand_all(cx, &h);
    assert_eq!(kept(cx, &h), ["", "docs", "users", "users/admin"]);
    assert!(shown(cx, h.win, row("users/admin/create.yml")));
    assert!(shown(cx, h.win, row("docs/intro.yml")));
    run_collapse_all(cx, &h);
    assert_eq!(kept(cx, &h), [""]);
    assert!(!shown(cx, h.win, row("users/list.yml")));
    assert!(
        shown(cx, h.win, row("health.yml")),
        "the collection stays open"
    );
}

#[gpui_kit::test]
fn the_actions_write_the_set_a_manual_toggle_writes(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, nested());
    // A manual chevron toggle and the bulk action leave the same kept set behind.
    cx.update_window(h.win, |_, w, cx| w.click(row("docs"), cx))
        .unwrap();
    frame(cx, h.win);
    assert_eq!(kept(cx, &h), ["", "docs"]);
    run_collapse_all(cx, &h);
    assert_eq!(kept(cx, &h), [""]);
    // A rebuild from elsewhere (the filter clearing) keeps what the action chose.
    h.ws.update(cx, |w, cx| w.open(h.root.clone(), cx));
    cx.run_until_parked();
    frame(cx, h.win);
    assert_eq!(kept(cx, &h), [""]);
    assert!(!shown(cx, h.win, row("docs/intro.yml")));
}

#[gpui_kit::test]
fn the_header_buttons_act_on_every_collection(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, nested());
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| w.click("sidebar-expand-all", cx))
        .unwrap();
    frame(cx, h.win);
    assert_eq!(kept(cx, &h), ["", "docs", "users", "users/admin"]);
    cx.update_window(h.win, |_, w, cx| w.click("sidebar-collapse-all", cx))
        .unwrap();
    frame(cx, h.win);
    assert_eq!(kept(cx, &h), [""]);
}

#[gpui_kit::test]
fn the_collection_menu_acts_on_that_collection(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, nested());
    for (entry, expected) in [
        (
            MenuEntry::ExpandAll,
            vec!["", "docs", "users", "users/admin"],
        ),
        (MenuEntry::CollapseAll, vec![""]),
    ] {
        cx.update_window(h.win, |_, w, cx| w.right_click(row(""), cx))
            .unwrap();
        frame(cx, h.win);
        cx.update_window(h.win, |_, w, cx| {
            let ix = menu_index(w, entry.label());
            w.within("popup-menu").click(ix, cx)
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(kept(cx, &h), expected, "{entry:?}");
    }
    assert!(!menu_entries(NodeKind::Folder).contains(&MenuEntry::CollapseAll));
}

#[test]
fn the_commands_are_registered_with_matching_menu_labels() {
    let named = |id: &str| registry().into_iter().find(|c| c.id == id).expect(id);
    assert_eq!(named("collection.collapse_all").label, "Collapse all");
    assert_eq!(named("collection.expand_all").label, "Expand all");
    assert_eq!(MenuEntry::CollapseAll.label(), "Collapse all");
    assert_eq!(MenuEntry::ExpandAll.label(), "Expand all");
}

fn kept_of(cx: &mut TestAppContext, h: &SidebarHarness, root: &std::path::Path) -> Vec<String> {
    let mut paths: Vec<String> = h
        .view
        .read_with(cx, |s, _| s.expanded_paths(root))
        .iter()
        .map(|p| p.as_str().to_owned())
        .collect();
    paths.sort();
    paths
}

#[gpui_kit::test]
fn header_actions_reach_every_collection_and_the_row_menu_only_its_own(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, nested());
    let other = write_tree(&[
        ("opencollection.yml", ROOT),
        ("api/folder.yml", &folder("api")),
        ("api/get.yml", &request("Get", "http://x/api")),
    ]);
    let second = other.path().to_path_buf();
    h.ws.update(cx, |w, cx| w.open(second.clone(), cx));
    cx.run_until_parked();
    frame(cx, h.win);

    run_expand_all(cx, &h);
    assert_eq!(kept(cx, &h), ["", "docs", "users", "users/admin"]);
    assert_eq!(kept_of(cx, &h, &second), ["", "api"]);
    run_collapse_all(cx, &h);
    assert_eq!(kept(cx, &h), [""]);
    assert_eq!(kept_of(cx, &h, &second), [""]);

    // The first collection's row menu opens only that collection's folders.
    cx.update_window(h.win, |_, w, cx| w.right_click(row(""), cx))
        .unwrap();
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| {
        let ix = menu_index(w, MenuEntry::ExpandAll.label());
        w.within("popup-menu").click(ix, cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(kept(cx, &h), ["", "docs", "users", "users/admin"]);
    assert_eq!(kept_of(cx, &h, &second), [""], "the other one is untouched");
}

#[gpui_kit::test]
fn collapse_all_over_a_selected_row_clears_the_selection(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, nested());
    run_expand_all(cx, &h);
    let tree = h.view.read_with(cx, |s, _| s.tree().clone());
    tree.update(cx, |t, cx| {
        let ix = t.index_of(&row("users/admin/create.yml"));
        assert!(ix.is_some());
        t.set_selected_index(ix, cx)
    });
    frame(cx, h.win);
    run_collapse_all(cx, &h);
    assert!(!shown(cx, h.win, row("users/admin/create.yml")));
    // Actual behavior: the hidden row is deselected, not left pointing at a stale index.
    assert_eq!(h.view.read_with(cx, |s, cx| s.selected(cx)), None);
    run_expand_all(cx, &h);
    // It is not remembered either: reopening the folders does not bring the selection back.
    assert_eq!(h.view.read_with(cx, |s, cx| s.selected(cx)), None);
}

#[gpui_kit::test]
fn the_actions_while_filtering_still_write_the_whole_set(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, nested());
    let filter = h.view.read_with(cx, |s, _| s.filter().clone());
    cx.update_window(h.win, |_, w, cx| filter.update(cx, |i, cx| i.focus(w, cx)))
        .unwrap();
    cx.update_window(h.win, |_, w, cx| w.input("list", cx))
        .unwrap();
    frame(cx, h.win);
    run_expand_all(cx, &h);
    assert_eq!(kept(cx, &h), ["", "docs", "users", "users/admin"]);
    run_collapse_all(cx, &h);
    assert_eq!(kept(cx, &h), [""]);
    run_expand_all(cx, &h);
    assert_eq!(kept(cx, &h), ["", "docs", "users", "users/admin"]);
}
