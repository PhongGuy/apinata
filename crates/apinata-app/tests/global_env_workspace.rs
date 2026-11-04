//! The workspace's global environment: paths, session, selection and what actors see.

use crate::support;

use apinata_app::command::Command;
use apinata_app::persist::Paths;
use apinata_app::session::Session;
use apinata_app::workspace::Workspace;
use apinata_core::{OverlayLayer, VarScope, VariableSet};
use gpui_kit::{Entity, TestAppContext};
use std::path::Path;
use support::*;

const G: &str = "name: g\nvariables:\n  - name: host\n    value: from-global\n  - name: shared\n    value: global\n";
const H: &str = "name: h\nvariables:\n  - name: host\n    value: from-h\n";
const VARS_ROOT: &str = "opencollection: 1.0.0\ninfo:\n  name: c\nrequest:\n  variables:\n    - name: shared\n      value: collection\n";

fn globals_dir() -> tempfile::TempDir {
    write_tree(&[("g.yml", G), ("h.yml", H)])
}

fn col() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", VARS_ROOT),
        ("a.yml", &request("a", "http://{{host}}/0")),
    ])
}

fn variables(cx: &mut TestAppContext, ws: &Entity<Workspace>, root: &Path) -> VariableSet {
    let (reply, answer) = async_channel::bounded(1);
    let item = ip("a.yml");
    ws.update(cx, |w, cx| {
        w.send(root, Command::Variables { item, reply }, cx)
    });
    cx.run_until_parked();
    answer
        .try_recv()
        .expect("the actor replied")
        .expect("built")
}

fn host(
    cx: &mut TestAppContext,
    ws: &Entity<Workspace>,
    root: &Path,
) -> Option<(VarScope, String)> {
    let set = variables(cx, ws, root);
    set.get("host")
        .map(|i| (i.scope(), i.value().unwrap_or_default().to_owned()))
}

fn with_globals(cx: &mut TestAppContext, ws: &Entity<Workspace>, dir: &tempfile::TempDir) {
    let dir = dir.path().to_path_buf();
    ws.update(cx, |w, cx| w.set_global_dir(dir, cx));
    cx.run_until_parked();
}

#[test]
fn paths_keep_global_environments_in_the_profile() {
    let paths = Paths::in_dir(Path::new("/p"));
    assert_eq!(
        paths.global_environments,
        Path::new("/p/global-environments")
    );
}

#[test]
fn the_session_global_is_optional_and_round_trips() {
    let old: Session = serde_json::from_str("{}").expect("an old session loads");
    assert_eq!(old.global_environment, None);
    assert!(
        !serde_json::to_string(&old)
            .unwrap()
            .contains("global_environment")
    );
    let set = Session {
        global_environment: Some("g".into()),
        ..Session::default()
    };
    let text = serde_json::to_string(&set).unwrap();
    let back: Session = serde_json::from_str(&text).unwrap();
    assert_eq!(back.global_environment.as_deref(), Some("g"));
}

#[gpui_kit::test]
fn a_selected_global_reaches_every_open_collection(cx: &mut TestAppContext) {
    let (c1, c2, g) = (col(), col(), globals_dir());
    let (_state, ws, r1) = open_dir(cx, &c1);
    let r2 = c2.path().to_path_buf();
    ws.update(cx, |w, cx| w.open(r2.clone(), cx));
    with_globals(cx, &ws, &g);
    assert_eq!(ws.read_with(cx, |w, _| w.global_environments()), ["g", "h"]);
    assert_eq!(host(cx, &ws, &r1), None, "nothing selected yet");
    ws.update(cx, |w, cx| {
        w.select_global_environment(Some("g".into()), cx)
    });
    cx.run_until_parked();
    for root in [&r1, &r2] {
        assert_eq!(
            host(cx, &ws, root),
            Some((VarScope::GlobalEnvironment, "from-global".into()))
        );
        let shared = variables(cx, &ws, root).get("shared").unwrap();
        assert_eq!(
            shared.scope(),
            VarScope::Collection,
            "collection beats global"
        );
    }
    let session = ws.read_with(cx, |w, _| w.session().clone());
    assert_eq!(
        session_value(cx, &session).global_environment.as_deref(),
        Some("g")
    );
    ws.update(cx, |w, cx| w.select_global_environment(None, cx));
    cx.run_until_parked();
    assert_eq!(host(cx, &ws, &r1), None);
    assert_eq!(session_value(cx, &session).global_environment, None);
}

#[gpui_kit::test]
fn a_collection_opened_later_gets_the_active_global(cx: &mut TestAppContext) {
    let (c1, c2, g) = (col(), col(), globals_dir());
    let (_state, ws, _r1) = open_dir(cx, &c1);
    with_globals(cx, &ws, &g);
    ws.update(cx, |w, cx| {
        w.select_global_environment(Some("h".into()), cx)
    });
    let r2 = c2.path().to_path_buf();
    ws.update(cx, |w, cx| w.open(r2.clone(), cx));
    cx.run_until_parked();
    assert_eq!(
        host(cx, &ws, &r2),
        Some((VarScope::GlobalEnvironment, "from-h".into()))
    );
}

#[gpui_kit::test]
fn the_session_restores_the_global_or_none_when_it_is_gone(cx: &mut TestAppContext) {
    let (c, g) = (col(), globals_dir());
    for (saved, expected) in [("g", Some("g")), ("gone", None)] {
        let state = tempfile::tempdir().unwrap();
        let root = c.path().to_path_buf();
        let start = Session {
            open: vec![root.clone()],
            global_environment: Some(saved.into()),
            ..Session::default()
        };
        let session = session_at(cx, &state, start);
        let ws = workspace(cx, &session);
        ws.update(cx, |w, cx| w.restore(cx));
        with_globals(cx, &ws, &g);
        assert_eq!(
            ws.read_with(cx, |w, _| w.selected_global_environment())
                .as_deref(),
            expected,
            "{saved}"
        );
        assert_eq!(
            session_value(cx, &session).global_environment.as_deref(),
            expected
        );
        if expected.is_some() {
            assert_eq!(
                host(cx, &ws, &root).map(|h| h.1).as_deref(),
                Some("from-global")
            );
        }
    }
}

#[gpui_kit::test]
fn switching_the_global_clears_script_global_values_but_a_reload_keeps_them(
    cx: &mut TestAppContext,
) {
    let (c, g) = (col(), globals_dir());
    let (_state, ws, root) = open_dir(cx, &c);
    with_globals(cx, &ws, &g);
    ws.update(cx, |w, cx| {
        w.select_global_environment(Some("g".into()), cx)
    });
    cx.run_until_parked();
    let overlay = ws
        .read_with(cx, |w, cx| w.env_overlay(&root, cx))
        .expect("open");
    overlay.set_text(OverlayLayer::Global, "x", "1");
    let has_x = |o: &apinata_core::EnvOverlay| {
        o.entries()
            .iter()
            .any(|v| v.layer() == OverlayLayer::Global && v.name() == "x")
    };
    ws.update(cx, |w, cx| w.reload_globals(cx));
    cx.run_until_parked();
    assert!(
        has_x(&overlay),
        "same name after a reload keeps script values"
    );
    ws.update(cx, |w, cx| {
        w.select_global_environment(Some("h".into()), cx)
    });
    cx.run_until_parked();
    assert!(!has_x(&overlay), "a switch clears them");
    assert_eq!(host(cx, &ws, &root).map(|h| h.1).as_deref(), Some("from-h"));
}

#[gpui_kit::test]
fn a_reload_picks_up_a_file_written_outside_the_app(cx: &mut TestAppContext) {
    let (c, g) = (col(), globals_dir());
    let (_state, ws, _root) = open_dir(cx, &c);
    with_globals(cx, &ws, &g);
    std::fs::write(g.path().join("k.yml"), "name: k\n").unwrap();
    assert_eq!(ws.read_with(cx, |w, _| w.global_environments()), ["g", "h"]);
    ws.update(cx, |w, cx| w.reload_globals(cx));
    cx.run_until_parked();
    assert_eq!(
        ws.read_with(cx, |w, _| w.global_environments()),
        ["g", "h", "k"]
    );
}

#[gpui_kit::test]
fn a_later_publish_still_applies_the_session_when_the_first_load_was_dropped(
    cx: &mut TestAppContext,
) {
    let (state, g) = (tempfile::tempdir().unwrap(), globals_dir());
    let start = Session {
        global_environment: Some("h".into()),
        ..Session::default()
    };
    let session = session_at(cx, &state, start);
    let ws = workspace(cx, &session);
    let dir = g.path().to_path_buf();
    ws.update(cx, |w, cx| {
        w.set_global_dir(dir, cx);
        w.reload_globals(cx);
    });
    cx.run_until_parked();
    let selected = ws.read_with(cx, |w, _| w.selected_global_environment());
    assert_eq!(selected.as_deref(), Some("h"));
}
