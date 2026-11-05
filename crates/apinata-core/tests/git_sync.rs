//! `pull` up to the fast-forward, against a bare origin by file path, plus helpers.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

use crate::git_branch::cloned;
use crate::git_support;

use std::fs;
use std::path::{Path, PathBuf};

use apinata_core::{CancelFlag, GitOptions, SyncError, pull};
use git_support::{Sandbox, path_with, real_git, write};

/// Commit `file` on origin's `main` from the seed work tree and return the new tip.
pub fn advance(sb: &Sandbox, file: &str) -> String {
    let seed = sb.path().join("seed");
    sb.git(&seed, &["switch", "-q", "main"]);
    write(&seed, file, &format!("{file}\n"));
    sb.commit_all(&seed, file);
    sb.git(&seed, &["push", "-q", "origin", "main"]);
    rev(sb, &seed, "HEAD")
}

/// The full id `name` resolves to in `repo`.
pub fn rev(sb: &Sandbox, repo: &Path, name: &str) -> String {
    sb.git(repo, &["rev-parse", name]).trim().to_owned()
}

/// Commit a new file `file` in `repo`.
pub fn local_commit(sb: &Sandbox, repo: &Path, file: &str) {
    write(repo, file, &format!("{file}\n"));
    sb.commit_all(repo, file);
}

/// Whether `repo` has no staged, unstaged or untracked change.
pub fn clean(sb: &Sandbox, repo: &Path) -> bool {
    sb.git(repo, &["status", "--porcelain"]).is_empty()
}

/// Options whose `git` (in `<sandbox>/<dir>`) logs each command line to the returned file and,
/// when the line holds ` <on> `, runs the shell `before` and `after` around the real git.
/// `$dir` in either names the wrapper's folder.
#[cfg(unix)]
pub fn wrapped(
    sb: &Sandbox,
    dir: &str,
    on: &str,
    before: &str,
    after: &str,
) -> (GitOptions, PathBuf) {
    let body = format!(
        "#!/bin/sh\ndir=${{0%/*}}\nprintf '%s\\n' \"$*\" >> \"$dir/log\"\ncase \"$*\" in *\" {on} \"*) {before} ;; esac\n'{real}' \"$@\"\ncode=$?\ncase \"$*\" in *\" {on} \"*) {after} ;; esac\nexit $code\n",
        real = real_git().display()
    );
    let bin = sb.fake_git(dir, &body);
    (sb.options().env("PATH", path_with(&bin)), bin.join("log"))
}

/// Exists on every target so importers compile; only unix tests call it.
#[cfg(not(unix))]
pub fn wrapped(_: &Sandbox, _: &str, _: &str, _: &str, _: &str) -> (GitOptions, PathBuf) {
    unreachable!("wrapped needs a unix shell")
}

/// The wrapper's log, empty when nothing ran.
pub fn logged(log: &Path) -> String {
    fs::read_to_string(log).unwrap_or_default()
}

/// A clone of an empty bare `empty.git`: `main` is unborn and tracks `origin/main`.
pub fn unborn(sb: &Sandbox) -> PathBuf {
    sb.git(
        sb.path(),
        &["init", "-q", "--bare", "-b", "main", "empty.git"],
    );
    sb.git(sb.path(), &["clone", "-q", "empty.git", "fresh"]);
    sb.path().join("fresh")
}

fn no_upstream(r: Result<apinata_core::Pulled, SyncError>) -> (String, Option<String>) {
    match r {
        Err(SyncError::NoUpstream { branch, gone }) => (branch, gone),
        other => panic!("expected NoUpstream, got {other:?}"),
    }
}

#[test]
fn pull_fast_forwards_to_the_upstream_with_full_ids() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    let before = rev(&sb, &root, "HEAD");
    let tip = advance(&sb, "c.txt");
    let p = pull(&root, &CancelFlag::new(), &sb.options()).unwrap();
    assert_eq!(
        (
            p.branch.as_str(),
            p.upstream.as_str(),
            p.from.as_str(),
            p.to.as_str(),
            p.ahead
        ),
        ("main", "origin/main", before.as_str(), tip.as_str(), 0)
    );
    assert_eq!(p.to.len(), 40);
    assert_eq!(rev(&sb, &root, "HEAD"), tip);
    assert_eq!(fs::read_to_string(root.join("c.txt")).unwrap(), "c.txt\n");
    assert!(clean(&sb, &root));
}

#[test]
fn pull_with_nothing_new_or_only_local_commits_moves_nothing() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    let head = rev(&sb, &root, "HEAD");
    let p = pull(&root, &CancelFlag::new(), &sb.options()).unwrap();
    assert_eq!(
        (p.from.as_str(), p.to.as_str(), p.ahead),
        (head.as_str(), head.as_str(), 0)
    );
    local_commit(&sb, &root, "d.txt");
    local_commit(&sb, &root, "e.txt");
    let head = rev(&sb, &root, "HEAD");
    let p = pull(&root, &CancelFlag::new(), &sb.options()).unwrap();
    assert_eq!(
        (p.from.as_str(), p.to.as_str(), p.ahead),
        (head.as_str(), head.as_str(), 2)
    );
}

#[test]
fn diverged_is_refused_after_the_fetch_with_the_tree_untouched() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    local_commit(&sb, &root, "d.txt");
    let head = rev(&sb, &root, "HEAD");
    let tip = advance(&sb, "c.txt");
    match pull(&root, &CancelFlag::new(), &sb.options()) {
        Err(SyncError::Diverged {
            upstream,
            ahead,
            behind,
        }) => {
            assert_eq!((upstream.as_str(), ahead, behind), ("origin/main", 1, 1))
        }
        other => panic!("expected Diverged, got {other:?}"),
    }
    assert_eq!(rev(&sb, &root, "HEAD"), head);
    assert!(!root.join("c.txt").exists());
    assert!(clean(&sb, &root));
    assert_eq!(rev(&sb, &root, "refs/remotes/origin/main"), tip);
}

#[cfg(unix)]
#[test]
fn a_change_or_a_detached_head_is_refused_before_the_fetch() {
    for (file, text) in [("a.txt", "edited\n"), ("new.txt", "untracked\n"), ("", "")] {
        let sb = Sandbox::new();
        let root = cloned(&sb);
        let old = rev(&sb, &root, "refs/remotes/origin/main");
        advance(&sb, "c.txt");
        if file.is_empty() {
            sb.git(&root, &["switch", "-q", "--detach"]);
        } else {
            write(&root, file, text);
        }
        let (opts, log) = wrapped(&sb, "wrap", "fetch", ":", ":");
        let r = pull(&root, &CancelFlag::new(), &opts);
        match file {
            "" => assert!(matches!(r, Err(SyncError::Detached)), "{r:?}"),
            _ => assert!(matches!(r, Err(SyncError::Dirty)), "{file}: {r:?}"),
        }
        assert!(!logged(&log).contains(" fetch "), "{file}");
        assert_eq!(rev(&sb, &root, "refs/remotes/origin/main"), old, "{file}");
    }
}

#[test]
fn a_directory_outside_any_repository_is_failed_not_detached() {
    let sb = Sandbox::new();
    let plain = sb.path().join("plain");
    fs::create_dir(&plain).unwrap();
    match pull(&plain, &CancelFlag::new(), &sb.options()) {
        Err(SyncError::Failed { stderr }) => {
            assert!(stderr.contains("not a git repository"), "{stderr}")
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn no_upstream_a_local_upstream_or_a_pruned_one_is_no_upstream() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    sb.git(&root, &["branch", "-q", "--unset-upstream"]);
    let r = pull(&root, &CancelFlag::new(), &sb.options());
    assert_eq!(no_upstream(r), ("main".to_owned(), None));
    sb.git(&root, &["switch", "-q", "-c", "loc"]);
    sb.git(&root, &["branch", "-q", "--set-upstream-to=main"]);
    let r = pull(&root, &CancelFlag::new(), &sb.options());
    assert_eq!(no_upstream(r), ("loc".to_owned(), None));
    sb.git(
        &root,
        &["switch", "-q", "--track", "-c", "work", "origin/feature"],
    );
    sb.git(
        &sb.path().join("seed"),
        &["push", "-q", "origin", ":feature"],
    );
    let r = pull(&root, &CancelFlag::new(), &sb.options());
    assert_eq!(
        no_upstream(r),
        ("work".to_owned(), Some("origin/feature".to_owned()))
    );
}

#[cfg(unix)]
#[test]
fn an_unborn_branch_is_refused_before_any_fetch() {
    let sb = Sandbox::new();
    let root = unborn(&sb);
    let (opts, log) = wrapped(&sb, "wrap", "fetch", ":", ":");
    match pull(&root, &CancelFlag::new(), &opts) {
        Err(SyncError::Failed { stderr }) => assert!(stderr.contains("no commits yet"), "{stderr}"),
        other => panic!("expected Failed, got {other:?}"),
    }
    assert!(!logged(&log).contains(" fetch "));
}
