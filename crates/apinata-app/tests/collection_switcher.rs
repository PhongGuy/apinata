//! The title bar's collection switcher: open collections and the recents not open.

use crate::support;

use apinata_app::session::Session;
use apinata_app::shell::Shell;
use apinata_app::style::{SPACE_3, at_zoom};
use apinata_app::tabs::TabKind;
use apinata_core::ItemPath;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, WindowOptions};
use std::path::PathBuf;
use support::*;

#[gpui_kit::test]
fn the_switcher_lists_open_and_recent_collections(cx: &mut TestAppContext) {
    support::init(cx);
    let (a, b) = (collection(), two_requests());
    let state = tempfile::tempdir().unwrap();
    let start = Session {
        recent: vec![b.path().to_path_buf()],
        ..Session::default()
    };
    let session = session_at(cx, &state, start);
    let ws = workspace(cx, &session);
    ws.update(cx, |w, cx| w.open(a.path().to_path_buf(), cx));
    let shared = ws.clone();
    let (win, _view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| Shell::new(shared, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    let open_switcher = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, cx| {
            w.within("title-bar").click("collection-switcher", cx)
        })
        .unwrap();
        frame(cx, win);
    };
    open_switcher(cx);
    cx.update_window(win, |_, w, cx| w.click(("switcher-settings", 0usize), cx))
        .unwrap();
    frame(cx, win);
    let kind = ws.read_with(cx, |w, _| w.tabs().active().map(|t| t.kind.clone()));
    assert_eq!(kind, Some(TabKind::Collection));
    open_switcher(cx);
    cx.update_window(win, |_, w, cx| w.click(("recent", 0usize), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(ws.read_with(cx, |w, _| w.collections().len()), 2);
}

#[gpui_kit::test]
fn the_switcher_starts_with_the_collection_actions(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("collection-switcher", cx)
    })
    .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        let tops: Vec<_> = [
            "switcher-open-collection",
            "switcher-import-collection",
            "switcher-new-collection",
        ]
        .into_iter()
        .map(|id| w.find(id).bounds().top())
        .collect();
        assert!(tops.windows(2).all(|p| p[0] < p[1]), "in order: {tops:?}");
        let first = w.find(("switch-open", 0usize)).bounds().top();
        assert!(tops[2] < first, "the actions come before the collections");
    })
    .unwrap();
}

#[gpui_kit::test]
fn switcher_open_prompts_for_a_directory(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    switcher_click(cx, s.win, "switcher-open-collection");
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|o| {
        assert!(o.directories && !o.files && !o.multiple);
        None
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn switcher_import_prompts_for_a_file(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    switcher_click(cx, s.win, "switcher-import-collection");
    cx.update_window(s.win, |_, w, cx| w.click("import-link-browse", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|o| {
        assert!(o.files && !o.directories && !o.multiple);
        None
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn switcher_new_prompts_for_a_new_directory(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    switcher_click(cx, s.win, "switcher-new-collection");
    assert!(cx.did_prompt_for_new_path());
    cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_switcher_actions_share_one_left_edge(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("collection-switcher", cx)
    })
    .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        let ids = [
            "switcher-open-collection",
            "switcher-import-collection",
            "switcher-new-collection",
        ];
        let icons: Vec<_> = (0..3usize)
            .map(|ix| w.find(("switcher-icon", ix)).bounds().left())
            .collect();
        assert!(icons.windows(2).all(|p| p[0] == p[1]), "icons {icons:?}");
        for (ix, id) in ids.iter().enumerate() {
            let row = w.find(*id).bounds();
            assert!(
                icons[ix] - row.left() <= at_zoom(SPACE_3, 1.0),
                "{id}: icon not at the row's left"
            );
        }
        assert!(
            w.try_find("switcher-divider").is_some(),
            "rows follow the divider"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn with_nothing_to_list_the_switcher_has_no_divider(cx: &mut TestAppContext) {
    support::init(cx);
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, Session::default());
    let ws = workspace(cx, &session);
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
    cx.update_window(win, |_, w, _| {
        assert!(w.try_find("switcher-new-collection").is_some());
        assert!(w.try_find("switcher-divider").is_none());
    })
    .unwrap();
}

const NAMED: &str = "opencollection: 1.0.0\ninfo:\n  name: Flows\n";

#[gpui_kit::test]
fn the_switcher_names_the_collection_by_its_file(cx: &mut TestAppContext) {
    support::init(cx);
    let col = write_tree(&[("opencollection.yml", NAMED)]);
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, Session::default());
    let ws = workspace(cx, &session);
    ws.update(cx, |w, cx| w.open(col.path().to_path_buf(), cx));
    let shared = ws.clone();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| Shell::new(shared, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    assert_eq!(
        view.read_with(cx, |v, cx| v.current_name(cx)).as_deref(),
        Some("Flows")
    );
    let elsewhere = tempfile::tempdir().unwrap();
    let dir = elsewhere
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        ws.read_with(cx, |w, cx| w.collection_name(elsewhere.path(), cx))
            .to_string(),
        dir,
        "a collection not open falls back to its directory name"
    );
}

fn open_switcher(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle) {
    cx.update_window(win, |_, w, cx| {
        w.within("title-bar").click("collection-switcher", cx)
    })
    .unwrap();
    frame(cx, win);
}

// Opens a second collection; returns it with its switcher index and root as the workspace keeps it.
fn second_collection(cx: &mut TestAppContext, s: &Started) -> (tempfile::TempDir, usize, PathBuf) {
    let b = collection();
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

#[gpui_kit::test]
fn an_open_row_reveals_its_root_in_the_sidebar(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let (_b, ix, root) = second_collection(cx, &s);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    press_in_shell(cx, &s, &key("b"));
    assert!(!s.shell.read_with(cx, |v, _| v.sidebar_visible()));
    open_switcher(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click(("switch-open", ix), cx))
        .unwrap();
    frame(cx, s.win);
    assert!(s.shell.read_with(cx, |v, _| v.sidebar_visible()));
    let selected = s
        .shell
        .read_with(cx, |v, cx| v.sidebar().read(cx).selected(cx));
    assert_eq!(selected, Some((root, ItemPath::root())));
    assert!(
        !present(cx, s.win, ("switch-open", 0usize)),
        "the popover closed"
    );
    let kind =
        s.ws.read_with(cx, |w, _| w.tabs().active().map(|t| t.kind.clone()));
    assert_eq!(
        kind,
        Some(TabKind::Request(ip("a.yml"))),
        "no settings tab opens"
    );
}

#[gpui_kit::test]
fn the_gear_opens_that_collections_settings(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let (_b, ix, root) = second_collection(cx, &s);
    open_switcher(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click(("switcher-settings", ix), cx))
        .unwrap();
    frame(cx, s.win);
    let active = s.ws.read_with(cx, |w, _| {
        w.tabs().active().map(|t| (t.root.clone(), t.kind.clone()))
    });
    assert_eq!(active, Some((root, TabKind::Collection)));
}

#[gpui_kit::test]
fn a_recent_switcher_entry_shows_the_collection_name(cx: &mut TestAppContext) {
    support::init(cx);
    let (a, named) = (
        collection(),
        write_tree(&[(
            "opencollection.yml",
            "opencollection: 1.0.0\ninfo:\n  name: Pretty Name\n",
        )]),
    );
    let state = tempfile::tempdir().unwrap();
    let start = Session {
        recent: vec![named.path().to_path_buf()],
        ..Session::default()
    };
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
    let label = cx
        .update_window(win, |_, w, _| {
            w.find(("recent", 0usize)).label().map(str::to_owned)
        })
        .unwrap();
    assert_eq!(label.as_deref(), Some("Pretty Name"));
}
