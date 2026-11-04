//! `file_diff` against sandbox repositories: both sides, status, hunks, sections, the path
//! guard and the partial-clone no-fetch rule.

use crate::git_support;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use apinata_core::{
    FileDiff, FileStatus, GIT_FILE_CAP, GitError, GitOptions, GitState, LineKind, Section,
    collection_status_with, file_diff_with,
};
use git_support::{Sandbox, packs, write};

const V1: &str = "info:\n  name: get\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: https://example.com/v1\n";

fn v2() -> String {
    V1.replace("/v1", "/v2")
}

/// Repository `r` whose collection `col/` has `a.yml` (V1) committed.
fn repo_with_a(sb: &Sandbox) -> PathBuf {
    let repo = sb.repo("r");
    write(&repo, "col/a.yml", V1);
    sb.commit_all(&repo, "base");
    repo
}

fn diff(sb: &Sandbox, root: &Path, rel: &str) -> FileDiff {
    file_diff_with(root, Path::new(rel), &sb.options()).expect("diff")
}

fn gutters(d: &FileDiff) -> Vec<String> {
    d.hunks
        .iter()
        .flat_map(|hunk| hunk.lines.iter())
        .map(|line| {
            let gutter = match line.kind {
                LineKind::Context => ' ',
                LineKind::Added => '+',
                LineKind::Removed => '-',
                _ => '?',
            };
            format!("{gutter}{}", line.text)
        })
        .collect()
}

fn rejected(root: &Path, rel: &Path, options: &GitOptions) {
    match file_diff_with(root, rel, options) {
        Err(GitError::Malformed(message)) => {
            assert!(!message.contains("hunter2"), "{message}");
        }
        other => panic!("{rel:?} was not refused: {other:?}"),
    }
}

const ALL: [Section; 3] = [Section::Info, Section::Method, Section::Url];

#[test]
fn a_modified_request_has_both_sides_its_sections_and_a_hunk() {
    let sb = Sandbox::new();
    let repo = repo_with_a(&sb);
    write(&repo, "col/a.yml", &v2());
    let d = diff(&sb, &repo.join("col"), "a.yml");
    assert_eq!(d.path, PathBuf::from("a.yml"));
    assert_eq!(d.status, Some(FileStatus::Modified));
    assert_eq!(d.old.as_deref(), Some(V1));
    assert_eq!(d.new.as_deref(), Some(v2().as_str()));
    assert!(!d.binary);
    assert_eq!(d.sections, [Section::Url]);
    assert_eq!(d.hunks.len(), 1);
    assert_eq!((d.hunks[0].old_start, d.hunks[0].new_start), (4, 4));
    assert_eq!(
        gutters(&d),
        [
            "   seq: 1",
            " http:",
            "   method: GET",
            "-  url: https://example.com/v1",
            "+  url: https://example.com/v2",
        ]
    );
}

#[test]
fn an_untracked_request_has_no_old_side() {
    let sb = Sandbox::new();
    let repo = repo_with_a(&sb);
    write(&repo, "col/b.yml", V1);
    let d = diff(&sb, &repo.join("col"), "b.yml");
    assert_eq!(d.status, Some(FileStatus::Untracked));
    assert_eq!(d.old, None);
    assert_eq!(d.new.as_deref(), Some(V1));
    assert_eq!(d.sections, ALL);
    assert_eq!((d.hunks[0].old_start, d.hunks[0].new_start), (0, 1));
    assert!(gutters(&d).iter().all(|line| line.starts_with('+')));
    assert_eq!(gutters(&d).len(), 7);
}

#[test]
fn a_deleted_request_has_no_new_side() {
    let sb = Sandbox::new();
    let repo = repo_with_a(&sb);
    fs::remove_file(repo.join("col/a.yml")).unwrap();
    let d = diff(&sb, &repo.join("col"), "a.yml");
    assert_eq!(d.status, Some(FileStatus::Deleted));
    assert_eq!(d.old.as_deref(), Some(V1));
    assert_eq!(d.new, None);
    assert_eq!(d.sections, ALL);
    assert_eq!((d.hunks[0].old_start, d.hunks[0].new_start), (1, 0));
    assert!(gutters(&d).iter().all(|line| line.starts_with('-')));
}

#[test]
fn a_path_neither_on_disk_nor_in_head_is_refused() {
    let sb = Sandbox::new();
    let repo = repo_with_a(&sb);
    match file_diff_with(&repo.join("col"), Path::new("nope.yml"), &sb.options()) {
        Err(GitError::Malformed(message)) => {
            assert!(
                message.contains("nope.yml is not on disk or in HEAD"),
                "{message}"
            );
        }
        other => panic!("nope.yml was not refused: {other:?}"),
    }
}

#[test]
fn a_renamed_request_reads_its_old_side_from_the_origin() {
    let sb = Sandbox::new();
    let repo = repo_with_a(&sb);
    sb.git(&repo, &["mv", "col/a.yml", "col/b.yml"]);
    write(&repo, "col/b.yml", &v2());
    let d = diff(&sb, &repo.join("col"), "b.yml");
    assert_eq!(
        d.status,
        Some(FileStatus::Renamed {
            from: PathBuf::from("a.yml")
        })
    );
    assert_eq!(d.old.as_deref(), Some(V1));
    assert_eq!(d.new.as_deref(), Some(v2().as_str()));
    assert_eq!(d.sections, [Section::Url]);
}

#[test]
fn a_non_utf8_side_is_binary_with_nothing_else() {
    let sb = Sandbox::new();
    let repo = repo_with_a(&sb);
    fs::write(repo.join("col/a.yml"), [0xff, 0xfe, 0x00, 0x01]).unwrap();
    let d = diff(&sb, &repo.join("col"), "a.yml");
    assert_eq!(d.status, Some(FileStatus::Modified));
    assert!(d.binary);
    assert_eq!((d.old, d.new), (None, None));
    assert!(d.sections.is_empty() && d.hunks.is_empty());
}

#[test]
fn an_unchanged_request_has_no_status_and_no_hunks() {
    let sb = Sandbox::new();
    let repo = repo_with_a(&sb);
    let d = diff(&sb, &repo.join("col"), "a.yml");
    assert_eq!(d.status, None);
    assert_eq!(d.old.as_deref(), Some(V1));
    assert_eq!(d.new.as_deref(), Some(V1));
    assert!(d.sections.is_empty() && d.hunks.is_empty());
}

#[test]
fn dropped_from_the_index_but_unchanged_on_disk_has_no_content_change() {
    let sb = Sandbox::new();
    let repo = repo_with_a(&sb);
    sb.git(&repo, &["rm", "-q", "--cached", "col/a.yml"]);
    let d = diff(&sb, &repo.join("col"), "a.yml");
    assert_eq!(d.status, Some(FileStatus::Modified));
    assert_eq!(d.old, d.new);
    assert!(d.hunks.is_empty());
}

#[test]
fn on_an_unborn_head_the_old_side_is_absent() {
    let sb = Sandbox::new();
    let repo = sb.repo("r");
    write(&repo, "col/a.yml", V1);
    sb.git(&repo, &["add", "col/a.yml"]);
    let d = diff(&sb, &repo.join("col"), "a.yml");
    assert_eq!(d.status, Some(FileStatus::Added));
    assert_eq!(d.old, None);
    assert!(!d.hunks.is_empty());
}

#[test]
fn a_partial_clone_never_fetches_a_missing_blob() {
    let sb = Sandbox::new();
    let src = repo_with_a(&sb);
    sb.git(&src, &["config", "uploadpack.allowFilter", "true"]);
    let url = format!("file://{}", src.display());
    sb.git(
        sb.path(),
        &["clone", "-q", "--filter=blob:none", &url, "part"],
    );
    let part = sb.path().join("part");
    // HEAD moves to a commit whose blob was never fetched; the worktree keeps V1.
    write(&src, "col/a.yml", &v2());
    sb.commit_all(&src, "two");
    sb.git(&part, &["fetch", "-q", "origin"]);
    sb.git(&part, &["reset", "-q", "--soft", "origin/main"]);
    let before = packs(&part);
    let root = part.join("col");

    let status = match collection_status_with(&root, &sb.options()).expect("status runs") {
        GitState::Repo(status) => status,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        status.files.get(Path::new("a.yml")),
        Some(&FileStatus::Modified)
    );
    assert!(matches!(
        file_diff_with(&root, Path::new("a.yml"), &sb.options()),
        Err(GitError::Failed { .. })
    ));
    assert_eq!(packs(&part), before);
}

#[test]
fn outside_a_repository_the_diff_fails() {
    let sb = Sandbox::new();
    let root = sb.path().join("plain");
    write(&root, "a.yml", V1);
    match file_diff_with(&root, Path::new("a.yml"), &sb.options()) {
        Err(GitError::Failed { stderr, .. }) => {
            assert!(stderr.contains("not a git repository"), "{stderr}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_file_past_the_status_cap_still_has_its_status() {
    let sb = Sandbox::new();
    let repo = repo_with_a(&sb);
    for i in 0..=GIT_FILE_CAP {
        write(&repo, &format!("col/bulk/f{i:05}.yml"), "x: 1\n");
    }
    let last = format!("bulk/f{GIT_FILE_CAP:05}.yml");
    let d = diff(&sb, &repo.join("col"), &last);
    assert_eq!(d.status, Some(FileStatus::Untracked));
}

#[cfg(unix)]
#[test]
fn a_hanging_show_is_stopped_at_the_time_limit() {
    let sb = Sandbox::new();
    let root = sb.path().join("col");
    write(&root, "a.yml", V1);
    // Answers rev-parse and status at once (a.yml modified), then hangs in `show`.
    let bin = sb.fake_git(
        "slow-show",
        "#!/bin/sh\ncase \"$*\" in\n  *rev-parse*) printf '\\n' ;;\n  *status*) printf '# branch.oid 0123456789abcdef0123456789abcdef01234567\\000'; printf '# branch.head main\\000'; printf '1 .M N... 100644 100644 100644 cb46010117ba62af321588c78f2e681868628485 cb46010117ba62af321588c78f2e681868628485 a.yml\\000' ;;\n  *show*) sleep 5 ;;\nesac\n",
    );
    let options = sb
        .options()
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .timeout(Duration::from_millis(300));
    let started = Instant::now();
    assert!(matches!(
        file_diff_with(&root, Path::new("a.yml"), &options),
        Err(GitError::Timeout)
    ));
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn without_git_the_diff_is_io_not_found() {
    let sb = Sandbox::new();
    let repo = repo_with_a(&sb);
    let options = sb.options().env("PATH", sb.empty_bin());
    match file_diff_with(&repo.join("col"), Path::new("a.yml"), &options) {
        Err(GitError::Io(e)) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
        other => panic!("{other:?}"),
    }
}

#[test]
fn paths_that_are_not_collection_files_are_refused_before_git_runs() {
    let sb = Sandbox::new();
    let repo = repo_with_a(&sb);
    let root = repo.join("col");
    write(&root, ".env", "TOKEN=hunter2\n");
    write(&root, "notes.txt", "hunter2\n");
    write(&repo, "outside.yml", "TOKEN: hunter2\n");
    // No git on PATH: a refusal must not depend on running it.
    let options = sb.options().env("PATH", sb.empty_bin());
    for rel in [
        ".env",
        "notes.txt",
        "../outside.yml",
        "sub/../a.yml",
        "./a.yml",
        "",
    ] {
        rejected(&root, Path::new(rel), &options);
    }
    rejected(&root, &repo.join("outside.yml"), &options);
    rejected(&root, &root.join("a.yml"), &options);
}

#[cfg(unix)]
#[test]
fn a_symlink_out_of_the_collection_is_refused_and_one_inside_is_read() {
    use std::os::unix::fs::symlink;
    let sb = Sandbox::new();
    let repo = repo_with_a(&sb);
    let root = repo.join("col");
    write(&repo, "secret/x.yml", "TOKEN: hunter2\n");
    symlink(repo.join("secret/x.yml"), root.join("link.yml")).unwrap();
    symlink(repo.join("secret"), root.join("dir")).unwrap();
    symlink(root.join("a.yml"), root.join("alias.yml")).unwrap();
    let options = sb.options();
    rejected(&root, Path::new("link.yml"), &options);
    rejected(&root, Path::new("dir/x.yml"), &options);
    rejected(&root, Path::new("dir/missing.yml"), &options);
    let alias = file_diff_with(&root, Path::new("alias.yml"), &options).expect("inside link");
    assert_eq!(alias.new.as_deref(), Some(V1));
}

#[cfg(unix)]
#[test]
fn a_committed_symlink_with_an_unchanged_target_has_no_hunks() {
    use std::os::unix::fs::symlink;
    let sb = Sandbox::new();
    let repo = repo_with_a(&sb);
    let root = repo.join("col");
    symlink("a.yml", root.join("alias.yml")).unwrap();
    sb.commit_all(&repo, "link");
    let d = diff(&sb, &root, "alias.yml");
    assert_eq!(d.status, None);
    assert!(d.hunks.is_empty(), "{:?}", d.hunks);

    // Repointing the link is a change, shown as the old and new link text.
    fs::remove_file(root.join("alias.yml")).unwrap();
    write(&repo, "col/b.yml", V1);
    symlink("b.yml", root.join("alias.yml")).unwrap();
    let d = diff(&sb, &root, "alias.yml");
    assert_eq!(gutters(&d), ["-a.yml", "+b.yml"]);
}
