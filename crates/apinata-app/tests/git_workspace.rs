//! The workspace's git status per collection: off the UI thread, refreshed on open, own writes,
//! watcher batches and request, coalesced, and quiet without git or a repository.

use crate::git_support;
use crate::support;

use std::path::Path;

use apinata_app::command::{Command, Target};
use apinata_app::git_status::{GitChange, GitStage, RefreshGate};
use apinata_app::session::Session;
use apinata_app::workspace::Workspace;
use apinata_core::GitOptions;
use git_support::{Sandbox, write};
use gpui_kit::{Entity, TestAppContext};
use support::*;

/// `files` in a fresh temp directory, made a repository on `main` and committed.
fn committed(sb: &Sandbox, files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = write_tree(files);
    sb.git(dir.path(), &["init", "-q", "-b", "main"]);
    sb.commit_all(dir.path(), "init");
    dir
}

fn two(sb: &Sandbox) -> tempfile::TempDir {
    committed(
        sb,
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &request("a", "http://x/a")),
            ("b.yml", &request("b", "http://x/b")),
        ],
    )
}

/// A workspace with git set to `options`, `root` opened and settled.
fn opened(
    cx: &mut TestAppContext,
    options: Option<GitOptions>,
    root: &Path,
) -> (tempfile::TempDir, Entity<Workspace>) {
    init(cx);
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, Session::default());
    let ws = workspace(cx, &session);
    ws.update(cx, |w, cx| w.set_git(options, cx));
    ws.update(cx, |w, cx| w.open(root.to_path_buf(), cx));
    cx.run_until_parked();
    (state, ws)
}

fn changes(
    cx: &mut TestAppContext,
    ws: &Entity<Workspace>,
    root: &Path,
) -> Option<Vec<(String, GitChange)>> {
    ws.read_with(cx, |w, _| w.git_changes(root)).map(|g| {
        g.files
            .iter()
            .map(|(p, c)| (p.to_string_lossy().into_owned(), c.clone()))
            .collect()
    })
}

fn runs(cx: &mut TestAppContext, ws: &Entity<Workspace>, root: &Path) -> u64 {
    ws.read_with(cx, |w, _| w.git_runs(root))
}

#[test]
fn the_gate_runs_one_at_a_time_and_remembers_one_more() {
    let mut g = RefreshGate::default();
    assert!(g.request());
    assert!(!g.request());
    assert!(!g.request());
    assert!(g.finish());
    assert!(!g.finish());
    assert!(g.request());
}

#[gpui_kit::test]
fn an_open_repository_collection_gets_its_changes(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = two(&sb);
    write(col.path(), "a.yml", &request("a", "http://x/changed"));
    write(col.path(), "new.yml", &request("n", "http://x/n"));
    let (_state, ws) = opened(cx, Some(sb.options()), col.path());
    assert_eq!(
        changes(cx, &ws, col.path()),
        Some(vec![
            ("a.yml".to_owned(), GitChange::Modified),
            ("new.yml".to_owned(), GitChange::Untracked),
        ])
    );
}

#[gpui_kit::test]
fn without_git_options_nothing_runs(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = two(&sb);
    write(col.path(), "a.yml", &request("a", "http://x/changed"));
    let (_state, ws) = opened(cx, None, col.path());
    assert_eq!(changes(cx, &ws, col.path()), None);
    assert_eq!(runs(cx, &ws, col.path()), 0);
}

#[gpui_kit::test]
fn outside_a_repository_is_quiet(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let root = sb.path().join("plain");
    write(&root, "opencollection.yml", ROOT);
    write(&root, "a.yml", &request("a", "http://x/a"));
    let (_state, ws) = opened(cx, Some(sb.options()), &root);
    assert_eq!(changes(cx, &ws, &root), None);
    assert_eq!(runs(cx, &ws, &root), 1);
}

#[gpui_kit::test]
fn missing_git_is_quiet(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = two(&sb);
    write(col.path(), "a.yml", &request("a", "http://x/changed"));
    let options = sb.options().env("PATH", sb.empty_bin());
    let (_state, ws) = opened(cx, Some(options), col.path());
    assert_eq!(changes(cx, &ws, col.path()), None);
}

#[gpui_kit::test]
fn a_save_refreshes_the_changes(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = two(&sb);
    let (_state, ws) = opened(cx, Some(sb.options()), col.path());
    assert_eq!(changes(cx, &ws, col.path()), Some(vec![]));
    ws.update(cx, |w, cx| {
        w.send(col.path(), edit_url("a.yml", "http://x/saved"), cx);
        w.send(col.path(), Command::Save(Target::Request(ip("a.yml"))), cx);
    });
    cx.run_until_parked();
    assert_eq!(
        changes(cx, &ws, col.path()),
        Some(vec![("a.yml".to_owned(), GitChange::Modified)])
    );
}

#[gpui_kit::test]
fn an_outside_edit_refreshes_through_the_watcher(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = two(&sb);
    let w = watched(cx, &col);
    w.ws.update(cx, |ws, cx| ws.set_git(Some(sb.options()), cx));
    cx.run_until_parked();
    assert_eq!(changes(cx, &w.ws, &w.root), Some(vec![]));
    std::fs::write(col.path().join("b.yml"), request("b", "http://x/outside")).unwrap();
    wait_for(cx, "the watcher's refresh", |cx| {
        changes(cx, &w.ws, &w.root) == Some(vec![("b.yml".to_owned(), GitChange::Modified)])
    });
}

#[gpui_kit::test]
fn requests_during_a_run_coalesce_into_one_more(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = two(&sb);
    let (_state, ws) = opened(cx, Some(sb.options()), col.path());
    let before = runs(cx, &ws, col.path());
    ws.update(cx, |w, cx| {
        w.refresh_git(col.path(), cx);
        w.refresh_git(col.path(), cx);
        w.refresh_git(col.path(), cx);
    });
    cx.run_until_parked();
    assert_eq!(runs(cx, &ws, col.path()), before + 2);
}

#[gpui_kit::test]
fn closing_during_a_run_drops_the_result(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = two(&sb);
    write(col.path(), "a.yml", &request("a", "http://x/changed"));
    let (_state, ws) = opened(cx, Some(sb.options()), col.path());
    ws.update(cx, |w, cx| {
        w.refresh_git(col.path(), cx);
        w.close(col.path(), cx);
    });
    cx.run_until_parked();
    assert_eq!(changes(cx, &ws, col.path()), None);
}

#[gpui_kit::test]
fn a_reopened_collection_ignores_the_run_of_its_closed_entry(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = two(&sb);
    write(col.path(), "a.yml", &request("a", "http://x/changed"));
    let (_state, ws) = opened(cx, Some(sb.options()), col.path());
    ws.update(cx, |w, cx| {
        w.refresh_git(col.path(), cx);
        w.close(col.path(), cx);
        w.open(col.path().to_path_buf(), cx);
    });
    cx.run_until_parked();
    // Only the new entry's own run (from its first snapshot) is counted and applied.
    assert_eq!(runs(cx, &ws, col.path()), 1);
    assert_eq!(
        changes(cx, &ws, col.path()),
        Some(vec![("a.yml".to_owned(), GitChange::Modified)])
    );
}

#[gpui_kit::test]
fn turning_git_off_clears_every_state(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = two(&sb);
    write(col.path(), "a.yml", &request("a", "http://x/changed"));
    let (_state, ws) = opened(cx, Some(sb.options()), col.path());
    assert!(changes(cx, &ws, col.path()).is_some());
    ws.update(cx, |w, cx| w.set_git(None, cx));
    assert_eq!(changes(cx, &ws, col.path()), None);
}

#[gpui_kit::test]
fn turning_git_off_during_a_run_drops_it_and_a_later_refresh_still_works(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = two(&sb);
    write(col.path(), "a.yml", &request("a", "http://x/changed"));
    let (_state, ws) = opened(cx, Some(sb.options()), col.path());
    let before = runs(cx, &ws, col.path());
    ws.update(cx, |w, cx| {
        w.refresh_git(col.path(), cx);
        w.set_git(None, cx);
    });
    cx.run_until_parked();
    // The run finished but its result, landing after git went off, is not kept.
    assert_eq!(runs(cx, &ws, col.path()), before + 1);
    assert_eq!(changes(cx, &ws, col.path()), None);

    ws.update(cx, |w, cx| w.set_git(Some(sb.options()), cx));
    cx.run_until_parked();
    write(col.path(), "b.yml", &request("b", "http://x/later"));
    ws.update(cx, |w, cx| w.refresh_git(col.path(), cx));
    cx.run_until_parked();
    assert_eq!(runs(cx, &ws, col.path()), before + 3);
    assert_eq!(
        changes(cx, &ws, col.path()),
        Some(vec![
            ("a.yml".to_owned(), GitChange::Modified),
            ("b.yml".to_owned(), GitChange::Modified),
        ])
    );
}

#[gpui_kit::test]
fn changes_carry_stage_state_and_the_branch(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = two(&sb);
    write(col.path(), "a.yml", &request("a", "http://x/changed"));
    write(col.path(), "new.yml", &request("n", "http://x/n"));
    sb.git(col.path(), &["add", "a.yml"]);
    let (_state, ws) = opened(cx, Some(sb.options()), col.path());
    let g = ws
        .read_with(cx, |w, _| w.git_changes(col.path()))
        .expect("changes");
    assert_eq!(g.stage.get(Path::new("a.yml")), Some(&GitStage::Staged));
    assert_eq!(g.stage.get(Path::new("new.yml")), Some(&GitStage::Unstaged));
    assert_eq!(g.branch.as_deref(), Some("main"));
    assert_eq!(g.staged_count(), 1);
}
