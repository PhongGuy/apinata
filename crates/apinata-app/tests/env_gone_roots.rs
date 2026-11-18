//! Stored secrets of collection folders that no longer exist, in the global environments editor.

use crate::env_orphans::{Listing, global_editor_with_ws};
use crate::support;

use apinata_app::orphan_section::{GoneRoot, GoneRoots};
use apinata_core::{SecretKey, SecretStore};
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use support::*;

/// An absolute path that certainly does not exist: a temporary folder, removed again.
fn deleted_folder() -> PathBuf {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    drop(dir);
    assert!(!path.exists());
    path
}

/// Stores a value for each `(env, var)` under `root`, returning the keys.
fn seed(store: &Listing, root: &Path, vars: &[(&str, &str)]) -> Vec<SecretKey> {
    let keys: Vec<SecretKey> = vars
        .iter()
        .map(|(env, var)| SecretKey::new(root, *env, *var))
        .collect();
    for key in &keys {
        store.set(key, "s3cret").unwrap();
    }
    keys
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

/// Two gone roots, sorted by path, and the keys stored under each.
fn two_gone(store: &Listing) -> ((PathBuf, Vec<SecretKey>), (PathBuf, Vec<SecretKey>)) {
    let (mut a, mut b) = (deleted_folder(), deleted_folder());
    if b < a {
        std::mem::swap(&mut a, &mut b);
    }
    let a_keys = seed(
        store,
        &a,
        &[("dev", "token"), ("dev", "key"), ("prod", "token")],
    );
    let b_keys = seed(store, &b, &[("dev", "token")]);
    ((a, a_keys), (b, b_keys))
}

#[gpui_kit::test]
fn secrets_under_a_deleted_folder_list_as_one_root_with_their_count(cx: &mut TestAppContext) {
    let store = Arc::new(Listing::default());
    let ((a, _), (b, _)) = two_gone(&store);
    let g = "name: g\n";
    let (_c, _g, _s, win, ed, _old, _ws) = global_editor_with_ws(cx, store.clone(), g);
    let want = GoneRoots::Roots(vec![
        GoneRoot::new(a.clone(), 3),
        GoneRoot::new(b.clone(), 1),
    ]);
    assert_eq!(ed.read_with(cx, |e, _| e.gone_roots().clone()), want);
    assert!(present(cx, win, "env-gone-roots"));
    assert_eq!(
        label(cx, win, ("env-gone-root-path", 0usize)),
        a.to_string_lossy()
    );
    assert_eq!(
        label(cx, win, ("env-gone-root-count", 0usize)),
        "3 stored secrets"
    );
    assert_eq!(
        label(cx, win, ("env-gone-root-count", 1usize)),
        "1 stored secret"
    );
    let hint = label(cx, win, "env-gone-roots-hint");
    assert!(hint.contains("not coming back"), "{hint}");
}

#[gpui_kit::test]
fn forget_asks_naming_the_path_and_count_and_cancel_keeps_them(cx: &mut TestAppContext) {
    let store = Arc::new(Listing::default());
    let ((a, a_keys), _) = two_gone(&store);
    let (_c, _g, _s, win, ed, _old, _ws) = global_editor_with_ws(cx, store.clone(), "name: g\n");
    let before = ed.read_with(cx, |e, _| e.gone_roots().clone());
    click(cx, win, ("env-gone-root-forget", 0usize));
    let prompt = confirm_window(cx).expect("a confirm dialog is open");
    let title = label(cx, prompt, "confirm-dialog");
    assert!(title.contains(&*a.to_string_lossy()), "{title}");
    assert!(title.contains("3 stored secrets"), "{title}");
    answer_confirm(cx, "confirm-cancel");
    cx.run_until_parked();
    for key in &a_keys {
        assert!(store.get(key).unwrap().is_some());
    }
    assert_eq!(ed.read_with(cx, |e, _| e.gone_roots().clone()), before);
}

#[gpui_kit::test]
fn forget_removes_that_roots_secrets_and_leaves_the_rest(cx: &mut TestAppContext) {
    let store = Arc::new(Listing::default());
    let ((_a, a_keys), (b, b_keys)) = two_gone(&store);
    let (_c, _g, _s, win, ed, old, _ws) = global_editor_with_ws(cx, store.clone(), "name: g\n");
    click(cx, win, ("env-gone-root-forget", 0usize));
    answer_confirm(cx, "confirm-delete");
    cx.run_until_parked();
    for key in &a_keys {
        assert_eq!(store.get(key).unwrap(), None, "{key:?}");
    }
    assert!(
        store.get(&b_keys[0]).unwrap().is_some(),
        "another gone root"
    );
    assert!(store.get(&old).unwrap().is_some(), "a live root's secret");
    let want = GoneRoots::Roots(vec![GoneRoot::new(b, 1)]);
    assert_eq!(ed.read_with(cx, |e, _| e.gone_roots().clone()), want);
}

#[gpui_kit::test]
fn a_folder_that_comes_back_during_the_prompt_keeps_its_secrets(cx: &mut TestAppContext) {
    let store = Arc::new(Listing::default());
    let ((a, a_keys), _) = two_gone(&store);
    let (_c, _g, _s, win, ed, _old, _ws) = global_editor_with_ws(cx, store.clone(), "name: g\n");
    click(cx, win, ("env-gone-root-forget", 0usize));
    std::fs::create_dir(&a).unwrap();
    answer_confirm(cx, "confirm-delete");
    cx.run_until_parked();
    std::fs::remove_dir(&a).unwrap();
    for key in &a_keys {
        assert!(store.get(key).unwrap().is_some(), "{key:?}");
    }
    let error = label(cx, win, "global-env-error");
    assert!(error.contains("exists"), "{error}");
    assert!(ed.read_with(cx, |e, _| e.gone_roots().clone()) != GoneRoots::Unknown);
}

#[gpui_kit::test]
fn with_no_gone_roots_the_group_is_absent(cx: &mut TestAppContext) {
    let store = Arc::new(Listing::default());
    let (_c, _g, _s, win, ed, _old, _ws) = global_editor_with_ws(cx, store.clone(), "name: g\n");
    assert_eq!(
        ed.read_with(cx, |e, _| e.gone_roots().clone()),
        GoneRoots::Roots(Vec::new())
    );
    assert!(!present(cx, win, "env-gone-roots"));
}
