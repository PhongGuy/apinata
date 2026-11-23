//! The commit, squash, tip and discard calls the team driver runs.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

use crate::git_branch::cloned;
use crate::git_support::{Sandbox, write};
use crate::git_sync::{advance, clean, local_commit, rev};

use std::fs;

use apinata_core::{
    CancelFlag, CommitError, RebasePulled, SyncError, commit_all, discard_changes, is_dirty,
    remote_tip, squash_unpushed, tracking_tip, unpushed,
};

#[test]
fn commit_all_stages_new_changed_and_deleted_files_but_not_ignored_ones() {
    let sb = Sandbox::new();
    let col = cloned(&sb);
    write(&col, ".gitignore", ".env\n");
    sb.commit_all(&col, "ignore");
    write(&col, ".env", "TOKEN=x\n");
    write(&col, "new.txt", "n\n");
    write(
        &col,
        "opencollection.yml",
        "opencollection: \"1.0.0\"\ninfo:\n  name: renamed\n",
    );
    fs::remove_file(col.join("a.txt")).unwrap();
    assert!(
        commit_all(&col, "Save changes", &sb.options())
            .unwrap()
            .is_some()
    );
    let files = sb.git(&col, &["show", "--name-status", "--format=", "HEAD"]);
    assert_eq!(files, "D\ta.txt\nA\tnew.txt\nM\topencollection.yml\n");
    assert_eq!(
        sb.git(&col, &["status", "--porcelain", "--ignored"]),
        "!! .env\n"
    );
}

#[test]
fn commit_all_with_nothing_changed_makes_no_commit() {
    let sb = Sandbox::new();
    let col = cloned(&sb);
    let before = rev(&sb, &col, "HEAD");
    assert!(
        commit_all(&col, "Save changes", &sb.options())
            .unwrap()
            .is_none()
    );
    assert_eq!(rev(&sb, &col, "HEAD"), before);
}

#[cfg(unix)]
#[test]
fn commit_all_runs_the_repository_hooks() {
    let sb = Sandbox::new();
    let col = cloned(&sb);
    sb.fake_tool(
        "dest/col/.git/hooks",
        "pre-commit",
        "#!/bin/sh\necho nope >&2\nexit 1\n",
    );
    write(&col, "new.txt", "n\n");
    let Err(CommitError::Failed { output }) = commit_all(&col, "Save changes", &sb.options())
    else {
        panic!("the hook refuses the commit");
    };
    assert!(output.contains("nope"), "{output}");
}

#[test]
fn squash_drops_a_refused_blob_from_the_unpushed_range_and_keeps_teammates_work() {
    let sb = Sandbox::new();
    let col = cloned(&sb);
    write(&col, "environments/dev.yml", "value: s3cr3t\n");
    sb.commit_all(&col, "bad");
    let bad = sb
        .git(&col, &["rev-parse", "HEAD:environments/dev.yml"])
        .trim()
        .to_owned();
    write(&col, "environments/dev.yml", "value: fixed\n");
    sb.commit_all(&col, "fix");
    let r = advance(&sb, "x.txt");
    let copies = sb.path().join("copies");
    let got = squash_unpushed(
        &col,
        "Save changes",
        &copies,
        &CancelFlag::new(),
        &sb.options(),
    )
    .unwrap();
    assert_eq!(
        got,
        RebasePulled::Rebased {
            to: rev(&sb, &col, "HEAD"),
            unpushed: 1
        }
    );
    assert_eq!(rev(&sb, &col, "HEAD^"), r);
    let objects = sb.git(&col, &["rev-list", "--objects", "origin/main..HEAD"]);
    assert!(!objects.contains(&bad), "{objects}");
    assert_eq!(
        fs::read_to_string(col.join("environments/dev.yml")).unwrap(),
        "value: fixed\n"
    );
    assert!(col.join("x.txt").is_file());
    assert!(clean(&sb, &col));
}

#[test]
fn remote_tip_reads_the_server_and_tracking_tip_the_last_fetch() {
    let sb = Sandbox::new();
    let col = cloned(&sb);
    let before = rev(&sb, &col, "origin/main");
    let r = advance(&sb, "x.txt");
    assert_eq!(
        remote_tip(&col, &CancelFlag::new(), &sb.options()).unwrap(),
        Some(r)
    );
    assert_eq!(tracking_tip(&col, &sb.options()).unwrap(), Some(before));
}

#[test]
fn unpushed_counts_local_commits_the_upstream_lacks() {
    let sb = Sandbox::new();
    let col = cloned(&sb);
    assert_eq!(unpushed(&col, &sb.options()).unwrap(), 0);
    local_commit(&sb, &col, "c.txt");
    local_commit(&sb, &col, "d.txt");
    assert_eq!(unpushed(&col, &sb.options()).unwrap(), 2);
}

#[test]
fn discard_changes_drops_edits_and_new_files_but_keeps_ignored_ones() {
    let sb = Sandbox::new();
    let col = cloned(&sb);
    write(&col, ".gitignore", ".env\n");
    sb.commit_all(&col, "ignore");
    write(&col, ".env", "TOKEN=x\n");
    write(&col, "a.txt", "changed\n");
    write(&col, "new.txt", "n\n");
    assert!(is_dirty(&col, &sb.options()).unwrap());
    discard_changes(&col, &sb.options()).unwrap();
    assert!(
        !is_dirty(&col, &sb.options()).unwrap(),
        "the ignored .env does not count"
    );
    assert_eq!(fs::read_to_string(col.join("a.txt")).unwrap(), "a\n");
    assert!(!col.join("new.txt").exists());
    assert!(col.join(".env").is_file());
    assert!(clean(&sb, &col));
}

#[test]
fn is_dirty_counts_an_untracked_file_when_status_hides_untracked_files() {
    let sb = Sandbox::new();
    let col = cloned(&sb);
    sb.git(&col, &["config", "status.showUntrackedFiles", "no"]);
    write(&col, "new.txt", "n\n");
    assert!(is_dirty(&col, &sb.options()).unwrap());
}

#[test]
fn squash_still_refuses_an_untracked_file_and_leaves_it() {
    let sb = Sandbox::new();
    let col = cloned(&sb);
    local_commit(&sb, &col, "c.txt");
    let l = rev(&sb, &col, "HEAD");
    write(&col, "loose.txt", "x\n");
    let got = squash_unpushed(
        &col,
        "Save changes",
        &sb.path().join("copies"),
        &CancelFlag::new(),
        &sb.options(),
    );
    assert!(matches!(got, Err(SyncError::Dirty)), "{got:?}");
    assert_eq!(rev(&sb, &col, "HEAD"), l);
    assert_eq!(fs::read_to_string(col.join("loose.txt")).unwrap(), "x\n");
}
