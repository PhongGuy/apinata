use crate::git_support;

use std::fs;
use std::path::{Path, PathBuf};

use apinata_core::{CancelFlag, CloneError, CloneRequest, GitOptions, clone_repo};
use git_support::{Sandbox, path_with};

fn parent(sb: &Sandbox) -> PathBuf {
    let p = sb.path().join("dest");
    fs::create_dir_all(&p).unwrap();
    p
}

/// An ambient GIT_SSH or GIT_SSH_COMMAND changes what the runner wraps, and a test cannot unset
/// a variable through `GitOptions`. So the calling test re-runs itself in a child without
/// them; returns true when it did, and the caller then returns.
fn rerun_without_ambient_ssh() -> bool {
    if std::env::var_os("GIT_SSH").is_none() && std::env::var_os("GIT_SSH_COMMAND").is_none() {
        return false;
    }
    let name = std::thread::current().name().unwrap().to_owned();
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([name.as_str(), "--exact", "--test-threads=1"])
        .env_remove("GIT_SSH")
        .env_remove("GIT_SSH_COMMAND")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{name} failed in a child without ambient ssh variables"
    );
    // `--exact` with a wrong name runs nothing and still exits 0, so require the test to have run.
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("1 passed"),
        "{name} did not run in the child: {stdout}"
    );
    true
}

fn run(url: &str, parent: &Path, opts: &GitOptions) -> Result<PathBuf, CloneError> {
    clone_repo(
        &CloneRequest::new(url, parent, "col"),
        &CancelFlag::new(),
        opts,
        &mut |_| {},
    )
}

/// A recorder that stands in for git and dumps only the GIT_* and SSH_* variables the clone
/// saw, so a failing assertion never prints the developer's whole environment. It fails
/// `git config` probes (exit 1) so the runner sees no configured ssh command.
#[cfg(unix)]
const RECORD: &str = "#!/bin/sh\ndir=${0%/*}\ncase \"$*\" in *\" config \"*) exit 1 ;; esac\nenv | grep -E '^(GIT|SSH)_' > \"$dir/env\"\nfor last; do :; done\nmkdir -p \"$last\"\n";

#[cfg(unix)]
fn seen_env(opts: impl FnOnce(GitOptions) -> GitOptions) -> Vec<String> {
    let sb = Sandbox::new();
    let bin = sb.fake_git("fake", RECORD);
    let parent = parent(&sb);
    let options = opts(sb.options().env("PATH", path_with(&bin)));
    run("/srv/x.git", &parent, &options).unwrap();
    fs::read_to_string(bin.join("env"))
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[cfg(unix)]
#[test]
fn git_sees_no_way_to_prompt() {
    if rerun_without_ambient_ssh() {
        return;
    }
    let env = seen_env(|o| {
        o.env("GIT_ASKPASS", "/bin/evil")
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("GIT_TERMINAL_PROMPT", "1")
    });
    for want in [
        "GIT_ASKPASS=",
        "SSH_ASKPASS_REQUIRE=never",
        "GIT_TERMINAL_PROMPT=0",
        "GIT_SSH_COMMAND=ssh -o BatchMode=yes",
    ] {
        assert!(env.iter().any(|l| l == want), "{want} missing in {env:?}");
    }
}

#[cfg(unix)]
#[test]
fn a_configured_ssh_command_keeps_its_options_and_gains_batch_mode() {
    if rerun_without_ambient_ssh() {
        return;
    }
    let env = seen_env(|o| o.env("GIT_SSH_COMMAND", "ssh -i /k"));
    assert!(
        env.iter()
            .any(|l| l == "GIT_SSH_COMMAND=ssh -i /k -o BatchMode=yes"),
        "{env:?}"
    );
}

/// A fake `ssh` that records its arguments and fails the way a key-less login does.
#[cfg(unix)]
const SSH: &str = "#!/bin/sh\ndir=${0%/*}\nprintf '%s\\n' \"$@\" > \"$dir/ssh-args\"\necho 'Permission denied (publickey).' >&2\nexit 255\n";

#[cfg(unix)]
#[test]
fn real_git_runs_ssh_in_batch_mode_and_honours_core_ssh_command() {
    if rerun_without_ambient_ssh() {
        return;
    }
    let sb = Sandbox::new();
    let bin = sb.fake_tool("sshbin", "ssh", SSH);
    let config = sb.path().join("ssh.gitconfig");
    fs::write(&config, "[core]\n\tsshCommand = ssh -i /configured-key\n").unwrap();
    let opts = sb
        .options()
        .env("PATH", path_with(&bin))
        .env("GIT_CONFIG_GLOBAL", &config);
    let parent = parent(&sb);
    let r = run("ssh://git@example.invalid/x.git", &parent, &opts);
    assert!(
        matches!(&r, Err(CloneError::Failed { stderr }) if stderr.contains("Permission denied")),
        "{r:?}"
    );
    let args = fs::read_to_string(bin.join("ssh-args")).unwrap();
    assert!(
        args.contains("BatchMode=yes") && args.contains("/configured-key"),
        "{args}"
    );
    assert!(!parent.join("col").exists());
}
