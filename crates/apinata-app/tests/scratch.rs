//! The hidden scratch collection: creation, recovery and what it never touches.

use crate::support;

use apinata_app::scratch;
use apinata_app::session::Session;
use apinata_core::Collection;
use gpui_kit::TestAppContext;
use support::*;

#[test]
fn ensure_creates_once_and_survives_corruption_and_deletion() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("scratch");
    scratch::ensure(&dir).unwrap();
    assert_eq!(Collection::open(&dir).unwrap().name(), scratch::NAME);
    scratch::ensure(&dir).unwrap();
    std::fs::write(dir.join("opencollection.yml"), ": : not yaml [").unwrap();
    scratch::ensure(&dir).unwrap();
    assert_eq!(Collection::open(&dir).unwrap().name(), scratch::NAME);
    std::fs::remove_dir_all(&dir).unwrap();
    scratch::ensure(&dir).unwrap();
    assert!(Collection::open(&dir).is_ok());
}

#[test]
fn ensure_refuses_to_delete_a_directory_not_named_scratch() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("mine");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("keep.txt"), "x").unwrap();
    assert!(scratch::ensure(&dir).is_err());
    assert!(dir.join("keep.txt").exists());
}

#[gpui_kit::test]
fn the_scratch_opens_but_is_never_recorded(cx: &mut TestAppContext) {
    init(cx);
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, Session::default());
    let ws = workspace(cx, &session);
    let dir = state.path().join("scratch");
    ws.update(cx, |w, cx| w.set_scratch_dir(dir.clone(), cx));
    let root = ws.update(cx, |w, cx| w.open_scratch(cx)).expect("opens");
    cx.run_until_parked();
    ws.read_with(cx, |w, cx| {
        assert!(w.is_scratch(&root));
        assert_eq!(w.collections().len(), 1);
        assert!(w.user_collections().is_empty());
        assert!(!w.scratch_has_work(cx));
    });
    let s = session_value(cx, &session);
    assert!(s.open.is_empty() && s.recent.is_empty(), "{s:?}");
}

#[gpui_kit::test]
fn current_root_never_falls_back_to_the_scratch(cx: &mut TestAppContext) {
    init(cx);
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, Session::default());
    let ws = workspace(cx, &session);
    ws.update(cx, |w, cx| {
        w.set_scratch_dir(state.path().join("scratch"), cx)
    });
    ws.update(cx, |w, cx| w.open_scratch(cx));
    cx.run_until_parked();
    assert!(ws.read_with(cx, |w, _| w.current_root().is_none()));
}

#[gpui_kit::test]
fn scratch_has_work_sees_a_draft_even_without_a_tab(cx: &mut TestAppContext) {
    init(cx);
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, Session::default());
    let ws = workspace(cx, &session);
    ws.update(cx, |w, cx| {
        w.set_scratch_dir(state.path().join("scratch"), cx)
    });
    let root = ws.update(cx, |w, cx| w.open_scratch(cx)).unwrap();
    cx.run_until_parked();
    ws.update(cx, |w, cx| {
        w.new_draft(&root, apinata_core::ItemPath::root(), cx)
    });
    cx.run_until_parked();
    ws.update(cx, |w, cx| {
        let ids: Vec<_> = w.tabs().entries().iter().map(|e| e.id).collect();
        w.update_tabs(cx, |t| ids.into_iter().for_each(|id| drop(t.close(id))));
    });
    assert!(ws.read_with(cx, |w, cx| w.scratch_has_work(cx)));
}

#[gpui_kit::test]
fn a_user_collection_stays_current_beside_the_scratch(cx: &mut TestAppContext) {
    init(cx);
    let col = collection();
    let (state, ws, root) = open_dir(cx, &col);
    let dir = state.path().join("scratch");
    ws.update(cx, |w, cx| w.set_scratch_dir(dir, cx));
    let scratch_root = ws.update(cx, |w, cx| w.open_scratch(cx)).unwrap();
    cx.run_until_parked();
    ws.read_with(cx, |w, _| {
        assert_eq!(w.current_root().as_deref(), Some(root.as_path()));
        assert_eq!(w.user_collections().len(), 1);
        assert!(w.is_scratch(&scratch_root));
    });
}

#[gpui_kit::test]
fn without_a_scratch_dir_open_scratch_does_nothing(cx: &mut TestAppContext) {
    init(cx);
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, Session::default());
    let ws = workspace(cx, &session);
    assert!(ws.update(cx, |w, cx| w.open_scratch(cx)).is_none());
}
