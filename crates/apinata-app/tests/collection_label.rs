//! One label rule for a collection everywhere: its `info.name`, a blank one counting as
//! missing, else the folder's basename.

use crate::support;

use apinata_app::session::Session;
use apinata_app::shell::Shell;
use apinata_app::tabs::TabKind;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, WindowOptions};
use support::*;

fn blank(name: &str) -> tempfile::TempDir {
    let yml = format!("opencollection: 1.0.0\ninfo:\n  name: \"{name}\"\n");
    write_tree(&[
        ("opencollection.yml", &yml),
        ("a.yml", &request("a", "http://x/0")),
    ])
}

fn dir_of(col: &tempfile::TempDir) -> String {
    col.path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

#[gpui_kit::test]
fn a_blank_name_falls_back_to_the_folder_everywhere(cx: &mut TestAppContext) {
    for name in ["", "   "] {
        let col = blank(name);
        let dir = dir_of(&col);
        let s = started(cx, &col);
        open_switcher(cx, &s);
        let (root_row, bar, open_row, title) = cx
            .update_window(s.win, |_, w, cx| {
                let root_row = w.find(("collection", 0usize)).label().map(str::to_owned);
                let bar = w
                    .within("title-bar")
                    .find("collection-switcher")
                    .label()
                    .map(str::to_owned);
                let open_row = w.find(("switch-open", 0usize)).label().map(str::to_owned);
                let title = s.ws.update(cx, |ws, cx| {
                    ws.open_settings(&s.root, TabKind::Collection, cx);
                    ws.tabs().active().cloned()
                });
                (root_row, bar, open_row, title)
            })
            .unwrap();
        assert_eq!(root_row.as_deref(), Some(dir.as_str()), "sidebar {name:?}");
        assert_eq!(bar.as_deref(), Some(dir.as_str()), "title bar {name:?}");
        assert_eq!(open_row.as_deref(), Some(dir.as_str()), "switcher {name:?}");
        let entry = title.expect("settings tab");
        let area = s
            .shell
            .read_with(cx, |v, cx| v.tab_area().read(cx).tab_label(&entry, cx));
        assert_eq!(area, format!("{dir} settings"), "tab {name:?}");
    }
}

#[gpui_kit::test]
fn same_named_collections_each_show_their_own_path(cx: &mut TestAppContext) {
    let named = "opencollection: 1.0.0\ninfo:\n  name: Twin\n";
    let (a, b) = (
        write_tree(&[("opencollection.yml", named)]),
        write_tree(&[("opencollection.yml", named)]),
    );
    let state = tempfile::tempdir().unwrap();
    let start = Session {
        recent: vec![b.path().to_path_buf()],
        ..Session::default()
    };
    support::init(cx);
    let session = session_at(cx, &state, start);
    let ws = workspace(cx, &session);
    ws.update(cx, |w, cx| w.open(a.path().to_path_buf(), cx));
    let (win, _view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| Shell::new(ws, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.within("title-bar").click("collection-switcher", cx)
    })
    .unwrap();
    frame(cx, win);
    let (open_path, recent_path, open_label, recent_label) = cx
        .update_window(win, |_, w, _| {
            (
                w.find(("switcher-open-path", 0usize))
                    .label()
                    .map(str::to_owned),
                w.find(("recent-path", 0usize)).label().map(str::to_owned),
                w.find(("switch-open", 0usize)).label().map(str::to_owned),
                w.find(("recent", 0usize)).label().map(str::to_owned),
            )
        })
        .unwrap();
    assert_eq!(open_label.as_deref(), Some("Twin"));
    assert_eq!(recent_label.as_deref(), Some("Twin"));
    assert_eq!(open_path, Some(a.path().display().to_string()));
    assert_eq!(recent_path, Some(b.path().display().to_string()));
}

fn open_switcher(cx: &mut TestAppContext, s: &Started) {
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("collection-switcher", cx)
    })
    .unwrap();
    frame(cx, s.win);
}
