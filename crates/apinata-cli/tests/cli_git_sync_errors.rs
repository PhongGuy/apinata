#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]

use crate::cli_git_clone::{cli, cloned, err_text};
use crate::cli_git_sync::{advance, fails, head, local_commit};
use crate::common;
use crate::git_support::{SCRUBBED, Sandbox, path_with, real_git};

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// A git whose call holding ` <word> ` runs `script`; every other call is the real git.
#[cfg(unix)]
fn fake_on(sb: &Sandbox, word: &str, script: &str) -> PathBuf {
    let body = format!(
        "#!/bin/sh\ndir=${{0%/*}}\ncase \"$*\" in *\" {word} \"*) {script} ;; esac\nexec '{}' \"$@\"\n",
        real_git().display()
    );
    sb.fake_git("fake", &body)
}

/// Hangs on a grandchild whose pid lands in `$dir/pid`.
#[cfg(target_os = "linux")]
const HANG: &str =
    "sleep 60 & echo $! > \"$dir/pid.tmp\"; mv \"$dir/pid.tmp\" \"$dir/pid\"; wait; exit 1";

/// Run `apinata <args>` in `col` with `bin` first on PATH, send SIGINT once `marker` exists,
/// and return the exit code and stderr.
#[cfg(target_os = "linux")]
fn interrupted(
    sb: &Sandbox,
    col: &Path,
    args: &[&str],
    bin: &Path,
    marker: &Path,
) -> (Option<i32>, String) {
    let mut cmd = common::apinata_std();
    cmd.args(args).current_dir(col).envs(sb.env());
    for name in SCRUBBED.into_iter().chain(["GIT_SSH", "GIT_SSH_COMMAND"]) {
        cmd.env_remove(name);
    }
    let child = cmd
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("PATH", path_with(bin))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let seen = (0..250).any(|_| {
        std::thread::sleep(Duration::from_millis(20));
        marker.exists()
    });
    let id = child.id().to_string();
    assert!(
        Command::new("kill")
            .args(["-INT", &id])
            .status()
            .unwrap()
            .success()
    );
    let out = child.wait_with_output().unwrap();
    assert!(seen, "{} never appeared", marker.display());
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn diverged_pull_and_rejected_push_say_what_to_run() {
    let sb = Sandbox::new();
    let col = cloned(&sb);
    local_commit(&sb, &col, "d.txt");
    let before = head(&sb, &col);
    advance(&sb, "c.txt");
    let err = fails(&sb, &col, &["git", "pull"]);
    assert!(
        err.contains("diverged") && err.contains("merge or rebase in a terminal"),
        "{err}"
    );
    assert_eq!(head(&sb, &col), before);
    let err = fails(&sb, &col, &["git", "push"]);
    assert!(
        err.contains("rejected") && err.contains("run apinata git pull first"),
        "{err}"
    );
}

#[cfg(unix)]
#[test]
fn a_hook_rejection_shows_the_remote_lines_without_the_pull_hint() {
    let sb = Sandbox::new();
    let col = cloned(&sb);
    local_commit(&sb, &col, "d.txt");
    sb.fake_tool(
        "origin.git/hooks",
        "pre-receive",
        "#!/bin/sh\necho 'nope from hook' >&2\nexit 1\n",
    );
    let err = fails(&sb, &col, &["git", "push"]);
    assert!(
        err.contains("remote: nope from hook") && !err.contains("apinata git pull"),
        "{err}"
    );
}

#[test]
fn push_without_or_with_a_gone_upstream_names_set_upstream() {
    let sb = Sandbox::new();
    let col = cloned(&sb);
    sb.git(&col, &["switch", "-q", "-c", "topic"]);
    let err = fails(&sb, &col, &["git", "push"]);
    assert!(
        err.contains("--set-upstream <REMOTE> to publish topic"),
        "{err}"
    );
    sb.git(
        &col,
        &["switch", "-q", "--track", "-c", "work", "origin/feature"],
    );
    sb.git(
        &sb.path().join("seed"),
        &["push", "-q", "origin", ":feature"],
    );
    sb.git(&col, &["fetch", "-q", "--prune", "origin"]);
    let err = fails(&sb, &col, &["git", "push"]);
    assert!(
        err.contains("--set-upstream <REMOTE> to re-create origin/feature"),
        "{err}"
    );
}

#[cfg(unix)]
#[test]
fn a_partial_pull_gives_the_recovery() {
    let sb = Sandbox::new();
    let col = cloned(&sb);
    advance(&sb, "c.txt");
    let held = col.join(".git/refs/heads/main.lock");
    let bin = fake_on(&sb, "merge", &format!(": > '{}'", held.display()));
    let out = cli(&sb, &col)
        .env("PATH", path_with(&bin))
        .args(["git", "pull"])
        .assert()
        .code(3);
    assert!(
        err_text(&out).contains("git reset --merge HEAD"),
        "{}",
        err_text(&out)
    );
    assert!(held.exists());
}

#[cfg(unix)]
#[test]
fn an_auth_failure_gets_the_credential_hint() {
    let sb = Sandbox::new();
    let col = cloned(&sb);
    sb.git(&col, &["remote", "set-url", "origin", "ssh://fake/x.git"]);
    let bin = sb.fake_tool(
        "sshbin",
        "ssh",
        "#!/bin/sh\necho 'Permission denied (publickey).' >&2\nexit 255\n",
    );
    for cmd in ["pull", "push"] {
        let out = cli(&sb, &col)
            .env("PATH", path_with(&bin))
            .args(["git", cmd])
            .assert()
            .code(3);
        assert!(
            err_text(&out).contains("credential helper or ssh-agent"),
            "{cmd}: {}",
            err_text(&out)
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn ctrl_c_during_the_fetch_or_the_push_exits_130() {
    for (cmd, word, note) in [("pull", "fetch", false), ("push", "push", true)] {
        let sb = Sandbox::new();
        let col = cloned(&sb);
        local_commit(&sb, &col, "d.txt");
        let before = head(&sb, &col);
        let bin = fake_on(&sb, word, HANG);
        let (code, err) = interrupted(&sb, &col, &["git", cmd], &bin, &bin.join("pid"));
        assert_eq!(code, Some(130), "{cmd}: {err}");
        assert_eq!(
            err.contains("may already have reached the remote"),
            note,
            "{cmd}: {err}"
        );
        assert_eq!(head(&sb, &col), before);
        let pid: u32 = std::fs::read_to_string(bin.join("pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(crate::git_support::wait_gone(pid), "{cmd}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn a_killed_push_that_left_a_lock_is_busy_with_the_note() {
    let sb = Sandbox::new();
    let col = cloned(&sb);
    local_commit(&sb, &col, "d.txt");
    let lock = col.join(".git/refs/remotes/origin/main.lock");
    let script = format!(
        "trap '' TERM; mkdir -p '{}'; : > '{}'; : > \"$dir/started\"; sleep 30; exit 1",
        lock.parent().unwrap().display(),
        lock.display()
    );
    let bin = fake_on(&sb, "push", &script);
    let (code, err) = interrupted(&sb, &col, &["git", "push"], &bin, &bin.join("started"));
    assert_eq!(code, Some(3), "{err}");
    assert!(
        err.contains("git is busy")
            && err.contains("note: the push may already have reached the remote"),
        "{err}"
    );
    assert!(lock.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn ctrl_c_after_the_fast_forward_started_still_pulls_with_a_note() {
    let sb = Sandbox::new();
    let col = cloned(&sb);
    let tip = advance(&sb, "c.txt");
    let bin = sb.fake_git(
        "fake",
        &format!(
            "#!/bin/sh\ndir=${{0%/*}}\ncase \"$*\" in *\" merge \"*) : > \"$dir/merging\"; sleep 1 ;; esac\nexec '{}' \"$@\"\n",
            real_git().display()
        ),
    );
    let (code, err) = interrupted(&sb, &col, &["git", "pull"], &bin, &bin.join("merging"));
    assert_eq!(code, Some(0), "{err}");
    assert!(
        err.contains("Ctrl+C came after the fast-forward had started; it finished"),
        "{err}"
    );
    assert_eq!(head(&sb, &col), tip);
}
