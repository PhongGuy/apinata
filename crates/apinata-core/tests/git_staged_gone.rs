//! Paths the index holds while they are gone from disk: every path a commit records has a row a
//! person can stage or unstage. Each form below runs against a real repository.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

use crate::git_support;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use apinata_core::{
    Branch, FileStatus, GitError, GitOptions, GitState, RepoStatus, StageState, SwitchError,
    branches, collection_status_with, commit, file_diff_with, stage, switch_branch, unstage,
};
use git_support::{Sandbox, path_with, write};

use FileStatus::{Added, Deleted, Modified};
use StageState::{Both, Staged, StagedGone, Unstaged};

const BODY: &str = "info:\n  name: body\n  type: http\n  seq: 1\nhttp:\n  url: http://x/\n";

type Rows = BTreeMap<String, (FileStatus, StageState)>;

/// One way to reach the state: the steps, the rows they list, the path gone from disk, its
/// indexed text, and what a commit records.
struct Form {
    name: &'static str,
    build: fn(&Sandbox, &Path, &Path),
    rows: &'static [(&'static str, FileStatus, StageState)],
    gone: &'static str,
    text: &'static str,
    recorded: &'static [&'static str],
}

fn added_then_removed(sb: &Sandbox, repo: &Path, col: &Path) {
    write(col, "n.yml", BODY);
    sb.git(repo, &["add", "col/n.yml"]);
    fs::remove_file(col.join("n.yml")).unwrap();
}

fn added_empty_then_removed(sb: &Sandbox, repo: &Path, col: &Path) {
    write(col, "n.yml", "");
    sb.git(repo, &["add", "col/n.yml"]);
    fs::remove_file(col.join("n.yml")).unwrap();
}

fn renamed_then_removed(sb: &Sandbox, repo: &Path, col: &Path) {
    sb.git(repo, &["mv", "col/old.yml", "col/new.yml"]);
    fs::remove_file(col.join("new.yml")).unwrap();
}

fn added_then_moved(sb: &Sandbox, repo: &Path, col: &Path) {
    write(col, "o.yml", BODY);
    sb.git(repo, &["add", "col/o.yml"]);
    fs::rename(col.join("o.yml"), col.join("p.yml")).unwrap();
    sb.git(repo, &["add", "-N", "col/p.yml"]);
}

fn added_empty_then_moved(sb: &Sandbox, repo: &Path, col: &Path) {
    write(col, "o.yml", "");
    sb.git(repo, &["add", "col/o.yml"]);
    fs::rename(col.join("o.yml"), col.join("p.yml")).unwrap();
    sb.git(repo, &["add", "-N", "col/p.yml"]);
}

fn renamed_then_moved(sb: &Sandbox, repo: &Path, col: &Path) {
    sb.git(repo, &["mv", "col/old.yml", "col/b.yml"]);
    fs::rename(col.join("b.yml"), col.join("d.yml")).unwrap();
    sb.git(repo, &["add", "-N", "col/d.yml"]);
}

fn copied_then_removed(sb: &Sandbox, repo: &Path, col: &Path) {
    sb.git(repo, &["config", "status.renames", "copies"]);
    fs::copy(col.join("old.yml"), col.join("cp.yml")).unwrap();
    let edited = format!("{BODY}docs: edited\n");
    write(col, "old.yml", &edited);
    sb.git(repo, &["add", "col"]);
    fs::remove_file(col.join("cp.yml")).unwrap();
}

fn forms() -> Vec<Form> {
    vec![
        Form {
            name: "AD",
            build: added_then_removed,
            rows: &[("n.yml", Added, StagedGone)],
            gone: "n.yml",
            text: BODY,
            recorded: &["A\tcol/n.yml"],
        },
        Form {
            name: "AD of an empty file",
            build: added_empty_then_removed,
            rows: &[("n.yml", Added, StagedGone)],
            gone: "n.yml",
            text: "",
            recorded: &["A\tcol/n.yml"],
        },
        Form {
            name: "RD",
            build: renamed_then_removed,
            rows: &[("new.yml", Added, StagedGone), ("old.yml", Deleted, Staged)],
            gone: "new.yml",
            text: BODY,
            recorded: &["A\tcol/new.yml", "D\tcol/old.yml"],
        },
        Form {
            name: "A. and .R",
            build: added_then_moved,
            rows: &[("o.yml", Added, StagedGone), ("p.yml", Added, Unstaged)],
            gone: "o.yml",
            text: BODY,
            recorded: &["A\tcol/o.yml"],
        },
        Form {
            name: "A. and .R of an empty file",
            build: added_empty_then_moved,
            rows: &[("o.yml", Added, StagedGone), ("p.yml", Added, Unstaged)],
            gone: "o.yml",
            text: "",
            recorded: &["A\tcol/o.yml"],
        },
        Form {
            name: "R. and .R",
            build: renamed_then_moved,
            rows: &[
                ("b.yml", Added, StagedGone),
                ("d.yml", Added, Unstaged),
                ("old.yml", Deleted, Staged),
            ],
            gone: "b.yml",
            text: BODY,
            recorded: &["A\tcol/b.yml", "D\tcol/old.yml"],
        },
        Form {
            name: "CD",
            build: copied_then_removed,
            rows: &[("cp.yml", Added, StagedGone), ("old.yml", Modified, Staged)],
            gone: "cp.yml",
            text: BODY,
            recorded: &["A\tcol/cp.yml", "M\tcol/old.yml"],
        },
    ]
}

/// Repository `r` with `col/old.yml` and `col/keep.yml`, then `form`'s steps; returns (repo, col).
fn built(sb: &Sandbox, form: &Form) -> (PathBuf, PathBuf) {
    let repo = sb.repo(form.name.replace([' ', '.'], "_").as_str());
    write(&repo, "col/old.yml", BODY);
    write(&repo, "col/keep.yml", "keep: true\n");
    sb.commit_all(&repo, "base");
    let col = repo.join("col");
    (form.build)(sb, &repo, &col);
    (repo, col)
}

fn status(sb: &Sandbox, col: &Path) -> RepoStatus {
    match collection_status_with(col, &sb.options().check_kept(true)).expect("status") {
        GitState::Repo(s) => s,
        other => panic!("{other:?}"),
    }
}

fn rows(sb: &Sandbox, col: &Path) -> Rows {
    let s = status(sb, col);
    s.files
        .into_iter()
        .map(|(path, file)| {
            let at = s.stage[&path];
            (path.to_string_lossy().into_owned(), (file, at))
        })
        .collect()
}

fn listed(form: &Form) -> Rows {
    let row = |(path, file, at): &(&str, FileStatus, StageState)| {
        ((*path).to_owned(), (file.clone(), *at))
    };
    form.rows.iter().map(row).collect()
}

fn listed_without_the_gone_path(form: &Form) -> Rows {
    let mut left = listed(form);
    left.remove(form.gone);
    left
}

#[test]
fn every_form_lists_each_path_a_commit_records() {
    for form in forms() {
        let sb = Sandbox::new();
        let (_repo, col) = built(&sb, &form);
        assert_eq!(rows(&sb, &col), listed(&form), "{}", form.name);
    }
}

#[test]
fn staging_the_row_drops_it_from_the_index() {
    for form in forms() {
        let sb = Sandbox::new();
        let (_repo, col) = built(&sb, &form);
        stage(&col, &[form.gone], &sb.options()).expect(form.name);
        assert_eq!(
            rows(&sb, &col),
            listed_without_the_gone_path(&form),
            "{}",
            form.name
        );
    }
}

#[test]
fn unstaging_the_row_drops_it_from_the_index() {
    for form in forms() {
        let sb = Sandbox::new();
        let (_repo, col) = built(&sb, &form);
        unstage(&col, &[form.gone], &sb.options()).expect(form.name);
        assert_eq!(
            rows(&sb, &col),
            listed_without_the_gone_path(&form),
            "{}",
            form.name
        );
    }
}

#[test]
fn a_commit_records_exactly_the_staged_rows() {
    for form in forms() {
        let sb = Sandbox::new();
        let (repo, col) = built(&sb, &form);
        commit(&col, "gone", &sb.options()).expect(form.name);
        let shown = sb.git(
            &repo,
            &["show", "--no-renames", "--name-status", "--format=", "HEAD"],
        );
        assert_eq!(
            shown.lines().collect::<Vec<_>>(),
            form.recorded,
            "{}",
            form.name
        );
    }
}

#[test]
fn staging_every_row_at_once_never_fails() {
    for form in forms() {
        let sb = Sandbox::new();
        let (_repo, col) = built(&sb, &form);
        let paths: Vec<&str> = form.rows.iter().map(|(path, ..)| *path).collect();
        stage(&col, &paths, &sb.options()).expect(form.name);
        let now = rows(&sb, &col);
        assert!(!now.contains_key(form.gone), "{}: {now:?}", form.name);
    }
}

#[test]
fn the_bulk_operations_clear_the_row() {
    for form in forms() {
        for unstaging in [false, true] {
            let sb = Sandbox::new();
            let (_repo, col) = built(&sb, &form);
            let done = match unstaging {
                false => stage(&col, &["."], &sb.options()),
                true => unstage(&col, &["."], &sb.options()),
            };
            done.expect(form.name);
            let now = rows(&sb, &col);
            assert!(!now.contains_key(form.gone), "{}: {now:?}", form.name);
        }
    }
}

#[test]
fn the_diff_shows_the_indexed_text() {
    for form in forms() {
        let sb = Sandbox::new();
        let (_repo, col) = built(&sb, &form);
        let diff = file_diff_with(&col, Path::new(form.gone), &sb.options()).expect(form.name);
        assert_eq!(diff.status, Some(Added), "{}", form.name);
        assert_eq!(diff.old, None, "{}", form.name);
        assert_eq!(diff.new.as_deref(), Some(form.text), "{}", form.name);
    }
}

#[test]
fn a_collection_at_the_repository_root_reads_the_same_blob() {
    let sb = Sandbox::new();
    let repo = sb.repo("r");
    write(&repo, "keep.yml", "keep: true\n");
    sb.commit_all(&repo, "base");
    write(&repo, "n.yml", BODY);
    sb.git(&repo, &["add", "n.yml"]);
    fs::remove_file(repo.join("n.yml")).unwrap();
    let diff = file_diff_with(&repo, Path::new("n.yml"), &sb.options()).expect("diff");
    assert_eq!(diff.new.as_deref(), Some(BODY));
}

#[test]
fn a_path_nothing_holds_still_errors() {
    let sb = Sandbox::new();
    let form = &forms()[0];
    let (_repo, col) = built(&sb, form);
    let got = file_diff_with(&col, Path::new("typo.yml"), &sb.options());
    assert!(matches!(got, Err(GitError::Malformed(_))), "{got:?}");
}

#[test]
fn a_file_over_the_parent_counts_as_gone_for_the_diff_and_the_stage() {
    let sb = Sandbox::new();
    let repo = sb.repo("r");
    write(&repo, "col/keep.yml", "keep: true\n");
    sb.commit_all(&repo, "base");
    let col = repo.join("col");
    write(&col, "s/n.yml", BODY);
    sb.git(&repo, &["add", "col/s/n.yml"]);
    fs::remove_dir_all(col.join("s")).unwrap();
    write(&col, "s", "a file now\n");
    assert_eq!(rows(&sb, &col)["s/n.yml"], (Added, StagedGone));
    let diff = file_diff_with(&col, Path::new("s/n.yml"), &sb.options()).expect("diff");
    assert_eq!(diff.new.as_deref(), Some(BODY));
    stage(&col, &["s/n.yml"], &sb.options()).expect("stage");
    assert!(!rows(&sb, &col).contains_key("s/n.yml"));
}

#[test]
fn a_directory_on_the_path_counts_as_gone_for_the_diff() {
    let sb = Sandbox::new();
    let form = &forms()[0];
    let (_repo, col) = built(&sb, form);
    write(&col, "n.yml/inner.txt", "x\n");
    assert_eq!(rows(&sb, &col)["n.yml"], (Added, StagedGone));
    let diff = file_diff_with(&col, Path::new("n.yml"), &sb.options()).expect("diff");
    assert_eq!(diff.new.as_deref(), Some(BODY));
}

#[test]
fn a_switch_over_the_row_is_dirty() {
    let sb = Sandbox::new();
    let form = &forms()[0];
    let (repo, col) = built(&sb, form);
    sb.git(&repo, &["branch", "topic", "main"]);
    let topic: Branch = branches(&repo, &sb.options())
        .unwrap()
        .into_iter()
        .find(|b| b.local.as_deref() == Some("topic"))
        .unwrap();
    let got = switch_branch(&repo, &topic, &sb.options());
    assert!(matches!(got, Err(SwitchError::Dirty)), "{got:?}");
    assert_eq!(rows(&sb, &col), listed(form));
}

/// A repository with `col/keep.yml` and `col/old.yml` committed, then `col/s/n.yml` staged new and
/// `s` replaced by a link to `dir`; returns (repo, col).
#[cfg(unix)]
fn staged_under_a_link(sb: &Sandbox) -> (PathBuf, PathBuf) {
    let repo = sb.repo("r");
    write(&repo, "col/keep.yml", "keep: true\n");
    write(&repo, "col/old/h.yml", BODY);
    sb.commit_all(&repo, "base");
    let col = repo.join("col");
    write(&col, "s/n.yml", BODY);
    sb.git(&repo, &["add", "col/s/n.yml"]);
    fs::remove_dir_all(col.join("s")).unwrap();
    write(&col, "dir/other.txt", "x\n");
    std::os::unix::fs::symlink("dir", col.join("s")).unwrap();
    (repo, col)
}

#[cfg(unix)]
#[test]
fn stage_drops_a_staged_path_under_a_parent_replaced_by_a_link() {
    let sb = Sandbox::new();
    let (_repo, col) = staged_under_a_link(&sb);
    assert_eq!(rows(&sb, &col)["s/n.yml"], (Added, StagedGone));
    stage(&col, &["s/n.yml"], &sb.options()).expect("stage");
    assert!(!rows(&sb, &col).contains_key("s/n.yml"));
}

#[cfg(unix)]
#[test]
fn stage_drops_a_path_the_link_target_also_has() {
    let sb = Sandbox::new();
    let (_repo, col) = staged_under_a_link(&sb);
    write(&col, "dir/n.yml", "through the link\n");
    assert_eq!(rows(&sb, &col)["s/n.yml"], (Added, StagedGone));
    stage(&col, &["s/n.yml"], &sb.options()).expect("stage");
    assert!(!rows(&sb, &col).contains_key("s/n.yml"));
    assert_eq!(
        fs::read_to_string(col.join("dir/n.yml")).unwrap(),
        "through the link\n"
    );
}

#[cfg(unix)]
#[test]
fn stage_under_a_link_still_stages_the_other_paths() {
    let sb = Sandbox::new();
    let (_repo, col) = staged_under_a_link(&sb);
    write(&col, "keep.yml", "keep: changed\n");
    write(&col, "fresh.yml", BODY);
    let paths = ["keep.yml", "s/n.yml", "fresh.yml"];
    stage(&col, &paths, &sb.options()).expect("stage");
    let now = rows(&sb, &col);
    assert!(!now.contains_key("s/n.yml"), "{now:?}");
    assert_eq!(now["keep.yml"], (Modified, Staged), "{now:?}");
    assert_eq!(now["fresh.yml"], (Added, Staged), "{now:?}");
}

#[cfg(unix)]
#[test]
fn stage_records_the_deletion_of_a_tracked_path_under_a_link() {
    let sb = Sandbox::new();
    let (_repo, col) = staged_under_a_link(&sb);
    fs::remove_dir_all(col.join("old")).unwrap();
    std::os::unix::fs::symlink("dir", col.join("old")).unwrap();
    assert_eq!(rows(&sb, &col)["old/h.yml"], (Deleted, Unstaged));
    stage(&col, &["old/h.yml"], &sb.options()).expect("stage");
    assert_eq!(rows(&sb, &col)["old/h.yml"], (Deleted, Staged));
}

#[cfg(unix)]
#[test]
fn unstage_drops_a_staged_path_under_a_link() {
    let sb = Sandbox::new();
    let (_repo, col) = staged_under_a_link(&sb);
    unstage(&col, &["s/n.yml"], &sb.options()).expect("unstage");
    assert!(!rows(&sb, &col).contains_key("s/n.yml"));
}

/// `:0:a.yml` is stage 0 of `a.yml`, so the shorter `:<path>` misreads a file named this way.
#[cfg(unix)]
#[test]
fn the_index_blob_of_a_file_named_like_a_stage_spec_is_its_own() {
    let sb = Sandbox::new();
    let repo = sb.repo("r");
    write(&repo, "keep.yml", "keep: true\n");
    sb.commit_all(&repo, "base");
    write(&repo, "a.yml", "the other file\n");
    write(&repo, "0:a.yml", BODY);
    sb.git(&repo, &["add", "--", "a.yml", "./0:a.yml"]);
    fs::remove_file(repo.join("0:a.yml")).unwrap();
    assert_eq!(rows(&sb, &repo)["0:a.yml"], (Added, StagedGone));
    let diff = file_diff_with(&repo, Path::new("0:a.yml"), &sb.options()).expect("diff");
    assert_eq!(diff.new.as_deref(), Some(BODY));
}

/// `BODY` as the index holds it after an edit; only a read of the index can show it once the file
/// is gone.
const EDITED: &str = "info:\n  name: body\n  type: http\n  seq: 1\nhttp:\n  url: http://x/edited\n";

/// Repository `r` with `col/old.yml` committed as `BODY`; returns (repo, col).
fn committed(sb: &Sandbox) -> (PathBuf, PathBuf) {
    let repo = sb.repo("r");
    write(&repo, "col/old.yml", BODY);
    write(&repo, "col/keep.yml", "keep: true\n");
    sb.commit_all(&repo, "base");
    let col = repo.join("col");
    (repo, col)
}

/// [`committed`] with an edit to `old.yml` staged, then the file removed: git's `MD`.
fn staged_edit_gone(sb: &Sandbox) -> (PathBuf, PathBuf) {
    let (repo, col) = committed(sb);
    write(&col, "old.yml", EDITED);
    sb.git(&repo, &["add", "col/old.yml"]);
    fs::remove_file(col.join("old.yml")).unwrap();
    (repo, col)
}

#[test]
fn a_staged_edit_gone_from_disk_is_a_modified_row() {
    let sb = Sandbox::new();
    let (_repo, col) = staged_edit_gone(&sb);
    let want = Rows::from([("old.yml".to_owned(), (Modified, StagedGone))]);
    assert_eq!(rows(&sb, &col), want);
}

#[test]
fn a_staged_edit_gone_from_disk_diffs_head_against_the_index() {
    let sb = Sandbox::new();
    let (_repo, col) = staged_edit_gone(&sb);
    let diff = file_diff_with(&col, Path::new("old.yml"), &sb.options()).expect("diff");
    assert_eq!(diff.status, Some(Modified));
    assert_eq!(diff.old.as_deref(), Some(BODY));
    assert_eq!(diff.new.as_deref(), Some(EDITED));
}

#[test]
fn a_commit_records_the_indexed_edit_of_a_file_gone_from_disk() {
    let sb = Sandbox::new();
    let (repo, col) = staged_edit_gone(&sb);
    commit(&col, "gone", &sb.options()).expect("commit");
    assert_eq!(
        sb.git(&repo, &["show", "--name-status", "--format=", "HEAD"])
            .trim(),
        "M\tcol/old.yml"
    );
    assert_eq!(sb.git(&repo, &["show", "HEAD:col/old.yml"]), EDITED);
    assert_eq!(rows(&sb, &col)["old.yml"], (Deleted, Unstaged));
}

#[test]
fn staging_a_gone_edit_drops_it_for_a_staged_deletion() {
    let sb = Sandbox::new();
    let (repo, col) = staged_edit_gone(&sb);
    stage(&col, &["old.yml"], &sb.options()).expect("stage");
    assert_eq!(rows(&sb, &col)["old.yml"], (Deleted, Staged));
    assert_eq!(sb.git(&repo, &["ls-files", "col/old.yml"]), "");
}

#[test]
fn unstaging_a_gone_edit_drops_it_and_leaves_the_file_as_in_head() {
    let sb = Sandbox::new();
    let (repo, col) = staged_edit_gone(&sb);
    unstage(&col, &["old.yml"], &sb.options()).expect("unstage");
    assert_eq!(rows(&sb, &col)["old.yml"], (Deleted, Unstaged));
    assert_eq!(sb.git(&repo, &["show", ":0:col/old.yml"]), BODY);
}

#[test]
fn a_staged_edit_edited_again_on_disk_stays_partly_staged() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    write(&col, "old.yml", EDITED);
    sb.git(&repo, &["add", "col/old.yml"]);
    let again = format!("{EDITED}docs: again\n");
    write(&col, "old.yml", &again);
    assert_eq!(rows(&sb, &col)["old.yml"], (Modified, Both));
    let diff = file_diff_with(&col, Path::new("old.yml"), &sb.options()).expect("diff");
    assert_eq!(diff.new.as_deref(), Some(again.as_str()));
}

#[test]
fn a_staged_edit_truncated_to_nothing_is_a_gone_edit_too() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    write(&col, "old.yml", "");
    sb.git(&repo, &["add", "col/old.yml"]);
    fs::remove_file(col.join("old.yml")).unwrap();
    assert_eq!(rows(&sb, &col)["old.yml"], (Modified, StagedGone));
    let diff = file_diff_with(&col, Path::new("old.yml"), &sb.options()).expect("diff");
    assert_eq!(diff.new.as_deref(), Some(""));
}

#[test]
fn a_directory_over_a_gone_edit_leaves_the_indexed_text() {
    let sb = Sandbox::new();
    let (_repo, col) = staged_edit_gone(&sb);
    write(&col, "old.yml/inner.txt", "x\n");
    assert_eq!(rows(&sb, &col)["old.yml"], (Modified, StagedGone));
    let diff = file_diff_with(&col, Path::new("old.yml"), &sb.options()).expect("diff");
    assert_eq!(diff.new.as_deref(), Some(EDITED));
}

#[cfg(unix)]
#[test]
fn a_staged_type_change_gone_from_disk_is_a_gone_edit() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    fs::remove_file(col.join("old.yml")).unwrap();
    std::os::unix::fs::symlink("keep.yml", col.join("old.yml")).unwrap();
    sb.git(&repo, &["add", "col/old.yml"]);
    fs::remove_file(col.join("old.yml")).unwrap();
    assert_eq!(rows(&sb, &col)["old.yml"], (Modified, StagedGone));
    let diff = file_diff_with(&col, Path::new("old.yml"), &sb.options()).expect("diff");
    assert_eq!(diff.new.as_deref(), Some("keep.yml"));
}

/// The index holds an edit of `p/a.yml` and `p` is now a link, whose target holds other text.
#[cfg(unix)]
fn staged_edit_under_a_link(sb: &Sandbox) -> (PathBuf, PathBuf) {
    let repo = sb.repo("r");
    write(&repo, "col/p/a.yml", BODY);
    sb.commit_all(&repo, "base");
    let col = repo.join("col");
    write(&col, "p/a.yml", EDITED);
    sb.git(&repo, &["add", "col/p/a.yml"]);
    fs::rename(col.join("p"), col.join("q")).unwrap();
    std::os::unix::fs::symlink("q", col.join("p")).unwrap();
    write(&col, "q/a.yml", "the link's target\n");
    (repo, col)
}

#[cfg(unix)]
#[test]
fn a_staged_edit_under_a_link_is_a_gone_edit() {
    let sb = Sandbox::new();
    let (_repo, col) = staged_edit_under_a_link(&sb);
    assert_eq!(rows(&sb, &col)["p/a.yml"], (Modified, StagedGone));
}

#[cfg(unix)]
#[test]
fn a_staged_edit_under_a_link_diffs_the_index_not_the_links_target() {
    let sb = Sandbox::new();
    let (_repo, col) = staged_edit_under_a_link(&sb);
    let diff = file_diff_with(&col, Path::new("p/a.yml"), &sb.options()).expect("diff");
    assert_eq!(diff.old.as_deref(), Some(BODY));
    assert_eq!(diff.new.as_deref(), Some(EDITED));
}

#[cfg(unix)]
#[test]
fn a_commit_records_the_indexed_edit_under_a_link() {
    let sb = Sandbox::new();
    let (repo, col) = staged_edit_under_a_link(&sb);
    commit(&col, "gone", &sb.options()).expect("commit");
    assert_eq!(sb.git(&repo, &["show", "HEAD:col/p/a.yml"]), EDITED);
}

#[cfg(unix)]
#[test]
fn stage_and_unstage_both_work_on_a_staged_edit_under_a_link() {
    let sb = Sandbox::new();
    let (_repo, col) = staged_edit_under_a_link(&sb);
    unstage(&col, &["p/a.yml"], &sb.options()).expect("unstage");
    assert_eq!(rows(&sb, &col)["p/a.yml"], (Deleted, Unstaged));

    let sb = Sandbox::new();
    let (_repo, col) = staged_edit_under_a_link(&sb);
    stage(&col, &["p/a.yml"], &sb.options()).expect("stage");
    assert_eq!(rows(&sb, &col)["p/a.yml"], (Deleted, Staged));
}

#[test]
fn a_gone_edit_with_nothing_left_keeps_no_file() {
    let sb = Sandbox::new();
    let (_repo, col) = staged_edit_gone(&sb);
    assert!(status(&sb, &col).kept_at.is_empty());
}

#[test]
fn a_gone_edit_moved_and_marked_add_n_is_kept_at_the_new_path() {
    let sb = Sandbox::new();
    let (repo, col) = staged_edit_gone(&sb);
    write(&col, "c.yml", EDITED);
    sb.git(&repo, &["add", "-N", "col/c.yml"]);
    let kept = status(&sb, &col).kept_at;
    assert_eq!(kept[Path::new("old.yml")], Path::new("c.yml"), "{kept:?}");
}

#[cfg(unix)]
#[test]
fn a_gone_edit_under_a_link_is_kept_at_the_links_target() {
    let sb = Sandbox::new();
    let (_repo, col) = staged_edit_under_a_link(&sb);
    write(&col, "q/a.yml", EDITED);
    let kept = status(&sb, &col).kept_at;
    assert_eq!(kept[Path::new("p/a.yml")], Path::new("q/a.yml"), "{kept:?}");
}

#[test]
fn a_status_that_does_not_ask_keeps_no_entry() {
    let sb = Sandbox::new();
    let (repo, col) = staged_edit_gone(&sb);
    write(&col, "c.yml", EDITED);
    sb.git(&repo, &["add", "-N", "col/c.yml"]);
    let GitState::Repo(repo) = collection_status_with(&col, &sb.options()).expect("status") else {
        panic!("a repository");
    };
    assert!(repo.kept_at.is_empty(), "{:?}", repo.kept_at);
}

#[cfg(unix)]
#[test]
fn a_link_leading_out_of_the_repository_is_never_read() {
    let sb = Sandbox::new();
    let (_repo, col) = staged_edit_under_a_link(&sb);
    write(&col, "q/a.yml", EDITED);
    let outside = sb.path().join("outside");
    fs::rename(col.join("q"), &outside).unwrap();
    fs::remove_file(col.join("p")).unwrap();
    std::os::unix::fs::symlink(&outside, col.join("p")).unwrap();
    assert!(status(&sb, &col).kept_at.is_empty());
}

#[test]
fn a_gone_edit_with_nothing_left_opens_no_file() {
    let sb = Sandbox::new();
    let (_repo, col) = staged_edit_gone(&sb);
    assert!(status(&sb, &col).opens_at.is_empty());
}

#[cfg(unix)]
#[test]
fn a_gone_edit_under_a_link_to_other_text_opens_the_links_target() {
    let sb = Sandbox::new();
    let (_repo, col) = staged_edit_under_a_link(&sb);
    let got = status(&sb, &col);
    assert!(got.kept_at.is_empty(), "{:?}", got.kept_at);
    assert_eq!(got.opens_at[Path::new("p/a.yml")], Path::new("q/a.yml"));
}

#[cfg(unix)]
#[test]
fn a_gone_edit_kept_under_a_link_opens_nothing_else() {
    let sb = Sandbox::new();
    let (_repo, col) = staged_edit_under_a_link(&sb);
    write(&col, "q/a.yml", EDITED);
    assert!(status(&sb, &col).opens_at.is_empty());
}

#[cfg(unix)]
#[test]
fn a_link_to_a_file_git_cannot_read_claims_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let sb = Sandbox::new();
    let (_repo, col) = staged_edit_under_a_link(&sb);
    let target = col.join("q/a.yml");
    fs::set_permissions(&target, fs::Permissions::from_mode(0o000)).unwrap();
    let got = status(&sb, &col);
    fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(got.opens_at.is_empty(), "{:?}", got.opens_at);
}

#[test]
fn a_lowered_file_cap_truncates_the_listing() {
    let sb = Sandbox::new();
    let (_repo, col) = staged_edit_gone(&sb);
    for name in ["x.yml", "y.yml", "z.yml"] {
        write(&col, name, BODY);
    }
    let options = sb.options().file_cap(2);
    let GitState::Repo(repo) = collection_status_with(&col, &options).expect("status") else {
        panic!("a repository");
    };
    assert_eq!((repo.files.len(), repo.truncated), (2, true));
}

#[cfg(unix)]
#[test]
fn a_link_out_of_the_repository_opens_nothing_it_names() {
    let sb = Sandbox::new();
    let (_repo, col) = staged_edit_under_a_link(&sb);
    let outside = sb.path().join("outside");
    fs::rename(col.join("q"), &outside).unwrap();
    fs::remove_file(col.join("p")).unwrap();
    std::os::unix::fs::symlink(&outside, col.join("p")).unwrap();
    assert!(status(&sb, &col).opens_at.is_empty());
}

#[cfg(unix)]
#[test]
fn a_link_leaving_the_collection_but_not_the_repository_is_named_in_full() {
    let sb = Sandbox::new();
    let (repo, col) = staged_edit_under_a_link(&sb);
    write(&col, "q/a.yml", EDITED);
    fs::rename(col.join("q"), repo.join("elsewhere")).unwrap();
    fs::remove_file(col.join("p")).unwrap();
    std::os::unix::fs::symlink("../elsewhere", col.join("p")).unwrap();
    assert_eq!(
        status(&sb, &col).kept_at[Path::new("p/a.yml")],
        repo.canonicalize().unwrap().join("elsewhere/a.yml")
    );
}

#[cfg(unix)]
#[test]
fn a_moved_link_to_a_fifo_is_not_opened() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    let made = std::process::Command::new("mkfifo")
        .arg(col.join("pipe"))
        .status()
        .expect("mkfifo runs");
    assert!(made.success());
    std::os::unix::fs::symlink("pipe", col.join("s.yml")).unwrap();
    sb.git(&repo, &["add", "col/s.yml"]);
    fs::rename(col.join("s.yml"), col.join("t.yml")).unwrap();
    sb.git(&repo, &["add", "-N", "col/t.yml"]);
    let started = std::time::Instant::now();
    assert!(status(&sb, &col).kept_at.is_empty());
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

/// Options whose git logs each command line to the returned file.
#[cfg(unix)]
fn logging_git(sb: &Sandbox) -> (GitOptions, PathBuf) {
    let calls = sb.path().join("calls");
    let body = format!(
        "#!/bin/sh\necho \"$*\" >> '{log}'\nexec '{git}' \"$@\"\n",
        log = calls.display(),
        git = git_support::real_git().display(),
    );
    let bin = sb.fake_git("logged", &body);
    (sb.options().env("PATH", path_with(&bin)), calls)
}

#[cfg(unix)]
#[test]
fn only_a_status_that_asks_hashes_a_candidate() {
    let sb = Sandbox::new();
    let (_repo, col) = staged_edit_under_a_link(&sb);
    write(&col, "q/a.yml", EDITED);
    let (options, calls) = logging_git(&sb);
    let hashed = || {
        fs::read_to_string(&calls)
            .unwrap_or_default()
            .contains("hash-object")
    };
    file_diff_with(&col, Path::new("p/a.yml"), &options).expect("diff");
    stage(&col, &["q/a.yml"], &options).expect("stage");
    unstage(&col, &["q/a.yml"], &options).expect("unstage");
    collection_status_with(&col, &options).expect("status");
    assert!(!hashed(), "diff, the writes and a plain status pay nothing");
    collection_status_with(&col, &options.check_kept(true)).expect("status");
    assert!(hashed(), "the check is live");
}

/// 120 staged edits under `p`, which is now a link to `q`, each file there holding its copy.
#[cfg(unix)]
fn many_under_a_link(sb: &Sandbox) -> (PathBuf, Vec<String>) {
    let repo = sb.repo("r");
    let col = repo.join("col");
    let names: Vec<String> = (0..120).map(|i| format!("f{i:03}.yml")).collect();
    for name in &names {
        write(&col, &format!("p/{name}"), BODY);
    }
    sb.commit_all(&repo, "base");
    for name in &names {
        write(&col, &format!("p/{name}"), EDITED);
    }
    sb.git(&repo, &["add", "col/p"]);
    fs::rename(col.join("p"), col.join("q")).unwrap();
    std::os::unix::fs::symlink("q", col.join("p")).unwrap();
    (col, names)
}

#[cfg(unix)]
#[test]
fn one_status_checks_a_bounded_number_of_candidates() {
    let sb = Sandbox::new();
    let (col, _) = many_under_a_link(&sb);
    let got = status(&sb, &col);
    let gone = got.stage.values().filter(|at| **at == StagedGone).count();
    assert_eq!((gone, got.kept_at.len()), (120, 100));
    assert!(got.kept_at.contains_key(Path::new("p/f000.yml")));
}

#[cfg(unix)]
#[test]
fn the_bound_counts_candidates_checked_not_files_that_match() {
    let sb = Sandbox::new();
    let (col, names) = many_under_a_link(&sb);
    for name in &names[..100] {
        write(&col, &format!("q/{name}"), "edited later\n");
    }
    assert!(
        status(&sb, &col).kept_at.is_empty(),
        "the last 20 go unchecked"
    );
}

#[test]
fn a_moved_file_edited_after_the_move_is_not_claimed() {
    let sb = Sandbox::new();
    let (repo, col) = staged_edit_gone(&sb);
    write(&col, "c.yml", &format!("{EDITED}x: 1\n"));
    sb.git(&repo, &["add", "-N", "col/c.yml"]);
    assert_eq!(rows(&sb, &col)["c.yml"], (Added, Unstaged));
    assert!(status(&sb, &col).kept_at.is_empty());
}

#[test]
fn renames_off_leaves_a_moved_file_unpaired() {
    let sb = Sandbox::new();
    let (repo, col) = staged_edit_gone(&sb);
    sb.git(&repo, &["config", "status.renames", "false"]);
    write(&col, "c.yml", EDITED);
    sb.git(&repo, &["add", "-N", "col/c.yml"]);
    assert!(status(&sb, &col).kept_at.is_empty());
}

#[cfg(unix)]
#[test]
fn a_links_file_edited_after_the_move_is_not_claimed() {
    let sb = Sandbox::new();
    let (_repo, col) = staged_edit_under_a_link(&sb);
    assert_eq!(rows(&sb, &col)["p/a.yml"], (Modified, StagedGone));
    assert!(status(&sb, &col).kept_at.is_empty());
}

#[cfg(unix)]
#[test]
fn a_link_to_an_unrelated_file_is_not_claimed() {
    let sb = Sandbox::new();
    let repo = sb.repo("r");
    write(&repo, "col/p/a.yml", BODY);
    write(&repo, "col/q/a.yml", "unrelated: true\n");
    sb.commit_all(&repo, "base");
    let col = repo.join("col");
    write(&col, "p/a.yml", EDITED);
    sb.git(&repo, &["add", "col/p/a.yml"]);
    fs::remove_dir_all(col.join("p")).unwrap();
    std::os::unix::fs::symlink("q", col.join("p")).unwrap();
    assert_eq!(rows(&sb, &col)["p/a.yml"], (Modified, StagedGone));
    assert!(status(&sb, &col).kept_at.is_empty());
}

#[cfg(unix)]
#[test]
fn a_links_file_is_compared_through_the_filters_git_would_use() {
    let sb = Sandbox::new();
    let (repo, col) = staged_edit_under_a_link(&sb);
    // Only the gone path carries the attribute, so hashing as `q/a.yml` would keep the CRLF.
    write(&repo, ".gitattributes", "col/p/a.yml text eol=crlf\n");
    write(&col, "q/a.yml", &EDITED.replace('\n', "\r\n"));
    let kept = status(&sb, &col).kept_at;
    assert_eq!(kept[Path::new("p/a.yml")], Path::new("q/a.yml"), "{kept:?}");
}

#[cfg(unix)]
#[test]
fn a_path_under_a_link_listed_deleted_diffs_as_a_deletion() {
    for write_op in [true, false] {
        let sb = Sandbox::new();
        let (_repo, col) = staged_edit_under_a_link(&sb);
        match write_op {
            true => unstage(&col, &["p/a.yml"], &sb.options()).expect("unstage"),
            false => stage(&col, &["p/a.yml"], &sb.options()).expect("stage"),
        }
        let diff = file_diff_with(&col, Path::new("p/a.yml"), &sb.options()).expect("diff");
        assert_eq!(diff.status, Some(Deleted));
        assert_eq!(diff.old.as_deref(), Some(BODY));
        assert_eq!(diff.new, None, "the link's target is another file");
    }
}
