//! A draft row under Unsaved names the folder its Save writes into, as a muted suffix that gives
//! way before the draft's name does; a draft at the collection root shows none.

use crate::support;

use apinata_app::sidebar::marker_id;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext};
use support::*;

const LONG: &str = "A folder whose display name alone is far wider than the sidebar";

fn collection() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
        ("users/folder.yml", &folder("Users")),
        ("users/todos/folder.yml", &folder("Todos")),
        ("long/folder.yml", &folder(LONG)),
    ])
}

// Opens a draft under `parent` and returns its item path, read off the active tab.
fn draft_in(cx: &mut TestAppContext, s: &Started, parent: &str) -> String {
    let root = s.root.clone();
    s.ws.update(cx, |w, cx| w.new_draft(&root, ip(parent), cx));
    frame(cx, s.win);
    s.ws.read_with(cx, |w, _| {
        w.tabs()
            .active()
            .and_then(|e| e.item())
            .map(|p| p.as_str().to_owned())
    })
    .expect("a draft tab")
}

fn label(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    id: gpui_kit::SharedString,
) -> Option<String> {
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

// The tooltip text shown after resting the pointer on the draft row at `path`, if any.
fn tooltip(cx: &mut TestAppContext, win: AnyWindowHandle, path: &str) -> Option<String> {
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.hover(row(path), cx))
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(800));
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        w.try_find(marker_id("draft-folder-tip", &row(path)))
            .map(|t| t.label().unwrap_or_default().to_owned())
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_root_draft_shows_no_folder(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    let draft = draft_in(cx, &s, "");
    assert!(shown(cx, s.win, row(&draft)));
    assert!(!shown(cx, s.win, marker_id("draft-folder", &row(&draft))));
    assert_eq!(tooltip(cx, s.win, &draft), None);
}

#[gpui_kit::test]
fn a_nested_draft_shows_its_folder_after_its_name(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    let draft = draft_in(cx, &s, "users/todos");
    let folder = marker_id("draft-folder", &row(&draft));
    assert!(shown(cx, s.win, folder.clone()));
    cx.update_window(s.win, |_, w, _| {
        let name = w.find(marker_id("name", &row(&draft))).bounds();
        let hint = w.find(folder.clone()).bounds();
        assert!(hint.left() >= name.right(), "{hint:?} after {name:?}");
    })
    .unwrap();
    assert_eq!(
        tooltip(cx, s.win, &draft).as_deref(),
        Some("Saves into Users/Todos")
    );
}

#[gpui_kit::test]
fn a_draft_rows_accessible_name_includes_its_folder(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    let root = draft_in(cx, &s, "");
    let nested = draft_in(cx, &s, "users/todos");
    let shown_name = s.ws.read_with(cx, |w, cx| {
        w.request(&s.root, &ip(&nested), cx).map(|v| v.name.clone())
    });
    let shown_name = shown_name.expect("the draft's view");
    assert_eq!(
        label(cx, s.win, row(&nested)),
        Some(format!("{shown_name}, saves into Users/Todos"))
    );
    assert_eq!(
        label(cx, s.win, row(&root)),
        None,
        "a root draft keeps its plain name"
    );
}

// The panel is narrow, so a long folder is cut and the draft's name keeps the width it has on
// a root draft's row.
#[gpui_kit::test]
fn a_long_folder_is_cut_before_the_drafts_name(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    let root = draft_in(cx, &s, "");
    let long = draft_in(cx, &s, "long");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        let sidebar = w.find("sidebar").bounds();
        let plain = w.find(marker_id("name", &row(&root))).bounds();
        let name = w.find(marker_id("name", &row(&long))).bounds();
        let hint = w.find(marker_id("draft-folder", &row(&long))).bounds();
        assert!(
            name.size.width + gpui_kit::px(1.) >= plain.size.width,
            "name {name:?} squeezed below {plain:?}"
        );
        assert!(hint.left() >= name.right(), "{hint:?} over {name:?}");
        assert!(hint.right() <= sidebar.right(), "{hint:?} past {sidebar:?}");
    })
    .unwrap();
}
