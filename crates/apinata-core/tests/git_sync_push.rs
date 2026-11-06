//! `push`: what succeeds, and what is refused before git pushes.
#![cfg_attr(not(unix), allow(unused_imports))]

use crate::git_branch::cloned;
use crate::git_support::{Sandbox, write};
use crate::git_sync::{local_commit, logged, rev, unborn, wrapped};

use std::fs;

use apinata_core::{CancelFlag, Pushed, SyncError, push};

fn ok(root: &std::path::Path, set: Option<&str>, sb: &Sandbox) -> Pushed {
    push(root, set, &CancelFlag::new(), &sb.options()).unwrap()
}

fn bare_has(sb: &Sandbox, name: &str) -> bool {
    let full = format!("refs/heads/{name}");
    !sb.git(
        &sb.path().join("origin.git"),
        &["for-each-ref", "--format=%(refname)", &full],
    )
    .is_empty()
}

#[test]
fn push_fast_forwards_or_is_up_to_date_with_full_ids_even_with_changes() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    let old = rev(&sb, &root, "HEAD");
    local_commit(&sb, &root, "d.txt");
    write(&root, "scratch.txt", "not committed\n");
    let new = rev(&sb, &root, "HEAD");
    let p = ok(&root, None, &sb);
    assert_eq!(
        (
            p.branch.as_str(),
            p.upstream.as_str(),
            p.from.as_deref(),
            p.to.as_str(),
            p.upstream_set
        ),
        (
            "main",
            "origin/main",
            Some(old.as_str()),
            new.as_str(),
            false
        )
    );
    assert_eq!(p.to.len(), 40);
    assert_eq!(
        rev(&sb, &sb.path().join("origin.git"), "refs/heads/main"),
        new
    );
    let p = ok(&root, None, &sb);
    assert_eq!(
        (p.from.as_deref(), p.to.as_str()),
        (Some(new.as_str()), new.as_str())
    );
}

#[test]
fn push_goes_to_the_upstream_branch_name_not_the_local_one() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    sb.git(
        &root,
        &["switch", "-q", "--track", "-c", "work", "origin/feature"],
    );
    local_commit(&sb, &root, "w.txt");
    let p = ok(&root, None, &sb);
    assert_eq!(
        (p.branch.as_str(), p.upstream.as_str()),
        ("work", "origin/feature")
    );
    assert_eq!(
        rev(&sb, &sb.path().join("origin.git"), "refs/heads/feature"),
        rev(&sb, &root, "HEAD")
    );
    assert!(!bare_has(&sb, "work"));
}

#[test]
fn set_upstream_publishes_a_new_branch_and_tracks_it() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    sb.git(&root, &["switch", "-q", "-c", "topic"]);
    local_commit(&sb, &root, "t.txt");
    let new = rev(&sb, &root, "HEAD");
    let p = ok(&root, Some("origin"), &sb);
    assert_eq!(
        (
            p.branch.as_str(),
            p.upstream.as_str(),
            p.from.as_deref(),
            p.to.as_str(),
            p.upstream_set
        ),
        ("topic", "origin/topic", None, new.as_str(), true)
    );
    assert_eq!(
        sb.git(&root, &["config", "--get", "branch.topic.merge"])
            .trim(),
        "refs/heads/topic"
    );
    assert!(bare_has(&sb, "topic"));
}

#[test]
fn no_upstream_a_gone_one_or_a_bad_remote_pushes_nothing() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    sb.git(&root, &["switch", "-q", "-c", "topic"]);
    let r = push(&root, None, &CancelFlag::new(), &sb.options());
    assert!(
        matches!(&r, Err(SyncError::NoUpstream { branch, gone: None }) if branch == "topic"),
        "{r:?}"
    );
    for remote in ["nope", "-x", "../origin.git", ""] {
        let r = push(&root, Some(remote), &CancelFlag::new(), &sb.options());
        assert!(
            matches!(&r, Err(SyncError::Failed { stderr }) if stderr.contains("remote")),
            "{remote}: {r:?}"
        );
    }
    assert!(!bare_has(&sb, "topic"));
    sb.git(
        &root,
        &["switch", "-q", "--track", "-c", "work", "origin/feature"],
    );
    sb.git(
        &sb.path().join("seed"),
        &["push", "-q", "origin", ":feature"],
    );
    sb.git(&root, &["fetch", "-q", "--prune", "origin"]);
    let r = push(&root, None, &CancelFlag::new(), &sb.options());
    assert!(
        matches!(&r, Err(SyncError::NoUpstream { branch, gone: Some(g) }) if branch == "work" && g == "origin/feature"),
        "{r:?}"
    );
    assert!(!bare_has(&sb, "feature"));
}

#[test]
fn a_directory_outside_any_repository_is_failed_not_detached() {
    let sb = Sandbox::new();
    let plain = sb.path().join("plain");
    fs::create_dir(&plain).unwrap();
    let r = push(&plain, None, &CancelFlag::new(), &sb.options());
    assert!(
        matches!(&r, Err(SyncError::Failed { stderr }) if stderr.contains("not a git repository")),
        "{r:?}"
    );
}

#[cfg(unix)]
#[test]
fn an_unborn_branch_is_refused_then_set_upstream_publishes_its_first_commit() {
    let sb = Sandbox::new();
    let root = unborn(&sb);
    let (opts, log) = wrapped(&sb, "wrap", "push", ":", ":");
    for set in [None, Some("origin")] {
        let r = push(&root, set, &CancelFlag::new(), &opts);
        assert!(
            matches!(&r, Err(SyncError::Failed { stderr }) if stderr.contains("no commits yet")),
            "{set:?}: {r:?}"
        );
    }
    assert!(!logged(&log).contains(" push "));
    local_commit(&sb, &root, "first.txt");
    let r = push(&root, None, &CancelFlag::new(), &sb.options());
    assert!(
        matches!(&r, Err(SyncError::NoUpstream { branch, gone: Some(g) }) if branch == "main" && g == "origin/main"),
        "{r:?}"
    );
    let empty = sb.path().join("empty.git");
    assert!(sb.git(&empty, &["for-each-ref"]).is_empty());
    let p = ok(&root, Some("origin"), &sb);
    assert_eq!((p.from.as_deref(), p.upstream_set), (None, true));
    assert_eq!(rev(&sb, &empty, "refs/heads/main"), rev(&sb, &root, "HEAD"));
}
