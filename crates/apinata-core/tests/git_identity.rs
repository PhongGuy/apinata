//! The identity preflight of `commit` and `rebase_pull`, and `set_identity` at both scopes.
//! Every git call here reads the sandbox's temp config files, never the machine's.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

use crate::git_branch::cloned;
use crate::git_support::{Sandbox, assert_no_ambient_identity, write};
use crate::git_sync::rev;

use std::fs;
use std::path::{Path, PathBuf};

use apinata_core::{
    CancelFlag, CommitError, GitError, GitOptions, IdentityScope, SyncError, commit, rebase_pull,
    set_identity, stage,
};

/// The sandbox's options with no identity variables, so identity comes from config alone.
fn no_identity(sb: &Sandbox) -> GitOptions {
    assert_no_ambient_identity();
    sb.env()
        .into_iter()
        .filter(|(k, _)| {
            let k = k.to_string_lossy();
            !k.ends_with("_NAME") && !k.ends_with("_EMAIL")
        })
        .fold(GitOptions::new(), |o, (k, v)| o.env(k, v))
}

/// Repository `r` with `col/a.yml` committed, then changed and staged with no identity
/// configured anywhere; returns (repo, col).
fn staged_without_identity(sb: &Sandbox) -> (PathBuf, PathBuf) {
    let repo = sb.repo("r");
    write(&repo, "col/a.yml", "a\n");
    sb.commit_all(&repo, "base");
    let col = repo.join("col");
    write(&col, "a.yml", "changed\n");
    stage(&col, &["a.yml"], &sb.options()).unwrap();
    (repo, col)
}

fn global_file(sb: &Sandbox) -> PathBuf {
    sb.path().join("global.gitconfig")
}

/// [`no_identity`] with the global config at its own temp file, apart from the system one.
fn with_global(sb: &Sandbox) -> GitOptions {
    fs::write(global_file(sb), "").unwrap();
    no_identity(sb).env("GIT_CONFIG_GLOBAL", global_file(sb))
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

fn count(sb: &Sandbox, repo: &Path) -> String {
    sb.git(repo, &["rev-list", "--count", "HEAD"])
        .trim()
        .to_owned()
}

#[cfg(unix)]
#[test]
fn no_identity_is_refused_before_git_commits() {
    let sb = Sandbox::new();
    let (repo, col) = staged_without_identity(&sb);
    // A hook that would leave a mark shows whether `git commit` ran at all.
    let mark = sb.path().join("ran");
    let hook = repo.join(".git/hooks/pre-commit");
    fs::create_dir_all(hook.parent().unwrap()).unwrap();
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg("printf '#!/bin/sh\\n: > \"%s\"\\n' \"$1\" > \"$2\" && chmod 755 \"$2\"")
        .arg("sh")
        .arg(&mark)
        .arg(&hook)
        .status()
        .unwrap();
    assert!(status.success());
    let r = commit(&col, "msg", &with_global(&sb));
    assert!(matches!(r, Err(CommitError::NoIdentity)), "{r:?}");
    assert!(!mark.exists(), "git commit ran");
    assert_eq!(count(&sb, &repo), "1");
}

#[test]
fn the_no_identity_text_is_plain() {
    let text = CommitError::NoIdentity.to_string();
    assert_eq!(text, "git needs your name and email before it can commit");
}

#[test]
fn the_repository_scope_writes_only_the_repository_config() {
    let sb = Sandbox::new();
    let (repo, col) = staged_without_identity(&sb);
    let options = with_global(&sb);
    set_identity(
        &col,
        "Ann Example",
        "ann@example.com",
        IdentityScope::Repository,
        &options,
    )
    .expect("set");
    let local = read(&repo.join(".git/config"));
    assert!(
        local.contains("name = Ann Example") && local.contains("email = ann@example.com"),
        "{local}"
    );
    assert_eq!(read(&global_file(&sb)), "");
    assert_eq!(read(&sb.path().join("empty.gitconfig")), "");
    commit(&col, "msg", &options).expect("commit");
    assert_eq!(
        sb.git(&repo, &["log", "-1", "--format=%an <%ae> | %cn <%ce>"]),
        "Ann Example <ann@example.com> | Ann Example <ann@example.com>\n"
    );
}

#[test]
fn all_repositories_writes_only_the_global_config() {
    let sb = Sandbox::new();
    let (repo, col) = staged_without_identity(&sb);
    let options = with_global(&sb);
    set_identity(
        &col,
        "Ann Example",
        "ann@example.com",
        IdentityScope::AllRepositories,
        &options,
    )
    .expect("set");
    let global = read(&global_file(&sb));
    assert!(
        global.contains("name = Ann Example") && global.contains("email = ann@example.com"),
        "{global}"
    );
    assert!(!read(&repo.join(".git/config")).contains("[user]"));
    assert_eq!(read(&sb.path().join("empty.gitconfig")), "");
    commit(&col, "msg", &options).expect("commit");
    assert_eq!(count(&sb, &repo), "2");
}

#[test]
fn unsafe_values_are_refused_and_nothing_is_written() {
    let sb = Sandbox::new();
    let (repo, col) = staged_without_identity(&sb);
    let options = with_global(&sb);
    let bad = [
        "", "   ", "-x", "--global", "a\nb", "a\rb", "a\0b", "<a", "a>",
    ];
    for value in bad {
        for (name, email) in [(value, "ann@example.com"), ("Ann", value)] {
            for scope in [IdentityScope::Repository, IdentityScope::AllRepositories] {
                let r = set_identity(&col, name, email, scope, &options);
                assert!(
                    matches!(r, Err(GitError::Malformed(_))),
                    "{name:?} {email:?}: {r:?}"
                );
            }
        }
    }
    assert!(!read(&repo.join(".git/config")).contains("[user]"));
    assert_eq!(read(&global_file(&sb)), "");
}

/// Git itself stores a value with a leading dash as data, so refusing one is this crate's
/// rule: such a name would read as an option wherever it is pasted into a command line.
#[test]
fn git_config_takes_a_leading_dash_as_a_value() {
    let sb = Sandbox::new();
    let repo = sb.repo("r");
    sb.git(&repo, &["config", "--local", "--", "user.name", "-x"]);
    assert_eq!(sb.git(&repo, &["config", "user.name"]), "-x\n");
}

#[test]
fn a_rebase_without_identity_is_no_identity_and_changes_nothing() {
    let sb = Sandbox::new();
    let col = cloned(&sb);
    write(&col, "local.txt", "l\n");
    sb.commit_all(&col, "local");
    let seed = sb.path().join("seed");
    sb.git(&seed, &["switch", "-q", "main"]);
    write(&seed, "remote.txt", "r\n");
    sb.commit_all(&seed, "remote");
    sb.git(&seed, &["push", "-q", "origin", "main"]);
    let head = rev(&sb, &col, "HEAD");
    let copies = sb.path().join("copies");
    let r = rebase_pull(&col, &copies, &CancelFlag::new(), &with_global(&sb));
    assert!(matches!(r, Err(SyncError::NoIdentity)), "{r:?}");
    assert_eq!(rev(&sb, &col, "HEAD"), head);
    assert!(!col.join(".git/rebase-merge").exists());
}
