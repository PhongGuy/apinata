//! The git branch label: HEAD parsing, the nearest repository, and the title bar label
//! that follows HEAD across window activations.

use crate::support;

use std::fs;

use apinata_app::git_head::{branch_label, git_dir_for, head_label};
use apinata_app::session::Session;
use apinata_app::shell::Shell;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext, WindowOptions};
use support::*;

fn repo(head: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join(".git/HEAD"), head).unwrap();
    dir
}

#[test]
fn a_branch_ref_shows_its_name() {
    assert_eq!(
        branch_label(repo("ref: refs/heads/main\n").path()).as_deref(),
        Some("main")
    );
    assert_eq!(
        branch_label(repo("ref: refs/heads/feature/x\n").path()).as_deref(),
        Some("feature/x")
    );
}

#[test]
fn a_detached_head_shows_seven_characters() {
    let r = repo("0123456789abcdef0123456789abcdef01234567\n");
    assert_eq!(branch_label(r.path()).as_deref(), Some("0123456"));
}

#[test]
fn a_reftable_repository_shows_nothing() {
    assert_eq!(
        branch_label(repo("ref: refs/heads/.invalid\n").path()),
        None
    );
}

#[test]
fn the_nearest_git_dir_above_the_collection_counts() {
    let r = repo("ref: refs/heads/main\n");
    let col = r.path().join("apis/users");
    fs::create_dir_all(&col).unwrap();
    assert_eq!(git_dir_for(&col), Some(r.path().join(".git")));
    assert_eq!(branch_label(&col).as_deref(), Some("main"));
}

#[test]
fn a_git_file_resolves_gitdir_relative_to_itself() {
    let main = repo("ref: refs/heads/main\n");
    let wt = main.path().join(".git/worktrees/wt");
    fs::create_dir_all(&wt).unwrap();
    fs::write(wt.join("HEAD"), "ref: refs/heads/topic\n").unwrap();
    let checkout = main.path().join("checkout");
    fs::create_dir(&checkout).unwrap();
    fs::write(checkout.join(".git"), "gitdir: ../.git/worktrees/wt\n").unwrap();
    assert_eq!(branch_label(&checkout).as_deref(), Some("topic"));
}

#[test]
fn an_absolute_gitdir_is_used_as_is() {
    let main = repo("ref: refs/heads/main\n");
    let elsewhere = tempfile::tempdir().unwrap();
    let line = format!("gitdir: {}\n", main.path().join(".git").display());
    fs::write(elsewhere.path().join(".git"), line).unwrap();
    assert_eq!(
        git_dir_for(elsewhere.path()),
        Some(main.path().join(".git"))
    );
    assert_eq!(branch_label(elsewhere.path()).as_deref(), Some("main"));
}

#[test]
fn odd_or_unreadable_heads_show_nothing() {
    assert_eq!(branch_label(repo("garbage\n").path()), None);
    assert_eq!(
        branch_label(repo("ref: refs/remotes/origin/main\n").path()),
        None
    );
    let dangling = tempfile::tempdir().unwrap();
    fs::write(dangling.path().join(".git"), "gitdir: missing\n").unwrap();
    assert_eq!(branch_label(dangling.path()), None);
    assert_eq!(head_label(&dangling.path().join("nowhere")), None);
}

#[test]
fn a_directory_outside_any_repository_finds_no_git_dir_of_its_own() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(git_dir_for(dir.path()), git_dir_for(&std::env::temp_dir()));
}

#[cfg(unix)]
#[test]
fn an_unreadable_git_entry_ends_the_search() {
    use std::os::unix::fs::PermissionsExt as _;
    let outer = repo("ref: refs/heads/outer\n");
    let inner = outer.path().join("inner");
    fs::create_dir_all(inner.join(".git")).unwrap();
    let _g = crate::support::ModeGuard::set(&inner, 0o000);
    // Root ignores permissions; only assert when the stat really fails.
    let denied = fs::symlink_metadata(inner.join(".git")).is_err();
    let found = (git_dir_for(&inner), branch_label(&inner));
    fs::set_permissions(&inner, fs::Permissions::from_mode(0o755)).unwrap();
    if denied {
        assert_eq!(found, (None, None));
    }
}

#[gpui_kit::test]
fn the_title_bar_follows_head_across_activations(cx: &mut TestAppContext) {
    support::init(cx);
    let r = repo("ref: refs/heads/main\n");
    let col = r.path().join("col");
    fs::create_dir(&col).unwrap();
    fs::write(col.join("opencollection.yml"), ROOT).unwrap();
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, Session::default());
    let ws = workspace(cx, &session);
    ws.update(cx, |w, cx| w.open(col.clone(), cx));
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| Shell::new(ws, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    assert_eq!(
        view.read_with(cx, |v, _| v.branch().cloned()).as_deref(),
        Some("main")
    );
    assert!(
        cx.update_window(win, |_, w, _| w
            .within("title-bar")
            .try_find("git-branch")
            .is_some())
            .unwrap()
    );
    fs::write(r.path().join(".git/HEAD"), "ref: refs/heads/dev\n").unwrap();
    let mut vcx = VisualTestContext::from_window(win, cx);
    vcx.deactivate_window();
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, win);
    assert_eq!(
        view.read_with(cx, |v, _| v.branch().cloned()).as_deref(),
        Some("dev")
    );
}
