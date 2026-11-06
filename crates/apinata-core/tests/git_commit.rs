//! `stage`, `unstage` and `commit` against real repositories built by the git CLI in a sandbox.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

use crate::git_support;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use apinata_core::{
    CommitError, CommitOutcome, FileStatus, GitError, GitOptions, GitState, StageState,
    collection_status_with, commit, stage, unstage,
};
use git_support::{Sandbox, assert_no_ambient_identity, write};

/// Repository `r` with `col/{a,b,c,d}.yml` and `other/o.txt` committed; returns (repo, col).
fn committed(sb: &Sandbox) -> (PathBuf, PathBuf) {
    let repo = sb.repo("r");
    for name in ["a", "b", "c", "d"] {
        write(&repo, &format!("col/{name}.yml"), &format!("{name}\n"));
    }
    write(&repo, "other/o.txt", "o\n");
    sb.commit_all(&repo, "base");
    let col = repo.join("col");
    (repo, col)
}

fn changes(sb: &Sandbox, col: &Path) -> BTreeMap<String, (FileStatus, StageState)> {
    match collection_status_with(col, &sb.options()).expect("status") {
        GitState::Repo(s) => s
            .files
            .into_iter()
            .map(|(path, f)| {
                let state = s.stage[&path];
                (path.to_string_lossy().into_owned(), (f, state))
            })
            .collect(),
        other => panic!("{other:?}"),
    }
}

fn state(sb: &Sandbox, col: &Path, rel: &str) -> StageState {
    changes(sb, col)
        .get(rel)
        .unwrap_or_else(|| panic!("{rel} unchanged"))
        .1
}

#[test]
fn stage_and_unstage_modified_untracked_and_deleted_files() {
    let sb = Sandbox::new();
    let (_repo, col) = committed(&sb);
    write(&col, "a.yml", "changed\n");
    write(&col, "new.yml", "n\n");
    fs::remove_file(col.join("b.yml")).unwrap();
    let all = ["a.yml", "new.yml", "b.yml"];
    for rel in all {
        assert_eq!(state(&sb, &col, rel), StageState::Unstaged, "{rel}");
    }
    stage(&col, &all, &sb.options()).expect("stage");
    for rel in all {
        assert_eq!(state(&sb, &col, rel), StageState::Staged, "{rel}");
    }
    unstage(&col, &all, &sb.options()).expect("unstage");
    for rel in all {
        assert_eq!(state(&sb, &col, rel), StageState::Unstaged, "{rel}");
    }
    assert_eq!(changes(&sb, &col)["new.yml"].0, FileStatus::Untracked);
}

#[test]
fn a_worktree_rename_stages_whole_through_its_two_paths() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    fs::rename(col.join("c.yml"), col.join("c2.yml")).unwrap();
    sb.git(&repo, &["add", "-N", "col/c2.yml"]);
    let before = changes(&sb, &col);
    assert_eq!(before["c2.yml"], (FileStatus::Added, StageState::Unstaged));
    assert_eq!(before["c.yml"], (FileStatus::Deleted, StageState::Unstaged));
    stage(&col, &["c2.yml", "c.yml"], &sb.options()).expect("stage");
    let after = changes(&sb, &col);
    assert_eq!(after.len(), 1, "{after:?}");
    assert_eq!(
        after["c2.yml"],
        (
            FileStatus::Renamed {
                from: PathBuf::from("c.yml")
            },
            StageState::Staged
        )
    );
}

#[test]
fn a_chain_of_two_renames_stages_through_the_new_path_alone() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    sb.git(&repo, &["mv", "col/c.yml", "col/b2.yml"]);
    fs::rename(col.join("b2.yml"), col.join("c3.yml")).unwrap();
    sb.git(&repo, &["add", "-N", "col/c3.yml"]);
    assert_eq!(state(&sb, &col, "c.yml"), StageState::Staged);
    stage(&col, &["c3.yml"], &sb.options()).expect("stage");
    assert_eq!(state(&sb, &col, "c3.yml"), StageState::Staged);
}

#[test]
fn unstaging_a_rename_also_resets_the_old_name() {
    let sb = Sandbox::new();
    let (_repo, col) = committed(&sb);
    fs::rename(col.join("c.yml"), col.join("c2.yml")).unwrap();
    stage(&col, &["c.yml", "c2.yml"], &sb.options()).expect("stage");
    let before = changes(&sb, &col);
    assert_eq!(
        before["c2.yml"],
        (
            FileStatus::Renamed {
                from: PathBuf::from("c.yml")
            },
            StageState::Staged
        )
    );
    unstage(&col, &["c2.yml"], &sb.options()).expect("unstage");
    let after = changes(&sb, &col);
    assert_eq!(
        after["c2.yml"],
        (FileStatus::Untracked, StageState::Unstaged)
    );
    assert_eq!(after["c.yml"], (FileStatus::Deleted, StageState::Unstaged));
}

#[test]
fn an_add_n_moved_away_stages_and_commits_through_its_new_path() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    write(&col, "n.yml", "n\n");
    sb.git(&repo, &["add", "-N", "col/n.yml"]);
    fs::rename(col.join("n.yml"), col.join("m.yml")).unwrap();
    stage(&col, &["m.yml"], &sb.options()).expect("stage");
    commit(&col, "m", &sb.options()).expect("commit");
    assert!(changes(&sb, &col).is_empty(), "{:?}", changes(&sb, &col));
    assert_eq!(
        sb.git(&repo, &["ls-tree", "--name-only", "HEAD", "col/m.yml"])
            .trim(),
        "col/m.yml"
    );
}

/// `a.yml` renamed to `gone.yml` in the index and removed from disk: `RD`, or `D.` for `a.yml`.
fn renamed_then_removed(sb: &Sandbox, repo: &Path, col: &Path) {
    sb.git(repo, &["mv", "col/a.yml", "col/gone.yml"]);
    fs::remove_file(col.join("gone.yml")).unwrap();
}

#[test]
fn a_renamed_away_origin_is_a_staged_deletion() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    renamed_then_removed(&sb, &repo, &col);
    let now = changes(&sb, &col);
    assert_eq!(now.len(), 2, "{now:?}");
    assert_eq!(now["a.yml"], (FileStatus::Deleted, StageState::Staged));
    assert_eq!(now["gone.yml"], (FileStatus::Added, StageState::StagedGone));
}

#[test]
fn a_renamed_away_origin_back_on_disk_stages_as_modified() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    renamed_then_removed(&sb, &repo, &col);
    write(&col, "a.yml", "back\n");
    assert_eq!(
        changes(&sb, &col)["a.yml"],
        (FileStatus::Modified, StageState::Both)
    );
    stage(&col, &["a.yml"], &sb.options()).expect("stage");
    assert_eq!(state(&sb, &col, "a.yml"), StageState::Staged);
}

#[test]
fn a_rename_moved_on_and_staged_leaves_a_staged_deletion_at_its_origin() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    sb.git(&repo, &["mv", "col/c.yml", "col/b2.yml"]);
    fs::rename(col.join("b2.yml"), col.join("d2.yml")).unwrap();
    sb.git(&repo, &["add", "-N", "col/d2.yml"]);
    sb.git(&repo, &["add", "-A", "--", "col/d2.yml"]);
    let now = changes(&sb, &col);
    assert_eq!(now.len(), 3, "{now:?}");
    assert_eq!(now["b2.yml"], (FileStatus::Added, StageState::StagedGone));
    assert_eq!(now["c.yml"], (FileStatus::Deleted, StageState::Staged));
    assert_eq!(now["d2.yml"], (FileStatus::Added, StageState::Staged));
}

#[test]
fn staging_a_renamed_away_origin_does_not_block_the_other_paths() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    renamed_then_removed(&sb, &repo, &col);
    write(&col, "b.yml", "changed\n");
    stage(&col, &["a.yml", "b.yml"], &sb.options()).expect("stage");
    assert_eq!(state(&sb, &col, "b.yml"), StageState::Staged);
    assert_eq!(state(&sb, &col, "a.yml"), StageState::Staged);
}

#[test]
fn staging_a_plain_staged_deletion_does_not_block_the_other_paths() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    sb.git(&repo, &["rm", "-q", "col/a.yml"]);
    write(&col, "b.yml", "changed\n");
    stage(&col, &["a.yml", "b.yml"], &sb.options()).expect("stage");
    assert_eq!(state(&sb, &col, "b.yml"), StageState::Staged);
    assert_eq!(
        changes(&sb, &col)["a.yml"],
        (FileStatus::Deleted, StageState::Staged)
    );
}

#[test]
fn a_staged_deletion_under_a_file_now_at_its_parent_does_not_block_the_other_paths() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    write(&col, "sub/x.yml", "x\n");
    sb.commit_all(&repo, "sub");
    sb.git(&repo, &["rm", "-q", "col/sub/x.yml"]);
    write(&col, "sub", "a file now\n");
    write(&col, "b.yml", "changed\n");
    stage(&col, &["sub/x.yml", "b.yml"], &sb.options()).expect("stage");
    assert_eq!(state(&sb, &col, "b.yml"), StageState::Staged);
    assert_eq!(state(&sb, &col, "sub/x.yml"), StageState::Staged);
}

#[test]
fn staging_only_staged_deletions_is_a_no_op() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    sb.git(&repo, &["rm", "-q", "col/a.yml"]);
    let before = changes(&sb, &col);
    stage(&col, &["a.yml"], &sb.options()).expect("stage");
    assert_eq!(changes(&sb, &col), before);
}

#[test]
fn a_mistyped_path_still_fails_and_stages_nothing() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    sb.git(&repo, &["rm", "-q", "col/a.yml"]);
    write(&col, "b.yml", "changed\n");
    let err = stage(&col, &["typo.yml", "a.yml", "b.yml"], &sb.options()).unwrap_err();
    assert!(matches!(err, GitError::Failed { .. }), "{err:?}");
    assert_eq!(state(&sb, &col, "b.yml"), StageState::Unstaged);
}

#[test]
fn a_deletion_with_a_staged_change_still_reaches_git() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    write(&col, "a.yml", "changed\n");
    sb.git(&repo, &["add", "col/a.yml"]);
    fs::remove_file(col.join("a.yml")).unwrap();
    assert_eq!(state(&sb, &col, "a.yml"), StageState::StagedGone);
    stage(&col, &["a.yml"], &sb.options()).expect("stage");
    assert_eq!(state(&sb, &col, "a.yml"), StageState::Staged);
}

#[cfg(unix)]
#[test]
fn stage_reads_the_status_only_for_a_path_gone_from_disk() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    write(&col, "b.yml", "changed\n");
    let calls = sb.path().join("calls");
    let body = format!(
        "#!/bin/sh\necho \"$*\" >> '{}'\nexec '{}' \"$@\"\n",
        calls.display(),
        git_support::real_git().display()
    );
    let bin = sb.fake_git("logged", &body);
    let options = sb.options().env("PATH", git_support::path_with(&bin));
    let statuses = || {
        fs::read_to_string(&calls)
            .unwrap()
            .lines()
            .filter(|line| line.contains("status"))
            .count()
    };
    stage(&col, &["b.yml"], &options).expect("stage");
    assert_eq!(statuses(), 0);
    sb.git(&repo, &["rm", "-q", "col/a.yml"]);
    stage(&col, &["a.yml"], &options).expect("stage");
    assert_eq!(statuses(), 1);
}

#[test]
fn a_file_changed_again_after_staging_is_both_until_restaged() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    write(&col, "a.yml", "one\n");
    stage(&col, &["a.yml"], &sb.options()).expect("stage");
    write(&col, "a.yml", "two\n");
    assert_eq!(state(&sb, &col, "a.yml"), StageState::Both);
    stage(&col, &["a.yml"], &sb.options()).expect("restage");
    assert_eq!(state(&sb, &col, "a.yml"), StageState::Staged);

    sb.git(&repo, &["rm", "-q", "--cached", "col/b.yml"]);
    let changes = changes(&sb, &col);
    assert_eq!(changes["b.yml"], (FileStatus::Modified, StageState::Both));
}

#[test]
fn an_unborn_repository_stages_and_unstages() {
    let sb = Sandbox::new();
    let repo = sb.repo("r");
    write(&repo, "col/a.yml", "a\n");
    let col = repo.join("col");
    stage(&col, &["a.yml"], &sb.options()).expect("stage");
    assert_eq!(
        changes(&sb, &col)["a.yml"],
        (FileStatus::Added, StageState::Staged)
    );
    unstage(&col, &["a.yml"], &sb.options()).expect("unstage");
    assert_eq!(
        changes(&sb, &col)["a.yml"],
        (FileStatus::Untracked, StageState::Unstaged)
    );
}

#[test]
fn a_nested_collection_stages_only_itself() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    write(&col, "a.yml", "changed\n");
    write(&repo, "other/o.txt", "changed\n");
    stage(&col, &["."], &sb.options()).expect("stage all");
    assert_eq!(
        sb.git(&repo, &["diff", "--cached", "--name-only"]),
        "col/a.yml\n"
    );
    unstage(&col, &["."], &sb.options()).expect("unstage all");
    assert_eq!(sb.git(&repo, &["diff", "--cached", "--name-only"]), "");
}

#[cfg(unix)]
#[test]
fn pathspec_magic_in_a_name_is_literal() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    write(&col, "b.yml", "changed\n");
    write(&col, "*", "star\n");
    write(&col, "sp ace.yml", "s\n");
    write(&col, ":(top)x", "t\n");
    stage(&col, &["*", "sp ace.yml", ":(top)x"], &sb.options()).expect("stage");
    let staged = sb.git(&repo, &["diff", "--cached", "--name-only"]);
    let mut names: Vec<&str> = staged.lines().collect();
    names.sort();
    assert_eq!(names, ["col/*", "col/:(top)x", "col/sp ace.yml"]);
}

#[test]
fn escaping_or_empty_paths_are_malformed_and_run_nothing() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    write(&col, "a.yml", "changed\n");
    let outside = repo.join("other/o.txt");
    let absolute = outside.to_string_lossy().into_owned();
    for bad in ["..", "../other/o.txt", "sub/../../x", "", absolute.as_str()] {
        for result in [
            stage(&col, &["a.yml", bad], &sb.options()),
            unstage(&col, &["a.yml", bad], &sb.options()),
        ] {
            assert!(matches!(result, Err(GitError::Malformed(_))), "{bad:?}");
        }
    }
    assert_eq!(sb.git(&repo, &["diff", "--cached", "--name-only"]), "");
}

#[test]
fn no_paths_is_a_no_op_and_a_non_repository_fails() {
    let sb = Sandbox::new();
    let (_repo, col) = committed(&sb);
    stage(&col, &[] as &[&str], &sb.options()).expect("no-op");
    unstage(&col, &[] as &[&str], &sb.options()).expect("no-op");

    let bare = sb.path().join("plain");
    fs::create_dir_all(&bare).unwrap();
    let err = stage(&bare, &["a.yml"], &sb.options()).unwrap_err();
    assert!(matches!(err, GitError::Failed { .. }), "{err:?}");
    let err = unstage(&bare, &["a.yml"], &sb.options()).unwrap_err();
    assert!(matches!(err, GitError::Failed { .. }), "{err:?}");
}

fn commit_err(r: Result<apinata_core::CommitOutcome, CommitError>) -> CommitError {
    r.expect_err("commit must be refused")
}

#[test]
fn an_empty_message_is_refused_before_anything_runs() {
    let sb = Sandbox::new();
    let missing = sb.path().join("does-not-exist");
    for msg in ["", "  \n\t\n"] {
        let err = commit_err(commit(&missing, msg, &sb.options()));
        assert!(matches!(err, CommitError::EmptyMessage), "{err:?}");
    }
}

#[test]
fn nothing_staged_in_the_collection_is_refused() {
    let sb = Sandbox::new();
    let (_repo, col) = committed(&sb);
    let err = commit_err(commit(&col, "msg", &sb.options()));
    assert!(matches!(err, CommitError::NothingStaged), "{err:?}");
    write(&col, "a.yml", "unstaged only\n");
    let err = commit_err(commit(&col, "msg", &sb.options()));
    assert!(matches!(err, CommitError::NothingStaged), "{err:?}");
}

#[test]
fn conflicted_files_are_refused() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    sb.git(&repo, &["checkout", "-q", "-b", "side"]);
    write(&col, "a.yml", "side\n");
    sb.commit_all(&repo, "side");
    sb.git(&repo, &["checkout", "-q", "main"]);
    write(&col, "a.yml", "main\n");
    sb.commit_all(&repo, "main");
    sb.git_fails(&repo, &["merge", "side"]);
    write(&col, "b.yml", "staged\n");
    stage(&col, &["b.yml"], &sb.options()).unwrap();
    let err = commit_err(commit(&col, "msg", &sb.options()));
    assert!(matches!(err, CommitError::Conflicted), "{err:?}");
}

#[test]
fn files_staged_outside_the_collection_are_refused_and_listed() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    write(&col, "a.yml", "mine\n");
    write(&repo, "other/o.txt", "theirs\n");
    sb.git(&repo, &["add", "col/a.yml", "other/o.txt"]);
    match commit_err(commit(&col, "msg", &sb.options())) {
        CommitError::StagedOutside { paths, count } => {
            assert_eq!(paths, [PathBuf::from("other/o.txt")]);
            assert_eq!(count, 1);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(sb.git(&repo, &["rev-list", "--count", "HEAD"]), "1\n");
}

#[test]
fn a_rename_out_of_the_collection_is_staged_outside() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    sb.git(&repo, &["mv", "col/a.yml", "other/a.yml"]);
    match commit_err(commit(&col, "msg", &sb.options())) {
        CommitError::StagedOutside { paths, .. } => {
            assert_eq!(paths, [PathBuf::from("other/a.yml")]);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_staged_outside_list_is_capped_at_twenty_with_a_total() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    for i in 0..25 {
        write(&repo, &format!("other/f{i:02}.txt"), "x\n");
    }
    sb.git(&repo, &["add", "other"]);
    match commit_err(commit(&col, "msg", &sb.options())) {
        CommitError::StagedOutside { paths, count } => {
            assert_eq!((paths.len(), count), (20, 25));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_collection_commit_records_only_the_collection() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    write(&col, "a.yml", "changed\n");
    write(&col, "b.yml", "left unstaged\n");
    stage(&col, &["a.yml"], &sb.options()).unwrap();
    let out = commit(&col, "update a\n\nlonger body", &sb.options()).expect("commit");
    assert_eq!(out.subject, "update a");
    assert_eq!(out.short_id, sb.short_head(&repo));
    assert_eq!(
        sb.git(&repo, &["show", "--name-only", "--format=", "HEAD"]),
        "col/a.yml\n"
    );
    assert_eq!(state(&sb, &col, "b.yml"), StageState::Unstaged);
}

#[test]
fn the_first_commit_of_an_unborn_repository_works() {
    let sb = Sandbox::new();
    let repo = sb.repo("r");
    write(&repo, "col/a.yml", "a\n");
    let col = repo.join("col");
    stage(&col, &["."], &sb.options()).unwrap();
    let out = commit(&col, "initial", &sb.options()).expect("commit");
    assert_eq!(out.short_id, sb.short_head(&repo));
}

/// Repository with the config identity of a person; the sandbox sets no identity variables.
fn repo_with_config_identity(sb: &Sandbox) -> (PathBuf, PathBuf) {
    assert_no_ambient_identity();
    let (repo, col) = committed_without_env_identity(sb);
    sb.git(&repo, &["config", "user.name", "Repo Person"]);
    sb.git(&repo, &["config", "user.email", "repo@example.com"]);
    (repo, col)
}

/// `committed` for a sandbox that has no identity variables: the base commit gets an
/// identity from a throwaway config that is removed again afterwards.
fn committed_without_env_identity(sb: &Sandbox) -> (PathBuf, PathBuf) {
    let repo = sb.repo("r");
    for name in ["a", "b"] {
        write(&repo, &format!("col/{name}.yml"), &format!("{name}\n"));
    }
    sb.git(&repo, &["config", "user.name", "Base"]);
    sb.git(&repo, &["config", "user.email", "base@example.com"]);
    sb.commit_all(&repo, "base");
    sb.git(&repo, &["config", "--unset", "user.name"]);
    sb.git(&repo, &["config", "--unset", "user.email"]);
    let col = repo.join("col");
    (repo, col)
}

#[test]
fn the_message_is_stored_as_git_cleans_it_with_no_trailer() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    write(&col, "a.yml", "changed\n");
    stage(&col, &["a.yml"], &sb.options()).unwrap();
    let out = commit(&col, "-fix: x  \n# keep this  \n\n\n", &sb.options()).expect("commit");
    assert_eq!(out.subject, "-fix: x");
    let raw = sb.git(&repo, &["cat-file", "commit", "HEAD"]);
    let body = raw.split_once("\n\n").expect("body").1;
    assert_eq!(body, "-fix: x\n# keep this\n");
    assert!(!raw.contains("Signed-off-by") && !raw.contains("Co-authored-by"));
}

#[test]
fn the_author_is_the_repo_config_identity() {
    let sb = Sandbox::without_identity();
    let (repo, col) = repo_with_config_identity(&sb);
    write(&col, "a.yml", "changed\n");
    stage(&col, &["a.yml"], &sb.options()).unwrap();
    commit(&col, "msg", &sb.options()).expect("commit");
    assert_eq!(
        sb.git(&repo, &["log", "-1", "--format=%an <%ae> | %cn <%ce>"]),
        "Repo Person <repo@example.com> | Repo Person <repo@example.com>\n"
    );
}

#[test]
fn no_identity_fails_with_gits_text_and_commits_nothing() {
    let sb = Sandbox::without_identity();
    assert_no_ambient_identity();
    let (repo, col) = committed_without_env_identity(&sb);
    write(&col, "a.yml", "changed\n");
    stage(&col, &["a.yml"], &sb.options()).unwrap();
    match commit_err(commit(&col, "msg", &sb.options())) {
        CommitError::Failed { output } => {
            assert!(
                output.contains("Author identity unknown") || output.contains("empty ident"),
                "{output}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(sb.git(&repo, &["rev-list", "--count", "HEAD"]), "1\n");
}

#[cfg(unix)]
fn hook(repo: &Path, name: &str, body: &str) {
    // A child shell writes the script so no descriptor of this process is open on it when
    // another test forks (ETXTBSY), as `Sandbox::fake_git` does.
    let path = repo.join(".git/hooks").join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg("printf '%s\\n' \"$1\" > \"$2\" && chmod 755 \"$2\"")
        .arg("sh")
        .arg(format!("#!/bin/sh\n{body}"))
        .arg(&path)
        .status()
        .expect("sh runs");
    assert!(status.success(), "hook written");
}

#[cfg(unix)]
#[test]
fn a_failing_pre_commit_hook_blocks_the_commit_and_its_output_is_returned() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    hook(
        &repo,
        "pre-commit",
        "echo 'lint says no' >&2\necho 'to stdout'\nexit 1",
    );
    write(&col, "a.yml", "changed\n");
    stage(&col, &["a.yml"], &sb.options()).unwrap();
    match commit_err(commit(&col, "msg", &sb.options())) {
        CommitError::Failed { output } => {
            assert!(
                output.contains("lint says no") && output.contains("to stdout"),
                "{output}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(sb.git(&repo, &["rev-list", "--count", "HEAD"]), "1\n");
    assert_eq!(state(&sb, &col, "a.yml"), StageState::Staged);
}

#[cfg(unix)]
#[test]
fn a_passing_commit_msg_hook_runs_and_the_commit_lands() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    hook(&repo, "commit-msg", ": > .git/commit-msg-ran");
    write(&col, "a.yml", "changed\n");
    stage(&col, &["a.yml"], &sb.options()).unwrap();
    commit(&col, "msg", &sb.options()).expect("commit");
    assert!(repo.join(".git/commit-msg-ran").exists());
    assert_eq!(sb.git(&repo, &["rev-list", "--count", "HEAD"]), "2\n");
}

#[cfg(unix)]
#[test]
fn a_held_index_lock_is_busy_naming_the_lock() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    write(&col, "a.yml", "changed\n");
    stage(&col, &["a.yml"], &sb.options()).unwrap();
    let held = repo.join(".git/index.lock");
    fs::write(&held, "").unwrap();
    match commit_err(commit(&col, "msg", &sb.options())) {
        CommitError::Git(GitError::Busy { lock }) => {
            assert_eq!(lock.canonicalize().unwrap(), held.canonicalize().unwrap())
        }
        other => panic!("{other:?}"),
    }
    assert!(held.exists(), "the lock was deleted");
}

#[cfg(unix)]
#[test]
fn a_held_head_lock_is_busy_and_commits_nothing() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    write(&col, "a.yml", "changed\n");
    stage(&col, &["a.yml"], &sb.options()).unwrap();
    let held = repo.join(".git/HEAD.lock");
    fs::write(&held, "").unwrap();
    match commit_err(commit(&col, "msg", &sb.options())) {
        CommitError::Git(GitError::Busy { lock }) => {
            assert_eq!(lock.canonicalize().unwrap(), held.canonicalize().unwrap())
        }
        other => panic!("{other:?}"),
    }
    assert!(held.exists(), "the lock was deleted");
    assert_eq!(sb.git(&repo, &["rev-list", "--count", "HEAD"]), "1\n");
}

#[cfg(unix)]
mod pipes {
    use super::*;
    use std::process::Command;
    use std::sync::mpsc::channel;
    use std::thread;
    use std::time::Duration;

    fn pipe_over(path: &Path) {
        let _ = fs::remove_file(path);
        let made = Command::new("mkfifo").arg(path).status().expect("mkfifo");
        assert!(made.success(), "mkfifo failed");
    }

    // Runs `job` on its own thread and fails the test if it outlives `limit`. Before failing it
    // opens `pipe` read-write until the job ends, so each git stuck opening the pipe reads end of
    // file at once and none outlives the sandbox.
    fn bounded<T: Send + 'static>(
        pipe: &Path,
        limit: Duration,
        job: impl FnOnce() -> T + Send + 'static,
    ) -> T {
        let (done, got) = channel();
        thread::spawn(move || {
            let _ = done.send(job());
        });
        if let Ok(value) = got.recv_timeout(limit) {
            return value;
        }
        for _ in 0..600 {
            drop(fs::OpenOptions::new().read(true).write(true).open(pipe));
            if got.recv_timeout(Duration::from_millis(100)).is_ok() {
                break;
            }
        }
        panic!(
            "the job outlived {limit:?} with a pipe at {}",
            pipe.display()
        );
    }

    // A commit under a limit past anything its own timeouts allow.
    fn commit_bounded(
        col: &Path,
        pipe: &Path,
        options: GitOptions,
    ) -> Result<CommitOutcome, CommitError> {
        let col = col.to_path_buf();
        bounded(pipe, Duration::from_secs(30), move || {
            commit(&col, "m", &options)
        })
    }

    #[test]
    fn a_pipe_over_a_staged_file_commits_the_staged_copy() {
        let sb = Sandbox::new();
        let (repo, col) = committed(&sb);
        write(&col, "a.yml", "staged\n");
        stage(&col, &["a.yml"], &sb.options()).expect("stage");
        let pipe = col.join("a.yml");
        pipe_over(&pipe);
        commit_bounded(&col, &pipe, sb.options()).expect("commits");
        assert_eq!(sb.git(&repo, &["show", "HEAD:col/a.yml"]), "staged\n");
    }

    #[test]
    fn a_pipe_git_itself_opens_ends_a_commit_at_the_time_limit() {
        let sb = Sandbox::new();
        let (repo, col) = committed(&sb);
        write(&col, "a.yml", "staged\n");
        stage(&col, &["a.yml"], &sb.options()).expect("stage");
        // Git opens attributes only to read a file's content, which it does when the stat no
        // longer matches the index; without this a slow stage leaves nothing to read.
        let moved = std::time::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        fs::File::options()
            .write(true)
            .open(col.join("a.yml"))
            .and_then(|f| f.set_modified(moved))
            .expect("move the mtime");
        let pipe = repo.join(".gitattributes");
        pipe_over(&pipe);
        let options = sb.options().timeout(Duration::from_secs(2));
        let error = commit_bounded(&col, &pipe, options).expect_err("git waits on the pipe");
        assert!(
            matches!(error, CommitError::Git(GitError::Timeout)),
            "{error}"
        );
        assert_eq!(
            error.to_string(),
            "git did not finish within the time limit"
        );
        assert!(
            !repo.join(".git/index.lock").exists(),
            "index.lock left behind"
        );
    }

    // Git waits on a pipe where it reads attributes or ignore rules; the stage ends at the
    // configured limit, not the longer one a write is given.
    fn stage_under_a_pipe(rel: &str) -> Result<(), GitError> {
        let sb = Sandbox::new();
        let (repo, col) = committed(&sb);
        write(&col, "a.yml", "changed\n");
        let pipe = repo.join(rel);
        pipe_over(&pipe);
        let options = sb.options().timeout(Duration::from_secs(1));
        bounded(&pipe, Duration::from_secs(15), move || {
            stage(&col, &["a.yml"], &options)
        })
    }

    #[test]
    fn staging_under_a_pipe_gitattributes_ends_at_the_time_limit() {
        let error = stage_under_a_pipe(".gitattributes").expect_err("git waits on the pipe");
        assert!(matches!(error, GitError::Timeout), "{error}");
    }

    #[test]
    fn staging_under_a_pipe_gitignore_ends_at_the_time_limit() {
        let error = stage_under_a_pipe(".gitignore").expect_err("git waits on the pipe");
        assert!(matches!(error, GitError::Timeout), "{error}");
    }

    #[test]
    fn staging_under_a_pipe_info_exclude_or_attributes_ends_at_the_time_limit() {
        for rel in [".git/info/exclude", ".git/info/attributes"] {
            let error = stage_under_a_pipe(rel).expect_err("git waits on the pipe");
            assert!(matches!(error, GitError::Timeout), "{rel}: {error}");
        }
    }

    // Staging a directory walks it, so a pipe in a folder below the path counts too.
    #[test]
    fn staging_a_folder_with_a_pipe_gitignore_below_ends_at_the_time_limit() {
        let sb = Sandbox::new();
        let (_repo, col) = committed(&sb);
        write(&col, "d/e/b.yml", "new\n");
        let pipe = col.join("d/e/.gitignore");
        pipe_over(&pipe);
        let options = sb.options().timeout(Duration::from_secs(1));
        let staged = bounded(&pipe, Duration::from_secs(15), move || {
            stage(&col, &["."], &options)
        });
        let error = staged.expect_err("git waits on the pipe");
        assert!(matches!(error, GitError::Timeout), "{error}");
    }

    #[test]
    fn staging_a_folder_with_a_pipe_gitattributes_below_ends_at_the_time_limit() {
        let sb = Sandbox::new();
        let (_repo, col) = committed(&sb);
        write(&col, "d/e/b.yml", "new\n");
        let pipe = col.join("d/e/.gitattributes");
        pipe_over(&pipe);
        let options = sb.options().timeout(Duration::from_secs(1));
        let staged = bounded(&pipe, Duration::from_secs(15), move || {
            stage(&col, &["d"], &options)
        });
        let error = staged.expect_err("git waits on the pipe");
        assert!(matches!(error, GitError::Timeout), "{error}");
    }

    // The probes read rules without running filters, so a clean filter slower than the
    // caller's limit runs only in the write, under its longer floor.
    #[test]
    fn a_clean_filter_slower_than_the_limit_still_stages() {
        let sb = Sandbox::new();
        let (repo, col) = committed(&sb);
        sb.git(&repo, &["config", "filter.slow.clean", "sleep 2; cat"]);
        write(&repo, ".gitattributes", "*.yml filter=slow\n");
        write(&col, "a.yml", "changed\n");
        let options = sb.options().timeout(Duration::from_secs(1));
        stage(&col, &["a.yml"], &options).expect("the write's floor covers the filter");
        assert_eq!(
            sb.git(&repo, &["diff", "--cached", "--name-only"]),
            "col/a.yml\n"
        );
    }

    #[test]
    fn staging_a_pipe_over_a_tracked_file_says_git_refused() {
        let sb = Sandbox::new();
        let (_repo, col) = committed(&sb);
        let pipe = col.join("a.yml");
        pipe_over(&pipe);
        let options = sb.options();
        let staged = bounded(&pipe, Duration::from_secs(30), move || {
            stage(&col, &["a.yml"], &options)
        });
        let error = staged.expect_err("git refuses a pipe");
        assert!(
            error.to_string().contains("can only add regular files"),
            "{error}"
        );
    }
}
