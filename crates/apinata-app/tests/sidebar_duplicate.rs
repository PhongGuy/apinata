//! Duplicate from the row menu: the unsaved-edits notice, nested folders and opaque items.

use crate::support;

use apinata_app::command::Command;
use apinata_app::sidebar::menu::{MenuEntry, menu_entries};
use apinata_app::sidebar::model::NodeKind;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext};
use support::*;

/// Right-click the row at `path` in `win` and pick Duplicate from its menu.
fn duplicate(cx: &mut TestAppContext, win: AnyWindowHandle, path: &str, kind: NodeKind) {
    assert!(
        menu_entries(kind).contains(&MenuEntry::CloneItem),
        "Duplicate offered"
    );
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.right_click(row(path), cx))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        let ix = menu_index(w, MenuEntry::CloneItem.label());
        w.within("popup-menu").click(ix, cx)
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, win);
}

fn toasts(cx: &mut TestAppContext, s: &Started) -> usize {
    cx.update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

fn edit(cx: &mut TestAppContext, s: &Started, rel: &str, url: &str) {
    let root = s.root.clone();
    s.ws.update(cx, |w, cx| w.send(&root, edit_url(rel, url), cx));
    cx.run_until_parked();
    frame(cx, s.win);
}

#[gpui_kit::test]
fn duplicating_a_request_with_unsaved_edits_says_the_copy_includes_them(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("A", "http://x/a")),
        ("b.yml", &request("B", "http://x/b")),
    ]);
    let s = started(cx, &col);
    let before = toasts(cx, &s);
    duplicate(cx, s.win, "a.yml", NodeKind::Request);
    assert!(col.path().join("A copy.yml").exists());
    assert_eq!(toasts(cx, &s), before, "a clean source pushes no notice");

    edit(cx, &s, "b.yml", "http://x/changed");
    duplicate(cx, s.win, "b.yml", NodeKind::Request);
    assert_eq!(
        toasts(cx, &s),
        before + 1,
        "one notice for the dirty source"
    );
    let text = std::fs::read_to_string(col.path().join("B copy.yml")).unwrap();
    assert!(text.contains("http://x/changed"), "{text}");
}

#[gpui_kit::test]
fn duplicating_a_folder_with_an_unsaved_request_inside_says_so(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("Users")),
        ("users/list.yml", &request("List", "http://x/users")),
        ("admin/folder.yml", &folder("Admin")),
        ("admin/get.yml", &request("Get", "http://x/admin")),
    ]);
    let s = started(cx, &col);
    let before = toasts(cx, &s);
    duplicate(cx, s.win, "admin", NodeKind::Folder);
    assert!(col.path().join("Admin copy/get.yml").exists());
    assert_eq!(toasts(cx, &s), before, "a clean folder pushes no notice");

    edit(cx, &s, "users/list.yml", "http://x/changed");
    duplicate(cx, s.win, "users", NodeKind::Folder);
    assert_eq!(toasts(cx, &s), before + 1, "one notice for the dirty child");
    let text = std::fs::read_to_string(col.path().join("Users copy/list.yml")).unwrap();
    assert!(text.contains("http://x/changed"), "{text}");
}

#[gpui_kit::test]
fn duplicating_a_folder_copies_its_nested_folders_and_their_requests(cx: &mut TestAppContext) {
    let h = open_sidebar_with(
        cx,
        vec![
            ("opencollection.yml", ROOT.to_string()),
            ("users/folder.yml", folder("Users")),
            ("users/sub/folder.yml", folder("Sub")),
            ("users/sub/inner.yml", request("Inner", "http://x/inner")),
            ("users/bare/deep.yml", request("Deep", "http://x/deep")),
        ],
    );
    duplicate(cx, h.win, "users", NodeKind::Folder);
    for rel in ["sub/folder.yml", "sub/inner.yml", "bare/deep.yml"] {
        assert!(h.root.join("Users copy").join(rel).exists(), "{rel}");
    }
    assert!(!h.root.join("Users copy/bare/folder.yml").exists());
    let snapshot = h.ws.read_with(cx, |w, cx| w.snapshot(&h.root, cx)).unwrap();
    let paths: Vec<&str> = snapshot
        .content
        .rows
        .iter()
        .map(|r| r.path.as_str())
        .collect();
    for path in [
        "Users copy/sub",
        "Users copy/sub/inner.yml",
        "Users copy/bare",
        "Users copy/bare/deep.yml",
    ] {
        assert!(paths.contains(&path), "{path} in {paths:?}");
    }
}

#[gpui_kit::test]
fn duplicate_on_an_opaque_row_copies_the_item(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    duplicate(cx, h.win, "ws.yml", NodeKind::Opaque);
    let text = std::fs::read_to_string(h.root.join("Socket copy.yml")).unwrap();
    assert!(text.contains("name: Socket copy\n"), "{text}");
    assert!(text.contains("type: websocket\n"), "{text}");
    assert_eq!(
        h.view.read_with(cx, |s, cx| s.selected(cx)),
        Some((h.root.clone(), ip("Socket copy.yml")))
    );
}

#[gpui_kit::test]
fn a_failed_duplicate_of_a_dirty_request_shows_no_unsaved_edits_notice(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("A", "http://x/a")),
        ("b.yml", &request("B", "http://x/b")),
    ]);
    let s = started(cx, &col);
    edit(cx, &s, "b.yml", "http://x/changed");
    // A read-only collection folder makes every copy fail to write.
    let _lock = ModeGuard::set(col.path(), 0o555);
    let before = toasts(cx, &s);
    duplicate(cx, s.win, "a.yml", NodeKind::Request);
    let clean = toasts(cx, &s) - before;
    assert!(clean >= 1, "the failure is reported");
    duplicate(cx, s.win, "b.yml", NodeKind::Request);
    assert!(!col.path().join("B copy.yml").exists());
    assert_eq!(
        toasts(cx, &s) - before - clean,
        clean,
        "a dirty source fails with the same toasts as a clean one, no unsaved notice"
    );
}

/// The top of the sidebar row at `path`.
fn row_top(cx: &mut TestAppContext, win: AnyWindowHandle, path: &str) -> gpui_kit::Pixels {
    frame(cx, win);
    cx.update_window(win, |_, w, _| w.find(row(path)).bounds().origin.y)
        .unwrap()
}

#[gpui_kit::test]
fn an_opaque_row_sits_among_the_requests_by_its_seq(cx: &mut TestAppContext) {
    let h = open_sidebar_with(
        cx,
        vec![
            ("opencollection.yml", ROOT.to_string()),
            ("a.yml", request("A", "http://x/a")),
            (
                "b.yml",
                "info:\n  name: Sock\n  type: websocket\n  seq: 2\nwebsocket:\n  url: ws://x\n"
                    .to_string(),
            ),
            (
                "c.yml",
                "info:\n  name: C\n  type: http\n  seq: 3\nhttp:\n  method: GET\n  url: http://x/c\n"
                    .to_string(),
            ),
        ],
    );
    // File names sort the opposite way to seq, so only a seq order passes.
    std::fs::rename(h.root.join("a.yml"), h.root.join("z.yml")).unwrap();
    std::fs::rename(h.root.join("b.yml"), h.root.join("y.yml")).unwrap();
    let root = h.root.clone();
    h.ws.update(cx, |w, cx| w.send(&root, Command::ReloadAll, cx));
    cx.run_until_parked();
    let snapshot = h.ws.read_with(cx, |w, cx| w.snapshot(&h.root, cx)).unwrap();
    let paths: Vec<&str> = snapshot
        .content
        .rows
        .iter()
        .map(|r| r.path.as_str())
        .collect();
    assert_eq!(paths, ["z.yml", "y.yml", "c.yml"]);
    let tops: Vec<_> = paths.iter().map(|p| row_top(cx, h.win, p)).collect();
    assert!(tops[0] < tops[1] && tops[1] < tops[2], "{tops:?}");
}

#[gpui_kit::test]
fn a_folder_duplicated_twice_shows_two_names_after_a_reload(cx: &mut TestAppContext) {
    let h = open_sidebar_with(
        cx,
        vec![
            ("opencollection.yml", ROOT.to_string()),
            ("users/folder.yml", folder("Users")),
            ("users/list.yml", request("List", "http://x/users")),
        ],
    );
    duplicate(cx, h.win, "users", NodeKind::Folder);
    duplicate(cx, h.win, "users", NodeKind::Folder);
    let root = h.root.clone();
    h.ws.update(cx, |w, cx| w.send(&root, Command::ReloadAll, cx));
    cx.run_until_parked();
    let snapshot = h.ws.read_with(cx, |w, cx| w.snapshot(&h.root, cx)).unwrap();
    let name = |path: &str| {
        snapshot
            .content
            .rows
            .iter()
            .find(|r| r.path.as_str() == path)
            .map(|r| r.name.clone())
    };
    assert_eq!(
        (name("Users copy"), name("Users copy 1")),
        (Some("Users copy".into()), Some("Users copy 1".into()))
    );
}

/// The active tab's request path and whether it is the preview, with the sidebar selection.
fn editor_and_selection(
    cx: &mut TestAppContext,
    s: &Started,
) -> (Option<(String, bool)>, Option<String>) {
    let tab = s.ws.read_with(cx, |w, _| {
        w.tabs()
            .active()
            .and_then(|e| Some((e.item()?.as_str().to_owned(), e.preview)))
    });
    let sidebar = s.shell.read_with(cx, |v, _| v.sidebar().clone());
    let selected = sidebar.read_with(cx, |v, cx| v.selected(cx));
    (tab, selected.map(|(_, p)| p.as_str().to_owned()))
}

#[gpui_kit::test]
fn duplicating_a_request_opens_the_copy_as_a_preview(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("A", "http://x/a")),
        ("b.yml", &request("B", "http://x/b")),
    ]);
    let s = started(cx, &col);
    duplicate(cx, s.win, "a.yml", NodeKind::Request);
    assert_eq!(
        editor_and_selection(cx, &s),
        (
            Some(("A copy.yml".to_owned(), true)),
            Some("A copy.yml".to_owned())
        )
    );
}

#[gpui_kit::test]
fn duplicating_the_open_request_moves_the_editor_to_the_copy(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("A", "http://x/a")),
        ("b.yml", &request("B", "http://x/b")),
    ]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "b.yml");
    frame(cx, s.win);
    duplicate(cx, s.win, "b.yml", NodeKind::Request);
    assert_eq!(
        editor_and_selection(cx, &s),
        (
            Some(("B copy.yml".to_owned(), true)),
            Some("B copy.yml".to_owned())
        )
    );
}

#[gpui_kit::test]
fn duplicating_a_folder_selects_the_copy_and_opens_nothing(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("Users")),
        ("users/list.yml", &request("List", "http://x/users")),
    ]);
    let s = started(cx, &col);
    duplicate(cx, s.win, "users", NodeKind::Folder);
    assert_eq!(
        editor_and_selection(cx, &s),
        (None, Some("Users copy".to_owned()))
    );
}
