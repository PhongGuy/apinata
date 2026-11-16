//! `rebase_pull`: unpushed commits replayed on the remote tip, and the conflict fallback that
//! keeps local bytes outside the repository before it resets.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

use crate::git_branch::cloned;
use crate::git_support::{Sandbox, write};
use crate::git_sync::{clean, rev};

use std::fs;
use std::path::{Path, PathBuf};

use apinata_core::team::ConflictCopies;
use apinata_core::{CancelFlag, ConflictKind, ConflictPath, RebasePulled, SyncError, rebase_pull};

/// Apply `edits` in `repo` (text, or `None` to delete), commit them, and return the new tip.
pub fn edit(sb: &Sandbox, repo: &Path, edits: &[(&str, Option<&str>)]) -> String {
    for (rel, text) in edits {
        match text {
            Some(text) => write(repo, rel, text),
            None => fs::remove_file(repo.join(rel)).expect("rm"),
        }
    }
    sb.commit_all(repo, "edit");
    rev(sb, repo, "HEAD")
}

/// A teammate's `edits` pushed to origin's `main` from the seed work tree; returns the new tip.
pub fn teammate(sb: &Sandbox, edits: &[(&str, Option<&str>)]) -> String {
    let seed = sb.path().join("seed");
    sb.git(&seed, &["switch", "-q", "main"]);
    sb.git(&seed, &["pull", "-q", "--ff-only", "origin", "main"]);
    let tip = edit(sb, &seed, edits);
    sb.git(&seed, &["push", "-q", "origin", "main"]);
    tip
}

/// A clone whose `main` already holds `files`, pushed by the teammate and fast-forwarded.
fn base_with(sb: &Sandbox, files: &[(&str, Option<&str>)]) -> PathBuf {
    let col = cloned(sb);
    if !files.is_empty() {
        teammate(sb, files);
        sb.git(&col, &["pull", "-q", "--ff-only"]);
    }
    col
}

fn pull(sb: &Sandbox, col: &Path, copies: &Path) -> Result<RebasePulled, SyncError> {
    rebase_pull(col, copies, &CancelFlag::new(), &sb.options())
}

fn kinds(paths: &[ConflictPath]) -> Vec<(&str, ConflictKind)> {
    paths.iter().map(|p| (p.path.as_str(), p.kind)).collect()
}

fn no_rebase_left(col: &Path) -> bool {
    !col.join(".git/rebase-merge").exists() && !col.join(".git/rebase-apply").exists()
}

#[test]
fn nothing_new_is_up_to_date_and_records_the_pre_sync_tip() {
    let sb = Sandbox::new();
    let col = base_with(&sb, &[]);
    let copies = sb.path().join("copies");
    assert_eq!(pull(&sb, &col, &copies).unwrap(), RebasePulled::UpToDate);
    assert_eq!(
        rev(&sb, &col, "refs/apinata/pre-sync"),
        rev(&sb, &col, "HEAD")
    );
    assert!(!copies.exists());
}

#[test]
fn without_local_commits_the_remote_tip_is_fast_forwarded() {
    let sb = Sandbox::new();
    let col = base_with(&sb, &[]);
    let r = teammate(&sb, &[("x.txt", Some("x\n"))]);
    let got = pull(&sb, &col, &sb.path().join("copies")).unwrap();
    assert_eq!(got, RebasePulled::FastForwarded { to: r.clone() });
    assert_eq!(rev(&sb, &col, "HEAD"), r);
    assert_eq!(fs::read_to_string(col.join("x.txt")).unwrap(), "x\n");
}

#[test]
fn unpushed_commits_on_other_files_are_replayed_on_the_remote_tip() {
    let sb = Sandbox::new();
    let col = base_with(&sb, &[]);
    edit(&sb, &col, &[("c.txt", Some("c\n"))]);
    edit(&sb, &col, &[("d.txt", Some("d\n"))]);
    let r = teammate(&sb, &[("x.txt", Some("x\n"))]);
    let got = pull(&sb, &col, &sb.path().join("copies")).unwrap();
    assert_eq!(
        got,
        RebasePulled::Rebased {
            to: rev(&sb, &col, "HEAD"),
            unpushed: 2
        }
    );
    assert_eq!(
        rev(&sb, &col, "HEAD~2"),
        r,
        "the remote history is kept as it is"
    );
    for f in ["c.txt", "d.txt", "x.txt"] {
        assert!(col.join(f).is_file(), "{f}");
    }
    assert!(clean(&sb, &col));
}

#[test]
fn a_conflict_keeps_the_local_bytes_outside_the_repo_and_resets_to_the_remote() {
    let sb = Sandbox::new();
    let col = base_with(&sb, &[("f/r.yml", Some("base\n"))]);
    let copies = sb.path().join("copies");
    edit(&sb, &col, &[("f/r.yml", Some("mine\n"))]);
    let l = edit(&sb, &col, &[("c.txt", Some("c\n"))]);
    let r = teammate(&sb, &[("f/r.yml", Some("theirs\n"))]);
    let RebasePulled::Conflicted { to, paths } = pull(&sb, &col, &copies).unwrap() else {
        panic!("expected a conflict");
    };
    assert_eq!(kinds(&paths), [("f/r.yml", ConflictKind::BothChanged)]);
    assert_eq!(
        fs::read_to_string(copies.join("f/r.yml")).unwrap(),
        "mine\n"
    );
    assert_eq!(fs::read_to_string(col.join("f/r.yml")).unwrap(), "theirs\n");
    assert_eq!(fs::read_to_string(col.join("c.txt")).unwrap(), "c\n");
    assert_eq!(to, rev(&sb, &col, "HEAD"));
    assert_eq!(
        rev(&sb, &col, "HEAD^"),
        r,
        "one fallback commit on the remote tip"
    );
    assert_eq!(rev(&sb, &col, "refs/apinata/pre-sync"), l);
    assert!(clean(&sb, &col));
    assert!(no_rebase_left(&col));
}

#[test]
fn a_file_deleted_remotely_but_changed_locally_is_copied_and_stays_deleted() {
    let sb = Sandbox::new();
    let col = base_with(&sb, &[]);
    let copies = sb.path().join("copies");
    edit(&sb, &col, &[("a.txt", Some("mine\n"))]);
    let r = teammate(&sb, &[("a.txt", None)]);
    let RebasePulled::Conflicted { to, paths } = pull(&sb, &col, &copies).unwrap() else {
        panic!("expected a conflict");
    };
    assert_eq!(kinds(&paths), [("a.txt", ConflictKind::DeletedRemotely)]);
    assert_eq!(to, r, "nothing else changed locally, so no fallback commit");
    assert!(!col.join("a.txt").exists());
    assert_eq!(fs::read_to_string(copies.join("a.txt")).unwrap(), "mine\n");
}

#[test]
fn a_file_deleted_locally_but_changed_remotely_keeps_the_remote_file_and_no_copy() {
    let sb = Sandbox::new();
    let col = base_with(&sb, &[]);
    let copies = sb.path().join("copies");
    edit(&sb, &col, &[("a.txt", None)]);
    teammate(&sb, &[("a.txt", Some("theirs\n"))]);
    let RebasePulled::Conflicted { paths, .. } = pull(&sb, &col, &copies).unwrap() else {
        panic!("expected a conflict");
    };
    assert_eq!(kinds(&paths), [("a.txt", ConflictKind::DeletedLocally)]);
    assert_eq!(fs::read_to_string(col.join("a.txt")).unwrap(), "theirs\n");
    assert!(!copies.join("a.txt").exists());
}

#[test]
fn a_local_delete_that_conflicted_is_remembered_beside_the_copies() {
    let sb = Sandbox::new();
    let col = base_with(&sb, &[]);
    let copies = ConflictCopies::new(sb.path().join("copies"));
    edit(&sb, &col, &[("a.txt", None)]);
    teammate(&sb, &[("a.txt", Some("theirs\n"))]);
    pull(&sb, &col, copies.dir()).unwrap();
    assert_eq!(copies.list().unwrap(), Vec::<String>::new());
    assert_eq!(copies.deleted().unwrap(), ["a.txt"]);
    assert!(copies.marks_dir().join("a.txt").is_file());
    copies.forget_deleted("a.txt").unwrap();
    assert_eq!(copies.deleted().unwrap(), Vec::<String>::new());
    assert!(copies.forget_deleted("../a.txt").is_err());
}

#[test]
fn a_copies_folder_that_cannot_be_written_leaves_the_repo_at_the_local_tip() {
    let sb = Sandbox::new();
    let col = base_with(&sb, &[]);
    let blocked = sb.path().join("copies");
    fs::write(&blocked, "a file, not a folder").unwrap();
    let l = edit(&sb, &col, &[("a.txt", Some("mine\n"))]);
    teammate(&sb, &[("a.txt", Some("theirs\n"))]);
    assert!(matches!(pull(&sb, &col, &blocked), Err(SyncError::Io(_))));
    assert_eq!(rev(&sb, &col, "HEAD"), l);
    assert_eq!(fs::read_to_string(col.join("a.txt")).unwrap(), "mine\n");
    assert!(no_rebase_left(&col));
    assert!(clean(&sb, &col));
}

#[test]
fn a_dirty_tree_is_refused_before_anything_runs() {
    let sb = Sandbox::new();
    let col = base_with(&sb, &[]);
    write(&col, "loose.txt", "x\n");
    assert!(matches!(
        pull(&sb, &col, &sb.path().join("copies")),
        Err(SyncError::Dirty)
    ));
    assert!(sb.git(&col, &["for-each-ref", "refs/apinata/"]).is_empty());
}

#[cfg(unix)]
#[test]
fn a_rebase_that_never_starts_is_reported_and_resets_nothing() {
    let sb = Sandbox::new();
    let col = base_with(&sb, &[]);
    let copies = sb.path().join("copies");
    sb.fake_tool(
        "dest/col/.git/hooks",
        "pre-rebase",
        "#!/bin/sh\necho no rebases >&2\nexit 1\n",
    );
    let l = edit(&sb, &col, &[("c.txt", Some("c\n"))]);
    teammate(&sb, &[("x.txt", Some("x\n"))]);
    let Err(SyncError::Failed { stderr }) = pull(&sb, &col, &copies) else {
        panic!("the hook refuses the rebase");
    };
    assert!(stderr.contains("no rebases"), "{stderr}");
    assert_eq!(rev(&sb, &col, "HEAD"), l);
    assert!(!copies.exists());
    assert!(no_rebase_left(&col));
}

#[cfg(unix)]
#[test]
fn a_save_during_the_fetch_reads_as_a_dirty_tree() {
    let sb = Sandbox::new();
    let col = base_with(&sb, &[]);
    let copies = sb.path().join("copies");
    let l = edit(&sb, &col, &[("c.txt", Some("c\n"))]);
    teammate(&sb, &[("x.txt", Some("x\n"))]);
    // The fetch's update of the tracking ref stands in for a save landing mid-sync.
    let hook = format!(
        "#!/bin/sh\n[ \"$1\" = committed ] && grep -q refs/remotes/ && echo saved >> '{}'\nexit 0\n",
        col.join("a.txt").display()
    );
    sb.fake_tool("dest/col/.git/hooks", "reference-transaction", &hook);
    let got = pull(&sb, &col, &copies);
    assert!(matches!(got, Err(SyncError::Dirty)), "{got:?}");
    assert_eq!(rev(&sb, &col, "HEAD"), l);
    assert!(no_rebase_left(&col));
}
