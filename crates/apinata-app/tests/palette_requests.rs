//! The palette lists every request of the open collections and opens the chosen one.

use crate::support;

use apinata_app::palette::request_entries;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

fn zoo() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("Users")),
        ("users/zebra.yml", &request("Fetch zebra", "http://x/z")),
        ("lion.yml", &request("Fetch lion", "http://x/l")),
        ("ws.yml", SOCKET),
    ])
}

#[gpui_kit::test]
fn entries_name_each_request_with_its_collection_and_folder(cx: &mut TestAppContext) {
    let col = zoo();
    let (_state, ws, root) = open_dir(cx, &col);
    let entries = ws.read_with(cx, request_entries);
    assert!(entries.iter().all(|e| e.root == root));
    let mut got: Vec<(String, String, String)> = entries
        .into_iter()
        .map(|e| (e.label.clone(), e.detail(), e.path.as_str().to_owned()))
        .collect();
    got.sort();
    let want = [
        ("Fetch lion", "c", "lion.yml"),
        ("Fetch zebra", "c / users", "users/zebra.yml"),
    ]
    .map(|(a, b, c)| (a.to_owned(), b.to_owned(), c.to_owned()));
    assert_eq!(got, want, "the websocket file is not offered");
}

#[gpui_kit::test]
fn typing_a_request_name_and_enter_opens_it(cx: &mut TestAppContext) {
    let col = zoo();
    let s = started(cx, &col);
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.input("zebra", cx))
        .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "enter");
    frame(cx, s.win);
    cx.run_until_parked();
    let open = cx
        .update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap();
    assert!(!open, "the palette closed");
    assert_eq!(tab_names(cx, &s.ws), ["users/zebra.yml"]);
    let preview =
        s.ws.read_with(cx, |w, _| w.tabs().active().map(|e| e.preview));
    assert_eq!(preview, Some(false), "a palette open keeps the tab");
}
