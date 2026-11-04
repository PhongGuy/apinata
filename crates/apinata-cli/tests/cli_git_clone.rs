use crate::common;
use crate::git_support;

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use common::apinata;
use git_support::{HANG_GIT, SCRUBBED, Sandbox, path_with, wait_gone, write};
use predicates::str::contains;

/// Drop ambient git and ssh overrides and ignore the system git config, so a run is hermetic.
fn scrub(cmd: &mut assert_cmd::Command) {
    for name in SCRUBBED.into_iter().chain(["GIT_SSH", "GIT_SSH_COMMAND"]) {
        cmd.env_remove(name);
    }
    cmd.env("GIT_CONFIG_NOSYSTEM", "1");
}

fn cli(sb: &Sandbox, cwd: &Path) -> assert_cmd::Command {
    let mut cmd = apinata();
    cmd.current_dir(cwd).envs(sb.env());
    scrub(&mut cmd);
    cmd
}

fn cloned(sb: &Sandbox) -> PathBuf {
    cli(sb, sb.path())
        .args(["git", "clone"])
        .arg(sb.origin())
        .arg("col")
        .assert()
        .code(0);
    sb.path().join("col")
}

fn out_text(out: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(out.get_output().stdout.clone()).unwrap()
}

fn err_text(out: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(out.get_output().stderr.clone()).unwrap()
}

#[test]
fn clone_creates_the_folder_and_reports_where() {
    let sb = Sandbox::new();
    let out = cli(&sb, sb.path())
        .args(["git", "clone"])
        .arg(sb.origin())
        .arg("col")
        .assert()
        .code(0);
    assert!(sb.path().join("col/opencollection.yml").is_file());
    assert!(out_text(&out).contains("Cloned into"));
    assert!(err_text(&out).contains("Cloning into"));
}

#[test]
fn clone_without_a_folder_uses_the_repository_name() {
    let sb = Sandbox::new();
    let bare = sb.origin();
    let work = sb.path().join("w");
    fs::create_dir(&work).unwrap();
    cli(&sb, &work)
        .args(["git", "clone"])
        .arg(&bare)
        .assert()
        .code(0);
    assert!(work.join("origin/a.txt").is_file());
}

#[test]
fn clone_into_a_non_empty_folder_is_a_usage_error_and_touches_nothing() {
    let sb = Sandbox::new();
    let bare = sb.origin();
    write(sb.path(), "col/keep.txt", "mine\n");
    cli(&sb, sb.path())
        .args(["git", "clone"])
        .arg(&bare)
        .arg("col")
        .assert()
        .code(3)
        .stderr(contains("already exists"));
    assert_eq!(
        fs::read_to_string(sb.path().join("col/keep.txt")).unwrap(),
        "mine\n"
    );
}

#[test]
fn clone_rejects_a_transport_url() {
    let sb = Sandbox::new();
    cli(&sb, sb.path())
        .args(["git", "clone", "ext::sh -c id"])
        .assert()
        .code(3)
        .stderr(contains("invalid repository URL"));
}

#[cfg(unix)]
#[test]
fn an_auth_failure_shows_the_hint_and_never_prints_the_password() {
    let sb = Sandbox::new();
    let bin = sb.fake_tool("sshbin", "ssh", "#!/bin/sh\ndir=${0%/*}\nprintf '%s\\n' \"$@\" > \"$dir/ssh-args\"\necho 'Permission denied (publickey).' >&2\nexit 255\n");
    let out = cli(&sb, sb.path())
        .env("PATH", path_with(&bin))
        .args(["git", "clone", "ssh://git:pw@example.invalid/x.git", "col"])
        .assert()
        .code(3);
    let err = err_text(&out);
    assert!(err.contains("Cloning ssh://example.invalid/x.git"), "{err}");
    assert!(!err.contains(":pw@"), "{err}");
    assert!(err.contains("credential helper or ssh-agent"), "{err}");
    assert!(
        fs::read_to_string(bin.join("ssh-args"))
            .unwrap()
            .contains("BatchMode=yes")
    );
    assert!(!sb.path().join("col").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn ctrl_c_cancels_the_clone_and_leaves_nothing() {
    let sb = Sandbox::new();
    let bin = sb.fake_git("fake", HANG_GIT);
    let mut child = common::apinata_std()
        .args(["git", "clone", "/srv/x.git", "col"])
        .current_dir(sb.path())
        .envs(sb.env())
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_SSH")
        .env_remove("GIT_SSH_COMMAND")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("PATH", path_with(&bin))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let mut err = BufReader::new(child.stderr.take().unwrap());
    let mut line = String::new();
    loop {
        line.clear();
        assert!(
            err.read_line(&mut line).unwrap() > 0,
            "apinata ended before the clone started"
        );
        if line.contains("Receiving objects") {
            break;
        }
    }
    let id = child.id().to_string();
    assert!(
        Command::new("kill")
            .args(["-INT", &id])
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(child.wait().unwrap().code(), Some(130));
    assert!(!sb.path().join("col").exists());
    let grandchild: u32 = fs::read_to_string(bin.join("pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(wait_gone(grandchild));
}

#[test]
fn branches_lists_text_and_json() {
    let sb = Sandbox::new();
    let col = cloned(&sb);
    let out = cli(&sb, &col).args(["git", "branches"]).assert().code(0);
    assert_eq!(
        out_text(&out),
        "origin/feature\norigin/main (tracked by main)\n"
    );
    let out = cli(&sb, &col)
        .args(["git", "branches", "--json"])
        .assert()
        .code(0);
    let json: serde_json::Value = serde_json::from_str(&out_text(&out)).unwrap();
    assert_eq!(
        json,
        serde_json::json!([
            {"remote": "origin", "name": "feature", "local": null},
            {"remote": "origin", "name": "main", "local": "main"}
        ])
    );
}

#[test]
fn branches_fetch_sees_a_branch_pushed_since_the_clone() {
    let sb = Sandbox::new();
    let col = cloned(&sb);
    sb.git(
        &sb.path().join("seed"),
        &["push", "-q", "origin", "feature:newer"],
    );
    let plain = cli(&sb, &col).args(["git", "branches"]).assert().code(0);
    assert!(!out_text(&plain).contains("origin/newer"));
    let out = cli(&sb, &col)
        .args(["git", "branches", "--fetch"])
        .assert()
        .code(0);
    assert!(out_text(&out).contains("origin/newer"));
}

#[test]
fn switch_checks_out_a_remote_branch_by_either_spelling() {
    let sb = Sandbox::new();
    let col = cloned(&sb);
    cli(&sb, &col)
        .args(["git", "switch", "origin/feature"])
        .assert()
        .code(0)
        .stdout(contains("Switched to feature"));
    assert_eq!(
        sb.git(&col, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(),
        "feature"
    );
    cli(&sb, &col)
        .args(["git", "switch", "main"])
        .assert()
        .code(0);
    assert_eq!(
        sb.git(&col, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(),
        "main"
    );
}

#[test]
fn switch_notes_an_upstream_it_set_or_left_alone() {
    let sb = Sandbox::new();
    let col = cloned(&sb);
    sb.git(&col, &["branch", "feature", "main"]);
    cli(&sb, &col)
        .args(["git", "switch", "origin/feature"])
        .assert()
        .code(0)
        .stdout(contains("Switched to feature"))
        .stderr(contains("note: feature now tracks origin/feature."));
    sb.git(&col, &["switch", "-q", "main"]);
    sb.git(
        &col,
        &["branch", "--set-upstream-to=origin/main", "feature"],
    );
    cli(&sb, &col)
        .args(["git", "switch", "origin/feature"])
        .assert()
        .code(0)
        .stdout(contains("Switched to feature"))
        .stderr(contains(
            "note: feature tracks origin/main, not origin/feature; its upstream was left as is.",
        ));
    assert_eq!(
        sb.git(&col, &["rev-parse", "--abbrev-ref", "feature@{upstream}"])
            .trim(),
        "origin/main"
    );
}

#[test]
fn switch_with_changes_refuses_and_an_unknown_branch_fails() {
    let sb = Sandbox::new();
    let col = cloned(&sb);
    write(&col, "new.txt", "x\n");
    cli(&sb, &col)
        .args(["git", "switch", "feature"])
        .assert()
        .code(3)
        .stderr(contains("Commit or discard your changes first"));
    assert_eq!(
        sb.git(&col, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(),
        "main"
    );
    cli(&sb, &col)
        .args(["git", "switch", "nope"])
        .assert()
        .code(3)
        .stderr(contains("no remote branch named nope"));
}

#[test]
fn values_starting_with_a_dash_are_not_taken_as_options() {
    let sb = Sandbox::new();
    cli(&sb, sb.path())
        .args(["git", "clone", "--upload-pack=touch pwned", "col"])
        .assert()
        .code(3)
        .stderr(contains("unexpected argument"));
    cli(&sb, sb.path())
        .args(["git", "clone", "--", "--upload-pack=touch pwned", "col"])
        .assert()
        .code(3)
        .stderr(contains("invalid repository URL"));
    assert!(!sb.path().join("pwned").exists());
    let col = cloned(&sb);
    cli(&sb, &col)
        .args(["git", "switch", "--", "-f"])
        .assert()
        .code(3)
        .stderr(contains("no remote branch named -f"));
}

/// Every credential form, through a real git failing on a dead local address.
#[test]
fn no_clone_url_form_ever_prints_the_password() {
    let sb = Sandbox::new();
    for url in [
        "https://user:s3cr3t@127.0.0.1:1/x.git/",
        "https://user:s3/cr3t@127.0.0.1:1/x.git",
        "https://user:s3?cr3t#x@127.0.0.1:1/x.git",
        "ssh://user:s3cr3t@127.0.0.1:1/x.git",
        "file://user:s3cr3t@127.0.0.1:1/x.git",
        "user:s3cr3t@127.0.0.1:x.git",
    ] {
        let out = cli(&sb, sb.path())
            .args(["git", "clone", url, "col"])
            .assert();
        let all = format!("{}{}", out_text(&out), err_text(&out));
        assert!(
            !all.contains("s3cr3t") && !all.contains("cr3t"),
            "{url}: {all}"
        );
        assert!(
            !all.contains("user:") && !all.contains("@127"),
            "{url}: {all}"
        );
    }
}

#[test]
fn file_urls_print_without_userinfo() {
    let sb = Sandbox::new();
    let out = cli(&sb, sb.path())
        .args([
            "git",
            "clone",
            "file://user:s3cr3t@127.0.0.1:1/x.git",
            "col",
        ])
        .assert();
    assert!(err_text(&out).contains("Cloning file://127.0.0.1:1/x.git"));
}

/// Errors go to stderr once; stdout carries only the result line.
#[cfg(unix)]
#[test]
fn a_failed_clone_prints_the_fatal_line_once_on_stderr_only() {
    let sb = Sandbox::new();
    let body = "#!/bin/sh\ncase \"$*\" in *\" config \"*) exit 1 ;; esac\necho 'fatal: unique-failure-text' >&2\nexit 128\n";
    let bin = sb.fake_git("fake", body);
    let out = cli(&sb, sb.path())
        .env("PATH", path_with(&bin))
        .args(["git", "clone", "/srv/x.git", "col"])
        .assert()
        .code(3);
    assert!(
        !out_text(&out).contains("unique-failure-text"),
        "{}",
        out_text(&out)
    );
    assert_eq!(err_text(&out).matches("unique-failure-text").count(), 1);
}
