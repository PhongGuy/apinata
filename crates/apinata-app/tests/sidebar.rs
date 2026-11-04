use crate::support;

use apinata_app::session::Session;
use apinata_app::shell::Shell;
use apinata_app::sidebar::model::node_id;
use apinata_core::ItemPath;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext, WindowOptions};
use support::*;

#[gpui_kit::test]
fn the_tree_shows_the_collection_with_folders_collapsed(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    for path in ["", "users", "health.yml", "status.yml", "ws.yml"] {
        assert!(shown(cx, h.win, row(path)), "{path}");
    }
    assert!(!shown(cx, h.win, row("users/list.yml")));
    assert!(shown(cx, h.win, ("collection", 0usize)));
}

#[gpui_kit::test]
fn an_opening_collection_shows_its_row_first(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let other = collection();
    h.ws.update(cx, |w, cx| w.open(other.path().to_path_buf(), cx));
    // Rendered without parking: the actor has not answered yet, but the row is there.
    let (opening, item) = (node_id(1, &ItemPath::root()), node_id(1, &ip("a.yml")));
    let now = cx
        .update_window(h.win, |_, w, cx| {
            w.render_frame(cx);
            (
                w.try_find(opening.clone()).is_some(),
                w.try_find(item.clone()).is_some(),
            )
        })
        .unwrap();
    assert_eq!(now, (true, false));
    assert!(shown(cx, h.win, item));
}

#[gpui_kit::test]
fn expansion_survives_later_snapshots(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    cx.update_window(h.win, |_, w, cx| w.click(row("users"), cx))
        .unwrap();
    assert!(shown(cx, h.win, row("users/list.yml")));
    h.ws.update(cx, |w, cx| {
        w.send(&h.root, edit_url("health.yml", "http://x/9"), cx)
    });
    assert!(shown(cx, h.win, row("users/list.yml")));
}

#[gpui_kit::test]
fn the_shell_hosts_the_sidebar(cx: &mut TestAppContext) {
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let col = collection();
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, Session::default());
    let ws = workspace(cx, &session);
    ws.update(cx, |w, cx| w.open(col.path().to_path_buf(), cx));
    let (win, shell) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Shell::new(ws, window, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    let inside = cx
        .update_window(win, |_, w, _| {
            w.within("sidebar").try_find(row("a.yml")).is_some()
        })
        .unwrap();
    assert!(inside);
    let tree = shell.read_with(cx, |s, cx| s.sidebar().read(cx).tree().clone());
    assert!(
        tree.read_with(cx, |t, _| t.entry(1).is_some()),
        "the collection row and a.yml are entries"
    );
}
