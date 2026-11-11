//! The current collection: chosen in the switcher, by a sidebar row or by a tab, and opened.

use crate::support;

use apinata_app::sidebar::marker_id;
use apinata_app::sidebar::model::node_id;
use apinata_core::ItemPath;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use std::path::PathBuf;
use support::*;

const SECOND: &str = "opencollection: 1.0.0\ninfo:\n  name: Second\n";

// Opens a second collection named "Second"; returns it with its index and root as kept.
fn second(cx: &mut TestAppContext, s: &Started) -> (tempfile::TempDir, usize, PathBuf) {
    let b = write_tree(&[
        ("opencollection.yml", SECOND),
        ("b.yml", &request("b", "http://x/1")),
    ]);
    s.ws.update(cx, |w, cx| w.open(b.path().to_path_buf(), cx));
    cx.run_until_parked();
    let (ix, root) = s.ws.read_with(cx, |w, _| {
        let ix = (w.collections().iter())
            .position(|c| c.root() == b.path())
            .expect("opened");
        (ix, w.collections()[ix].root().to_path_buf())
    });
    (b, ix, root)
}

fn open_switcher(cx: &mut TestAppContext, s: &Started) {
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("collection-switcher", cx)
    })
    .unwrap();
    frame(cx, s.win);
}

fn choose(cx: &mut TestAppContext, s: &Started, ix: usize) {
    open_switcher(cx, s);
    cx.update_window(s.win, |_, w, cx| w.click(("switch-open", ix), cx))
        .unwrap();
    frame(cx, s.win);
}

fn current(cx: &mut TestAppContext, s: &Started) -> Option<PathBuf> {
    s.ws.read_with(cx, |w, _| w.current_root())
}

#[gpui_kit::test]
fn choosing_a_collection_in_the_switcher_makes_it_current(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let (_b, ix, root) = second(cx, &s);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    assert_eq!(current(cx, &s), Some(s.root.clone()));
    choose(cx, &s, ix);
    assert_eq!(current(cx, &s), Some(root));
    let name = s.shell.read_with(cx, |v, cx| v.current_name(cx));
    assert_eq!(name.as_deref(), Some("Second"));
}

#[gpui_kit::test]
fn activating_a_tab_of_another_collection_makes_it_current(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let (_b, ix, root) = second(cx, &s);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    choose(cx, &s, ix);
    assert_eq!(current(cx, &s), Some(root));
    s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate(tab)));
    frame(cx, s.win);
    assert_eq!(current(cx, &s), Some(s.root.clone()));
}

#[gpui_kit::test]
fn selecting_a_root_row_makes_its_collection_current(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let (_b, ix, root) = second(cx, &s);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.click(node_id(ix, &ItemPath::root()), cx)
    })
    .unwrap();
    frame(cx, s.win);
    assert_eq!(current(cx, &s), Some(root));
}

#[gpui_kit::test]
fn the_switcher_checks_the_current_collection_only(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let (_b, ix, _root) = second(cx, &s);
    let first = 1 - ix;
    open_tab(cx, &s.ws, &s.root, "a.yml");
    open_switcher(cx, &s);
    assert!(present(cx, s.win, ("switch-current", first)));
    assert!(!present(cx, s.win, ("switch-current", ix)));
    cx.update_window(s.win, |_, w, cx| w.click(("switch-open", ix), cx))
        .unwrap();
    frame(cx, s.win);
    open_switcher(cx, &s);
    assert!(present(cx, s.win, ("switch-current", ix)));
    assert!(!present(cx, s.win, ("switch-current", first)));
}

#[gpui_kit::test]
fn switcher_rows_span_the_list_like_the_actions(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let (_b, ix, _root) = second(cx, &s);
    open_switcher(cx, &s);
    cx.update_window(s.win, |_, w, _| {
        let action = w.find("switcher-open-collection").bounds();
        let row = w.find(("switch-open", ix)).bounds();
        let gear = w.find(("switcher-settings", ix)).bounds();
        assert_eq!(row.left(), action.left());
        assert_eq!(row.right(), gear.left(), "the row fills up to its gear");
    })
    .unwrap();
}

#[gpui_kit::test]
fn opening_a_collection_makes_it_current_and_reveals_it(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    let b = write_tree(&[("opencollection.yml", SECOND)]);
    let path = b.path().to_path_buf();
    switcher_click(cx, s.win, "switcher-open-collection");
    cx.simulate_path_prompt_response(|_| Some(vec![path]));
    frame(cx, s.win);
    let root = s.ws.read_with(cx, |w, _| {
        (w.collections().iter())
            .map(|c| c.root().to_path_buf())
            .find(|r| r.ends_with(b.path().file_name().unwrap()))
            .expect("opened")
    });
    assert_eq!(current(cx, &s), Some(root.clone()));
    let selected = s
        .shell
        .read_with(cx, |v, cx| v.sidebar().read(cx).selected(cx));
    assert_eq!(selected, Some((root, ItemPath::root())));
}

#[gpui_kit::test]
fn a_collection_root_row_has_a_chevron(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    assert!(shown(
        cx,
        h.win,
        marker_id("chevron", &node_id(0, &ItemPath::root()))
    ));
}
