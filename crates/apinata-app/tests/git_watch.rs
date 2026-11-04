//! A commit, stage or checkout made outside the app refreshes the collection's git status
//! through the watch on the git directory, without a save, a dialog or a focus change.

use crate::git_support;
use crate::support;

use std::path::Path;

use apinata_app::git_status::GitChange;
use apinata_app::session::Session;
use apinata_app::watch::git;
use apinata_app::watch::set::notify_factory;
use apinata_app::workspace::Workspace;
use git_support::{Sandbox, write};
use gpui_kit::{Entity, TestAppContext};
use support::*;

fn committed(sb: &Sandbox, name: &str) -> std::path::PathBuf {
    let repo = sb.repo(name);
    write(&repo, "opencollection.yml", ROOT);
    write(&repo, "a.yml", &request("a", "http://x/a"));
    sb.commit_all(&repo, "init");
    repo
}

/// A workspace with git and the real watcher on, `root` opened and both running.
fn watching_git(
    cx: &mut TestAppContext,
    sb: &Sandbox,
    root: &Path,
) -> (tempfile::TempDir, Entity<Workspace>) {
    cx.executor().allow_parking();
    init(cx);
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, Session::default());
    let ws = workspace(cx, &session);
    let options = sb.options();
    ws.update(cx, |w, cx| {
        w.set_git(Some(options), cx);
        w.set_watch_factory(Some(notify_factory()), cx);
    });
    ws.update(cx, |w, cx| w.open(root.to_path_buf(), cx));
    wait_for(cx, "git status and watching", |cx| {
        ws.read_with(cx, |w, _| w.git_changes(root).is_some()) && ws_snap(cx, &ws, root).seq >= 2
    });
    (state, ws)
}

fn files(cx: &mut TestAppContext, ws: &Entity<Workspace>, root: &Path) -> Vec<(String, GitChange)> {
    ws.read_with(cx, |w, _| w.git_changes(root))
        .map(|g| {
            g.files
                .iter()
                .map(|(p, c)| (p.to_string_lossy().into_owned(), c.clone()))
                .collect()
        })
        .unwrap_or_default()
}

fn branch(cx: &mut TestAppContext, ws: &Entity<Workspace>, root: &Path) -> Option<String> {
    ws.read_with(cx, |w, _| w.git_changes(root))
        .and_then(|g| g.branch.clone())
}

#[gpui_kit::test]
fn a_commit_made_outside_clears_the_changes(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let repo = committed(&sb, "r");
    write(&repo, "a.yml", &request("a", "http://x/changed"));
    let (_state, ws) = watching_git(cx, &sb, &repo);
    assert_eq!(files(cx, &ws, &repo).len(), 1);
    sb.commit_all(&repo, "save");
    wait_for(cx, "the commit to clear the changes", |cx| {
        files(cx, &ws, &repo).is_empty()
    });
}

#[gpui_kit::test]
fn a_stage_made_outside_shows_in_the_status(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let repo = committed(&sb, "r");
    let (_state, ws) = watching_git(cx, &sb, &repo);
    // Tracked, so only the index changes: no file in the collection is touched.
    sb.git(&repo, &["rm", "-q", "--cached", "a.yml"]);
    wait_for(cx, "the stage to show", |cx| {
        files(cx, &ws, &repo).iter().any(|(p, _)| p == "a.yml")
    });
}

#[gpui_kit::test]
fn a_checkout_made_outside_changes_the_branch(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let repo = committed(&sb, "r");
    let (_state, ws) = watching_git(cx, &sb, &repo);
    assert_eq!(branch(cx, &ws, &repo).as_deref(), Some("main"));
    sb.git(&repo, &["checkout", "-q", "-b", "other"]);
    wait_for(cx, "the new branch", |cx| {
        branch(cx, &ws, &repo).as_deref() == Some("other")
    });
}

#[gpui_kit::test]
fn a_linked_worktree_refreshes_from_its_own_git_dir(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let main = committed(&sb, "r");
    let wt = sb.path().join("wt");
    sb.git(
        &main,
        &["worktree", "add", "-q", "-b", "wt", wt.to_str().unwrap()],
    );
    assert!(wt.join(".git").is_file());
    write(&wt, "a.yml", &request("a", "http://x/changed"));
    let (_state, ws) = watching_git(cx, &sb, &wt);
    assert_eq!(files(cx, &ws, &wt).len(), 1);
    sb.commit_all(&wt, "save");
    wait_for(cx, "the commit to clear the changes", |cx| {
        files(cx, &ws, &wt).is_empty()
    });
}

#[test]
fn the_watched_dirs_are_the_git_dir_and_the_heads() {
    let sb = Sandbox::new();
    let repo = committed(&sb, "r");
    let dirs = git::dirs(&repo);
    assert!(dirs.contains(&repo.join(".git")), "{dirs:?}");
    assert!(dirs.contains(&repo.join(".git/refs/heads")), "{dirs:?}");
    // Never the object store, which a commit fills with files.
    assert!(dirs.iter().all(|d| !d.ends_with("objects")), "{dirs:?}");
}

#[test]
fn a_worktree_watches_its_git_dir_and_the_shared_heads() {
    let sb = Sandbox::new();
    let main = committed(&sb, "r");
    let wt = sb.path().join("wt");
    sb.git(
        &main,
        &["worktree", "add", "-q", "-b", "wt", wt.to_str().unwrap()],
    );
    let dirs = git::dirs(&wt);
    assert!(dirs.contains(&main.join(".git/worktrees/wt")), "{dirs:?}");
    assert!(dirs.contains(&main.join(".git/refs/heads")), "{dirs:?}");
}

#[test]
fn only_head_index_and_branch_files_matter() {
    let rel = |p: &str| git::relevant(Path::new(p));
    assert!(rel("/r/.git/HEAD"));
    assert!(rel("/r/.git/index"));
    assert!(rel("/r/.git/packed-refs"));
    assert!(rel("/r/.git/refs/heads/main"));
    assert!(!rel("/r/.git/index.lock"));
    assert!(!rel("/r/.git/refs/heads/main.lock"));
    assert!(!rel("/r/.git/FETCH_HEAD"));
    assert!(!rel("/r/.git/objects/ab"));
    assert!(!rel("/r/.git/refs/tags/v1"));
}
