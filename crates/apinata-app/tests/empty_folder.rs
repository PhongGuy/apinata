//! Opening an empty folder: the offer to create a collection there, and what stays refused.

use crate::support;

use apinata_app::empty_folder::offer_detail;
use apinata_app::model::CollectionModel;
use apinata_app::session::Session;
use apinata_app::shell::Shell;
use apinata_app::workspace::WorkspaceEvent;
use apinata_core::{Collection, MemoryStore};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, WindowOptions};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use support::*;

// A folder named "shop api" holding only what a fresh repository leaves behind.
fn empty_folder() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("shop api");
    fs::create_dir_all(root.join(".git")).unwrap();
    (dir, root)
}

fn shell(cx: &mut TestAppContext) -> (tempfile::TempDir, AnyWindowHandle, Entity<Shell>) {
    support::init(cx);
    let files = tempfile::tempdir().unwrap();
    let session = session_at(cx, &files, Session::default());
    let ws = workspace(cx, &session);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| Shell::new(ws, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    (files, win, view)
}

fn open_root(cx: &mut TestAppContext, view: &Entity<Shell>, root: &Path) {
    let ws = view.read_with(cx, |v, _| v.workspace().clone());
    ws.update(cx, |w, cx| w.open_revealed(root.to_path_buf(), cx));
    cx.run_until_parked();
}

fn opened(cx: &mut TestAppContext, view: &Entity<Shell>) -> Vec<PathBuf> {
    view.read_with(cx, |v, cx| {
        v.workspace()
            .read(cx)
            .collections()
            .iter()
            .map(|c| c.root().to_path_buf())
            .collect()
    })
}

#[test]
fn the_offer_names_the_folder_it_is_about() {
    assert_eq!(
        offer_detail(Path::new("/work/shop api")),
        "/work/shop api is not a collection. The folder is empty, so a new collection can be \
         created in it."
    );
}

#[gpui_kit::test]
fn an_empty_folder_fails_to_open_as_an_empty_folder(cx: &mut TestAppContext) {
    let (_dir, root) = empty_folder();
    let model = cx.update(|cx| CollectionModel::open(root, Arc::new(MemoryStore::new()), cx));
    cx.run_until_parked();
    assert!(model.read_with(cx, |m, _| m.is_empty_folder()
        && !m.is_legacy()
        && m.open_error().is_some()));
}

#[gpui_kit::test]
fn a_folder_with_other_content_is_not_an_empty_folder(cx: &mut TestAppContext) {
    let (_dir, root) = empty_folder();
    fs::write(root.join("notes.txt"), "x").unwrap();
    let model = cx.update(|cx| CollectionModel::open(root, Arc::new(MemoryStore::new()), cx));
    cx.run_until_parked();
    assert!(model.read_with(cx, |m, _| !m.is_empty_folder() && m.open_error().is_some()));
}

#[gpui_kit::test]
fn the_workspace_reports_an_empty_folder_and_forgets_it(cx: &mut TestAppContext) {
    let files = tempfile::tempdir().unwrap();
    let (_dir, root) = empty_folder();
    let session = session_at(cx, &files, Session::default());
    let ws = workspace(cx, &session);
    let seen = events(cx, &ws);
    ws.update(cx, |w, cx| w.open(root.clone(), cx));
    cx.run_until_parked();
    assert!(ws.read_with(cx, |w, _| w.collections().is_empty()));
    assert_eq!(*seen.borrow(), [WorkspaceEvent::EmptyFolder { root }]);
}

#[gpui_kit::test]
fn opening_an_empty_folder_offers_to_create_and_cancel_writes_nothing(cx: &mut TestAppContext) {
    let (_files, _win, view) = shell(cx);
    let (_dir, root) = empty_folder();
    open_root(cx, &view, &root);
    let win = confirm_window(cx).expect("the offer");
    cx.update_window(win, |_, w, _| {
        for id in ["empty-folder-create", "empty-folder-cancel"] {
            assert!(w.try_find(id).is_some(), "{id} missing");
        }
    })
    .unwrap();
    answer_confirm(cx, "empty-folder-cancel");
    assert!(opened(cx, &view).is_empty());
    assert!(!root.join("opencollection.yml").exists());
}

#[gpui_kit::test]
fn create_writes_the_new_collection_file_and_opens_it(cx: &mut TestAppContext) {
    let (_files, _win, view) = shell(cx);
    let (_dir, root) = empty_folder();
    open_root(cx, &view, &root);
    answer_confirm(cx, "empty-folder-create");
    cx.run_until_parked();
    assert_eq!(opened(cx, &view), std::slice::from_ref(&root));
    // The same bytes New collection writes for a folder of this name.
    let reference = tempfile::tempdir().unwrap();
    let made = reference.path().join("shop api");
    Collection::create(&made, "shop api").unwrap();
    assert_eq!(
        fs::read(root.join("opencollection.yml")).unwrap(),
        fs::read(made.join("opencollection.yml")).unwrap()
    );
    let current = view.read_with(cx, |v, cx| v.workspace().read(cx).current_root());
    assert_eq!(current, Some(root));
}

#[gpui_kit::test]
fn enter_creates_the_collection(cx: &mut TestAppContext) {
    let (_files, win, view) = shell(cx);
    let (_dir, root) = empty_folder();
    open_root(cx, &view, &root);
    assert!(confirm_open(cx));
    cx.simulate_keystrokes(win, "enter");
    cx.run_until_parked();
    assert_eq!(opened(cx, &view), std::slice::from_ref(&root));
}

#[gpui_kit::test]
fn a_folder_with_other_content_keeps_the_plain_alert(cx: &mut TestAppContext) {
    let (_files, win, view) = shell(cx);
    let (_dir, root) = empty_folder();
    fs::write(root.join("notes.txt"), "x").unwrap();
    open_root(cx, &view, &root);
    assert!(!confirm_open(cx));
    let alert = cx
        .update_window(win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap();
    assert!(alert, "the could-not-open alert");
    assert!(!root.join("opencollection.yml").exists());
}
