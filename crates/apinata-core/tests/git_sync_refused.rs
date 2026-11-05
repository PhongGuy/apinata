//! Pushes the remote refuses, a held config lock, and the lock a killed push leaves.
#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]

use crate::git_branch::cloned;
use crate::git_support::{Sandbox, path_with, real_git};
use crate::git_sync::{advance, local_commit, rev};

use std::fs;
use std::path::Path;

use apinata_core::{CancelFlag, GitOptions, SyncError, push};

fn reason(r: Result<apinata_core::Pushed, SyncError>) -> String {
    match r {
        Err(SyncError::Rejected { reason, .. }) => reason,
        other => panic!("expected Rejected, got {other:?}"),
    }
}

fn origin_main(sb: &Sandbox) -> String {
    rev(sb, &sb.path().join("origin.git"), "refs/heads/main")
}

#[test]
fn a_push_behind_the_remote_is_rejected_even_with_a_forcing_refspec_in_config() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    local_commit(&sb, &root, "d.txt");
    let tip = advance(&sb, "c.txt");
    assert_eq!(
        reason(push(&root, None, &CancelFlag::new(), &sb.options())),
        "fetch first"
    );
    sb.git(&root, &["fetch", "-q", "origin"]);
    assert_eq!(
        reason(push(&root, None, &CancelFlag::new(), &sb.options())),
        "non-fast-forward"
    );
    sb.git(
        &root,
        &["config", "remote.origin.push", "+refs/heads/*:refs/heads/*"],
    );
    assert_eq!(
        reason(push(&root, None, &CancelFlag::new(), &sb.options())),
        "non-fast-forward"
    );
    assert_eq!(origin_main(&sb), tip);
}

#[cfg(unix)]
#[test]
fn a_pre_receive_hook_rejection_carries_the_remote_lines() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    local_commit(&sb, &root, "d.txt");
    sb.fake_tool(
        "origin.git/hooks",
        "pre-receive",
        "#!/bin/sh\necho 'nope from hook' >&2\nexit 1\n",
    );
    match push(&root, None, &CancelFlag::new(), &sb.options()) {
        Err(SyncError::Rejected { reason, stderr }) => {
            assert_eq!(reason, "pre-receive hook declined");
            assert!(stderr.contains("remote: nope from hook"), "{stderr}");
        }
        other => panic!("expected Rejected, got {other:?}"),
    }
}

#[test]
fn a_lock_in_the_remote_is_rejected_not_busy() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    local_commit(&sb, &root, "d.txt");
    let held = sb.path().join("origin.git/refs/heads/main.lock");
    fs::write(&held, "").unwrap();
    reason(push(&root, None, &CancelFlag::new(), &sb.options()));
    assert!(held.exists());
}

fn busy_at(r: Result<apinata_core::Pushed, SyncError>, held: &Path) {
    match r {
        Err(SyncError::Busy { lock }) => {
            assert_eq!(lock.canonicalize().unwrap(), held.canonicalize().unwrap())
        }
        other => panic!("expected Busy at {}, got {other:?}", held.display()),
    }
    assert!(held.exists(), "{} was deleted", held.display());
}

#[test]
fn a_held_config_lock_refuses_set_upstream_before_pushing() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    sb.git(&root, &["switch", "-q", "-c", "topic"]);
    local_commit(&sb, &root, "t.txt");
    let held = root.join(".git/config.lock");
    fs::write(&held, "").unwrap();
    busy_at(
        push(&root, Some("origin"), &CancelFlag::new(), &sb.options()),
        &held,
    );
    let bare = sb.path().join("origin.git");
    assert!(
        sb.git(&bare, &["for-each-ref", "refs/heads/topic"])
            .is_empty()
    );
}

/// Options whose `git push` ignores TERM, creates `.git/<lock>` (when given), marks
/// `<bin>/started` and hangs; every other call is the real git.
#[cfg(target_os = "linux")]
fn stubborn_push(
    sb: &Sandbox,
    root: &Path,
    lock: Option<&str>,
) -> (GitOptions, std::path::PathBuf) {
    let take = lock
        .map(|l| {
            let p = root.join(".git").join(l);
            format!(
                "mkdir -p '{}'; : > '{}'; ",
                p.parent().unwrap().display(),
                p.display()
            )
        })
        .unwrap_or_default();
    let body = format!(
        "#!/bin/sh\ndir=${{0%/*}}\ncase \"$*\" in *\" push \"*) trap '' TERM; {take}: > \"$dir/started\"; sleep 30; exit 1 ;; esac\nexec '{}' \"$@\"\n",
        real_git().display()
    );
    let bin = sb.fake_git("stubborn", &body);
    (
        sb.options().env("PATH", path_with(&bin)),
        bin.join("started"),
    )
}

/// Push under `opts`, cancelling once `started` exists.
#[cfg(target_os = "linux")]
fn cancelled_push(
    root: &Path,
    set: Option<&str>,
    opts: &GitOptions,
    started: &Path,
) -> Result<apinata_core::Pushed, SyncError> {
    let cancel = CancelFlag::new();
    std::thread::scope(|s| {
        s.spawn(|| {
            let seen = (0..250).any(|_| {
                std::thread::sleep(std::time::Duration::from_millis(20));
                started.exists()
            });
            cancel.cancel();
            assert!(seen, "the fake push never started");
        });
        push(root, set, &cancel, opts)
    })
}

#[cfg(target_os = "linux")]
#[test]
fn a_killed_push_reports_the_lock_it_left_or_is_cancelled() {
    for (set, branch, name) in [
        (None, "main", Some("refs/remotes/origin/main.lock")),
        (Some("origin"), "topic", Some("config.lock")),
        (None, "main", None),
    ] {
        let sb = Sandbox::new();
        let root = cloned(&sb);
        if branch != "main" {
            sb.git(&root, &["switch", "-q", "-c", branch]);
        }
        local_commit(&sb, &root, "d.txt");
        let (opts, started) = stubborn_push(&sb, &root, name);
        let r = cancelled_push(&root, set, &opts, &started);
        match name {
            Some(name) => busy_at(r, &root.join(".git").join(name)),
            None => assert!(matches!(r, Err(SyncError::Cancelled)), "{r:?}"),
        }
    }
}
