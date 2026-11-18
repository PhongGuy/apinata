//! A secret rename that reaches a collection outside team mode: an outside commit or pull, a
//! branch switch made in the app, and the Check for renamed secrets command.

use crate::git_support::{Sandbox, write};
use crate::support::*;

use std::path::Path;
use std::sync::Arc;

use apinata_app::renames_dialog::{self, CheckSecretRenames};
use apinata_app::watch::set::notify_factory;
use apinata_core::{Collection, MemoryStore, SecretStore};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};

const DEV: &str = "name: dev\nvariables:\n  - name: token\n    secret: true\n";
const DEV_RENAMED: &str = "name: dev\nvariables:\n  - name: api_token\n    secret: true\n";
// A variable renamed inside one environment is a likely rename, never a sure one.
const ROW: &str = "dev / token → dev / api_token (renamed? check before moving)";

// A committed collection on `main` whose dev environment has the secret `token`.
fn repo(sb: &Sandbox) -> tempfile::TempDir {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/a")),
        ("environments/dev.yml", DEV),
    ]);
    sb.git(col.path(), &["init", "-q", "-b", "main"]);
    sb.commit_all(col.path(), "base");
    col
}

fn head(sb: &Sandbox, col: &Path) -> String {
    sb.git(col, &["rev-parse", "HEAD"]).trim().to_owned()
}

fn key(col: &Path, var: &str) -> apinata_core::SecretKey {
    Collection::open(col).unwrap().secret_key("dev", var)
}

// The real app over `col` with a value stored for dev/token and git on, once it has seen HEAD;
// `watch` turns the file watcher on, which is how an outside commit is noticed.
fn app(
    cx: &mut TestAppContext,
    sb: &Sandbox,
    col: &tempfile::TempDir,
    watch: bool,
) -> (Started, Arc<MemoryStore>) {
    cx.executor().allow_parking();
    let store = Arc::new(MemoryStore::new());
    store.set(&key(col.path(), "token"), "s3cret").unwrap();
    let s = started_with_store(cx, col, store.clone());
    s.ws.update(cx, |w, cx| {
        w.set_git(Some(sb.options()), cx);
        if watch {
            w.set_watch_factory(Some(notify_factory()), cx);
        }
    });
    let at = head(sb, col.path());
    wait_for(cx, "the first HEAD", |cx| seen(cx, &s) == Some(at.clone()));
    (s, store)
}

fn seen(cx: &mut TestAppContext, s: &Started) -> Option<String> {
    s.ws.read_with(cx, |w, _| w.seen_head(&s.root))
}

fn checks(cx: &mut TestAppContext, s: &Started) -> u64 {
    s.ws.read_with(cx, |w, _| w.pull_checks(&s.root))
}

fn open_rows(cx: &mut TestAppContext, s: &Started) -> usize {
    s.ws.read_with(cx, |w, _| w.secret_renames(&s.root))
        .map_or(0, |r| r.open_rows().len())
}

// Waits until the app has looked at the commit `at` and finished any rename check it brought.
fn wait_checked(cx: &mut TestAppContext, s: &Started, at: &str, before: u64) {
    wait_for(cx, "the pull check", |cx| {
        seen(cx, s).as_deref() == Some(at) && checks(cx, s) > before
    });
}

fn toasts(cx: &mut TestAppContext, s: &Started) -> usize {
    cx.update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

fn dialog_lines(cx: &mut TestAppContext) -> Option<Vec<String>> {
    let view = cx.update(|cx| renames_dialog::current(cx))?;
    Some(view.read_with(cx, |v, cx| v.rename_lines(cx)))
}

fn rename_outside(sb: &Sandbox, col: &Path) -> String {
    write(col, "environments/dev.yml", DEV_RENAMED);
    sb.commit_all(col, "rename token");
    head(sb, col)
}

#[gpui_kit::test]
fn an_outside_commit_renaming_a_secret_offers_the_move_and_move_keeps_the_value(
    cx: &mut TestAppContext,
) {
    let sb = Sandbox::new();
    let col = repo(&sb);
    let (s, store) = app(cx, &sb, &col, true);
    rename_outside(&sb, col.path());
    wait_for(cx, "the rename row", |cx| open_rows(cx, &s) == 1);
    assert!(toasts(cx, &s) >= 1, "the offer is announced");
    let root = s.root.clone();
    cx.update_window(s.win, |_, w, cx| renames_dialog::open(&s.ws, root, w, cx))
        .unwrap();
    frame(cx, s.win);
    assert_eq!(dialog_lines(cx), Some(vec![ROW.to_owned()]));
    cx.update_window(s.win, |_, w, cx| w.click(("team-rename-move", 0usize), cx))
        .unwrap();
    let (old, new) = (key(col.path(), "token"), key(col.path(), "api_token"));
    wait_for(cx, "the move", |_| {
        store.get(&new).unwrap().as_deref() == Some("s3cret")
    });
    assert_eq!(store.get(&old).unwrap(), None);
    wait_for(cx, "the row to go", |cx| open_rows(cx, &s) == 0);
}

#[gpui_kit::test]
fn an_outside_commit_without_a_rename_offers_nothing(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = repo(&sb);
    let (s, _store) = app(cx, &sb, &col, true);
    let before = checks(cx, &s);
    write(col.path(), "a.yml", &request("a", "http://x/b"));
    sb.commit_all(col.path(), "edit a");
    let at = head(&sb, col.path());
    wait_checked(cx, &s, &at, before);
    assert_eq!(open_rows(cx, &s), 0);
    assert_eq!(toasts(cx, &s), 0);
}

#[gpui_kit::test]
fn a_branch_switch_in_the_app_offers_nothing(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = repo(&sb);
    // `feature` is ahead of `main` by the rename, so only the switch's origin tells it apart.
    sb.git(col.path(), &["switch", "-q", "-c", "feature"]);
    let at = rename_outside(&sb, col.path());
    sb.git(col.path(), &["switch", "-q", "main"]);
    let (s, _store) = app(cx, &sb, &col, true);
    let before = checks(cx, &s);
    crate::branch_shell::open_picker(cx, &s);
    let picker = cx
        .update(|cx| apinata_app::branch_dialog::current(cx))
        .unwrap();
    let branch = picker.read_with(cx, |p, _| {
        (p.branches().iter())
            .find(|b| b.local.as_deref() == Some("feature"))
            .cloned()
            .unwrap()
    });
    picker.update(cx, |p, cx| p.switch(branch, cx));
    wait_for(cx, "the switch to end", |cx| {
        !s.ws.read_with(cx, |w, _| w.is_switching(&s.root))
    });
    wait_checked(cx, &s, &at, before);
    // The watcher may report the switch after it ended; any check that brings must still be
    // quiet, so give it time to land.
    for _ in 0..10 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        cx.run_until_parked();
    }
    assert_eq!(open_rows(cx, &s), 0);
}

#[gpui_kit::test]
fn the_check_command_finds_the_rename_since_orig_head(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = repo(&sb);
    let base = head(&sb, col.path());
    rename_outside(&sb, col.path());
    sb.git(col.path(), &["update-ref", "ORIG_HEAD", &base]);
    let (s, _store) = app(cx, &sb, &col, false);
    assert_eq!(open_rows(cx, &s), 0, "opening alone offers nothing");
    cx.update_window(s.win, |_, w, cx| {
        w.dispatch_action(Box::new(CheckSecretRenames), cx)
    })
    .unwrap();
    wait_for(cx, "the rename dialog", |cx| {
        dialog_lines(cx).is_some_and(|l| !l.is_empty())
    });
    assert_eq!(dialog_lines(cx), Some(vec![ROW.to_owned()]));
}

#[gpui_kit::test]
fn the_check_command_says_so_when_nothing_was_renamed(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = repo(&sb);
    let (s, _store) = app(cx, &sb, &col, false);
    cx.update_window(s.win, |_, w, cx| {
        w.dispatch_action(Box::new(CheckSecretRenames), cx)
    })
    .unwrap();
    wait_for(cx, "the answer", |cx| toasts(cx, &s) == 1);
    assert!(
        !cx.update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
            .unwrap()
    );
}
