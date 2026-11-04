use crate::support;

use apinata_app::command::{Command, Field, Target, TreeOp};
use apinata_app::session::{
    CollectionSession, SavedExample, SavedPanel, SavedPanelKind, SavedTab, Session,
};
use apinata_app::snapshot::DiskState;
use apinata_app::tabs::TabKind;
use apinata_app::workspace::{Workspace, WorkspaceEvent, tabs_closed_message};
use gpui_kit::{Entity, TestAppContext};
use std::path::Path;
use support::*;

#[gpui_kit::test]
fn open_item_opens_requests_of_open_roots_only(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    assert!(
        ws.update(cx, |w, cx| w.open_item(&root, ip("a.yml"), true, cx))
            .is_some()
    );
    assert_eq!(
        ws.update(cx, |w, cx| w.open_item(&root, ip("nope.yml"), false, cx)),
        None
    );
    assert_eq!(
        ws.update(cx, |w, cx| w.open_item(
            Path::new("/nowhere"),
            ip("a.yml"),
            false,
            cx
        )),
        None
    );
    assert_eq!(tab_names(cx, &ws), ["a.yml"]);
    ws.update(cx, |w, cx| w.close(&root, cx));
    assert!(tab_names(cx, &ws).is_empty());
}

#[gpui_kit::test]
fn a_renamed_item_keeps_its_tab(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let id = open_tab(cx, &ws, &root, "a.yml");
    let rename = TreeOp::Rename {
        item: ip("a.yml"),
        name: "c".into(),
        rename_file: true,
    };
    ws.update(cx, |w, cx| w.send(&root, Command::Tree(rename), cx));
    cx.run_until_parked();
    let entry = ws
        .read_with(cx, |w, _| w.tabs().get(id).cloned())
        .expect("kept");
    assert_eq!(entry.kind, TabKind::Request(ip("c.yml")));
}

#[gpui_kit::test]
fn a_clean_item_removed_on_disk_closes_its_tab(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let seen = events(cx, &ws);
    open_tab(cx, &ws, &root, "a.yml");
    std::fs::remove_file(col.path().join("a.yml")).unwrap();
    ws.update(cx, |w, cx| {
        w.send(&root, Command::Reload(vec![col.path().join("a.yml")]), cx)
    });
    cx.run_until_parked();
    assert!(tab_names(cx, &ws).is_empty());
    let closed = WorkspaceEvent::TabsClosed {
        root: root.clone(),
        items: vec![ip("a.yml")],
        tabs: 1,
    };
    assert!(seen.borrow().contains(&closed));
}

#[gpui_kit::test]
fn a_dirty_item_removed_on_disk_keeps_its_tab(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    open_tab(cx, &ws, &root, "a.yml");
    ws.update(cx, |w, cx| {
        w.send(&root, edit_url("a.yml", "http://x/mine"), cx)
    });
    cx.run_until_parked();
    std::fs::remove_file(col.path().join("a.yml")).unwrap();
    ws.update(cx, |w, cx| {
        w.send(&root, Command::Reload(vec![col.path().join("a.yml")]), cx)
    });
    cx.run_until_parked();
    assert_eq!(tab_names(cx, &ws), ["a.yml"]);
    let view = ws
        .read_with(cx, |w, cx| w.request(&root, &ip("a.yml"), cx))
        .expect("kept");
    assert_eq!(view.disk, DiskState::Removed);
}

#[gpui_kit::test]
fn tabs_are_saved_in_the_session_and_restored(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    open_tab(cx, &ws, &root, "a.yml");
    open_tab(cx, &ws, &root, "b.yml");
    let session = ws.read_with(cx, |w, _| w.session().clone());
    assert_eq!(
        session_value(cx, &session).collections[&root].tabs,
        ["a.yml", "b.yml"]
    );

    let state = tempfile::tempdir().unwrap();
    let mut start = Session {
        open: vec![root.clone()],
        ..Session::default()
    };
    let saved = CollectionSession {
        tabs: vec!["b.yml".into(), "gone.yml".into()],
        ..CollectionSession::default()
    };
    start.collections.insert(root.clone(), saved);
    let session = session_at(cx, &state, start);
    let again = workspace(cx, &session);
    again.update(cx, |w, cx| w.restore(cx));
    cx.run_until_parked();
    assert_eq!(tab_names(cx, &again), ["b.yml"]);
}

#[gpui_kit::test]
fn in_flight_follows_the_acknowledgement(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let t = Target::Request(ip("a.yml"));
    ws.update(cx, |w, cx| {
        w.send(&root, edit_url("a.yml", "http://x/1"), cx)
    });
    assert!(ws.read_with(cx, |w, cx| w.in_flight(&root, &t, Field::UrlParams, cx)));
    cx.run_until_parked();
    assert!(!ws.read_with(cx, |w, cx| w.in_flight(&root, &t, Field::UrlParams, cx)));
}

#[gpui_kit::test]
fn reopen_skips_items_that_are_gone(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let a = open_tab(cx, &ws, &root, "a.yml");
    let b = open_tab(cx, &ws, &root, "b.yml");
    ws.update(cx, |w, cx| {
        w.update_tabs(cx, |t| {
            t.close(b);
            t.close(a);
        })
    });
    std::fs::remove_file(col.path().join("a.yml")).unwrap();
    ws.update(cx, |w, cx| {
        w.send(&root, Command::Reload(vec![col.path().join("a.yml")]), cx)
    });
    cx.run_until_parked();
    assert!(ws.update(cx, |w, cx| w.reopen_tab(cx)).is_some());
    assert_eq!(tab_names(cx, &ws), ["b.yml"]);
    assert_eq!(ws.update(cx, |w, cx| w.reopen_tab(cx)), None);
}

fn previews(cx: &mut TestAppContext, ws: &Entity<Workspace>) -> Vec<(String, bool)> {
    ws.read_with(cx, |w, _| {
        w.tabs()
            .entries()
            .iter()
            .filter_map(|e| e.item().map(|p| (p.as_str().to_owned(), e.preview)))
            .collect()
    })
}

#[gpui_kit::test]
fn the_preview_tab_is_saved_and_restored_as_a_preview(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    open_tab(cx, &ws, &root, "a.yml");
    ws.update(cx, |w, cx| w.open_item(&root, ip("b.yml"), true, cx))
        .expect("a request");
    let session = ws.read_with(cx, |w, _| w.session().clone());
    let saved = session_value(cx, &session).collections[&root].clone();
    assert_eq!(saved.tabs, ["a.yml", "b.yml"]);
    assert_eq!(saved.preview.as_deref(), Some("b.yml"));

    let state = tempfile::tempdir().unwrap();
    let mut start = Session {
        open: vec![root.clone()],
        ..Session::default()
    };
    start.collections.insert(root.clone(), saved);
    let stored = session_at(cx, &state, start);
    let again = workspace(cx, &stored);
    again.update(cx, |w, cx| w.restore(cx));
    cx.run_until_parked();
    assert_eq!(
        previews(cx, &again),
        [("a.yml".to_owned(), false), ("b.yml".to_owned(), true)]
    );
}

#[gpui_kit::test]
fn pinning_the_preview_clears_it_from_the_session(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let tab = ws
        .update(cx, |w, cx| w.open_item(&root, ip("a.yml"), true, cx))
        .expect("a request");
    let session = ws.read_with(cx, |w, _| w.session().clone());
    assert_eq!(
        session_value(cx, &session).collections[&root]
            .preview
            .as_deref(),
        Some("a.yml")
    );
    ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.keep(tab)));
    let saved = session_value(cx, &session).collections[&root].clone();
    assert_eq!(saved.tabs, ["a.yml"]);
    assert_eq!(saved.preview, None);
}

#[gpui_kit::test]
fn a_stale_saved_preview_restores_no_preview(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_s, _ws, root) = open_dir(cx, &col);
    let state = tempfile::tempdir().unwrap();
    let mut start = Session {
        open: vec![root.clone()],
        ..Session::default()
    };
    let saved = CollectionSession {
        tabs: vec!["a.yml".into()],
        preview: Some("gone.yml".into()),
        ..CollectionSession::default()
    };
    start.collections.insert(root.clone(), saved);
    let stored = session_at(cx, &state, start);
    let again = workspace(cx, &stored);
    again.update(cx, |w, cx| w.restore(cx));
    cx.run_until_parked();
    assert_eq!(previews(cx, &again), [("a.yml".to_owned(), false)]);
}

/// `a.yml` with two saved examples.
fn with_examples() -> tempfile::TempDir {
    let file = format!(
        "{}examples:\n  - name: ok\n    response:\n      status: 200\n  - name: missing\n    response:\n      status: 404\n",
        request("a", "http://x/a")
    );
    write_tree(&[("opencollection.yml", ROOT), ("a.yml", &file)])
}

fn kinds(cx: &mut TestAppContext, ws: &Entity<Workspace>) -> Vec<TabKind> {
    ws.read_with(cx, |w, _| {
        w.tabs().entries().iter().map(|e| e.kind.clone()).collect()
    })
}

#[gpui_kit::test]
fn example_tabs_are_saved_in_the_session_and_restored(cx: &mut TestAppContext) {
    let col = with_examples();
    let (_state, ws, root) = open_dir(cx, &col);
    open_tab(cx, &ws, &root, "a.yml");
    ws.update(cx, |w, cx| w.open_example(&root, ip("a.yml"), 1, false, cx))
        .expect("the example tab");
    cx.run_until_parked();
    let session = ws.read_with(cx, |w, _| w.session().clone());
    let saved = session_value(cx, &session).collections[&root].clone();
    assert_eq!(saved.tabs, ["a.yml"]);
    assert_eq!(
        saved.examples,
        [SavedExample {
            item: "a.yml".into(),
            index: 1,
        }]
    );

    let mut stale = saved.clone();
    stale.examples.push(SavedExample {
        item: "a.yml".into(),
        index: 7,
    });
    stale.examples.push(SavedExample {
        item: "gone.yml".into(),
        index: 0,
    });
    let state = tempfile::tempdir().unwrap();
    let mut start = Session {
        open: vec![root.clone()],
        ..Session::default()
    };
    start.collections.insert(root.clone(), stale);
    let stored = session_at(cx, &state, start);
    let again = workspace(cx, &stored);
    again.update(cx, |w, cx| w.restore(cx));
    cx.run_until_parked();
    assert_eq!(
        kinds(cx, &again),
        [
            TabKind::Request(ip("a.yml")),
            TabKind::Example {
                item: ip("a.yml"),
                index: 1,
            },
        ]
    );
    let active = again.read_with(cx, |w, _| w.tabs().active().map(|e| e.kind.clone()));
    assert_eq!(
        active,
        Some(TabKind::Example {
            item: ip("a.yml"),
            index: 1,
        })
    );
}

#[test]
fn a_session_saved_before_example_tabs_still_loads() {
    let old = r#"{"open":["/c"],"collections":{"/c":{"tabs":["a.yml"],"environment":"dev","preview":"a.yml"}}}"#;
    let session: Session = serde_json::from_str(old).expect("an old session loads");
    let saved = &session.collections[Path::new("/c")];
    assert_eq!(saved.tabs, ["a.yml"]);
    assert_eq!(saved.preview.as_deref(), Some("a.yml"));
    assert!(saved.examples.is_empty());
}

/// `a.yml` with two saved examples, and `b.yml`.
fn with_examples_and_b() -> tempfile::TempDir {
    let file = format!(
        "{}examples:\n  - name: ok\n    response:\n      status: 200\n  - name: missing\n    response:\n      status: 404\n",
        request("a", "http://x/a")
    );
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &file),
        ("b.yml", &request("b", "http://x/b")),
    ])
}

fn saved_active(cx: &mut TestAppContext, ws: &Entity<Workspace>, root: &Path) -> Option<SavedTab> {
    let session = ws.read_with(cx, |w, _| w.session().clone());
    session_value(cx, &session).collections[root].active.clone()
}

#[gpui_kit::test]
fn the_active_tab_is_saved_even_when_only_the_activation_changes(cx: &mut TestAppContext) {
    let col = with_examples_and_b();
    let (_state, ws, root) = open_dir(cx, &col);
    let a = open_tab(cx, &ws, &root, "a.yml");
    open_tab(cx, &ws, &root, "b.yml");
    ws.update(cx, |w, cx| w.open_example(&root, ip("a.yml"), 1, false, cx))
        .expect("the example tab");
    cx.run_until_parked();
    assert_eq!(
        saved_active(cx, &ws, &root),
        Some(SavedTab::Example(SavedExample {
            item: "a.yml".into(),
            index: 1,
        }))
    );
    ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate(a)));
    cx.run_until_parked();
    assert_eq!(
        saved_active(cx, &ws, &root),
        Some(SavedTab::Request("a.yml".into()))
    );
}

#[gpui_kit::test]
fn a_restored_example_sits_after_its_request_and_the_saved_active_tab_returns(
    cx: &mut TestAppContext,
) {
    let col = with_examples_and_b();
    let root = col.path().to_path_buf();
    let state = tempfile::tempdir().unwrap();
    let saved = CollectionSession {
        tabs: vec!["a.yml".into(), "b.yml".into()],
        examples: vec![SavedExample {
            item: "a.yml".into(),
            index: 1,
        }],
        active: Some(SavedTab::Example(SavedExample {
            item: "a.yml".into(),
            index: 1,
        })),
        ..CollectionSession::default()
    };
    let mut start = Session {
        open: vec![root.clone()],
        ..Session::default()
    };
    start.collections.insert(root.clone(), saved);
    let stored = session_at(cx, &state, start);
    let again = workspace(cx, &stored);
    again.update(cx, |w, cx| w.restore(cx));
    cx.run_until_parked();
    let example = TabKind::Example {
        item: ip("a.yml"),
        index: 1,
    };
    assert_eq!(
        kinds(cx, &again),
        [
            TabKind::Request(ip("a.yml")),
            example.clone(),
            TabKind::Request(ip("b.yml")),
        ]
    );
    let active = again.read_with(cx, |w, _| w.tabs().active().map(|e| e.kind.clone()));
    assert_eq!(active, Some(example));
}

// A saved example whose request tab was not saved has no neighbour to sit beside, so it
// restores after every restored request tab.
#[gpui_kit::test]
fn a_restored_example_without_its_request_tab_sits_at_the_end(cx: &mut TestAppContext) {
    let col = with_examples_and_b();
    let root = col.path().to_path_buf();
    let state = tempfile::tempdir().unwrap();
    let saved = CollectionSession {
        tabs: vec!["b.yml".into()],
        examples: vec![SavedExample {
            item: "a.yml".into(),
            index: 0,
        }],
        ..CollectionSession::default()
    };
    let mut start = Session {
        open: vec![root.clone()],
        ..Session::default()
    };
    start.collections.insert(root.clone(), saved);
    let stored = session_at(cx, &state, start);
    let again = workspace(cx, &stored);
    again.update(cx, |w, cx| w.restore(cx));
    cx.run_until_parked();
    assert_eq!(
        kinds(cx, &again),
        [
            TabKind::Request(ip("b.yml")),
            TabKind::Example {
                item: ip("a.yml"),
                index: 0,
            },
        ]
    );
}

// Two roots restore in either order: the root without a saved active tab never takes the
// activation from the other root's restored tab, and the next save keeps that choice.
#[gpui_kit::test]
fn a_root_without_a_saved_active_tab_leaves_the_restored_one_active(cx: &mut TestAppContext) {
    let with = with_examples_and_b();
    let without = two_requests();
    let (a, b) = (with.path().to_path_buf(), without.path().to_path_buf());
    let example = SavedExample {
        item: "a.yml".into(),
        index: 1,
    };
    for open in [vec![a.clone(), b.clone()], vec![b.clone(), a.clone()]] {
        let state = tempfile::tempdir().unwrap();
        let mut start = Session {
            open: open.clone(),
            ..Session::default()
        };
        start.collections.insert(
            a.clone(),
            CollectionSession {
                tabs: vec!["a.yml".into()],
                examples: vec![example.clone()],
                active: Some(SavedTab::Example(example.clone())),
                ..CollectionSession::default()
            },
        );
        start.collections.insert(
            b.clone(),
            CollectionSession {
                tabs: vec!["a.yml".into(), "b.yml".into()],
                ..CollectionSession::default()
            },
        );
        let stored = session_at(cx, &state, start);
        let again = workspace(cx, &stored);
        again.update(cx, |w, cx| w.restore(cx));
        cx.run_until_parked();
        let active = again.read_with(cx, |w, _| {
            w.tabs().active().map(|e| (e.root.clone(), e.kind.clone()))
        });
        let restored = TabKind::Example {
            item: ip("a.yml"),
            index: 1,
        };
        assert_eq!(active, Some((a.clone(), restored)), "open order {open:?}");
        assert_eq!(
            saved_active(cx, &again, &a),
            Some(SavedTab::Example(example.clone())),
            "open order {open:?}"
        );
        assert_eq!(saved_active(cx, &again, &b), None, "open order {open:?}");
    }
}

#[test]
fn a_session_saved_before_the_active_tab_still_loads_and_the_field_has_its_shape() {
    let old = r#"{"open":["/c"],"collections":{"/c":{"tabs":["a.yml"],"examples":[{"item":"a.yml","index":0}]}}}"#;
    let session: Session = serde_json::from_str(old).expect("an old session loads");
    let saved = &session.collections[Path::new("/c")];
    assert_eq!(saved.active, None);
    assert_eq!(saved.examples.len(), 1);
    let json = serde_json::to_string(saved).unwrap();
    assert!(
        !json.contains("active"),
        "no active tab writes no key: {json}"
    );
    let with = CollectionSession {
        active: Some(SavedTab::Example(SavedExample {
            item: "a.yml".into(),
            index: 0,
        })),
        ..saved.clone()
    };
    let json = serde_json::to_string(&with).unwrap();
    assert!(
        json.contains(r#""active":{"example":{"item":"a.yml","index":0}}"#),
        "{json}"
    );
    let request = CollectionSession {
        active: Some(SavedTab::Request("a.yml".into())),
        ..saved.clone()
    };
    let json = serde_json::to_string(&request).unwrap();
    assert!(json.contains(r#""active":{"request":"a.yml"}"#), "{json}");
    let back: CollectionSession = serde_json::from_str(&json).unwrap();
    assert_eq!(back, request);
}

#[gpui_kit::test]
fn a_saved_active_example_that_no_longer_exists_restores_the_requests_without_it(
    cx: &mut TestAppContext,
) {
    let col = with_examples_and_b();
    let root = col.path().to_path_buf();
    let state = tempfile::tempdir().unwrap();
    let gone = SavedExample {
        item: "a.yml".into(),
        index: 7,
    };
    let saved = CollectionSession {
        tabs: vec!["a.yml".into(), "b.yml".into()],
        examples: vec![gone.clone()],
        active: Some(SavedTab::Example(gone)),
        ..CollectionSession::default()
    };
    let mut start = Session {
        open: vec![root.clone()],
        ..Session::default()
    };
    start.collections.insert(root.clone(), saved);
    let stored = session_at(cx, &state, start);
    let again = workspace(cx, &stored);
    again.update(cx, |w, cx| w.restore(cx));
    cx.run_until_parked();
    assert_eq!(
        kinds(cx, &again),
        [TabKind::Request(ip("a.yml")), TabKind::Request(ip("b.yml"))]
    );
    let active = again.read_with(cx, |w, _| w.tabs().active().map(|e| e.kind.clone()));
    assert!(
        !matches!(active, Some(TabKind::Example { .. })),
        "no example is active: {active:?}"
    );
}

#[test]
fn the_tabs_closed_message_names_one_item_and_counts_many_tabs() {
    let (a, b) = (ip("a.yml"), ip("b.yml"));
    assert_eq!(tabs_closed_message(&[], 0), None);
    assert_eq!(
        tabs_closed_message(std::slice::from_ref(&a), 1).unwrap(),
        "a is gone, so its tab closed"
    );
    // A nested path names the item, not its folders or extension.
    assert_eq!(
        tabs_closed_message(&[ip("users/list.yml")], 1).unwrap(),
        "list is gone, so its tab closed"
    );
    assert_eq!(
        tabs_closed_message(&[a.clone(), b.clone()], 2).unwrap(),
        "2 tabs closed because their requests are gone"
    );
    // One runner tab holding two items is still one tab.
    assert_eq!(
        tabs_closed_message(&[a, b], 1).unwrap(),
        "1 tab closed because its requests are gone"
    );
}

#[gpui_kit::test]
fn removing_several_open_requests_emits_one_event_with_the_tab_count(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let seen = events(cx, &ws);
    open_tab(cx, &ws, &root, "a.yml");
    open_tab(cx, &ws, &root, "b.yml");
    let files = [col.path().join("a.yml"), col.path().join("b.yml")];
    for f in &files {
        std::fs::remove_file(f).unwrap();
    }
    ws.update(cx, |w, cx| {
        w.send(&root, Command::Reload(files.to_vec()), cx)
    });
    cx.run_until_parked();
    let seen = seen.borrow();
    let closes: Vec<_> = seen
        .iter()
        .filter(|e| matches!(e, WorkspaceEvent::TabsClosed { .. }))
        .collect();
    assert_eq!(closes.len(), 1, "{closes:?}");
    assert!(matches!(
        closes[0],
        WorkspaceEvent::TabsClosed { tabs: 2, .. }
    ));
}

fn with_folder() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/a")),
        ("b.yml", &request("b", "http://x/b")),
        ("users/folder.yml", &folder("users")),
    ])
}

fn panel(kind: SavedPanelKind, position: usize) -> SavedPanel {
    SavedPanel { kind, position }
}

#[test]
fn a_session_without_panel_tabs_loads_and_writes_no_key() {
    let old = r#"{"open":["/c"],"collections":{"/c":{"tabs":["a.yml"]}}}"#;
    let session: Session = serde_json::from_str(old).expect("an old session loads");
    let saved = &session.collections[Path::new("/c")];
    assert!(saved.panels.is_empty());
    assert!(!serde_json::to_string(saved).unwrap().contains("panels"));
}

#[test]
fn panel_tabs_round_trip_through_json() {
    let saved = CollectionSession {
        panels: vec![
            panel(SavedPanelKind::Environments, 0),
            panel(SavedPanelKind::Runner(String::new()), 2),
            panel(SavedPanelKind::Folder("users".into()), 3),
            panel(SavedPanelKind::Collection, 4),
        ],
        active: Some(SavedTab::Panel(SavedPanelKind::Environments)),
        ..CollectionSession::default()
    };
    let json = serde_json::to_string(&saved).unwrap();
    assert_eq!(
        serde_json::from_str::<CollectionSession>(&json).unwrap(),
        saved
    );
}

#[gpui_kit::test]
fn panel_tabs_are_saved_with_their_position_and_active_state(cx: &mut TestAppContext) {
    let col = with_folder();
    let (_state, ws, root) = open_dir(cx, &col);
    open_tab(cx, &ws, &root, "a.yml");
    ws.update(cx, |w, cx| {
        w.update_tabs(cx, |t| t.open(&root, TabKind::Environments, false))
    });
    open_tab(cx, &ws, &root, "b.yml");
    ws.update(cx, |w, cx| {
        w.update_tabs(cx, |t| t.open(&root, TabKind::Runner(ip("users")), false))
    });
    ws.update(cx, |w, cx| {
        w.open_runner_items(&root, vec![ip("users"), ip("a.yml")], cx)
    });
    cx.run_until_parked();
    let session = ws.read_with(cx, |w, _| w.session().clone());
    let saved = session_value(cx, &session).collections[&root].clone();
    let panels = serde_json::to_value(&saved.panels).unwrap();
    assert_eq!(
        panels,
        serde_json::json!([
            {"kind": "environments", "position": 1},
            {"kind": {"runner": "users"}, "position": 3},
            {"kind": {"selection": ["a.yml", "users"]}, "position": 4},
        ])
    );
    assert_eq!(saved.tabs, ["a.yml", "b.yml"]);
    let active = serde_json::to_value(&saved.active).unwrap();
    assert_eq!(
        active,
        serde_json::json!({"panel": {"selection": ["a.yml", "users"]}})
    );
}

#[gpui_kit::test]
fn panel_tabs_return_in_place_and_the_active_one_is_active(cx: &mut TestAppContext) {
    let col = with_folder();
    let root = col.path().to_path_buf();
    let state = tempfile::tempdir().unwrap();
    let saved = CollectionSession {
        tabs: vec!["a.yml".into(), "b.yml".into()],
        panels: vec![
            panel(SavedPanelKind::Collection, 0),
            panel(SavedPanelKind::Environments, 2),
            panel(SavedPanelKind::Runner(String::new()), 3),
            panel(SavedPanelKind::Folder("users".into()), 4),
        ],
        active: Some(SavedTab::Panel(SavedPanelKind::Environments)),
        ..CollectionSession::default()
    };
    let mut start = Session {
        open: vec![root.clone()],
        ..Session::default()
    };
    start.collections.insert(root.clone(), saved);
    let stored = session_at(cx, &state, start);
    let again = workspace(cx, &stored);
    again.update(cx, |w, cx| w.restore(cx));
    cx.run_until_parked();
    assert_eq!(
        kinds(cx, &again),
        [
            TabKind::Collection,
            TabKind::Request(ip("a.yml")),
            TabKind::Environments,
            TabKind::Runner(ip("")),
            TabKind::Folder(ip("users")),
            TabKind::Request(ip("b.yml")),
        ]
    );
    let active = again.read_with(cx, |w, _| w.tabs().active().map(|e| e.kind.clone()));
    assert_eq!(active, Some(TabKind::Environments));
}

#[gpui_kit::test]
fn a_panel_tab_whose_folder_is_gone_is_dropped_silently(cx: &mut TestAppContext) {
    let col = with_folder();
    let root = col.path().to_path_buf();
    let state = tempfile::tempdir().unwrap();
    let saved = CollectionSession {
        tabs: vec!["a.yml".into()],
        panels: vec![
            panel(SavedPanelKind::Folder("gone".into()), 0),
            panel(SavedPanelKind::Runner("gone".into()), 1),
            panel(SavedPanelKind::Runner(String::new()), 2),
        ],
        ..CollectionSession::default()
    };
    let mut start = Session {
        open: vec![root.clone()],
        ..Session::default()
    };
    start.collections.insert(root.clone(), saved);
    let stored = session_at(cx, &state, start);
    let again = workspace(cx, &stored);
    again.update(cx, |w, cx| w.restore(cx));
    cx.run_until_parked();
    assert_eq!(
        kinds(cx, &again),
        [TabKind::Request(ip("a.yml")), TabKind::Runner(ip(""))]
    );
}

// The app's own delete closes the tabs it covers without a toast, a folder's included.
#[gpui_kit::test]
fn a_delete_from_the_app_closes_its_tabs_without_an_event(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/a")),
        ("users/folder.yml", &folder("users")),
        ("users/list.yml", &request("list", "http://x/l")),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let seen = events(cx, &ws);
    open_tab(cx, &ws, &root, "a.yml");
    open_tab(cx, &ws, &root, "users/list.yml");
    for item in ["a.yml", "users"] {
        let op = TreeOp::Delete { item: ip(item) };
        ws.update(cx, |w, cx| w.send(&root, Command::Tree(op), cx));
    }
    cx.run_until_parked();
    assert!(tab_names(cx, &ws).is_empty());
    let seen = seen.borrow();
    assert!(
        !seen
            .iter()
            .any(|e| matches!(e, WorkspaceEvent::TabsClosed { .. })),
        "{seen:?}"
    );
}

// The session written before run-selection tabs were kept, with every other tab kind.
const OLD_SESSION: &str = r#"{
  "recent": ["/c"], "open": ["/c"], "sidebar_width": 250.0,
  "window": {"x": 1.0, "y": 2.0, "width": 800.0, "height": 600.0, "mode": "windowed"},
  "collections": {"/c": {
    "tabs": ["a.yml"], "environment": "dev",
    "panels": [{"kind": "environments", "position": 1}, {"kind": {"runner": ""}, "position": 2}],
    "active": {"panel": {"runner": ""}},
    "pinned": [{"request": "a.yml"}, {"panel": "environments"}]
  }}
}"#;

fn old_collection() -> CollectionSession {
    CollectionSession {
        tabs: vec!["a.yml".into()],
        environment: Some("dev".into()),
        panels: vec![
            panel(SavedPanelKind::Environments, 1),
            panel(SavedPanelKind::Runner(String::new()), 2),
        ],
        active: Some(SavedTab::Panel(SavedPanelKind::Runner(String::new()))),
        pinned: vec![
            SavedTab::Request("a.yml".into()),
            SavedTab::Panel(SavedPanelKind::Environments),
        ],
        ..CollectionSession::default()
    }
}

#[test]
fn a_session_saved_before_selection_tabs_loads_unchanged() {
    let session: Session = serde_json::from_str(OLD_SESSION).expect("an old session loads");
    assert_eq!(session.collections[Path::new("/c")], old_collection());
    assert_eq!(session.sidebar_width, Some(250.0));
    assert!(session.window.is_some());
}

// A newer build may save a panel kind this one does not know; only that entry is lost.
#[test]
fn an_unknown_panel_kind_is_dropped_and_the_rest_of_the_session_loads() {
    let newer = OLD_SESSION
        .replace(
            r#""panels": ["#,
            r#""panels": [{"kind": {"graph": "x"}, "position": 0}, {"kind": "timeline", "position": 3}, "#,
        )
        .replace(
            r#""pinned": ["#,
            r#""pinned": [{"panel": {"graph": "x"}}, {"later": 1}, "#,
        )
        .replace(
            r#""collections": {"#,
            r#""collections": {"/d": {"tabs": ["b.yml"], "active": {"request": "b.yml"}}, "#,
        );
    let session: Session = serde_json::from_str(&newer).expect("a newer session loads");
    assert_eq!(session.collections[Path::new("/c")], old_collection());
    let other = &session.collections[Path::new("/d")];
    assert_eq!(other.active, Some(SavedTab::Request("b.yml".into())));
    assert_eq!(session.open, [Path::new("/c")]);
    assert!(session.window.is_some());
}

#[test]
fn an_unknown_active_panel_kind_loads_as_no_active_tab() {
    let newer = OLD_SESSION.replace(
        r#""active": {"panel": {"runner": ""}}"#,
        r#""active": {"panel": {"graph": "x"}}"#,
    );
    let session: Session = serde_json::from_str(&newer).expect("a newer session loads");
    let expected = CollectionSession {
        active: None,
        ..old_collection()
    };
    assert_eq!(session.collections[Path::new("/c")], expected);
    assert!(session.window.is_some());
}

// Restores `saved` for `col` into a fresh workspace.
fn restored(
    cx: &mut TestAppContext,
    col: &tempfile::TempDir,
    saved: &str,
) -> (tempfile::TempDir, Entity<Workspace>) {
    let root = col.path().to_path_buf();
    let state = tempfile::tempdir().unwrap();
    let saved: CollectionSession = serde_json::from_str(saved).unwrap();
    let mut start = Session {
        open: vec![root.clone()],
        ..Session::default()
    };
    start.collections.insert(root, saved);
    let stored = session_at(cx, &state, start);
    let again = workspace(cx, &stored);
    again.update(cx, |w, cx| w.restore(cx));
    cx.run_until_parked();
    (state, again)
}

// A member gone on disk since the save is dropped; the rest come back, pinned and active.
#[gpui_kit::test]
fn a_selection_tab_returns_without_its_members_gone_on_disk(cx: &mut TestAppContext) {
    let col = with_folder();
    let selection = r#"{"selection": ["a.yml", "gone.yml", "users"]}"#;
    let saved = format!(
        r#"{{"tabs": ["b.yml"], "panels": [{{"kind": {selection}, "position": 0}}],
            "active": {{"panel": {selection}}}, "pinned": [{{"panel": {selection}}}]}}"#
    );
    let (_state, again) = restored(cx, &col, &saved);
    let kept = TabKind::RunSelection(vec![ip("a.yml"), ip("users")]);
    assert_eq!(
        kinds(cx, &again),
        [kept.clone(), TabKind::Request(ip("b.yml"))]
    );
    let active = again.read_with(cx, |w, _| {
        w.tabs().active().map(|e| (e.kind.clone(), e.pinned))
    });
    assert_eq!(active, Some((kept, true)));
}

#[gpui_kit::test]
fn a_selection_tab_whose_members_are_all_gone_is_dropped(cx: &mut TestAppContext) {
    let col = with_folder();
    let saved = r#"{"tabs": ["a.yml"],
        "panels": [{"kind": {"selection": ["gone.yml", "lost"]}, "position": 1}]}"#;
    let (_state, again) = restored(cx, &col, saved);
    assert_eq!(kinds(cx, &again), [TabKind::Request(ip("a.yml"))]);
}
