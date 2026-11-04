//! Hermetic git repositories for the git tests of core, the CLI and MCP. Every git call made
//! here, and every `GitOptions` built here, ignores the machine's git config, keeps `HOME` in
//! the sandbox and cannot discover a repository above it.
#![allow(dead_code)] // each test binary uses a different subset

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use apinata_core::GitOptions;
use tempfile::TempDir;

/// Variables a parent git process (a hook running the tests) may set; none may reach a child.
pub const SCRUBBED: [&str; 6] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_COMMON_DIR",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
];

pub struct Sandbox {
    dir: TempDir,
    identity: bool,
}

impl Sandbox {
    pub fn new() -> Sandbox {
        Sandbox::build(true)
    }

    /// Like `new`, but `env()` sets none of GIT_AUTHOR_NAME/EMAIL and GIT_COMMITTER_NAME/EMAIL
    /// (the dates stay), so identity comes from repo config or is missing.
    pub fn without_identity() -> Sandbox {
        Sandbox::build(false)
    }

    fn build(identity: bool) -> Sandbox {
        let dir = TempDir::new().expect("tempdir");
        fs::create_dir(dir.path().join("home")).expect("home");
        fs::write(dir.path().join("empty.gitconfig"), "").expect("config");
        Sandbox { dir, identity }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Empty global and system config, `HOME` in the sandbox, discovery stopped at the sandbox,
    /// and a fixed identity and date so commits never depend on the machine.
    pub fn env(&self) -> Vec<(OsString, OsString)> {
        let config = self.path().join("empty.gitconfig");
        let mut env: Vec<(OsString, OsString)> = vec![
            ("GIT_CONFIG_GLOBAL".into(), config.clone().into()),
            ("GIT_CONFIG_SYSTEM".into(), config.into()),
            ("HOME".into(), self.path().join("home").into()),
            ("GIT_CEILING_DIRECTORIES".into(), self.path().into()),
        ];
        if self.identity {
            for (key, value) in [
                ("GIT_AUTHOR_NAME", "Test"),
                ("GIT_AUTHOR_EMAIL", "test@example.com"),
                ("GIT_COMMITTER_NAME", "Test"),
                ("GIT_COMMITTER_EMAIL", "test@example.com"),
            ] {
                env.push((key.into(), value.into()));
            }
        }
        for (key, value) in [
            ("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z"),
            ("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z"),
        ] {
            env.push((key.into(), value.into()));
        }
        env
    }

    /// Core options carrying [`Sandbox::env`].
    pub fn options(&self) -> GitOptions {
        self.env()
            .into_iter()
            .fold(GitOptions::new(), |options, (key, value)| {
                options.env(key, value)
            })
    }

    fn command(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut cmd = Command::new("git");
        cmd.args(args).current_dir(cwd).envs(self.env());
        for name in SCRUBBED {
            cmd.env_remove(name);
        }
        cmd
    }

    /// Run git in `cwd`, require success, and return its stdout.
    pub fn git(&self, cwd: &Path, args: &[&str]) -> String {
        let out = self.command(cwd, args).output().expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).expect("utf-8 stdout")
    }

    /// Run git in `cwd`, require failure, and return its stderr.
    pub fn git_fails(&self, cwd: &Path, args: &[&str]) -> String {
        let out = self.command(cwd, args).output().expect("git runs");
        assert!(!out.status.success(), "git {args:?} succeeded");
        String::from_utf8_lossy(&out.stderr).into_owned()
    }

    /// A new repository `name` in the sandbox, on branch `main`, with no commits.
    pub fn repo(&self, name: &str) -> PathBuf {
        self.git(self.path(), &["init", "-q", "-b", "main", name]);
        self.path().join(name)
    }

    /// Stage everything in `repo` and commit it.
    pub fn commit_all(&self, repo: &Path, message: &str) {
        self.git(repo, &["add", "-A"]);
        self.git(repo, &["commit", "-q", "-m", message]);
    }

    /// The first seven characters of `HEAD`'s id, as `RepoStatus::head` reports it.
    pub fn short_head(&self, repo: &Path) -> String {
        self.git(repo, &["rev-parse", "HEAD"]).trim()[..7].to_owned()
    }

    /// An empty directory, for a `PATH` that finds no git.
    pub fn empty_bin(&self) -> PathBuf {
        let dir = self.path().join("empty-bin");
        fs::create_dir_all(&dir).expect("bin");
        dir
    }

    /// A directory `name` holding an executable `git` script with `body`. A child shell
    /// writes it, so no descriptor of this process is open on it when another test forks.
    #[cfg(unix)]
    pub fn fake_git(&self, name: &str, body: &str) -> PathBuf {
        self.fake_tool(name, "git", body)
    }

    /// Like `fake_git` for any program name: a directory holding an executable `name` script.
    #[cfg(unix)]
    pub fn fake_tool(&self, dir: &str, name: &str, body: &str) -> PathBuf {
        let dir = self.path().join(dir);
        fs::create_dir_all(&dir).expect("bin");
        let status = Command::new("sh")
            .arg("-c")
            .arg("printf '%s' \"$1\" > \"$2\" && chmod 755 \"$2\"")
            .arg("sh")
            .arg(body)
            .arg(dir.join(name))
            .status()
            .expect("sh runs");
        assert!(status.success(), "fake tool written");
        dir
    }

    /// A bare `origin.git`: `main` holds opencollection.yml and a.txt, `feature` adds b.txt.
    /// The pushing work tree stays at `<sandbox>/seed` (on `feature`, remote `origin`).
    pub fn origin(&self) -> PathBuf {
        let bare = self.path().join("origin.git");
        self.git(
            self.path(),
            &["init", "-q", "--bare", "-b", "main", "origin.git"],
        );
        let work = self.repo("seed");
        write(
            &work,
            "opencollection.yml",
            "opencollection: \"1.0.0\"\ninfo:\n  name: t\n",
        );
        write(&work, "a.txt", "a\n");
        self.commit_all(&work, "base");
        self.git(&work, &["remote", "add", "origin", bare.to_str().unwrap()]);
        self.git(&work, &["push", "-q", "origin", "main"]);
        self.git(&work, &["switch", "-q", "-c", "feature"]);
        write(&work, "b.txt", "b\n");
        self.commit_all(&work, "feature");
        self.git(&work, &["push", "-q", "origin", "feature"]);
        bare
    }
}

/// `bin` first, then the process `PATH`, for `GitOptions::env("PATH", ..)`.
pub fn path_with(bin: &Path) -> OsString {
    let mut path = OsString::from(bin);
    path.push(":");
    path.push(std::env::var_os("PATH").unwrap_or_default());
    path
}

/// The `git` the process `PATH` finds first, for a fake git that wraps the real one.
pub fn real_git() -> PathBuf {
    std::env::var_os("PATH")
        .unwrap()
        .to_str()
        .unwrap()
        .split(':')
        .map(|d| Path::new(d).join("git"))
        .find(|p| p.is_file())
        .expect("git on PATH")
}

/// Whether `pid` is alive and not a zombie (Linux).
pub fn process_running(pid: u32) -> bool {
    match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => !stat.rsplit(") ").next().unwrap_or("").starts_with('Z'),
        Err(_) => false,
    }
}

/// Polls up to 5 s for `pid` to be gone.
pub fn wait_gone(pid: u32) -> bool {
    (0..100).any(|_| {
        std::thread::sleep(std::time::Duration::from_millis(50));
        !process_running(pid)
    })
}

/// A fake `git` for clone cancel tests: answers the ssh-command lookup with "unset", then creates the
/// destination (its last argument), reports 10%, and hangs on a grandchild whose pid it writes to `pid`.
pub const HANG_GIT: &str = r#"#!/bin/sh
dir=${0%/*}
case "$*" in *" config "*) exit 1 ;; esac
for last; do :; done
mkdir -p "$last"; echo partial > "$last/part"
sleep 60 &
echo $! > "$dir/pid"
echo "Receiving objects:  10% (1/10)" >&2
wait
"#;

/// Write `text` at `rel` under `root`, creating parent directories.
pub fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, text).expect("write");
}

/// The number of pack files in `repo`'s object store.
pub fn packs(repo: &Path) -> usize {
    fs::read_dir(repo.join(".git/objects/pack"))
        .map(|dir| {
            dir.filter_map(Result::ok)
                .filter(|e| e.path().extension().is_some_and(|x| x == "pack"))
                .count()
        })
        .unwrap_or(0)
}

/// Panics with a clear message when the test process itself sets one of the four identity
/// variables, since the child would inherit it and beat repo config.
pub fn assert_no_ambient_identity() {
    for key in [
        "GIT_AUTHOR_NAME",
        "GIT_AUTHOR_EMAIL",
        "GIT_COMMITTER_NAME",
        "GIT_COMMITTER_EMAIL",
    ] {
        assert!(
            std::env::var_os(key).is_none(),
            "{key} is set in the test process; it would override repo config in child git calls"
        );
    }
}
