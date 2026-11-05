//! Stored secrets of environments with no file, in both environment editors.

use crate::env_orphans::{Listing, global_editor_with_ws};
use crate::support;

use apinata_app::orphan_section::{Fileless, FilelessGroup};
use apinata_core::{Collection, GlobalEnvironments, SecretKey, SecretStore};
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use support::*;

const DEV: &str = "name: dev\nvariables:\n  - name: host\n    value: x\n";

fn group(env: &str, vars: &[&str]) -> FilelessGroup {
    FilelessGroup::new(env, vars.iter().map(|v| v.to_string()).collect())
}

/// A store holding `gone`'s `a` and `b` and `lost`'s `c` under the collection at `root`.
fn seeded(root: &Path) -> (Arc<Listing>, Vec<SecretKey>) {
    let c = Collection::open(root).unwrap();
    let store = Arc::new(Listing::default());
    let keys = [("gone", "a"), ("gone", "b"), ("lost", "c")]
        .map(|(env, var)| c.secret_key(env, var))
        .to_vec();
    for key in &keys {
        store.set(key, "s3cret").unwrap();
    }
    (store, keys)
}

fn label(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) -> String {
    use gpui_kit::test::TestWindowExt as _;
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).label().map(str::to_owned).unwrap_or_default()
    })
    .unwrap()
}

fn click(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<gpui_kit::ElementId>) {
    use gpui_kit::test::TestWindowExt as _;
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(id, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

/// The open prompt's title and detail.
fn prompt(cx: &mut TestAppContext) -> (String, String) {
    let win = confirm_window(cx).expect("a confirm dialog is open");
    (
        label(cx, win, "confirm-dialog"),
        label(cx, win, "confirm-detail"),
    )
}

fn assert_warns(title: &str, detail: &str) {
    assert!(
        title.contains("\"a\"") && title.contains("\"gone\""),
        "{title}"
    );
    assert!(detail.contains("may only be elsewhere"), "{detail}");
    assert!(detail.contains("not mounted"), "{detail}");
    assert!(detail.contains("for good"), "{detail}");
}

fn shown_groups() -> Fileless {
    Fileless::Groups(vec![group("gone", &["a", "b"]), group("lost", &["c"])])
}

#[gpui_kit::test]
fn fileless_secrets_show_under_their_environment(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", DEV)]);
    let (store, _keys) = seeded(col.path());
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    assert_eq!(
        ed.read_with(cx, |e, _| e.fileless().clone()),
        shown_groups()
    );
    assert!(present(cx, win, "env-fileless"));
    assert_eq!(label(cx, win, ("env-fileless-env", 0usize)), "gone");
    assert_eq!(label(cx, win, ("env-fileless-env", 1usize)), "lost");
    assert!(present(cx, win, ("env-fileless-remove", 2usize)));
    assert_eq!(
        store.lists.load(Ordering::SeqCst),
        1,
        "one list answers both sections"
    );
}

#[gpui_kit::test]
fn remove_warns_and_cancel_keeps_the_value(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", DEV)]);
    let (store, keys) = seeded(col.path());
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    click(cx, win, ("env-fileless-remove", 0usize));
    let (title, detail) = prompt(cx);
    assert_warns(&title, &detail);
    answer_confirm(cx, "confirm-cancel");
    cx.run_until_parked();
    assert!(store.get(&keys[0]).unwrap().is_some());
    assert_eq!(
        ed.read_with(cx, |e, _| e.fileless().clone()),
        shown_groups()
    );
}

#[gpui_kit::test]
fn confirming_removes_the_value_and_its_row(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", DEV)]);
    let (store, keys) = seeded(col.path());
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    click(cx, win, ("env-fileless-remove", 0usize));
    answer_confirm(cx, "confirm-delete");
    cx.run_until_parked();
    assert_eq!(store.get(&keys[0]).unwrap(), None);
    assert!(
        store.get(&keys[1]).unwrap().is_some(),
        "only the one asked about goes"
    );
    let want = Fileless::Groups(vec![group("gone", &["b"]), group("lost", &["c"])]);
    assert_eq!(ed.read_with(cx, |e, _| e.fileless().clone()), want);
}

#[gpui_kit::test]
fn the_empty_editor_shows_them_too(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let (store, keys) = seeded(col.path());
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    assert!(present(cx, win, "env-empty"));
    assert_eq!(
        ed.read_with(cx, |e, _| e.fileless().clone()),
        shown_groups()
    );
    click(cx, win, ("env-fileless-remove", 2usize));
    answer_confirm(cx, "confirm-delete");
    cx.run_until_parked();
    assert_eq!(store.get(&keys[2]).unwrap(), None);
    let want = Fileless::Groups(vec![group("gone", &["a", "b"])]);
    assert_eq!(ed.read_with(cx, |e, _| e.fileless().clone()), want);
}

#[gpui_kit::test]
fn a_global_fileless_secret_shows_warns_and_goes(cx: &mut TestAppContext) {
    let store = Arc::new(Listing::default());
    let g = "name: g\nvariables:\n  - name: host\n    value: a\n";
    let (_c, globals, _s, win, ed, _old, _ws) = global_editor_with_ws(cx, store.clone(), g);
    let gl = GlobalEnvironments::open(globals.path()).unwrap();
    let keys: Vec<SecretKey> = [("gone", "a"), ("gone", "b"), ("lost", "c")]
        .iter()
        .map(|(env, var)| gl.secret_key(env, var))
        .collect();
    for key in &keys {
        store.set(key, "s3cret").unwrap();
    }
    // A person's selection lists again.
    cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.choose("h", w, cx)))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        ed.read_with(cx, |e, _| e.fileless().clone()),
        shown_groups()
    );
    assert_eq!(label(cx, win, ("env-fileless-env", 0usize)), "gone");

    click(cx, win, ("env-fileless-remove", 0usize));
    let (title, detail) = prompt(cx);
    assert_warns(&title, &detail);
    answer_confirm(cx, "confirm-cancel");
    cx.run_until_parked();
    assert!(store.get(&keys[0]).unwrap().is_some());

    click(cx, win, ("env-fileless-remove", 0usize));
    answer_confirm(cx, "confirm-delete");
    cx.run_until_parked();
    assert_eq!(store.get(&keys[0]).unwrap(), None);
    let want = Fileless::Groups(vec![group("gone", &["b"]), group("lost", &["c"])]);
    assert_eq!(ed.read_with(cx, |e, _| e.fileless().clone()), want);
}

#[gpui_kit::test]
fn a_file_that_comes_back_during_the_prompt_keeps_the_value(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", DEV)]);
    let (store, keys) = seeded(col.path());
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    assert!(present(cx, win, ("env-fileless-row", 0usize)));
    click(cx, win, ("env-fileless-remove", 0usize));
    std::fs::write(col.path().join("environments/gone.yml"), "name: gone\n").unwrap();
    answer_confirm(cx, "confirm-delete");
    cx.run_until_parked();
    assert!(store.get(&keys[0]).unwrap().is_some(), "the value is kept");
    let error = ed.read_with(cx, |e, _| e.error()).unwrap_or_default();
    assert!(error.contains("has a file"), "{error}");
    assert!(present(cx, win, "env-error"));
}

#[gpui_kit::test]
fn a_global_file_that_comes_back_during_the_prompt_keeps_the_value(cx: &mut TestAppContext) {
    let store = Arc::new(Listing::default());
    let g = "name: g\nvariables:\n  - name: host\n    value: a\n";
    let (_c, globals, _s, win, ed, _old, ws) = global_editor_with_ws(cx, store.clone(), g);
    let key = GlobalEnvironments::open(globals.path())
        .unwrap()
        .secret_key("gone", "a");
    store.set(&key, "s3cret").unwrap();
    cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.choose("h", w, cx)))
        .unwrap();
    cx.run_until_parked();
    assert!(present(cx, win, ("env-fileless-row", 0usize)));
    click(cx, win, ("env-fileless-remove", 0usize));
    std::fs::write(globals.path().join("gone.yml"), "name: gone\n").unwrap();
    answer_confirm(cx, "confirm-delete");
    cx.run_until_parked();
    assert!(store.get(&key).unwrap().is_some(), "the value is kept");
    let error = ws.read_with(cx, |w, _| w.global_error().map(str::to_owned));
    let error = error.unwrap_or_default();
    assert!(error.contains("has a file"), "{error}");
    assert!(present(cx, win, "global-env-error"));
}

#[gpui_kit::test]
fn the_empty_editor_lists_the_store_once(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let (store, _keys) = seeded(col.path());
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (_win, _ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    assert_eq!(store.lists.load(Ordering::SeqCst), 1);
}

#[gpui_kit::test]
fn a_global_selection_lists_the_store_once(cx: &mut TestAppContext) {
    let store = Arc::new(Listing::default());
    let g = "name: g\n";
    let (_c, _g, _s, win, ed, _old, _ws) = global_editor_with_ws(cx, store.clone(), g);
    let before = store.lists.load(Ordering::SeqCst);
    cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.choose("h", w, cx)))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(store.lists.load(Ordering::SeqCst), before + 1);
}
