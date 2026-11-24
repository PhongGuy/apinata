//! Making a new branch from HEAD and switching to it, changes and all.

use crate::git_branch::cloned;
use crate::git_support;

use std::fs;
use std::path::Path;

use apinata_core::{SwitchError, Upstream, create_branch};
use git_support::{Sandbox, write};

fn head(sb: &Sandbox, root: &Path) -> String {
    sb.git(root, &["symbolic-ref", "--short", "HEAD"])
        .trim()
        .to_owned()
}

fn local_branches(sb: &Sandbox, root: &Path) -> String {
    sb.git(root, &["for-each-ref", "--format=%(refname)", "refs/heads"])
}

#[test]
fn a_dirty_tree_comes_along_to_the_new_branch() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    write(&root, "a.txt", "changed\n");
    write(&root, "staged.txt", "new\n");
    sb.git(&root, &["add", "staged.txt"]);
    write(&root, "loose.txt", "untracked\n");
    let status = sb.git(&root, &["status", "--porcelain"]);
    let tip = sb.git(&root, &["rev-parse", "HEAD"]);
    let done = create_branch(&root, "topic", &sb.options()).unwrap();
    assert_eq!(done.branch, "topic");
    assert_eq!(done.upstream, Upstream::Unchanged);
    assert_eq!(head(&sb, &root), "topic");
    assert_eq!(sb.git(&root, &["rev-parse", "HEAD"]), tip);
    assert_eq!(sb.git(&root, &["status", "--porcelain"]), status);
    assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "changed\n");
    // A branch made from HEAD tracks nothing.
    assert!(
        sb.git(&root, &["config", "--local", "--list"])
            .lines()
            .all(|l| !l.starts_with("branch.topic."))
    );
}

#[test]
fn an_invalid_name_is_refused_before_git_writes() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    let before = local_branches(&sb, &root);
    for name in [
        "",
        "-f",
        "--orphan",
        "a..b",
        "has space",
        "ends.lock",
        "a/",
        "@{-1}",
        "HEAD",
        "x~1",
    ] {
        let r = create_branch(&root, name, &sb.options());
        match r {
            Err(SwitchError::Failed { stderr }) => {
                assert!(
                    stderr.contains("not a valid branch name"),
                    "{name}: {stderr}"
                )
            }
            other => panic!("{name}: expected a refusal, got {other:?}"),
        }
        assert_eq!(head(&sb, &root), "main", "{name}");
        assert_eq!(local_branches(&sb, &root), before, "{name}");
    }
}

#[test]
fn an_existing_name_is_refused_and_leaves_that_branch_alone() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    sb.git(&root, &["branch", "topic", "origin/feature"]);
    let tip = sb.git(&root, &["rev-parse", "topic"]);
    for name in ["topic", "main"] {
        let r = create_branch(&root, name, &sb.options());
        match r {
            Err(SwitchError::Failed { stderr }) => {
                assert!(stderr.contains("already exists"), "{name}: {stderr}")
            }
            other => panic!("{name}: expected a refusal, got {other:?}"),
        }
    }
    assert_eq!(head(&sb, &root), "main");
    assert_eq!(sb.git(&root, &["rev-parse", "topic"]), tip);
}

#[test]
fn a_repository_with_no_commits_gets_the_new_branch_as_its_head() {
    let sb = Sandbox::new();
    let root = sb.repo("fresh");
    write(&root, "a.txt", "first\n");
    let done = create_branch(&root, "start", &sb.options()).unwrap();
    assert_eq!(done.branch, "start");
    assert_eq!(head(&sb, &root), "start");
    assert!(root.join("a.txt").is_file());
}

#[test]
fn a_held_head_lock_is_busy_and_makes_no_branch() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    let before = local_branches(&sb, &root);
    let held = root.join(".git/HEAD.lock");
    fs::write(&held, "").unwrap();
    match create_branch(&root, "topic", &sb.options()) {
        Err(SwitchError::Busy { lock }) => {
            assert_eq!(lock.canonicalize().unwrap(), held.canonicalize().unwrap())
        }
        other => panic!("expected Busy, got {other:?}"),
    }
    assert!(held.exists(), "HEAD.lock was deleted");
    assert_eq!(local_branches(&sb, &root), before);
}
