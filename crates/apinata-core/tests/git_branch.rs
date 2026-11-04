use crate::git_support;

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use apinata_core::{
    Branch, CancelFlag, CloneRequest, FetchError, GitError, GitOptions, RemoteBranch, branches,
    clone_repo, fetch, remote_branches, remotes,
};
use git_support::{Sandbox, path_with};

pub fn cloned(sb: &Sandbox) -> PathBuf {
    let bare = sb.origin();
    let parent = sb.path().join("dest");
    fs::create_dir_all(&parent).unwrap();
    clone_repo(
        &CloneRequest::new(bare.to_str().unwrap(), &parent, "col"),
        &CancelFlag::new(),
        &sb.options(),
        &mut |_| {},
    )
    .unwrap()
}

fn tuples(list: Vec<RemoteBranch>) -> Vec<(String, String, Option<String>)> {
    list.into_iter()
        .map(|b| (b.remote, b.name, b.local))
        .collect()
}

fn row(name: &str, local: Option<&str>) -> (String, String, Option<String>) {
    ("origin".into(), name.into(), local.map(str::to_owned))
}

#[test]
fn lists_remote_branches_skipping_head_and_marking_the_tracked_one() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    // The clone did create origin/HEAD, so the skip below is meaningful.
    sb.git(&root, &["symbolic-ref", "refs/remotes/origin/HEAD"]);
    let list = remote_branches(&root, &sb.options()).unwrap();
    assert_eq!(
        tuples(list),
        [row("feature", None), row("main", Some("main"))]
    );
}

#[test]
fn a_tracking_local_branch_is_reported_as_local() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    sb.git(
        &root,
        &["switch", "-q", "--track", "-c", "work", "origin/feature"],
    );
    let list = tuples(remote_branches(&root, &sb.options()).unwrap());
    assert_eq!(
        list,
        [row("feature", Some("work")), row("main", Some("main"))]
    );
}

#[test]
fn remotes_lists_names_in_order() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    assert_eq!(remotes(&root, &sb.options()).unwrap(), ["origin"]);
    sb.git(
        &root,
        &[
            "remote",
            "add",
            "other",
            sb.path().join("origin.git").to_str().unwrap(),
        ],
    );
    assert_eq!(remotes(&root, &sb.options()).unwrap(), ["origin", "other"]);
}

#[test]
fn fetch_brings_in_new_branches_and_prunes_deleted_ones() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    let opts = sb.options();
    sb.git(
        &sb.path().join("seed"),
        &["push", "-q", "origin", "feature:newer"],
    );
    assert!(!tuples(remote_branches(&root, &opts).unwrap()).contains(&row("newer", None)));
    fetch(&root, "origin", &CancelFlag::new(), &opts).unwrap();
    assert!(tuples(remote_branches(&root, &opts).unwrap()).contains(&row("newer", None)));
    sb.git(&sb.path().join("origin.git"), &["branch", "-D", "feature"]);
    fetch(&root, "origin", &CancelFlag::new(), &opts).unwrap();
    let names: Vec<String> = remote_branches(&root, &opts)
        .unwrap()
        .into_iter()
        .map(|b| b.name)
        .collect();
    assert_eq!(names, ["main", "newer"]);
}

#[test]
fn fetch_of_an_unknown_remote_is_refused_by_name() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    let r = fetch(&root, "nope", &CancelFlag::new(), &sb.options());
    assert!(
        matches!(&r, Err(FetchError::Failed { stderr }) if stderr.contains("nope")),
        "{r:?}"
    );
}

#[test]
fn an_option_shaped_remote_never_starts_git() {
    let sb = Sandbox::new();
    let opts = sb.options().env("PATH", sb.empty_bin());
    let r = fetch(sb.path(), "--upload-pack=x", &CancelFlag::new(), &opts);
    assert!(
        matches!(&r, Err(FetchError::Failed { stderr }) if stderr.contains("remote name")),
        "{r:?}"
    );
}

#[test]
fn a_flag_set_before_fetch_cancels() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    let cancel = CancelFlag::new();
    cancel.cancel();
    assert!(matches!(
        fetch(&root, "origin", &cancel, &sb.options()),
        Err(FetchError::Cancelled)
    ));
}

#[cfg(unix)]
#[test]
fn cancel_during_a_fetch_stops_it() {
    let sb = Sandbox::new();
    let body = "#!/bin/sh\ncase \"$*\" in *\" config \"*) exit 1 ;; *\" remote\") echo origin; exit 0 ;; esac\nsleep 60 &\nwait\n";
    let bin = sb.fake_git("fake", body);
    let opts: GitOptions = sb.options().env("PATH", path_with(&bin));
    let cancel = CancelFlag::new();
    let started = Instant::now();
    let r = std::thread::scope(|s| {
        s.spawn(|| {
            std::thread::sleep(Duration::from_millis(300));
            cancel.cancel();
        });
        fetch(sb.path(), "origin", &cancel, &opts)
    });
    assert!(matches!(r, Err(FetchError::Cancelled)), "{r:?}");
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn a_held_remote_ref_lock_makes_the_fetch_busy_and_is_kept() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    let seed = sb.path().join("seed");
    sb.git(&seed, &["switch", "-q", "main"]);
    git_support::write(&seed, "c.txt", "c\n");
    sb.commit_all(&seed, "newer main");
    sb.git(&seed, &["push", "-q", "origin", "main"]);
    let held = root.join(".git/refs/remotes/origin/main.lock");
    fs::write(&held, "").unwrap();
    match fetch(&root, "origin", &CancelFlag::new(), &sb.options()) {
        Err(e @ FetchError::Busy { .. }) => {
            let FetchError::Busy { lock } = &e else {
                unreachable!()
            };
            assert_eq!(lock.canonicalize().unwrap(), held.canonicalize().unwrap());
            let same = GitError::Busy { lock: lock.clone() };
            assert_eq!(e.to_string(), same.to_string());
        }
        other => panic!("expected Busy, got {other:?}"),
    }
    assert!(held.exists(), "the ref lock was deleted");
}

#[test]
fn listing_outside_a_repository_is_a_git_error() {
    let sb = Sandbox::new();
    let plain = sb.path().join("plain");
    fs::create_dir(&plain).unwrap();
    assert!(matches!(
        remote_branches(&plain, &sb.options()),
        Err(GitError::Failed { .. })
    ));
}

/// Each row as (local, upstream `remote/name`, head), for compact comparison.
fn rows(list: Vec<Branch>) -> Vec<(Option<String>, Option<String>, bool)> {
    list.into_iter()
        .map(|b| {
            let up = b.upstream.map(|u| format!("{}/{}", u.remote, u.name));
            (b.local, up, b.head)
        })
        .collect()
}

fn line(
    local: Option<&str>,
    upstream: Option<&str>,
    head: bool,
) -> (Option<String>, Option<String>, bool) {
    (local.map(str::to_owned), upstream.map(str::to_owned), head)
}

#[test]
fn branches_lists_locals_by_name_then_untracked_remote_branches() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    sb.git(&root, &["branch", "topic", "main"]);
    let list = branches(&root, &sb.options()).unwrap();
    assert_eq!(
        rows(list.clone()),
        [
            line(Some("main"), Some("origin/main"), true),
            line(Some("topic"), None, false),
            line(None, Some("origin/feature"), false),
        ]
    );
    // The local row's upstream is the remote list's own entry.
    let main = remote_branches(&root, &sb.options())
        .unwrap()
        .into_iter()
        .find(|b| b.name == "main")
        .unwrap();
    assert_eq!(list[0].upstream.as_ref(), Some(&main));
}

#[test]
fn branches_marks_head_on_a_local_only_branch() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    sb.git(&root, &["switch", "-q", "-c", "topic"]);
    let heads: Vec<_> = branches(&root, &sb.options())
        .unwrap()
        .into_iter()
        .filter(|b| b.head)
        .map(|b| b.local)
        .collect();
    assert_eq!(heads, [Some("topic".to_owned())]);
}

#[test]
fn a_same_named_local_branch_that_tracks_nothing_keeps_the_remote_row() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    sb.git(&root, &["branch", "--no-track", "feature", "main"]);
    assert_eq!(
        rows(branches(&root, &sb.options()).unwrap()),
        [
            line(Some("feature"), None, false),
            line(Some("main"), Some("origin/main"), true),
            line(None, Some("origin/feature"), false),
        ]
    );
}

#[test]
fn a_gone_or_local_upstream_folds_to_none() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    sb.git(&root, &["branch", "--track", "gone", "origin/feature"]);
    sb.git(&root, &["update-ref", "-d", "refs/remotes/origin/feature"]);
    sb.git(&root, &["branch", "--track", "onlocal", "main"]);
    assert_eq!(
        rows(branches(&root, &sb.options()).unwrap()),
        [
            line(Some("gone"), None, false),
            line(Some("main"), Some("origin/main"), true),
            line(Some("onlocal"), None, false),
        ]
    );
}

#[test]
fn a_detached_head_marks_no_row() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    sb.git(&root, &["switch", "-q", "--detach", "main"]);
    let list = branches(&root, &sb.options()).unwrap();
    assert!(!list.is_empty());
    assert!(list.iter().all(|b| !b.head), "{list:?}");
}
