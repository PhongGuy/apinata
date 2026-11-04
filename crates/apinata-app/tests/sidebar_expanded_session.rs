//! The sidebar's expanded folders are part of the saved session: they are written through the
//! session store's debounce and come back when the collection opens, minus vanished folders.

use crate::support;

use apinata_app::session::{CollectionSession, Session};
use apinata_app::sidebar::{CollapseAll, ExpandAll, Sidebar};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

fn nested() -> Vec<(&'static str, String)> {
    let mut files = sidebar_files();
    files.push(("users/admin/folder.yml", folder("admin")));
    files.push((
        "users/admin/create.yml",
        request("Create admin", "http://x/admin"),
    ));
    files
}

// Opens `files` with a session that already holds `saved` for the collection.
fn open_with_saved(
    cx: &mut TestAppContext,
    files: Vec<(&'static str, String)>,
    saved: Option<&[&str]>,
) -> SidebarHarness {
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let refs: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
    let col = write_tree(&refs);
    let root = col.path().to_path_buf();
    let state = tempfile::tempdir().unwrap();
    let mut start = Session::default();
    start.collections.insert(
        root.clone(),
        CollectionSession {
            expanded: saved.map(|l| l.iter().map(|s| s.to_string()).collect()),
            ..Default::default()
        },
    );
    let session = session_at(cx, &state, start);
    let ws = workspace(cx, &session);
    ws.update(cx, |w, cx| w.open(root.clone(), cx));
    cx.run_until_parked();
    let shared = ws.clone();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Sidebar::new(shared, window, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    SidebarHarness {
        col,
        state,
        root,
        win,
        ws,
        view,
    }
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

fn stored(cx: &mut TestAppContext, h: &SidebarHarness) -> Vec<String> {
    let session = h.ws.read_with(cx, |w, _| w.session().clone());
    let mut v = session_value(cx, &session)
        .collections
        .get(&h.root)
        .and_then(|c| c.expanded.clone())
        .unwrap_or_default();
    v.sort();
    v
}

#[test]
fn an_older_session_file_without_the_field_loads_and_an_empty_set_is_not_written() {
    let old = r#"{"collections":{"/c":{"tabs":["a.yml"],"environment":null}}}"#;
    let s: Session = serde_json::from_str(old).unwrap();
    assert_eq!(s.collections[std::path::Path::new("/c")].expanded, None);
    let out = serde_json::to_string(&s).unwrap();
    assert!(!out.contains("expanded"), "{out}");
}

#[test]
fn the_expanded_folders_survive_a_save_and_reload() {
    let mut s = Session::default();
    s.collections.insert(
        "/c".into(),
        CollectionSession {
            expanded: Some(vec!["".into(), "users".into(), "users/admin".into()]),
            ..Default::default()
        },
    );
    let back: Session = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(back, s);
}

#[gpui_kit::test]
fn expanding_every_folder_lands_in_the_session(cx: &mut TestAppContext) {
    let h = open_with_saved(cx, nested(), None);
    cx.update_window(h.win, |_, w, cx| {
        h.view.update(cx, |s, cx| s.expand_all(&ExpandAll, w, cx))
    })
    .unwrap();
    frame(cx, h.win);
    assert_eq!(stored(cx, &h), ["", "users", "users/admin"]);
}

#[gpui_kit::test]
fn a_saved_set_is_restored_and_a_vanished_folder_is_dropped(cx: &mut TestAppContext) {
    let h = open_with_saved(
        cx,
        nested(),
        Some(&["", "users", "users/admin", "renamed/away"]),
    );
    assert_eq!(kept(cx, &h), ["", "users", "users/admin"]);
    assert!(shown(cx, h.win, row("users/admin/create.yml")));
    assert_eq!(stored(cx, &h), ["", "users", "users/admin"]);
}

#[gpui_kit::test]
fn without_a_saved_set_the_collection_row_alone_is_open(cx: &mut TestAppContext) {
    let h = open_with_saved(cx, nested(), None);
    assert_eq!(kept(cx, &h), [""]);
    assert!(!shown(cx, h.win, row("users/list.yml")));
}

#[gpui_kit::test]
fn a_saved_empty_set_reopens_with_the_collection_row_collapsed(cx: &mut TestAppContext) {
    let h = open_with_saved(cx, nested(), Some(&[]));
    assert_eq!(kept(cx, &h), Vec::<String>::new());
    assert!(!shown(cx, h.win, row("health.yml")));
}

#[gpui_kit::test]
fn collapsing_the_collection_row_by_hand_is_saved(cx: &mut TestAppContext) {
    let h = open_with_saved(cx, nested(), None);
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| w.click(("collection", 0usize), cx))
        .unwrap();
    frame(cx, h.win);
    assert_eq!(kept(cx, &h), Vec::<String>::new());
    assert_eq!(
        h.ws.read_with(cx, |w, _| w.session().clone())
            .read_with(cx, |p, _| p.get().collections[&h.root].expanded.clone()),
        Some(vec![])
    );
}

#[gpui_kit::test]
fn collapse_all_is_saved_with_the_collection_row_open(cx: &mut TestAppContext) {
    let h = open_with_saved(cx, nested(), Some(&["", "users"]));
    cx.update_window(h.win, |_, w, cx| {
        h.view
            .update(cx, |s, cx| s.collapse_all(&CollapseAll, w, cx))
    })
    .unwrap();
    frame(cx, h.win);
    assert_eq!(stored(cx, &h), [""]);
}
