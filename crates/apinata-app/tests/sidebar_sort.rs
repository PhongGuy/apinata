//! Sort by name from the folder and collection menus and the command palette.

use crate::support;

use apinata_app::commands::registry;
use apinata_app::sidebar::SortByName;
use apinata_app::sidebar::menu::{MenuEntry, menu_entries};
use apinata_app::sidebar::model::NodeKind;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext};
use support::*;

fn seq_on_disk(col: &tempfile::TempDir, rel: &str) -> u64 {
    let text = std::fs::read_to_string(col.path().join(rel)).unwrap();
    text.lines()
        .find_map(|l| l.trim().strip_prefix("seq: "))
        .and_then(|s| s.parse().ok())
        .unwrap()
}

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("t/folder.yml", &folder("t")),
        ("t/z.yml", &request("zed", "http://x/z")),
        ("t/a.yml", &request("alpha", "http://x/a")),
        ("t/m.yml", &request("mid", "http://x/m")),
        ("top.yml", &request("top", "http://x/top")),
    ])
}

#[test]
fn folder_and_collection_menus_offer_sort_by_name() {
    for kind in [NodeKind::Folder, NodeKind::Collection] {
        assert!(
            menu_entries(kind).contains(&MenuEntry::SortByName),
            "{kind:?}"
        );
    }
    assert!(!menu_entries(NodeKind::Request).contains(&MenuEntry::SortByName));
    let label = registry()
        .into_iter()
        .find(|c| c.id == "collection.sort_by_name")
        .expect("palette command")
        .label;
    assert_eq!(MenuEntry::SortByName.label(), label);
}

#[gpui_kit::test]
fn the_folder_menu_renumbers_its_items_by_name(cx: &mut TestAppContext) {
    let col = tree();
    let s = started(cx, &col);
    choose_menu(cx, s.win, "t", NodeKind::Folder, MenuEntry::SortByName);
    frame(cx, s.win);
    assert_eq!(seq_on_disk(&col, "t/a.yml"), 1);
    assert_eq!(seq_on_disk(&col, "t/m.yml"), 2);
    assert_eq!(seq_on_disk(&col, "t/z.yml"), 3);
    assert_eq!(seq_on_disk(&col, "top.yml"), 1);
}

#[gpui_kit::test]
fn the_collection_menu_sorts_the_root_only(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("b.yml", &request("b", "http://x/b")),
        ("a.yml", &request("a", "http://x/a")),
        ("g/folder.yml", &folder("g")),
        ("g/y.yml", &request("y", "http://x/y")),
    ]);
    let s = started(cx, &col);
    choose_menu(cx, s.win, "", NodeKind::Collection, MenuEntry::SortByName);
    frame(cx, s.win);
    assert_eq!(seq_on_disk(&col, "a.yml"), 1);
    assert_eq!(seq_on_disk(&col, "b.yml"), 2);
    assert_eq!(seq_on_disk(&col, "g/folder.yml"), 3);
    assert_eq!(seq_on_disk(&col, "g/y.yml"), 1);
}

#[gpui_kit::test]
fn the_action_sorts_the_selected_folder(cx: &mut TestAppContext) {
    let col = tree();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| w.click(row("t"), cx))
        .unwrap();
    frame(cx, s.win);
    VisualTestContext::from_window(s.win, cx).dispatch_action(SortByName);
    cx.run_until_parked();
    frame(cx, s.win);
    assert_eq!(seq_on_disk(&col, "t/a.yml"), 1);
    assert_eq!(seq_on_disk(&col, "t/z.yml"), 3);
}
