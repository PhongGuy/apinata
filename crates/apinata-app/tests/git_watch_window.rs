//! A commit made outside the app redraws the real window: the title bar count and the
//! sidebar's change letters follow the repository without a save, a dialog or a focus change.

use crate::git_support::{Sandbox, write};
use crate::support::*;

use apinata_app::sidebar::marker_id;
use apinata_app::watch::set::notify_factory;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};

fn label(cx: &mut TestAppContext, s: &Started) -> Option<String> {
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        w.within("title-bar")
            .try_find("git-label")
            .and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

/// The real app over `col`, with git and the real watcher on, once the first status ran.
fn watching_app(cx: &mut TestAppContext, sb: &Sandbox, col: &tempfile::TempDir) -> Started {
    cx.executor().allow_parking();
    let s = started(cx, col);
    let options = sb.options();
    s.ws.update(cx, |w, cx| {
        w.set_git(Some(options), cx);
        w.set_watch_factory(Some(notify_factory()), cx);
    });
    let root = s.root.clone();
    wait_for(cx, "the first git status", |cx| {
        s.ws.read_with(cx, |w, _| w.git_runs(&root) > 0)
    });
    s
}

fn wait_for_label(cx: &mut TestAppContext, s: &Started, want: &str) {
    wait_for(cx, &format!("the title bar to read {want:?}"), |cx| {
        label(cx, s).as_deref() == Some(want)
    });
}

#[gpui_kit::test]
fn a_commit_made_outside_redraws_the_title_bar_and_the_rows(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    sb.git(col.path(), &["init", "-q", "-b", "main"]);
    sb.commit_all(col.path(), "init");
    write(col.path(), "a.yml", &request("a", "http://x/a"));
    sb.git(col.path(), &["add", "-A"]);
    let s = watching_app(cx, &sb, &col);
    wait_for_label(cx, &s, "main, 1 changed");
    let mark = marker_id("git", &row("a.yml"));
    assert!(shown(cx, s.win, mark.clone()));
    sb.git(col.path(), &["commit", "-q", "-m", "save"]);
    wait_for_label(cx, &s, "main");
    assert!(!shown(cx, s.win, mark));
}

#[gpui_kit::test]
fn a_first_commit_made_outside_redraws_the_title_bar(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    write(col.path(), "a.yml", &request("a", "http://x/a"));
    sb.git(col.path(), &["init", "-q", "-b", "main"]);
    sb.git(col.path(), &["add", "-A"]);
    let s = watching_app(cx, &sb, &col);
    wait_for_label(cx, &s, "main, 2 changed");
    sb.git(col.path(), &["commit", "-q", "-m", "init"]);
    wait_for_label(cx, &s, "main");
}

#[gpui_kit::test]
fn a_repository_made_after_opening_is_watched(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    write(col.path(), "a.yml", &request("a", "http://x/a"));
    let s = watching_app(cx, &sb, &col);
    assert_eq!(label(cx, &s), None);
    sb.git(col.path(), &["init", "-q", "-b", "main"]);
    sb.git(col.path(), &["add", "-A"]);
    wait_for_label(cx, &s, "main, 2 changed");
    sb.git(col.path(), &["commit", "-q", "-m", "init"]);
    wait_for_label(cx, &s, "main");
}
