use crate::support;

use apinata_app::model::CollectionModel;
use apinata_app::session::Session;
use apinata_app::workspace::WorkspaceEvent;
use apinata_core::MemoryStore;
use gpui_kit::TestAppContext;
use std::sync::Arc;
use support::*;

const REQUEST: &str =
    "meta {\n  name: a\n  type: http\n  seq: 1\n}\n\nget {\n  url: https://example.com\n}\n";

fn legacy() -> tempfile::TempDir {
    write_tree(&[
        (
            "bruno.json",
            r#"{"version": "1", "name": "Legacy", "type": "collection"}"#,
        ),
        ("a.bru", REQUEST),
    ])
}

#[gpui_kit::test]
fn a_legacy_root_fails_to_open_as_legacy(cx: &mut TestAppContext) {
    let dir = legacy();
    let model = cx.update(|cx| {
        CollectionModel::open(dir.path().to_path_buf(), Arc::new(MemoryStore::new()), cx)
    });
    cx.run_until_parked();
    assert!(model.read_with(cx, |m, _| m.is_legacy()
        && m.open_error().is_some()
        && m.snapshot().is_none()));
}

#[gpui_kit::test]
fn other_open_failures_are_not_legacy(cx: &mut TestAppContext) {
    let model = cx.update(|cx| {
        CollectionModel::open(
            "/nonexistent/apinata-root".into(),
            Arc::new(MemoryStore::new()),
            cx,
        )
    });
    cx.run_until_parked();
    assert!(model.read_with(cx, |m, _| !m.is_legacy() && m.open_error().is_some()));
}

#[gpui_kit::test]
fn the_workspace_reports_a_legacy_root_and_forgets_it(cx: &mut TestAppContext) {
    let files = tempfile::tempdir().unwrap();
    let dir = legacy();
    let root = dir.path().to_path_buf();
    let start = Session {
        open: vec![root.clone()],
        ..Session::default()
    };
    let session = session_at(cx, &files, start);
    let ws = workspace(cx, &session);
    let seen = events(cx, &ws);
    ws.update(cx, |w, cx| w.open(root.clone(), cx));
    cx.run_until_parked();
    assert!(ws.read_with(cx, |w, _| w.collections().is_empty()));
    assert_eq!(*seen.borrow(), [WorkspaceEvent::LegacyFormat { root }]);
    assert!(session_value(cx, &session).open.is_empty());
}
