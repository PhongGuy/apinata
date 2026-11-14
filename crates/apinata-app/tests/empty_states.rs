//! The no-collection state, which takes over the sidebar's recent collections, and the
//! no-request state.

use crate::support;

use apinata_app::session::Session;
use apinata_app::shell::Shell;
use apinata_app::style::{PANE_INSET, at_zoom};
use apinata_app::tabs::TabKind;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, WindowOptions, px};
use support::*;

#[gpui_kit::test]
fn with_no_collection_the_window_offers_open_and_recents(cx: &mut TestAppContext) {
    support::init(cx);
    let col = collection();
    let state = tempfile::tempdir().unwrap();
    let start = Session {
        recent: vec![col.path().to_path_buf()],
        ..Session::default()
    };
    let session = session_at(cx, &state, start);
    let ws = workspace(cx, &session);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| Shell::new(ws, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        let welcome = w.within("no-collection");
        assert!(welcome.try_find("app-icon").is_some());
        assert!(welcome.try_find(("empty-action", 0usize)).is_some());
        assert!(welcome.try_find(("recent", 0usize)).is_some());
    })
    .unwrap();
    cx.update_window(win, |_, w, cx| {
        w.within("no-collection")
            .click(("empty-action", 1usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.click(("recent", 0usize), cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(
        view.read_with(cx, |v, cx| v.workspace().read(cx).collections().len()),
        1
    );
    assert!(!present(cx, win, "no-collection"));
    assert!(present(cx, win, "tab-area"));
}

#[gpui_kit::test]
fn the_empty_tab_area_offers_its_actions_with_hints(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, _| {
        let empty = w.within("tab-empty");
        for ix in 0..3usize {
            assert!(
                empty.try_find(("empty-action", ix)).is_some(),
                "action {ix}"
            );
        }
        assert!(
            empty.try_find(("empty-action", 4usize)).is_none(),
            "exactly four actions"
        );
        assert!(
            empty.try_find(("empty-kbd", 0usize)).is_some(),
            "New request shows its key"
        );
    })
    .unwrap();
    cx.update_window(s.win, |_, w, cx| {
        w.within("tab-empty").click(("empty-action", 2usize), cx)
    })
    .unwrap();
    frame(cx, s.win);
    let active =
        s.ws.read_with(cx, |w, _| w.tabs().active().map(|t| t.kind.clone()));
    assert_eq!(active, Some(TabKind::Environments));
}

#[gpui_kit::test]
fn the_environments_empty_state_is_centred(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _view) = env_editor(cx, &ws, &root);
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        let pane = w.find("env-empty-pane").bounds();
        let empty = w.find("env-empty").bounds();
        let (p, e) = (pane.center(), empty.center());
        assert!(
            (p.x - e.x).abs() < px(1.) && (p.y - e.y).abs() < px(1.),
            "{empty:?} in {pane:?}"
        );
        // A label that hugs its text is narrower than the pane, so centring is not vacuous.
        assert!(
            empty.size.width < pane.size.width,
            "{empty:?} fills {pane:?}"
        );
        let editor = w.find("env-editor").bounds();
        let create = w.find("env-create").bounds();
        assert!(
            create.left() >= editor.left() + at_zoom(PANE_INSET, 1.0),
            "list hugs the edge: {create:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_empty_tab_area_names_the_environments_action_like_the_menu(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let label = cx
        .update_window(s.win, |_, w, _| {
            w.within("tab-empty")
                .find(("empty-action", 2usize))
                .label()
                .map(str::to_owned)
        })
        .unwrap();
    assert_eq!(label.as_deref(), Some("Manage environments"));
}

// Starts a shell with `roots` as the recents and nothing open.
fn shell_with_recents(
    cx: &mut TestAppContext,
    roots: Vec<std::path::PathBuf>,
) -> gpui_kit::AnyWindowHandle {
    support::init(cx);
    let state = tempfile::tempdir().unwrap();
    let session = session_at(
        cx,
        &state,
        Session {
            recent: roots,
            ..Session::default()
        },
    );
    let ws = workspace(cx, &session);
    let (win, _view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| Shell::new(ws, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    win
}

#[gpui_kit::test]
fn a_recent_entry_is_labelled_by_the_collection_name_with_its_path(cx: &mut TestAppContext) {
    let named = write_tree(&[(
        "opencollection.yml",
        "opencollection: 1.0.0\ninfo:\n  name: Pretty Name\n",
    )]);
    let bare = tempfile::tempdir().unwrap();
    let bare_path = bare.path().to_path_buf();
    let win = shell_with_recents(cx, vec![named.path().to_path_buf(), bare_path.clone()]);
    let (heading, name0, path0, name1) = cx
        .update_window(win, |_, w, _| {
            let welcome = w.within("no-collection");
            let label = |id: gpui_kit::ElementId| {
                welcome
                    .try_find(id)
                    .and_then(|e| e.label().map(str::to_owned))
            };
            (
                label("recent-heading".into()),
                label(("recent", 0usize).into()),
                label(("recent-path", 0usize).into()),
                label(("recent", 1usize).into()),
            )
        })
        .unwrap();
    assert_eq!(heading.as_deref(), Some("Recent"));
    assert_eq!(name0.as_deref(), Some("Pretty Name"));
    assert_eq!(path0.as_deref(), Some(named.path().to_str().unwrap()));
    let base = bare_path.file_name().unwrap().to_str().unwrap();
    assert_eq!(name1.as_deref(), Some(base));
}

// The Recent list lines up with the centred actions above it: the heading and each entry's
// name and path shrink to their text and sit centred on the title's axis.
#[gpui_kit::test]
fn the_recent_list_is_centred_under_the_actions(cx: &mut TestAppContext) {
    let named = write_tree(&[(
        "opencollection.yml",
        "opencollection: 1.0.0\ninfo:\n  name: Pretty Name\n",
    )]);
    let win = shell_with_recents(cx, vec![named.path().to_path_buf()]);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let welcome = w.within("no-collection");
        let axis = welcome.find("empty-title").bounds().center().x;
        let row = welcome.find(("recent", 0usize)).bounds();
        for id in [
            gpui_kit::ElementId::from("recent-heading"),
            ("recent-name", 0usize).into(),
            ("recent-path", 0usize).into(),
        ] {
            let b = welcome.find(id.clone()).bounds();
            assert!(b.size.width < row.size.width, "{id:?} {b:?} fills {row:?}");
            assert!(
                (b.center().x - axis).abs() < px(1.),
                "{id:?} {b:?} off {axis:?}"
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_recent_heading_is_absent_without_recents(cx: &mut TestAppContext) {
    let win = shell_with_recents(cx, Vec::new());
    assert!(!present(cx, win, "recent-heading"));
}

#[gpui_kit::test]
fn a_recent_whose_folder_is_gone_falls_back_to_its_basename(cx: &mut TestAppContext) {
    let gone = tempfile::tempdir().unwrap().path().join("vanished");
    let win = shell_with_recents(cx, vec![gone]);
    let label = cx
        .update_window(win, |_, w, _| {
            w.within("no-collection")
                .find(("recent", 0usize))
                .label()
                .map(str::to_owned)
        })
        .unwrap();
    assert_eq!(label.as_deref(), Some("vanished"));
}

#[gpui_kit::test]
fn render_reads_recent_names_from_memory_until_a_refresh(cx: &mut TestAppContext) {
    let col = write_tree(&[(
        "opencollection.yml",
        "opencollection: 1.0.0\ninfo:\n  name: Before\n",
    )]);
    let state = tempfile::tempdir().unwrap();
    let session = session_at(
        cx,
        &state,
        Session {
            recent: vec![col.path().to_path_buf()],
            ..Session::default()
        },
    );
    support::init(cx);
    let ws = workspace(cx, &session);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| Shell::new(ws, w, cx))
            })
        })
        .unwrap();
    let label = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, _| {
            w.within("no-collection")
                .find(("recent", 0usize))
                .label()
                .map(str::to_owned)
        })
        .unwrap()
    };
    frame(cx, win);
    assert_eq!(label(cx).as_deref(), Some("Before"));
    // The file changes with no event: a render that read the disk would show the new name.
    std::fs::write(
        col.path().join("opencollection.yml"),
        "opencollection: 1.0.0\ninfo:\n  name: After\n",
    )
    .unwrap();
    view.update(cx, |_, cx| cx.notify());
    frame(cx, win);
    assert_eq!(label(cx).as_deref(), Some("Before"));
    view.update(cx, |s, cx| s.refresh_recent_names(cx));
    frame(cx, win);
    assert_eq!(label(cx).as_deref(), Some("After"));
}
