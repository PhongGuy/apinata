//! Pull and push under held locks, cancel, ssh and leaking remotes.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

use crate::git_branch::cloned;
use crate::git_clone_env::rerun_without_ambient_ssh;
use crate::git_support::{Sandbox, path_with, real_git};
use crate::git_sync::{advance, clean, local_commit, rev, wrapped};

use std::fs;
use std::path::Path;
use std::time::Duration;

use apinata_core::{CancelFlag, SyncError, pull, push};

/// Run `work` with a flag that another thread sets once `marker` exists (polled up to 5 s).
pub fn cancel_when<T: Send>(marker: &Path, work: impl FnOnce(&CancelFlag) -> T + Send) -> T {
    let cancel = CancelFlag::new();
    std::thread::scope(|s| {
        s.spawn(|| {
            let seen = (0..250).any(|_| {
                std::thread::sleep(Duration::from_millis(20));
                marker.exists()
            });
            cancel.cancel();
            assert!(seen, "{} never appeared", marker.display());
        });
        work(&cancel)
    })
}

#[test]
fn a_lock_held_before_the_pull_is_busy_and_kept() {
    for name in ["index.lock", "HEAD.lock", "refs/heads/main.lock"] {
        let sb = Sandbox::new();
        let root = cloned(&sb);
        let head = rev(&sb, &root, "HEAD");
        advance(&sb, "c.txt");
        let held = root.join(".git").join(name);
        fs::write(&held, "").unwrap();
        let r = pull(&root, &CancelFlag::new(), &sb.options());
        assert!(
            matches!(&r, Err(SyncError::Busy { lock }) if lock.ends_with(name)),
            "{name}: {r:?}"
        );
        assert!(held.exists(), "{name} was deleted");
        assert_eq!(rev(&sb, &root, "HEAD"), head, "{name}");
        assert!(!root.join("c.txt").exists(), "{name}");
    }
}

#[test]
fn a_flag_set_before_the_call_cancels_pull_and_push() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    let old = rev(&sb, &root, "refs/remotes/origin/main");
    let tip = advance(&sb, "c.txt");
    let cancel = CancelFlag::new();
    cancel.cancel();
    let r = pull(&root, &cancel, &sb.options());
    assert!(matches!(r, Err(SyncError::Cancelled)), "{r:?}");
    assert_eq!(rev(&sb, &root, "refs/remotes/origin/main"), old);
    local_commit(&sb, &root, "d.txt");
    let r = push(&root, None, &cancel, &sb.options());
    assert!(matches!(r, Err(SyncError::Cancelled)), "{r:?}");
    assert_eq!(
        rev(&sb, &sb.path().join("origin.git"), "refs/heads/main"),
        tip
    );
}

#[cfg(target_os = "linux")]
#[test]
fn cancel_during_a_hanging_fetch_leaves_the_tree_untouched() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    let head = rev(&sb, &root, "HEAD");
    advance(&sb, "c.txt");
    let body = format!(
        "#!/bin/sh\ndir=${{0%/*}}\ncase \"$*\" in *\" fetch \"*) sleep 60 & echo $! > \"$dir/pid.tmp\"; mv \"$dir/pid.tmp\" \"$dir/pid\"; wait; exit 1 ;; esac\nexec '{}' \"$@\"\n",
        real_git().display()
    );
    let bin = sb.fake_git("hang", &body);
    let opts = sb.options().env("PATH", path_with(&bin));
    let r = cancel_when(&bin.join("pid"), |cancel| pull(&root, cancel, &opts));
    assert!(matches!(r, Err(SyncError::Cancelled)), "{r:?}");
    assert_eq!(rev(&sb, &root, "HEAD"), head);
    assert!(clean(&sb, &root));
    let pid: u32 = fs::read_to_string(bin.join("pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(crate::git_support::wait_gone(pid));
}

#[cfg(unix)]
#[test]
fn a_cancel_after_the_fast_forward_started_still_pulls() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    let tip = advance(&sb, "c.txt");
    let (opts, log) = wrapped(&sb, "wrap", "merge", ": > \"$dir/merging\"; sleep 1", ":");
    let p = cancel_when(&log.with_file_name("merging"), |cancel| {
        pull(&root, cancel, &opts)
    })
    .unwrap();
    assert_eq!(p.to, tip);
    assert_eq!(rev(&sb, &root, "HEAD"), tip);
    assert!(clean(&sb, &root));
}

/// A fake `ssh` that records its arguments and the GIT_/SSH_ variables it saw, then fails the
/// way a key-less login does.
const SSH: &str = "#!/bin/sh\ndir=${0%/*}\nprintf '%s\\n' \"$@\" > \"$dir/ssh-args\"\nenv | grep -E '^(GIT|SSH)_' > \"$dir/ssh-env\"\necho 'Permission denied (publickey).' >&2\nexit 255\n";

#[cfg(unix)]
#[test]
fn pull_and_push_over_ssh_never_prompt() {
    if rerun_without_ambient_ssh() {
        return;
    }
    let sb = Sandbox::new();
    let root = cloned(&sb);
    local_commit(&sb, &root, "d.txt");
    sb.git(&root, &["remote", "set-url", "origin", "ssh://fake/x.git"]);
    let bin = sb.fake_tool("sshbin", "ssh", SSH);
    let opts = sb.options().env("PATH", path_with(&bin));
    for which in ["pull", "push"] {
        let r = match which {
            "pull" => pull(&root, &CancelFlag::new(), &opts).map(|_| ()),
            _ => push(&root, None, &CancelFlag::new(), &opts).map(|_| ()),
        };
        assert!(
            matches!(&r, Err(SyncError::Failed { stderr }) if stderr.contains("Permission denied")),
            "{which}: {r:?}"
        );
        assert!(
            fs::read_to_string(bin.join("ssh-args"))
                .unwrap()
                .contains("BatchMode=yes"),
            "{which}"
        );
        let env = fs::read_to_string(bin.join("ssh-env")).unwrap();
        for want in [
            "GIT_TERMINAL_PROMPT=0",
            "GIT_ASKPASS=",
            "SSH_ASKPASS_REQUIRE=never",
        ] {
            assert!(
                env.lines().any(|l| l == want),
                "{which}: {want} missing in {env}"
            );
        }
        fs::remove_file(bin.join("ssh-args")).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn a_failed_or_rejected_sync_never_carries_the_password() {
    const PW: &str = "s3cr3t";
    let words = format!("fatal: unable to access 'https://user:{PW}@h/r.git/': x user:{PW}@h:r");
    for (on, stdout) in [
        ("fetch", ""),
        ("push", ""),
        (
            "push",
            "!\trefs/heads/main:refs/heads/main\t[rejected] (non-fast-forward)",
        ),
    ] {
        let sb = Sandbox::new();
        let root = cloned(&sb);
        local_commit(&sb, &root, "d.txt");
        let body = format!(
            "#!/bin/sh\ncase \"$*\" in *\" {on} \"*) printf '%s\\n' \"{words}\" >&2; printf '%s\\n' '{stdout}'; exit 1 ;; esac\nexec '{}' \"$@\"\n",
            real_git().display()
        );
        let bin = sb.fake_git("leak", &body);
        let opts = sb.options().env("PATH", path_with(&bin));
        let r = match on {
            "fetch" => pull(&root, &CancelFlag::new(), &opts).map(|_| ()),
            _ => push(&root, None, &CancelFlag::new(), &opts).map(|_| ()),
        };
        let stderr = match (&r, stdout.is_empty()) {
            (Err(SyncError::Failed { stderr }), true) => stderr,
            (Err(SyncError::Rejected { stderr, .. }), false) => stderr,
            other => panic!("{on} {stdout:?}: {other:?}"),
        };
        assert!(stderr.contains("fatal") && !stderr.contains(PW), "{stderr}");
        assert!(!format!("{r:?}").contains(PW));
    }
}
