//! A repository whose index is locked by another git: stage and unstage report the lock as
//! `Busy`, change nothing and never delete it.

use crate::git_support;

use std::fs;
use std::path::Path;

use apinata_core::{GitError, stage, unstage};
use git_support::{Sandbox, write};

fn assert_busy(result: Result<(), GitError>, lock: &Path) {
    match result {
        Err(GitError::Busy { lock: named }) => {
            assert_eq!(named.canonicalize().unwrap(), lock.canonicalize().unwrap())
        }
        other => panic!("expected Busy, got {other:?}"),
    }
    assert!(lock.exists(), "the lock was deleted");
}

#[test]
fn stage_under_a_held_lock_is_busy_and_stages_nothing() {
    let sb = Sandbox::new();
    let repo = sb.repo("r");
    write(&repo, "a.yml", "a: 1\n");
    sb.commit_all(&repo, "init");
    write(&repo, "b.yml", "b: 1\n");
    let lock = repo.join(".git/index.lock");
    fs::write(&lock, "").unwrap();

    assert_busy(stage(&repo, &["b.yml"], &sb.options()), &lock);
    assert_eq!(sb.git(&repo, &["diff", "--cached", "--name-only"]), "");
}

#[test]
fn unstage_under_a_held_lock_is_busy_and_unstages_nothing() {
    let sb = Sandbox::new();
    let repo = sb.repo("r");
    write(&repo, "a.yml", "a: 1\n");
    sb.commit_all(&repo, "init");
    write(&repo, "a.yml", "a: 2\n");
    sb.git(&repo, &["add", "a.yml"]);
    let lock = repo.join(".git/index.lock");
    fs::write(&lock, "").unwrap();

    assert_busy(unstage(&repo, &["a.yml"], &sb.options()), &lock);
    assert_eq!(
        sb.git(&repo, &["diff", "--cached", "--name-only"]),
        "a.yml\n"
    );
}

// Windows words this differently; the core's unit tests pin both texts.
#[cfg(not(windows))]
#[test]
fn busy_tells_the_user_where_the_lock_is() {
    let lock = Path::new("/r/.git/index.lock");
    let text = GitError::Busy {
        lock: lock.to_owned(),
    }
    .to_string();
    assert_eq!(
        text,
        "git is busy: another git process is using this repository \
         (/r/.git/index.lock exists; delete it if no git process is running)"
    );
}
