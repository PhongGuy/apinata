//! `apinata git status` and `apinata git diff` on sandbox repositories.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

use crate::common;
use crate::git_support;

use std::fs;
use std::path::{Path, PathBuf};

use common::{ROOT, apinata};
use git_support::{SCRUBBED, Sandbox, real_git, write};
use serde_json::{Value, json};

const GET: &str = "info:\n  name: get\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: https://example.com/v1\n";
const DEL: &str = "info:\n  name: del\n  type: http\n  seq: 2\nhttp:\n  method: DELETE\n  url: https://example.com/d\n";

fn cli(sb: &Sandbox, cwd: &Path) -> assert_cmd::Command {
    let mut cmd = apinata();
    cmd.current_dir(cwd).envs(sb.env());
    for name in SCRUBBED {
        cmd.env_remove(name);
    }
    cmd
}

fn stdout(out: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(out.get_output().stdout.clone()).unwrap()
}

fn stderr(out: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(out.get_output().stderr.clone()).unwrap()
}

/// Repository `r` with collection `col/`: users/get.yml edited, users/del.yml deleted,
/// old.yml renamed to new.yml and fresh.yml untracked. Returns the collection root.
fn dirty(sb: &Sandbox) -> PathBuf {
    let repo = sb.repo("r");
    write(&repo, "col/opencollection.yml", ROOT);
    write(&repo, "col/users/get.yml", GET);
    write(&repo, "col/users/del.yml", DEL);
    write(&repo, "col/old.yml", "info:\n  name: old\n");
    sb.commit_all(&repo, "base");
    write(&repo, "col/users/get.yml", &GET.replace("/v1", "/v2"));
    fs::remove_file(repo.join("col/users/del.yml")).unwrap();
    sb.git(&repo, &["mv", "col/old.yml", "col/new.yml"]);
    write(&repo, "col/fresh.yml", "info:\n  name: fresh\n");
    repo.join("col")
}

/// A committed, clean collection `col/` in repository `r`.
fn clean(sb: &Sandbox) -> PathBuf {
    let repo = sb.repo("r");
    write(&repo, "col/opencollection.yml", ROOT);
    sb.commit_all(&repo, "base");
    repo.join("col")
}

#[test]
fn status_prints_one_line_per_file_then_the_branch() {
    let sb = Sandbox::new();
    let col = dirty(&sb);
    let head = sb.short_head(&col);
    let out = cli(&sb, &col).args(["git", "status"]).assert().code(0);
    insta::assert_snapshot!(stdout(&out).replace(&head, "<head>"), @r"
untracked  fresh.yml
renamed    new.yml (from old.yml)
deleted    users/del.yml
modified   users/get.yml
on main at <head>
");
}

#[test]
fn status_takes_the_collection_as_an_argument() {
    let sb = Sandbox::new();
    let col = dirty(&sb);
    let out = cli(&sb, sb.path())
        .args(["git", "status"])
        .arg(&col)
        .assert()
        .code(0);
    assert!(stdout(&out).contains("modified   users/get.yml\n"));
}

#[test]
fn status_json_is_the_serialized_state() {
    let sb = Sandbox::new();
    let col = dirty(&sb);
    let out = cli(&sb, &col)
        .args(["git", "status", "--json"])
        .assert()
        .code(0);
    let value: Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(
        value,
        json!({
            "state": "repo",
            "branch": "main",
            "head": sb.short_head(&col),
            "files": {
                "fresh.yml": {"kind": "untracked"},
                "new.yml": {"kind": "renamed", "from": "old.yml"},
                "users/del.yml": {"kind": "deleted"},
                "users/get.yml": {"kind": "modified"}
            },
            "truncated": false
        })
    );
}

#[test]
fn a_clean_tree_prints_only_the_branch() {
    let sb = Sandbox::new();
    let col = clean(&sb);
    let out = cli(&sb, &col).args(["git", "status"]).assert().code(0);
    assert_eq!(
        stdout(&out),
        format!("on main at {}\n", sb.short_head(&col))
    );
}

#[test]
fn a_detached_head_says_so() {
    let sb = Sandbox::new();
    let col = clean(&sb);
    sb.git(&col, &["checkout", "-q", "--detach"]);
    let out = cli(&sb, &col).args(["git", "status"]).assert().code(0);
    assert_eq!(
        stdout(&out),
        format!("detached at {}\n", sb.short_head(&col))
    );
}

#[test]
fn an_unborn_head_says_no_commits_yet() {
    let sb = Sandbox::new();
    let repo = sb.repo("r");
    write(&repo, "col/opencollection.yml", ROOT);
    let out = cli(&sb, &repo.join("col"))
        .args(["git", "status"])
        .assert()
        .code(0);
    assert_eq!(
        stdout(&out),
        "untracked  opencollection.yml\non main, no commits yet\n"
    );
}

#[test]
fn outside_a_repository_status_says_so_and_succeeds() {
    let sb = Sandbox::new();
    let col = sb.path().join("plain");
    write(&col, "opencollection.yml", ROOT);
    let out = cli(&sb, &col).args(["git", "status"]).assert().code(0);
    assert_eq!(stdout(&out), "");
    let err = stderr(&out);
    assert_eq!(err.lines().count(), 1, "{err}");
    assert!(err.ends_with(" is not in a git repository.\n"), "{err}");
}

#[test]
fn outside_a_repository_status_json_prints_the_state() {
    let sb = Sandbox::new();
    let col = sb.path().join("plain");
    write(&col, "opencollection.yml", ROOT);
    let out = cli(&sb, &col)
        .args(["git", "status", "--json"])
        .assert()
        .code(0);
    let value: Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(value, json!({"state": "not_a_repo"}));
    assert_eq!(stderr(&out), "");
}

#[test]
fn without_git_status_says_so_and_succeeds() {
    let sb = Sandbox::new();
    let col = dirty(&sb);
    let out = cli(&sb, &col)
        .env("PATH", sb.empty_bin())
        .args(["git", "status"])
        .assert()
        .code(0);
    assert_eq!(stdout(&out), "");
    assert_eq!(
        stderr(&out),
        "git was not found on PATH, so changes cannot be shown.\n"
    );
}

#[cfg(unix)]
#[test]
fn a_failing_git_exits_three_with_its_message() {
    let sb = Sandbox::new();
    let col = clean(&sb);
    let bin = sb.fake_git("broken", "#!/bin/sh\necho 'fatal: boom' >&2\nexit 2\n");
    let out = cli(&sb, &col)
        .env("PATH", &bin)
        .args(["git", "status"])
        .assert()
        .code(3);
    assert!(stderr(&out).contains("fatal: boom"), "{}", stderr(&out));
}

// A `git` that appends each argument list to `log`, then runs the real one.
#[cfg(unix)]
fn logging_git(sb: &Sandbox, log: &Path) -> PathBuf {
    let body = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexec '{}' \"$@\"\n",
        log.display(),
        real_git().display()
    );
    sb.fake_git("logging", &body)
}

#[cfg(unix)]
#[test]
fn status_and_diff_never_hash_a_candidate_for_a_gone_row() {
    let sb = Sandbox::new();
    let col = clean(&sb);
    let repo = col.parent().unwrap().to_path_buf();
    // A staged file moved on disk and marked `add -N`: the app's check would hash the new path.
    write(&col, "a.yml", GET);
    sb.git(&repo, &["add", "col/a.yml"]);
    fs::rename(col.join("a.yml"), col.join("b.yml")).unwrap();
    sb.git(&repo, &["add", "-N", "col/b.yml"]);
    let log = sb.path().join("git.log");
    let bin = logging_git(&sb, &log);
    for args in [["git", "status", "."], ["git", "diff", "b.yml"]] {
        cli(&sb, &col).env("PATH", &bin).args(args).assert().code(0);
    }
    let calls = fs::read_to_string(&log).unwrap();
    assert!(calls.contains("status --porcelain"), "{calls}");
    assert!(!calls.contains("hash-object"), "{calls}");
    assert!(!calls.contains(":0:"), "{calls}");
}

#[test]
fn a_conflicted_file_is_listed_as_conflicted() {
    let sb = Sandbox::new();
    let col = clean(&sb);
    write(&col, "conf.yml", "base\n");
    sb.commit_all(&col, "conf");
    sb.git(&col, &["checkout", "-q", "-b", "side"]);
    write(&col, "conf.yml", "side\n");
    sb.commit_all(&col, "side");
    sb.git(&col, &["checkout", "-q", "main"]);
    write(&col, "conf.yml", "main\n");
    sb.commit_all(&col, "main");
    sb.git_fails(&col, &["merge", "-q", "side"]);
    let out = cli(&sb, &col).args(["git", "status"]).assert().code(0);
    assert!(
        stdout(&out).starts_with("conflicted conf.yml\n"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn a_truncated_status_says_so_before_the_branch() {
    let sb = Sandbox::new();
    let col = clean(&sb);
    for i in 0..=5000 {
        write(&col, &format!("bulk/f{i:05}.yml"), "x: 1\n");
    }
    let out = cli(&sb, &col).args(["git", "status"]).assert().code(0);
    let text = stdout(&out);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 5002);
    assert_eq!(lines[4999], "untracked  bulk/f04999.yml");
    assert_eq!(lines[5000], "showing the first 5000 changed files");
    assert!(lines[5001].starts_with("on main at "));
}

#[test]
fn without_a_collection_status_is_a_usage_error() {
    let sb = Sandbox::new();
    let dir = sb.path().join("empty");
    fs::create_dir_all(&dir).unwrap();
    cli(&sb, &dir).args(["git", "status"]).assert().code(3);
}

#[test]
fn diff_prints_sections_then_a_unified_diff() {
    let sb = Sandbox::new();
    let col = dirty(&sb);
    let out = cli(&sb, &col)
        .args(["git", "diff", "users/get.yml"])
        .assert()
        .code(0);
    insta::assert_snapshot!(stdout(&out), @r"
sections: url
--- a/users/get.yml
+++ b/users/get.yml
@@ -4,4 +4,4 @@
   seq: 1
 http:
   method: GET
-  url: https://example.com/v1
+  url: https://example.com/v2
");
}

#[test]
fn diff_of_a_deleted_file_ends_at_dev_null() {
    let sb = Sandbox::new();
    let col = dirty(&sb);
    let out = cli(&sb, &col)
        .args(["git", "diff", "users/del.yml"])
        .assert()
        .code(0);
    insta::assert_snapshot!(stdout(&out), @r"
sections: info, method, url
--- a/users/del.yml
+++ /dev/null
@@ -1,7 +0,0 @@
-info:
-  name: del
-  type: http
-  seq: 2
-http:
-  method: DELETE
-  url: https://example.com/d
");
}

#[test]
fn diff_of_an_untracked_file_starts_at_dev_null() {
    let sb = Sandbox::new();
    let col = dirty(&sb);
    let out = cli(&sb, &col)
        .args(["git", "diff", "fresh.yml"])
        .assert()
        .code(0);
    insta::assert_snapshot!(stdout(&out), @r"
sections: info
--- /dev/null
+++ b/fresh.yml
@@ -0,0 +1,2 @@
+info:
+  name: fresh
");
}

#[test]
fn diff_of_a_pure_rename_has_no_content_change() {
    let sb = Sandbox::new();
    let col = dirty(&sb);
    let out = cli(&sb, &col)
        .args(["git", "diff", "new.yml"])
        .assert()
        .code(0);
    assert_eq!(stdout(&out), "no content change\n");
}

#[test]
fn diff_of_a_binary_file_says_so() {
    let sb = Sandbox::new();
    let col = dirty(&sb);
    fs::write(col.join("users/get.yml"), [0xff, 0xfe, 0x00]).unwrap();
    let out = cli(&sb, &col)
        .args(["git", "diff", "users/get.yml"])
        .assert()
        .code(0);
    assert_eq!(stdout(&out), "binary file changed\n");
}

// `git diff t.yml` with a pipe where the file was, bounded so a read of the pipe fails the test.
#[cfg(unix)]
fn diff_under_a_pipe(sb: &Sandbox, col: &Path) -> assert_cmd::assert::Assert {
    let pipe = col.join("t.yml");
    fs::remove_file(&pipe).unwrap();
    let made = std::process::Command::new("mkfifo").arg(&pipe).status();
    assert!(made.expect("mkfifo").success());
    cli(sb, col)
        .args(["git", "diff", "t.yml"])
        .timeout(std::time::Duration::from_secs(30))
        .assert()
}

#[cfg(unix)]
#[test]
fn diff_of_a_new_staged_file_under_a_pipe_shows_the_staged_copy() {
    let sb = Sandbox::new();
    let col = clean(&sb);
    write(&col, "t.yml", GET);
    sb.git(col.parent().unwrap(), &["add", "col/t.yml"]);
    let out = diff_under_a_pipe(&sb, &col).code(0);
    assert!(
        stdout(&out).contains("+  url: https://example.com/v1"),
        "{}",
        stdout(&out)
    );
}

#[cfg(unix)]
#[test]
fn diff_of_a_staged_edit_under_a_pipe_shows_the_staged_copy() {
    let sb = Sandbox::new();
    let col = clean(&sb);
    let repo = col.parent().unwrap().to_path_buf();
    write(&col, "t.yml", GET);
    sb.commit_all(&repo, "t");
    write(&col, "t.yml", &GET.replace("/v1", "/v2"));
    sb.git(&repo, &["add", "col/t.yml"]);
    let out = diff_under_a_pipe(&sb, &col).code(0);
    assert!(
        stdout(&out).contains("+  url: https://example.com/v2"),
        "{}",
        stdout(&out)
    );
}

#[cfg(unix)]
#[test]
fn diff_of_a_committed_file_under_a_pipe_reads_it_as_gone() {
    let sb = Sandbox::new();
    let col = clean(&sb);
    write(&col, "t.yml", GET);
    sb.commit_all(col.parent().unwrap(), "t");
    let out = diff_under_a_pipe(&sb, &col).code(0);
    assert!(
        stdout(&out).contains("-  url: https://example.com/v1"),
        "{}",
        stdout(&out)
    );
}

#[cfg(unix)]
fn mkfifo(path: &Path) {
    let made = std::process::Command::new("mkfifo").arg(path).status();
    assert!(made.expect("mkfifo").success());
}

// Git compares a tracked file's content, and so reads `.gitattributes`, only when its stat no
// longer matches the index or shares the index's second; a new mtime forces the comparison.
#[cfg(unix)]
fn restamp(path: &Path) {
    let file = fs::File::open(path).unwrap();
    let an_hour_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    file.set_modified(an_hour_ago).unwrap();
}

// `t.yml`, a link staged at `b.yml` and then retargeted at the pipe `p`, in the given repository.
#[cfg(unix)]
fn staged_link_retargeted_at_a_pipe(sb: &Sandbox, col: &Path) {
    use std::os::unix::fs::symlink;
    let repo = col.parent().unwrap();
    mkfifo(&col.join("p"));
    fs::remove_file(col.join("t.yml")).unwrap();
    symlink("b.yml", col.join("t.yml")).unwrap();
    sb.git(repo, &["add", "col/t.yml"]);
    fs::remove_file(col.join("t.yml")).unwrap();
    symlink("p", col.join("t.yml")).unwrap();
}

#[cfg(unix)]
#[test]
fn diff_of_a_tracked_link_retargeted_at_a_pipe_shows_its_target_text() {
    use std::os::unix::fs::symlink;
    let sb = Sandbox::new();
    let col = clean(&sb);
    symlink("a.yml", col.join("t.yml")).unwrap();
    sb.commit_all(col.parent().unwrap(), "link");
    staged_link_retargeted_at_a_pipe(&sb, &col);
    let out = cli(&sb, &col)
        .args(["git", "diff", "t.yml"])
        .timeout(std::time::Duration::from_secs(30))
        .assert()
        .code(0);
    let text = stdout(&out);
    assert!(text.contains("-a.yml") && text.contains("+p"), "{text}");
    assert!(!text.contains("+b.yml"), "{text}");
}

#[cfg(unix)]
#[test]
fn diff_of_a_new_staged_link_retargeted_at_a_pipe_shows_the_staged_copy() {
    let sb = Sandbox::new();
    let col = clean(&sb);
    write(&col, "t.yml", GET);
    staged_link_retargeted_at_a_pipe(&sb, &col);
    let out = cli(&sb, &col)
        .args(["git", "diff", "t.yml"])
        .timeout(std::time::Duration::from_secs(30))
        .assert()
        .code(0);
    assert!(stdout(&out).contains("+b.yml"), "{}", stdout(&out));
}

#[cfg(unix)]
#[test]
fn status_ends_at_the_time_limit_when_git_waits_on_a_pipe() {
    let sb = Sandbox::new();
    let col = clean(&sb);
    mkfifo(&col.parent().unwrap().join(".gitattributes"));
    restamp(&col.join("opencollection.yml"));
    let out = cli(&sb, &col)
        .args(["git", "status"])
        .timeout(std::time::Duration::from_secs(30))
        .assert()
        .code(3);
    let text = stderr(&out);
    assert!(
        text.contains("did not finish within the time limit"),
        "{text}"
    );
}

#[test]
fn diff_resolves_the_file_against_the_working_directory() {
    let sb = Sandbox::new();
    let col = dirty(&sb);
    let out = cli(&sb, &col.join("users"))
        .args(["git", "diff", "get.yml"])
        .assert()
        .code(0);
    assert!(
        stdout(&out).starts_with("sections: url\n--- a/users/get.yml\n"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn diff_marks_a_last_line_without_a_newline_like_git() {
    let sb = Sandbox::new();
    let col = dirty(&sb);
    write(&col, "users/get.yml", GET.trim_end_matches('\n'));
    let out = cli(&sb, &col)
        .args(["git", "diff", "users/get.yml"])
        .assert()
        .code(0);
    let text = stdout(&out);
    assert!(
        text.ends_with("+  url: https://example.com/v1\n\\ No newline at end of file\n"),
        "{text}"
    );
}

#[test]
fn diff_json_is_the_serialized_file_diff() {
    let sb = Sandbox::new();
    let col = dirty(&sb);
    let out = cli(&sb, &col)
        .args(["git", "diff", "users/get.yml", "--json"])
        .assert()
        .code(0);
    let value: Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(value["path"], "users/get.yml");
    assert_eq!(value["status"], json!({"kind": "modified"}));
    assert_eq!(value["old"], GET);
    assert_eq!(value["new"], GET.replace("/v1", "/v2"));
    assert_eq!(value["binary"], false);
    assert_eq!(value["sections"], json!(["url"]));
    assert_eq!(
        value["hunks"],
        json!([{
            "old_start": 4,
            "new_start": 4,
            "lines": [
                {"kind": "context", "text": "  seq: 1"},
                {"kind": "context", "text": "http:"},
                {"kind": "context", "text": "  method: GET"},
                {"kind": "removed", "text": "  url: https://example.com/v1"},
                {"kind": "added", "text": "  url: https://example.com/v2"}
            ]
        }])
    );
}

#[test]
fn diff_refuses_a_file_that_is_not_a_request_with_exit_three() {
    let sb = Sandbox::new();
    let col = dirty(&sb);
    write(&col, ".env", "TOKEN=hunter2\n");
    let out = cli(&sb, &col)
        .args(["git", "diff", ".env"])
        .assert()
        .code(3);
    assert_eq!(stdout(&out), "");
    let err = stderr(&out);
    assert!(err.contains(".env"), "{err}");
    assert!(!err.contains("hunter2"), "{err}");
}

#[test]
fn diff_refuses_a_file_that_is_not_on_disk_or_in_head() {
    let sb = Sandbox::new();
    let col = dirty(&sb);
    let out = cli(&sb, &col)
        .args(["git", "diff", "nope.yml"])
        .assert()
        .code(3);
    assert_eq!(stdout(&out), "");
    assert_eq!(stderr(&out), "error: nope.yml is not on disk or in HEAD\n");
}

#[test]
fn diff_normalises_the_path_and_refuses_the_collection_itself() {
    let sb = Sandbox::new();
    let col = dirty(&sb);
    for file in ["users//get.yml", "users/./get.yml", "./users/get.yml"] {
        let out = cli(&sb, &col)
            .args(["git", "diff", file, "--json"])
            .assert()
            .code(0);
        let value: Value = serde_json::from_str(&stdout(&out)).unwrap();
        assert_eq!(value["path"], "users/get.yml", "{file}");
        let out = cli(&sb, &col).args(["git", "diff", file]).assert().code(0);
        assert!(
            stdout(&out).contains("--- a/users/get.yml\n+++ b/users/get.yml\n"),
            "{file}: {}",
            stdout(&out)
        );
    }
    let out = cli(&sb, &col).args(["git", "diff", "."]).assert().code(3);
    assert_eq!(stdout(&out), "");
    assert_eq!(
        stderr(&out),
        "error: . is the collection, not a request file\n"
    );
}

#[test]
fn diff_refuses_a_path_outside_the_collection_without_calling_git() {
    let sb = Sandbox::new();
    let col = dirty(&sb);
    let outside = col.parent().unwrap().join("outside.yml");
    write(col.parent().unwrap(), "outside.yml", "TOKEN: hunter2\n");
    for file in [outside.to_str().unwrap(), "../outside.yml"] {
        let out = cli(&sb, &col)
            .env("PATH", sb.empty_bin())
            .args(["git", "diff", file])
            .assert()
            .code(3);
        let err = stderr(&out);
        assert!(err.contains("is outside the collection"), "{err}");
        assert!(!err.contains("hunter2"), "{err}");
        assert_eq!(stdout(&out), "");
    }
}

#[test]
fn outside_a_repository_diff_says_so_and_succeeds() {
    let sb = Sandbox::new();
    let col = sb.path().join("plain");
    write(&col, "opencollection.yml", ROOT);
    write(&col, "a.yml", GET);
    let out = cli(&sb, &col)
        .args(["git", "diff", "a.yml"])
        .assert()
        .code(0);
    assert_eq!(stdout(&out), "");
    assert!(stderr(&out).ends_with(" is not in a git repository.\n"));
}

#[test]
fn git_help_says_exit_3_also_covers_a_refused_file() {
    let out = apinata().args(["git", "--help"]).assert().success();
    let help = stdout(&out);
    let line = help
        .lines()
        .find(|l| l.trim_start().starts_with("3 "))
        .unwrap_or_else(|| panic!("an exit 3 line in {help}"));
    assert!(line.contains("git failed"), "{line}");
    assert!(line.contains("outside the collection"), "{line}");
}

/// The subject of the newest commit in `repo`.
fn head_subject(sb: &Sandbox, repo: &Path) -> String {
    sb.git(repo, &["log", "-1", "--format=%s"])
        .trim()
        .to_owned()
}

#[test]
fn commit_records_what_is_staged_and_prints_the_short_id_and_subject() {
    let sb = Sandbox::new();
    let col = dirty(&sb);
    sb.git(&col, &["add", "users/get.yml"]);
    let out = cli(&sb, &col)
        .args(["git", "commit", "-m", "edit url\n\nlonger body"])
        .assert()
        .code(0);
    let head = sb.short_head(&col);
    assert_eq!(stdout(&out), format!("{head} edit url\n"));
    assert_eq!(head_subject(&sb, &col), "edit url");
}

#[test]
fn commit_all_stages_the_whole_collection() {
    let sb = Sandbox::new();
    let col = dirty(&sb);
    cli(&sb, &col)
        .args(["git", "commit", "-m", "all", "--all"])
        .assert()
        .code(0);
    assert_eq!(sb.git(&col, &["status", "--porcelain"]), "");
}

#[test]
fn commit_with_paths_stages_only_those_relative_to_the_working_directory() {
    let sb = Sandbox::new();
    let col = dirty(&sb);
    cli(&sb, &col.join("users"))
        .args(["git", "commit", "-m", "one", "--collection"])
        .arg(&col)
        .arg("get.yml")
        .assert()
        .code(0);
    let left = sb.git(&col, &["status", "--porcelain"]);
    assert!(!left.contains("get.yml"), "{left}");
    assert!(left.contains("fresh.yml"), "{left}");
}

#[test]
fn commit_with_nothing_staged_fails_with_exit_3() {
    let sb = Sandbox::new();
    let col = clean(&sb);
    write(&col, "fresh.yml", "info:\n  name: fresh\n");
    let out = cli(&sb, &col)
        .args(["git", "commit", "-m", "m"])
        .assert()
        .code(3);
    assert!(stderr(&out).contains("nothing"), "{}", stderr(&out));
}

#[test]
fn commit_with_an_empty_message_is_a_usage_error() {
    let sb = Sandbox::new();
    let col = dirty(&sb);
    cli(&sb, &col)
        .args(["git", "commit", "-m", "  ", "--all"])
        .assert()
        .code(3);
    assert_eq!(head_subject(&sb, &col), "base");
}

#[test]
fn commit_all_with_paths_is_a_usage_error() {
    let sb = Sandbox::new();
    let col = dirty(&sb);
    cli(&sb, &col)
        .args(["git", "commit", "-m", "m", "--all", "users/get.yml"])
        .assert()
        .code(3);
}

#[test]
fn commit_outside_a_repository_fails_with_exit_3() {
    let sb = Sandbox::new();
    let col = sb.path().join("plain");
    write(&col, "opencollection.yml", ROOT);
    let out = cli(&sb, &col)
        .args(["git", "commit", "-m", "m", "--all"])
        .assert()
        .code(3);
    assert!(
        stderr(&out).contains("not in a git repository"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn commit_json_has_the_short_id_and_subject() {
    let sb = Sandbox::new();
    let col = dirty(&sb);
    let out = cli(&sb, &col)
        .args(["git", "commit", "-m", "json", "--all", "--json"])
        .assert()
        .code(0);
    let value: Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(
        value,
        json!({"short_id": sb.short_head(&col), "subject": "json"})
    );
}

#[test]
fn commit_refuses_a_parent_or_absolute_path_outside_the_collection() {
    let sb = Sandbox::new();
    let col = dirty(&sb);
    let elsewhere = sb.path().display().to_string();
    for path in ["../x.yml", elsewhere.as_str()] {
        let out = cli(&sb, &col)
            .args(["git", "commit", "-m", "m", path])
            .assert()
            .code(3);
        assert!(stderr(&out).contains("outside the collection"), "{path}");
    }
    assert_eq!(head_subject(&sb, &col), "base");
}
