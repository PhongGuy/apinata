use crate::support;

use apinata_app::commands::registry;
use apinata_app::palette::palette_items;
use apinata_app::sidebar::menu::{BulkEntry, MenuEntry, bulk_entries, menu_entries};
use apinata_app::sidebar::model::NodeKind;
use apinata_app::sidebar::paste::ClipMode;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

fn press(cx: &mut TestAppContext, h: &SidebarHarness, key: &str) {
    cx.update_window(h.win, |_, w, cx| w.press(key, cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, h.win);
}

fn clip(cx: &mut TestAppContext, h: &SidebarHarness) -> (usize, Option<ClipMode>) {
    h.view
        .read_with(cx, |s, _| (s.clipboard_len(), s.clipboard_mode()))
}

fn files() -> Vec<(&'static str, String)> {
    vec![
        ("opencollection.yml", ROOT.to_string()),
        ("a.yml", request("A", "http://x/a")),
        ("t/folder.yml", folder("T")),
    ]
}

#[test]
fn single_row_menus_offer_copy_cut_and_paste_where_they_apply() {
    use MenuEntry as E;
    for kind in [NodeKind::Request, NodeKind::Folder] {
        let e = menu_entries(kind);
        assert!(e.contains(&E::CopyItem) && e.contains(&E::CutItem) && e.contains(&E::PasteItem));
    }
    let col = menu_entries(NodeKind::Collection);
    assert!(col.contains(&E::PasteItem));
    assert!(!col.contains(&E::CopyItem) && !col.contains(&E::CutItem));
    let opaque = menu_entries(NodeKind::Opaque);
    assert!(opaque.contains(&E::CopyItem) && opaque.contains(&E::CutItem));
    assert_eq!(
        (
            E::CopyItem.label(),
            E::CutItem.label(),
            E::PasteItem.label()
        ),
        ("Copy", "Cut", "Paste")
    );
}

#[test]
fn bulk_menu_copies_and_cuts_before_delete() {
    let three = apinata_app::sidebar::multi::Tally {
        requests: 3,
        ..Default::default()
    };
    assert_eq!(
        bulk_entries(three, None),
        [
            BulkEntry::Copy(3),
            BulkEntry::Cut(3),
            BulkEntry::Delete(three)
        ]
    );
    assert_eq!(BulkEntry::Copy(3).label(), "Copy 3 items");
    assert_eq!(BulkEntry::Cut(1).label(), "Cut 1 item");
}

#[gpui_kit::test]
fn copy_then_paste_from_the_context_menu_writes_the_copy(cx: &mut TestAppContext) {
    let col = write_tree(
        &files()
            .iter()
            .map(|(p, t)| (*p, t.as_str()))
            .collect::<Vec<_>>(),
    );
    let s = started(cx, &col);
    choose_menu(cx, s.win, "a.yml", NodeKind::Request, MenuEntry::CopyItem);
    choose_menu(cx, s.win, "t", NodeKind::Folder, MenuEntry::PasteItem);
    frame(cx, s.win);
    assert!(col.path().join("t/a.yml").exists());
    assert!(col.path().join("a.yml").exists());
}

#[gpui_kit::test]
fn cut_then_paste_from_the_context_menu_moves(cx: &mut TestAppContext) {
    let col = write_tree(
        &files()
            .iter()
            .map(|(p, t)| (*p, t.as_str()))
            .collect::<Vec<_>>(),
    );
    let s = started(cx, &col);
    choose_menu(cx, s.win, "a.yml", NodeKind::Request, MenuEntry::CutItem);
    choose_menu(cx, s.win, "t", NodeKind::Folder, MenuEntry::PasteItem);
    frame(cx, s.win);
    assert!(col.path().join("t/a.yml").exists());
    assert!(!col.path().join("a.yml").exists());
}

#[gpui_kit::test]
fn chords_with_the_tree_focused_copy_cut_and_paste(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, files());
    cx.update_window(h.win, |_, w, cx| w.click(row("a.yml"), cx))
        .unwrap();
    focus_tree(cx, &h);
    frame(cx, h.win);
    press(cx, &h, &secondary("c"));
    assert_eq!(clip(cx, &h), (1, Some(ClipMode::Copy)));
    press(cx, &h, &secondary("x"));
    assert_eq!(clip(cx, &h), (1, Some(ClipMode::Cut)));
    press(cx, &h, &secondary("c"));
    cx.update_window(h.win, |_, w, cx| w.click(row("t"), cx))
        .unwrap();
    focus_tree(cx, &h);
    press(cx, &h, &secondary("v"));
    assert!(h.root.join("t/a.yml").exists());
}

#[gpui_kit::test]
fn chords_in_the_filter_box_leave_the_clipboard_alone(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, files());
    cx.update_window(h.win, |_, w, cx| w.click(row("a.yml"), cx))
        .unwrap();
    focus_tree(cx, &h);
    press(cx, &h, &secondary("c"));
    assert_eq!(clip(cx, &h).0, 1);
    press(cx, &h, &secondary("f"));
    let filter = h.view.read_with(cx, |s, _| s.filter().clone());
    let focused = cx
        .update_window(h.win, |_, w, cx| {
            use gpui_kit::Focusable as _;
            filter.read(cx).focus_handle(cx).is_focused(w)
        })
        .unwrap();
    assert!(focused, "the filter box has focus");
    press(cx, &h, &secondary("x"));
    assert_eq!(clip(cx, &h), (1, Some(ClipMode::Copy)), "cut ignored");
    press(cx, &h, &secondary("v"));
    assert!(!h.root.join("a (copy).yml").exists());
    assert!(!h.root.join("t/a.yml").exists());
}

#[gpui_kit::test]
fn chords_in_the_rename_input_leave_the_clipboard_alone(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, files());
    cx.update_window(h.win, |_, w, cx| w.click(row("a.yml"), cx))
        .unwrap();
    focus_tree(cx, &h);
    press(cx, &h, "f2");
    press(cx, &h, &secondary("c"));
    press(cx, &h, &secondary("x"));
    assert_eq!(clip(cx, &h), (0, None));
}

#[test]
fn the_palette_lists_the_three_commands_with_their_shortcuts() {
    let cmds = registry();
    for (id, label, chord) in [
        ("request.copy", "Copy selected items", "secondary-c"),
        ("request.cut", "Cut selected items", "secondary-x"),
        ("request.paste", "Paste into selected folder", "secondary-v"),
    ] {
        let c = cmds.iter().find(|c| c.id == id).expect(id);
        assert_eq!(
            (c.label, c.default_chords.as_slice()),
            (label, [chord].as_slice())
        );
        assert!(c.in_palette);
        assert!(palette_items().iter().any(|p| p.label == label), "{label}");
    }
}

#[gpui_kit::test]
fn paste_in_the_url_bar_is_text_and_never_a_sidebar_paste(cx: &mut TestAppContext) {
    let col = write_tree(
        &files()
            .iter()
            .map(|(p, t)| (*p, t.as_str()))
            .collect::<Vec<_>>(),
    );
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| w.click(row("a.yml"), cx))
        .unwrap();
    frame(cx, s.win);
    let sb = s.shell.read_with(cx, |sh, _| sh.sidebar().clone());
    cx.update_window(s.win, |_, w, cx| {
        sb.update(cx, |sb, cx| sb.copy_selection(w, cx))
    })
    .unwrap();
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.click("url", cx);
        w.press("end", cx);
        w.press(&secondary("v"), cx);
        w.press(&secondary("x"), cx);
    })
    .unwrap();
    frame(cx, s.win);
    let url = view_of(cx, &s.ws, &s.root, "a.yml").url.clone();
    assert!(
        url.contains("a.yml"),
        "the clipboard text landed in the URL: {url}"
    );
    assert!(!col.path().join("a (copy).yml").exists());
    assert_eq!(
        sb.read_with(cx, |s, _| s.clipboard_mode()),
        Some(ClipMode::Copy)
    );
}
