//! `collection_status` against real repositories built by the git CLI in a sandbox.

use crate::git_support;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use apinata_core::{
    FileStatus, GIT_FILE_CAP, GitError, GitOptions, GitState, RepoStatus, StageState,
    collection_status_with, file_diff_with,
};
use git_support::{Sandbox, path_with, write};

fn repo_status(state: GitState) -> RepoStatus {
    match state {
        GitState::Repo(status) => status,
        other => panic!("expected a repository, got {other:?}"),
    }
}

fn status_of(sb: &Sandbox, root: &Path) -> RepoStatus {
    repo_status(collection_status_with(root, &sb.options()).expect("status runs"))
}

/// Repository `r`: `col/` holds one committed request file per name, `other/` sits beside it.
fn committed(sb: &Sandbox, names: &[&str]) -> PathBuf {
    let repo = sb.repo("r");
    for name in names {
        write(
            &repo,
            &format!("col/{name}"),
            &format!("info:\n  name: {name}\n"),
        );
    }
    write(&repo, "other/o.txt", "o\n");
    sb.commit_all(&repo, "base");
    repo
}

#[test]
fn a_clean_collection_has_no_files() {
    let sb = Sandbox::new();
    let repo = committed(&sb, &["keep.yml"]);
    let status = status_of(&sb, &repo.join("col"));
    assert!(status.files.is_empty());
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert_eq!(status.head, Some(sb.short_head(&repo)));
    assert!(!status.truncated);
}

#[test]
fn an_empty_repository_has_a_branch_but_no_head() {
    let sb = Sandbox::new();
    let repo = sb.repo("r");
    fs::create_dir_all(repo.join("col")).unwrap();
    let status = status_of(&sb, &repo.join("col"));
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert_eq!(status.head, None);
    assert!(status.files.is_empty());
}

#[test]
fn a_detached_head_has_a_head_but_no_branch() {
    let sb = Sandbox::new();
    let repo = committed(&sb, &["keep.yml"]);
    sb.git(&repo, &["checkout", "-q", "--detach"]);
    let status = status_of(&sb, &repo.join("col"));
    assert_eq!(status.branch, None);
    assert_eq!(status.head, Some(sb.short_head(&repo)));
}

#[test]
fn a_directory_outside_any_repository_is_not_a_repo() {
    let sb = Sandbox::new();
    let root = sb.path().join("plain/col");
    fs::create_dir_all(&root).unwrap();
    assert_eq!(
        collection_status_with(&root, &sb.options()).unwrap(),
        GitState::NotARepo
    );
}

#[test]
fn without_git_on_path_the_state_is_unavailable() {
    let sb = Sandbox::new();
    let repo = committed(&sb, &["keep.yml"]);
    let options = sb.options().env("PATH", sb.empty_bin());
    assert_eq!(
        collection_status_with(&repo.join("col"), &options).unwrap(),
        GitState::Unavailable
    );
}

#[test]
fn status_never_rewrites_the_index() {
    let sb = Sandbox::new();
    let repo = committed(&sb, &["keep.yml"]);
    let index = repo.join(".git/index");
    let mtime = || fs::metadata(&index).unwrap().modified().unwrap();
    let before = mtime();
    // Same bytes, new mtime: a refreshing status would rewrite the index now.
    fs::write(repo.join("col/keep.yml"), "info:\n  name: keep.yml\n").unwrap();
    assert!(status_of(&sb, &repo.join("col")).files.is_empty());
    assert_eq!(mtime(), before);
    // The check is live: a plain `git status` does rewrite it here.
    sb.git(&repo, &["status"]);
    assert_ne!(mtime(), before);
}

#[test]
fn a_missing_collection_root_is_an_error_not_unavailable() {
    let sb = Sandbox::new();
    let root = sb.path().join("gone/col");
    match collection_status_with(&root, &sb.options()) {
        Err(GitError::Io(e)) => {
            assert_eq!(e.kind(), std::io::ErrorKind::NotFound);
            assert!(e.to_string().contains("gone/col"), "{e}");
        }
        other => panic!("expected an Io error naming the root, got {other:?}"),
    }
}

fn files(entries: &[(&str, FileStatus)]) -> BTreeMap<PathBuf, FileStatus> {
    entries
        .iter()
        .map(|(path, status)| (PathBuf::from(path), status.clone()))
        .collect()
}

fn append(path: &Path, text: &str) {
    let mut old = fs::read_to_string(path).expect("read");
    old.push_str(text);
    fs::write(path, old).expect("write");
}

#[test]
fn every_kind_in_a_subdirectory_collection_is_collection_relative() {
    let sb = Sandbox::new();
    let repo = committed(
        &sb,
        &["mod.yml", "del.yml", "ren.yml", "keep.yml", "sp ace.yml"],
    );
    append(&repo.join("col/mod.yml"), "changed: true\n");
    append(&repo.join("col/sp ace.yml"), "changed: true\n");
    fs::remove_file(repo.join("col/del.yml")).unwrap();
    // `git mv` needs the destination directory to exist.
    write(&repo, "col/sub/untracked.yml", "u\n");
    sb.git(&repo, &["mv", "col/ren.yml", "col/sub/renamed.yml"]);
    write(&repo, "col/staged.yml", "info:\n  name: staged\n");
    sb.git(&repo, &["add", "col/staged.yml"]);
    write(&repo, "other/untracked.txt", "u\n");
    append(&repo.join("other/o.txt"), "o\n");

    let status = status_of(&sb, &repo.join("col"));
    assert_eq!(
        status.files,
        files(&[
            ("del.yml", FileStatus::Deleted),
            ("mod.yml", FileStatus::Modified),
            ("sp ace.yml", FileStatus::Modified),
            ("staged.yml", FileStatus::Added),
            (
                "sub/renamed.yml",
                FileStatus::Renamed {
                    from: PathBuf::from("ren.yml")
                }
            ),
            ("sub/untracked.yml", FileStatus::Untracked),
        ])
    );
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert_eq!(status.head, Some(sb.short_head(&repo)));
    assert!(!status.truncated);
}

#[test]
fn a_collection_at_the_repository_root_sees_every_path() {
    let sb = Sandbox::new();
    let repo = committed(&sb, &["keep.yml"]);
    append(&repo.join("other/o.txt"), "o\n");
    write(&repo, "col/n.yml", "n\n");
    assert_eq!(
        status_of(&sb, &repo).files,
        files(&[
            ("col/n.yml", FileStatus::Untracked),
            ("other/o.txt", FileStatus::Modified)
        ])
    );
}

#[test]
fn a_merge_conflict_is_conflicted() {
    let sb = Sandbox::new();
    let repo = committed(&sb, &["conf.yml"]);
    sb.git(&repo, &["checkout", "-q", "-b", "side"]);
    write(&repo, "col/conf.yml", "side\n");
    sb.commit_all(&repo, "side");
    sb.git(&repo, &["checkout", "-q", "main"]);
    write(&repo, "col/conf.yml", "main\n");
    sb.commit_all(&repo, "main");
    sb.git_fails(&repo, &["merge", "-q", "side"]);
    assert_eq!(
        status_of(&sb, &repo.join("col")).files,
        files(&[("conf.yml", FileStatus::Conflicted)])
    );
}

#[test]
fn staged_then_deleted_is_a_row_gone_from_disk() {
    let sb = Sandbox::new();
    let repo = committed(&sb, &["keep.yml"]);
    write(&repo, "col/new.yml", "n\n");
    sb.git(&repo, &["add", "col/new.yml"]);
    fs::remove_file(repo.join("col/new.yml")).unwrap();
    let status = status_of(&sb, &repo.join("col"));
    assert_eq!(status.files, files(&[("new.yml", FileStatus::Added)]));
    assert_eq!(status.stage[Path::new("new.yml")], StageState::StagedGone);
}

#[test]
fn modified_staged_then_deleted_stays_modified() {
    let sb = Sandbox::new();
    let repo = committed(&sb, &["mod.yml"]);
    append(&repo.join("col/mod.yml"), "x: 1\n");
    sb.git(&repo, &["add", "col/mod.yml"]);
    fs::remove_file(repo.join("col/mod.yml")).unwrap();
    assert_eq!(
        status_of(&sb, &repo.join("col")).files,
        files(&[("mod.yml", FileStatus::Modified)])
    );
}

#[test]
fn a_staged_deletion_is_deleted() {
    let sb = Sandbox::new();
    // `git rm` of a directory's last file removes the directory, so keep a second file.
    let repo = committed(&sb, &["del.yml", "keep.yml"]);
    sb.git(&repo, &["rm", "-q", "col/del.yml"]);
    assert_eq!(
        status_of(&sb, &repo.join("col")).files,
        files(&[("del.yml", FileStatus::Deleted)])
    );
}

#[test]
fn removed_from_the_index_but_on_disk_is_modified() {
    let sb = Sandbox::new();
    let repo = committed(&sb, &["keep.yml"]);
    sb.git(&repo, &["rm", "-q", "--cached", "col/keep.yml"]);
    assert_eq!(
        status_of(&sb, &repo.join("col")).files,
        files(&[("keep.yml", FileStatus::Modified)])
    );
}

#[test]
fn a_rename_whose_new_file_is_gone_is_deleted_at_the_old_path_and_added_at_the_new() {
    let sb = Sandbox::new();
    let repo = committed(&sb, &["ren.yml"]);
    sb.git(&repo, &["mv", "col/ren.yml", "col/moved.yml"]);
    fs::remove_file(repo.join("col/moved.yml")).unwrap();
    assert_eq!(
        status_of(&sb, &repo.join("col")).files,
        files(&[
            ("moved.yml", FileStatus::Added),
            ("ren.yml", FileStatus::Deleted)
        ])
    );
}

#[test]
fn an_intent_to_add_rename_is_the_new_path_added_and_the_old_deleted() {
    let sb = Sandbox::new();
    let repo = committed(&sb, &["old.yml", "keep.yml"]);
    fs::rename(repo.join("col/old.yml"), repo.join("col/new.yml")).unwrap();
    sb.git(&repo, &["add", "-N", "col/new.yml"]);
    assert_eq!(
        status_of(&sb, &repo.join("col")).files,
        files(&[
            ("new.yml", FileStatus::Added),
            ("old.yml", FileStatus::Deleted)
        ])
    );
}

#[test]
fn a_worktree_copy_lists_the_add_n_path_added_and_the_origin_modified() {
    let sb = Sandbox::new();
    let repo = committed(&sb, &["old.yml", "keep.yml"]);
    sb.git(&repo, &["config", "status.renames", "copies"]);
    let col = repo.join("col");
    fs::copy(col.join("old.yml"), col.join("cp.yml")).unwrap();
    sb.git(&repo, &["add", "-N", "col/cp.yml"]);
    append(&col.join("old.yml"), "x: 1\n");
    let status = status_of(&sb, &col);
    assert_eq!(
        status.files,
        files(&[
            ("cp.yml", FileStatus::Added),
            ("old.yml", FileStatus::Modified)
        ])
    );
    assert_eq!(status.stage[Path::new("cp.yml")], StageState::Unstaged);
    assert_eq!(status.stage[Path::new("old.yml")], StageState::Unstaged);
    let diff = file_diff_with(&col, Path::new("cp.yml"), &sb.options()).expect("diff");
    assert_eq!(diff.status, Some(FileStatus::Added));
    assert_eq!(diff.old, None);
}

#[test]
fn two_add_n_paths_from_a_removed_file_list_both_and_the_deletion() {
    let sb = Sandbox::new();
    let repo = committed(&sb, &["old.yml", "keep.yml"]);
    sb.git(&repo, &["config", "status.renames", "copies"]);
    let col = repo.join("col");
    fs::copy(col.join("old.yml"), col.join("a.yml")).unwrap();
    fs::rename(col.join("old.yml"), col.join("b.yml")).unwrap();
    sb.git(&repo, &["add", "-N", "col/a.yml", "col/b.yml"]);
    assert_eq!(
        status_of(&sb, &col).files,
        files(&[
            ("a.yml", FileStatus::Added),
            ("b.yml", FileStatus::Added),
            ("old.yml", FileStatus::Deleted)
        ])
    );
}

#[test]
fn a_staged_rename_renamed_again_in_the_worktree_deletes_the_first_origin() {
    let sb = Sandbox::new();
    let repo = committed(&sb, &["a.yml", "keep.yml"]);
    sb.git(&repo, &["mv", "col/a.yml", "col/b.yml"]);
    fs::rename(repo.join("col/b.yml"), repo.join("col/c.yml")).unwrap();
    sb.git(&repo, &["add", "-N", "col/c.yml"]);
    let status = status_of(&sb, &repo.join("col"));
    assert_eq!(
        status.files,
        files(&[
            ("a.yml", FileStatus::Deleted),
            ("b.yml", FileStatus::Added),
            ("c.yml", FileStatus::Added)
        ])
    );
    assert_eq!(status.stage[Path::new("a.yml")], StageState::Staged);
    assert_eq!(status.stage[Path::new("b.yml")], StageState::StagedGone);
    assert_eq!(status.stage[Path::new("c.yml")], StageState::Unstaged);
}

#[test]
fn a_first_origin_recreated_after_two_renames_is_modified() {
    let sb = Sandbox::new();
    let repo = committed(&sb, &["a.yml", "keep.yml"]);
    sb.git(&repo, &["mv", "col/a.yml", "col/b.yml"]);
    fs::rename(repo.join("col/b.yml"), repo.join("col/c.yml")).unwrap();
    sb.git(&repo, &["add", "-N", "col/c.yml"]);
    write(&repo, "col/a.yml", "info:\n  name: back\n");
    let status = status_of(&sb, &repo.join("col"));
    assert_eq!(
        status.files,
        files(&[
            ("a.yml", FileStatus::Modified),
            ("b.yml", FileStatus::Added),
            ("c.yml", FileStatus::Added)
        ])
    );
    assert_eq!(status.stage[Path::new("a.yml")], StageState::Both);
}

#[test]
fn a_staged_edit_renamed_in_the_worktree_stays_indexed_beside_the_new_path() {
    let sb = Sandbox::new();
    let repo = committed(&sb, &["a.yml", "keep.yml"]);
    write(&repo, "col/a.yml", "info:\n  name: edited\n");
    sb.git(&repo, &["add", "col/a.yml"]);
    fs::rename(repo.join("col/a.yml"), repo.join("col/b.yml")).unwrap();
    sb.git(&repo, &["add", "-N", "col/b.yml"]);
    let status = status_of(&sb, &repo.join("col"));
    assert_eq!(
        status.files,
        files(&[
            ("a.yml", FileStatus::Modified),
            ("b.yml", FileStatus::Added)
        ])
    );
    assert_eq!(status.stage[Path::new("a.yml")], StageState::StagedGone);
    assert_eq!(status.stage[Path::new("b.yml")], StageState::Unstaged);
}

#[test]
fn a_staged_add_renamed_in_the_worktree_keeps_a_row_for_each_path() {
    let sb = Sandbox::new();
    let repo = committed(&sb, &["keep.yml"]);
    write(&repo, "col/n.yml", "info:\n  name: n\n");
    sb.git(&repo, &["add", "col/n.yml"]);
    fs::rename(repo.join("col/n.yml"), repo.join("col/m.yml")).unwrap();
    sb.git(&repo, &["add", "-N", "col/m.yml"]);
    let col = repo.join("col");
    assert_eq!(
        status_of(&sb, &col).files,
        files(&[("m.yml", FileStatus::Added), ("n.yml", FileStatus::Added)])
    );
    let diff = file_diff_with(&col, Path::new("m.yml"), &sb.options()).expect("diff");
    assert_eq!(diff.status, Some(FileStatus::Added));
}

#[test]
fn an_unborn_head_has_a_branch_but_no_head() {
    let sb = Sandbox::new();
    let repo = sb.repo("r");
    write(&repo, "col/a.yml", "a\n");
    sb.git(&repo, &["add", "col/a.yml"]);
    write(&repo, "col/b.yml", "b\n");
    let status = status_of(&sb, &repo.join("col"));
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert_eq!(status.head, None);
    assert_eq!(
        status.files,
        files(&[
            ("a.yml", FileStatus::Added),
            ("b.yml", FileStatus::Untracked)
        ])
    );
}

#[test]
fn a_linked_worktree_reports_its_own_changes_and_branch() {
    let sb = Sandbox::new();
    let repo = committed(&sb, &["mod.yml"]);
    sb.git(&repo, &["worktree", "add", "-q", "../wt", "-b", "wtb"]);
    let wt = sb.path().join("wt");
    append(&wt.join("col/mod.yml"), "x: 1\n");
    let status = status_of(&sb, &wt.join("col"));
    assert_eq!(status.files, files(&[("mod.yml", FileStatus::Modified)]));
    assert_eq!(status.branch.as_deref(), Some("wtb"));
    assert!(status_of(&sb, &repo.join("col")).files.is_empty());
}

#[test]
fn more_than_the_cap_is_truncated_to_the_first_paths() {
    let sb = Sandbox::new();
    let repo = committed(&sb, &["keep.yml"]);
    for i in 0..=GIT_FILE_CAP {
        write(&repo, &format!("col/bulk/f{i:05}.yml"), "x\n");
    }
    let status = status_of(&sb, &repo.join("col"));
    assert_eq!(status.files.len(), GIT_FILE_CAP);
    assert!(status.truncated);
    assert_eq!(
        status.files.keys().next(),
        Some(&PathBuf::from("bulk/f00000.yml"))
    );
    assert_eq!(
        status.files.keys().next_back(),
        Some(&PathBuf::from(format!("bulk/f{:05}.yml", GIT_FILE_CAP - 1)))
    );
}

/// A fresh file marked `add -N`, then removed by `gone`, in `col/` of `repo`.
fn intent_to_add(sb: &Sandbox, repo: &Path, name: &str, gone: impl FnOnce(&Path)) {
    write(repo, &format!("col/{name}"), "new\n");
    sb.git(repo, &["add", "-N", &format!("col/{name}")]);
    gone(&repo.join("col").join(name));
}

fn assert_diff_refused(sb: &Sandbox, col: &Path, name: &str) {
    let got = file_diff_with(col, Path::new(name), &sb.options());
    assert!(matches!(got, Err(GitError::Malformed(_))), "{got:?}");
}

#[test]
fn an_add_n_on_disk_is_added_and_unstaged() {
    let sb = Sandbox::new();
    let repo = committed(&sb, &["keep.yml"]);
    intent_to_add(&sb, &repo, "n.yml", |_| ());
    let col = repo.join("col");
    let status = status_of(&sb, &col);
    assert_eq!(status.files, files(&[("n.yml", FileStatus::Added)]));
    assert_eq!(status.stage[Path::new("n.yml")], StageState::Unstaged);
    let diff = file_diff_with(&col, Path::new("n.yml"), &sb.options()).expect("diff");
    assert_eq!(diff.status, Some(FileStatus::Added));
    assert_eq!((diff.old, diff.new.as_deref()), (None, Some("new\n")));
}

#[test]
fn an_add_n_moved_away_leaves_only_the_untracked_path() {
    let sb = Sandbox::new();
    let repo = committed(&sb, &["keep.yml"]);
    intent_to_add(&sb, &repo, "a.yml", |a| {
        fs::rename(a, a.with_file_name("b.yml")).unwrap()
    });
    let status = status_of(&sb, &repo.join("col"));
    assert_eq!(status.files, files(&[("b.yml", FileStatus::Untracked)]));
    assert_eq!(status.stage[Path::new("b.yml")], StageState::Unstaged);
    assert_diff_refused(&sb, &repo.join("col"), "a.yml");
}

#[test]
fn an_add_n_removed_on_an_unborn_branch_is_not_listed() {
    let sb = Sandbox::new();
    let repo = sb.repo("r");
    intent_to_add(&sb, &repo, "a.yml", |a| fs::remove_file(a).unwrap());
    let status = status_of(&sb, &repo.join("col"));
    assert!(status.files.is_empty(), "{:?}", status.files);
    assert_diff_refused(&sb, &repo.join("col"), "a.yml");
}

#[test]
fn an_empty_add_n_moved_and_added_again_leaves_only_the_new_path() {
    let sb = Sandbox::new();
    let repo = committed(&sb, &["keep.yml"]);
    let col = repo.join("col");
    write(&col, "n.yml", "");
    sb.git(&repo, &["add", "-N", "col/n.yml"]);
    fs::rename(col.join("n.yml"), col.join("o.yml")).unwrap();
    sb.git(&repo, &["add", "-N", "col/o.yml"]);
    let status = status_of(&sb, &col);
    assert_eq!(status.files, files(&[("o.yml", FileStatus::Added)]));
    assert_diff_refused(&sb, &col, "n.yml");
}

#[test]
fn a_tracked_empty_file_moved_and_added_with_n_keeps_its_deletion() {
    let sb = Sandbox::new();
    let repo = committed(&sb, &["keep.yml"]);
    let col = repo.join("col");
    write(&col, "e.yml", "");
    sb.commit_all(&repo, "empty");
    fs::rename(col.join("e.yml"), col.join("f.yml")).unwrap();
    sb.git(&repo, &["add", "-N", "col/f.yml"]);
    let status = status_of(&sb, &col);
    assert_eq!(
        status.files,
        files(&[("e.yml", FileStatus::Deleted), ("f.yml", FileStatus::Added)])
    );
}

#[test]
fn a_deleted_tracked_empty_file_stays_deleted() {
    let sb = Sandbox::new();
    let repo = committed(&sb, &["keep.yml"]);
    write(&repo, "col/empty.yml", "");
    sb.commit_all(&repo, "empty");
    fs::remove_file(repo.join("col/empty.yml")).unwrap();
    let status = status_of(&sb, &repo.join("col"));
    assert_eq!(status.files, files(&[("empty.yml", FileStatus::Deleted)]));
    let diff = file_diff_with(&repo.join("col"), Path::new("empty.yml"), &sb.options());
    assert_eq!(diff.expect("diff").status, Some(FileStatus::Deleted));
}

#[test]
fn head_paths_match_a_name_with_a_space_and_a_quote() {
    let sb = Sandbox::new();
    let repo = committed(&sb, &["keep.yml"]);
    let tracked = "sp ace \"q\".yml";
    write(&repo, &format!("col/{tracked}"), "");
    sb.commit_all(&repo, "odd");
    fs::remove_file(repo.join("col").join(tracked)).unwrap();
    intent_to_add(&sb, &repo, "new \"r\".yml", |a| fs::remove_file(a).unwrap());
    let status = status_of(&sb, &repo.join("col"));
    assert_eq!(status.files, files(&[(tracked, FileStatus::Deleted)]));
}

#[test]
fn a_collection_at_the_repository_root_hides_the_same_way() {
    let sb = Sandbox::new();
    let repo = committed(&sb, &["keep.yml"]);
    write(&repo, "col/empty.yml", "");
    sb.commit_all(&repo, "empty");
    fs::remove_file(repo.join("col/empty.yml")).unwrap();
    intent_to_add(&sb, &repo, "a.yml", |a| fs::remove_file(a).unwrap());
    let status = status_of(&sb, &repo);
    assert_eq!(
        status.files,
        files(&[("col/empty.yml", FileStatus::Deleted)])
    );
}

#[test]
fn a_tracked_path_re_added_with_n_then_removed_stays_deleted() {
    let sb = Sandbox::new();
    let repo = committed(&sb, &["a.yml"]);
    sb.git(&repo, &["rm", "-q", "--cached", "col/a.yml"]);
    sb.git(&repo, &["add", "-N", "col/a.yml"]);
    fs::remove_file(repo.join("col/a.yml")).unwrap();
    let status = status_of(&sb, &repo.join("col"));
    assert_eq!(status.files, files(&[("a.yml", FileStatus::Deleted)]));
    assert_eq!(status.stage[Path::new("a.yml")], StageState::Both);
}

/// A committed `n.yml`, dropped from the index and marked `add -N` again, still on disk.
fn deleted_then_added(sb: &Sandbox, repo: &Path) {
    sb.git(repo, &["rm", "-q", "--cached", "col/n.yml"]);
    sb.git(repo, &["add", "-N", "col/n.yml"]);
}

#[test]
fn a_tracked_path_re_added_with_n_on_disk_is_not_deleted() {
    let sb = Sandbox::new();
    let repo = committed(&sb, &["n.yml", "keep.yml"]);
    deleted_then_added(&sb, &repo);
    let col = repo.join("col");
    let status = status_of(&sb, &col);
    assert_eq!(status.files, files(&[("n.yml", FileStatus::Modified)]));
    assert_eq!(status.stage[Path::new("n.yml")], StageState::Both);
    let diff = file_diff_with(&col, Path::new("n.yml"), &sb.options()).expect("diff");
    assert_eq!(diff.status, Some(FileStatus::Modified));
    assert_eq!(diff.new.as_deref(), Some("info:\n  name: n.yml\n"));
}

#[test]
fn a_tracked_path_re_added_with_n_as_a_copy_still_gives_a_status() {
    let sb = Sandbox::new();
    let repo = committed(&sb, &["n.yml", "old.yml"]);
    sb.git(&repo, &["config", "status.renames", "copies"]);
    let col = repo.join("col");
    sb.git(&repo, &["rm", "-q", "--cached", "col/n.yml"]);
    fs::copy(col.join("old.yml"), col.join("n.yml")).unwrap();
    sb.git(&repo, &["add", "-N", "col/n.yml"]);
    append(&col.join("old.yml"), "x: 1\n");
    let status = status_of(&sb, &col);
    assert_eq!(
        status.files,
        files(&[
            ("n.yml", FileStatus::Modified),
            ("old.yml", FileStatus::Modified)
        ])
    );
    assert_eq!(status.stage[Path::new("n.yml")], StageState::Both);
}

#[test]
fn a_tracked_path_re_added_with_n_as_a_rename_lists_the_origin_deleted() {
    let sb = Sandbox::new();
    let repo = committed(&sb, &["n.yml", "old.yml"]);
    let col = repo.join("col");
    sb.git(&repo, &["rm", "-q", "--cached", "col/n.yml"]);
    fs::rename(col.join("old.yml"), col.join("n.yml")).unwrap();
    sb.git(&repo, &["add", "-N", "col/n.yml"]);
    let status = status_of(&sb, &col);
    assert_eq!(
        status.files,
        files(&[
            ("n.yml", FileStatus::Modified),
            ("old.yml", FileStatus::Deleted)
        ])
    );
    let diff = file_diff_with(&col, Path::new("n.yml"), &sb.options()).expect("diff");
    assert_eq!(diff.status, Some(FileStatus::Modified));
    assert_eq!(diff.old.as_deref(), Some("info:\n  name: n.yml\n"));
    assert_eq!(diff.new.as_deref(), Some("info:\n  name: old.yml\n"));
}

#[cfg(unix)]
mod head_lookup {
    use super::*;

    /// Options whose git logs each command line to `calls`, and fails `ls-tree` when asked.
    fn logged(sb: &Sandbox, ls_tree_fails: bool) -> (GitOptions, PathBuf) {
        let calls = sb.path().join("calls");
        let refuse = if ls_tree_fails { "exit 1" } else { ":" };
        let body = format!(
            "#!/bin/sh\necho \"$*\" >> '{log}'\ncase \"$*\" in *ls-tree*) {refuse} ;; esac\n\
             exec '{git}' \"$@\"\n",
            log = calls.display(),
            git = git_support::real_git().display(),
        );
        let bin = sb.fake_git("logged", &body);
        (sb.options().env("PATH", path_with(&bin)), calls)
    }

    fn lookups(calls: &Path) -> usize {
        fs::read_to_string(calls)
            .unwrap_or_default()
            .lines()
            .filter(|line| line.contains("ls-tree"))
            .count()
    }

    #[test]
    fn a_candidate_costs_one_lookup_however_many_there_are() {
        let sb = Sandbox::new();
        let repo = committed(&sb, &["keep.yml"]);
        for name in ["a.yml", "b.yml"] {
            intent_to_add(&sb, &repo, name, |a| fs::remove_file(a).unwrap());
        }
        let (options, calls) = logged(&sb, false);
        let state = collection_status_with(&repo.join("col"), &options).expect("status");
        assert!(repo_status(state).files.is_empty());
        assert_eq!(lookups(&calls), 1);
    }

    #[test]
    fn no_candidate_and_an_unborn_branch_make_no_lookup() {
        let sb = Sandbox::new();
        let repo = committed(&sb, &["a.yml"]);
        fs::remove_file(repo.join("col/a.yml")).unwrap();
        let (options, calls) = logged(&sb, false);
        collection_status_with(&repo.join("col"), &options).expect("status");

        let unborn = sb.repo("u");
        intent_to_add(&sb, &unborn, "a.yml", |a| fs::remove_file(a).unwrap());
        collection_status_with(&unborn.join("col"), &options).expect("status");
        assert_eq!(lookups(&calls), 0);
    }

    #[test]
    fn a_failed_lookup_shows_the_row_and_the_status_succeeds() {
        let sb = Sandbox::new();
        let repo = committed(&sb, &["keep.yml"]);
        intent_to_add(&sb, &repo, "a.yml", |a| fs::remove_file(a).unwrap());
        let (options, calls) = logged(&sb, true);
        let state = collection_status_with(&repo.join("col"), &options).expect("status");
        assert_eq!(
            repo_status(state).files,
            files(&[("a.yml", FileStatus::Deleted)])
        );
        assert_eq!(lookups(&calls), 1);
    }
}
