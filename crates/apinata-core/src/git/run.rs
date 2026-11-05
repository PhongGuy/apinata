//! One git call (read or write): fixed flags and environment, a time limit, and the raw result.

use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use super::{GitError, GitOptions};

/// Flags that keep every call from taking optional locks, starting an fsmonitor daemon or running
/// an `ext::` transport helper, whatever the user's config allows.
pub(super) const PREFIX: [&str; 5] = [
    "--no-optional-locks",
    "-c",
    "core.fsmonitor=false",
    "-c",
    "protocol.ext.allow=never",
];

/// Variables that would point discovery away from the collection root.
pub(super) const SCRUBBED: [&str; 8] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
    "GIT_PREFIX",
];

/// Set last, so neither the caller's environment nor `GitOptions::env` can override them.
pub(super) const FIXED: [(&str, &str); 4] = [
    ("GIT_OPTIONAL_LOCKS", "0"),
    ("GIT_NO_LAZY_FETCH", "1"),
    ("GIT_TERMINAL_PROMPT", "0"),
    ("LC_ALL", "C"),
];

/// The least time the pipe readers get after git exits.
const PIPE_GRACE: Duration = Duration::from_millis(250);

/// How long a stopped git and the rest of its group get to clean up and exit after SIGTERM,
/// before SIGKILL.
#[cfg(unix)]
const TERM_GRACE: Duration = Duration::from_secs(2);

/// The limit of the lock lookup after a write had to be killed.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(2);

/// What a finished git call produced.
pub(crate) struct Output {
    pub(crate) status: ExitStatus,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

impl Output {
    /// `Failed` for this call, carrying the command line and git's trimmed stderr.
    pub(crate) fn failed<S: AsRef<OsStr>>(&self, args: &[S]) -> GitError {
        GitError::Failed {
            command: command_line(args),
            stderr: String::from_utf8_lossy(&self.stderr).trim().to_owned(),
        }
    }
}

/// `git` followed by `args`, for error messages.
fn command_line<S: AsRef<OsStr>>(args: &[S]) -> String {
    let mut line = String::from("git");
    for arg in args {
        line.push(' ');
        line.push_str(&arg.as_ref().to_string_lossy());
    }
    line
}

/// Run `git <PREFIX> <args>` in `cwd`. A spawn failure (git missing is `NotFound`) comes back
/// as `GitError::Io`; a call still running at the limit, or holding its pipes past it and
/// `PIPE_GRACE`, is stopped (see [`stop_group`]) and is `Timeout`.
pub(crate) fn git<S: AsRef<OsStr>>(
    args: &[S],
    cwd: &Path,
    options: &GitOptions,
) -> Result<Output, GitError> {
    let argv: Vec<OsString> = PREFIX
        .iter()
        .map(OsString::from)
        .chain(args.iter().map(|a| a.as_ref().to_owned()))
        .collect();
    spawn(&argv, None, cwd, options).map_err(Halt::into_error)
}

/// Like [`git`], with `input` written to git's stdin.
pub(crate) fn git_input<S: AsRef<OsStr>>(
    args: &[S],
    input: Vec<u8>,
    cwd: &Path,
    options: &GitOptions,
) -> Result<Output, GitError> {
    let argv: Vec<OsString> = PREFIX
        .iter()
        .map(OsString::from)
        .chain(args.iter().map(|a| a.as_ref().to_owned()))
        .collect();
    spawn(&argv, Some(input), cwd, options).map_err(Halt::into_error)
}

/// Like [`git`] for a call that changes the repository: the argv is `PREFIX`,
/// `--literal-pathspecs`, a `-c` per entry of `config`, then `args`. A git refused because
/// another holds a lock (index, HEAD or a ref) is `Busy`, and so is a timed-out git that had to
/// be killed while such a lock exists. No lock is ever deleted: whose it is cannot be proven.
pub(crate) fn git_write<S: AsRef<OsStr>>(
    args: &[S],
    config: &[&str],
    cwd: &Path,
    options: &GitOptions,
) -> Result<Output, GitError> {
    match spawn(&write_argv(args, config), None, cwd, options) {
        Ok(out) => match held_lock(&out, cwd) {
            Some(lock) => Err(GitError::Busy { lock }),
            None => Ok(out),
        },
        Err(Halt::Killed) => Err(match stale_lock(cwd, options, is_commit(args)) {
            Some(lock) => GitError::Busy { lock },
            None => GitError::Timeout,
        }),
        Err(Halt::Error(e)) => Err(e),
    }
}

/// The lock named by git's refusal, if that is why this call failed.
fn held_lock(out: &Output, cwd: &Path) -> Option<PathBuf> {
    if out.status.success() {
        return None;
    }
    refused_lock(&String::from_utf8_lossy(&out.stderr), cwd)
}

/// The lock named by git's "Unable to create '<lock>': File exists" refusal in `stderr`,
/// joined to `cwd` when git printed it relative.
pub(super) fn refused_lock(stderr: &str, cwd: &Path) -> Option<PathBuf> {
    let (_, rest) = stderr.split_once("Unable to create '")?;
    let (lock, _) = rest.split_once("': File exists")?;
    lock.ends_with(".lock").then(|| cwd.join(lock))
}

/// Whether `args` is a commit, the only write here that takes HEAD and ref locks.
fn is_commit<S: AsRef<OsStr>>(args: &[S]) -> bool {
    args.first().is_some_and(|a| a.as_ref() == "commit")
}

/// The first existing lock a killed write may have left: `index.lock`, then for a commit
/// (`refs`) HEAD, the current branch and packed-refs, each lookup under a short limit of its
/// own. A failed branch lookup (detached HEAD) only drops the branch from the list.
fn stale_lock(cwd: &Path, options: &GitOptions, refs: bool) -> Option<PathBuf> {
    let lookup = options.clone().timeout(LOOKUP_TIMEOUT);
    let mut names = vec!["index.lock".to_owned()];
    if refs {
        names.push("HEAD.lock".to_owned());
        names.extend(
            git(&["symbolic-ref", "-q", "HEAD"], cwd, &lookup)
                .ok()
                .filter(|out| out.status.success())
                .and_then(|out| String::from_utf8(out.stdout).ok())
                .map(|name| format!("{}.lock", name.trim_end_matches('\n'))),
        );
        names.push("packed-refs.lock".to_owned());
    }
    git_paths(cwd, options, &names)?
        .into_iter()
        .find(|lock| lock.exists())
}

/// Where each of `names` lives in the repository at `cwd` (`rev-parse --git-path`, so linked
/// worktrees resolve), joined to `cwd`, in order; `None` when the lookup fails or outlives
/// `LOOKUP_TIMEOUT`.
pub(super) fn git_paths<S: AsRef<str>>(
    cwd: &Path,
    options: &GitOptions,
    names: &[S],
) -> Option<Vec<PathBuf>> {
    let mut args = vec!["rev-parse"];
    for name in names {
        args.extend(["--git-path", name.as_ref()]);
    }
    let out = git(&args, cwd, &options.clone().timeout(LOOKUP_TIMEOUT)).ok()?;
    if !out.status.success() {
        return None;
    }
    let paths = String::from_utf8(out.stdout).ok()?;
    Some(paths.lines().map(|p| cwd.join(p)).collect())
}

/// Why [`spawn`] returned no output: an error, or a timeout that ended in SIGKILL.
enum Halt {
    Error(GitError),
    Killed,
}

impl Halt {
    fn into_error(self) -> GitError {
        match self {
            Halt::Error(e) => e,
            Halt::Killed => GitError::Timeout,
        }
    }
}

impl From<GitError> for Halt {
    fn from(e: GitError) -> Halt {
        Halt::Error(e)
    }
}

impl From<std::io::Error> for Halt {
    fn from(e: std::io::Error) -> Halt {
        Halt::Error(e.into())
    }
}

/// Stop `child` and its process group: SIGTERM first, so git and its hooks can clean up (git
/// removes its own lock files), then SIGKILL to whatever of the group outlives `TERM_GRACE`.
/// Reaps the child, and is true when git itself needed SIGKILL (always so on Windows).
pub(super) fn stop_group(child: &mut Child) -> bool {
    #[cfg(unix)]
    if signal_group(child, "-TERM") {
        let deadline = Instant::now() + TERM_GRACE;
        while Instant::now() < deadline {
            match child.try_wait() {
                // Git's hooks and filters share the grace: wait for the group to empty, then
                // KILL any member that ignored TERM (an emptied group's ESRCH is harmless).
                Ok(Some(_)) => {
                    while Instant::now() < deadline && signal_group(child, "-0") {
                        thread::sleep(Duration::from_millis(20));
                    }
                    signal_group(child, "-KILL");
                    return false;
                }
                Ok(None) => thread::sleep(Duration::from_millis(5)),
                Err(_) => break,
            }
        }
        // Git is still unreaped here, so its pid still names the group it leads.
        if !signal_group(child, "-KILL") {
            let _ = child.kill();
        }
        let _ = child.wait();
        return true;
    }
    // Without the `kill` helper, or on Windows, git is killed at once. On Windows that is
    // TerminateProcess, which skips git's lock cleanup; the gentler ExitProcess-in-the-child
    // stop that Git for Windows' own kill uses is not done here.
    let _ = child.kill();
    let _ = child.wait();
    true
}

/// Send `signal` to the process group `child` leads, through the `kill` helper.
#[cfg(unix)]
fn signal_group(child: &Child, signal: &str) -> bool {
    Command::new("kill")
        .args([signal, "--", &format!("-{}", child.id())])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// The full argv of a write call, without the leading `git`.
fn write_argv<S: AsRef<OsStr>>(args: &[S], config: &[&str]) -> Vec<OsString> {
    let mut argv: Vec<OsString> = PREFIX.iter().map(OsString::from).collect();
    argv.push("--literal-pathspecs".into());
    for c in config {
        argv.push("-c".into());
        argv.push((*c).into());
    }
    argv.extend(args.iter().map(|a| a.as_ref().to_owned()));
    argv
}

/// Spawn git with the already-built `argv` under the fixed environment and the time limit, its
/// stdin fed `input` or else empty.
fn spawn(
    argv: &[OsString],
    input: Option<Vec<u8>>,
    cwd: &Path,
    options: &GitOptions,
) -> Result<Output, Halt> {
    let mut cmd = Command::new("git");
    cmd.args(argv)
        .current_dir(cwd)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in &options.env {
        cmd.env(key, value);
    }
    for name in SCRUBBED {
        cmd.env_remove(name);
    }
    cmd.envs(FIXED);
    // Git leads its own group, so a stop reaches its filters and hooks too.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);

    let deadline = Instant::now() + options.timeout;
    let mut child = cmd.spawn()?;
    // Each pipe drains on its own thread so a full pipe never stalls the child; the threads are
    // never joined, because a grandchild may keep a pipe open long after git is gone.
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    // The input goes in from its own thread too, so git stuck before reading it cannot stall
    // the time limit; a git that exits early only makes the write fail, which is ignored.
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });
    }

    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let killed = stop_group(&mut child);
            return Err(if killed {
                Halt::Killed
            } else {
                Halt::Error(GitError::Timeout)
            });
        }
        thread::sleep(Duration::from_millis(5));
    };
    // A git that exits just before the deadline still gets `PIPE_GRACE` for its readers to
    // deliver; only a pipe held open past both (by a grandchild) is a timeout.
    let collect = |rx: mpsc::Receiver<std::io::Result<Vec<u8>>>| {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(remaining.max(PIPE_GRACE)) {
            Ok(bytes) => bytes.map_err(GitError::from),
            Err(_) => Err(GitError::Timeout),
        }
    };
    let stdout = collect(stdout)?;
    let stderr = collect(stderr)?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

/// Read `pipe` to its end on a new thread and send the bytes once.
fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> mpsc::Receiver<std::io::Result<Vec<u8>>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = match pipe {
            Some(mut pipe) => pipe.read_to_end(&mut bytes).map(|_| bytes),
            None => Ok(bytes),
        };
        let _ = tx.send(result);
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::write_argv;

    #[test]
    fn write_argv_adds_literal_pathspecs_and_config_before_the_subcommand() {
        let argv = write_argv(&["commit", "-q"], &["user.useConfigOnly=true"]);
        let got: Vec<&str> = argv.iter().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(
            got,
            [
                "--no-optional-locks",
                "-c",
                "core.fsmonitor=false",
                "-c",
                "protocol.ext.allow=never",
                "--literal-pathspecs",
                "-c",
                "user.useConfigOnly=true",
                "commit",
                "-q"
            ]
        );
        assert_eq!(write_argv(&["add"], &[]).len(), 7);
    }

    #[test]
    fn refused_lock_reads_the_fetch_and_switch_refusals() {
        use super::refused_lock;
        use std::path::Path;
        let cwd = Path::new("/repo");
        let fetch = "error: cannot lock ref 'refs/remotes/origin/main': Unable to create \
                     '/repo/.git/refs/remotes/origin/main.lock': File exists.\n\nAnother git \
                     process seems to be running in this repository";
        assert_eq!(
            refused_lock(fetch, cwd).as_deref(),
            Some(Path::new("/repo/.git/refs/remotes/origin/main.lock"))
        );
        let switch = "fatal: Unable to create '.git/HEAD.lock': File exists.";
        assert_eq!(
            refused_lock(switch, cwd).as_deref(),
            Some(Path::new("/repo/.git/HEAD.lock"))
        );
        let other = "fatal: Unable to create 'out/a.txt': File exists.";
        assert_eq!(refused_lock(other, cwd), None);
        assert_eq!(refused_lock("fatal: could not read", cwd), None);
    }
}

#[cfg(all(test, target_os = "linux"))]
pub(super) mod stop_tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    /// Long enough that a hook or filter has started under load before the stop arrives.
    const LIMIT: Duration = Duration::from_secs(3);

    /// A hermetic repository with a local identity, its write options limited to `LIMIT`, and
    /// a scratch directory beside it for pid and marker files.
    pub(in crate::git) struct Repo {
        dir: tempfile::TempDir,
        pub(in crate::git) root: PathBuf,
        options: GitOptions,
    }

    impl Repo {
        pub(in crate::git) fn new() -> Repo {
            let dir = tempfile::tempdir().unwrap();
            let config = dir.path().join("empty.gitconfig");
            fs::write(&config, "").unwrap();
            fs::create_dir(dir.path().join("home")).unwrap();
            let root = dir.path().join("repo");
            let options = GitOptions::new()
                .timeout(LIMIT)
                .env("GIT_CONFIG_GLOBAL", &config)
                .env("GIT_CONFIG_SYSTEM", &config)
                .env("HOME", dir.path().join("home"))
                .env("GIT_CEILING_DIRECTORIES", dir.path());
            let repo = Repo { dir, root, options };
            repo.setup(repo.dir.path(), &["init", "-q", "-b", "main", "repo"]);
            repo.setup(&repo.root, &["config", "user.name", "Test"]);
            repo.setup(&repo.root, &["config", "user.email", "test@example.com"]);
            repo
        }

        fn scratch(&self, name: &str) -> PathBuf {
            self.dir.path().join(name)
        }

        fn setup(&self, cwd: &Path, args: &[&str]) {
            let mut cmd = Command::new("git");
            cmd.args(args).current_dir(cwd);
            for (key, value) in &self.options.env {
                cmd.env(key, value);
            }
            for name in SCRUBBED {
                cmd.env_remove(name);
            }
            let out = cmd.output().unwrap();
            assert!(out.status.success(), "git {args:?}: {out:?}");
        }

        fn hook(&self, name: &str, body: &str) {
            let path = self.root.join(".git/hooks").join(name);
            fs::write(&path, body).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        /// A file staged and ready for `commit`.
        fn staged(&self) {
            fs::write(self.root.join("a.txt"), "a\n").unwrap();
            self.setup(&self.root, &["add", "a.txt"]);
        }

        fn lock(&self) -> PathBuf {
            self.root.join(".git/index.lock")
        }

        fn write(&self, args: &[&str]) -> Result<Output, GitError> {
            git_write(args, &[], &self.root, &self.options)
        }
    }

    fn running(pid: u32) -> bool {
        match fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => !stat.rsplit(") ").next().unwrap_or("").starts_with('Z'),
            Err(_) => false,
        }
    }

    /// Fails clearly when the hook or filter that writes `file` never started within the limit.
    fn started(file: &Path) {
        assert!(
            file.exists(),
            "the hook or filter had not started within {LIMIT:?}; the machine is too loaded"
        );
    }

    /// The pid written to `file`, once it is gone (polled up to 5 s).
    fn gone(file: &Path) -> bool {
        started(file);
        let pid: u32 = fs::read_to_string(file).unwrap().trim().parse().unwrap();
        (0..100).any(|_| {
            thread::sleep(Duration::from_millis(50));
            !running(pid)
        })
    }

    #[test]
    fn a_timed_out_add_leaves_no_index_lock_and_no_filter() {
        let repo = Repo::new();
        let pid = repo.scratch("pid");
        let filter = format!("echo $$ > '{}'; exec sleep 30", pid.display());
        repo.setup(&repo.root, &["config", "filter.slow.clean", &filter]);
        fs::write(repo.root.join(".gitattributes"), "* filter=slow\n").unwrap();
        fs::write(repo.root.join("a.txt"), "a\n").unwrap();

        let r = repo.write(&["add", "-A", "--", "."]);
        assert!(matches!(r, Err(GitError::Timeout)), "{:?}", r.err());
        assert!(!repo.lock().exists(), "index.lock left behind");
        assert!(gone(&pid), "the clean filter survived the timeout");
    }

    #[test]
    fn a_timed_out_commit_leaves_no_index_lock_and_no_hook() {
        let repo = Repo::new();
        repo.staged();
        let pid = repo.scratch("pid");
        repo.hook(
            "pre-commit",
            &format!("#!/bin/sh\necho $$ > '{}'\nexec sleep 30\n", pid.display()),
        );

        let r = repo.write(&["commit", "-q", "-m", "x"]);
        assert!(matches!(r, Err(GitError::Timeout)), "{:?}", r.err());
        assert!(!repo.lock().exists(), "index.lock left behind");
        assert!(gone(&pid), "the hook survived the timeout");
    }

    #[test]
    fn a_commit_stuck_opening_a_pipe_ends_at_the_limit_with_no_lock() {
        let repo = Repo::new();
        repo.staged();
        let attributes = repo.root.join(".gitattributes");
        assert!(
            Command::new("mkfifo")
                .arg(&attributes)
                .status()
                .unwrap()
                .success()
        );
        let started = Instant::now();
        let r = repo.write(&["commit", "-q", "-m", "x"]);
        assert!(matches!(r, Err(GitError::Timeout)), "{:?}", r.err());
        assert!(
            started.elapsed() < LIMIT + TERM_GRACE + 2 * LOOKUP_TIMEOUT + Duration::from_secs(1)
        );
        assert!(!repo.lock().exists(), "index.lock left behind");
    }

    #[test]
    fn a_hook_gets_term_before_any_kill() {
        let repo = Repo::new();
        repo.staged();
        let marker = repo.scratch("marker");
        let ready = repo.scratch("ready");
        repo.hook(
            "pre-commit",
            &format!(
                "#!/bin/sh\ntrap 'sleep 0.3; echo term > \"{}\"; exit 1' TERM\n: > '{}'\nsleep 30 &\nwait\n",
                marker.display(),
                ready.display()
            ),
        );

        let r = repo.write(&["commit", "-q", "-m", "x"]);
        started(&ready);
        assert!(matches!(r, Err(GitError::Timeout)), "{:?}", r.err());
        assert_eq!(fs::read_to_string(&marker).ok().as_deref(), Some("term\n"));
    }

    /// Ignores TERM, takes the lock for `add` and hangs; answers the lock lookup.
    const STUBBORN_GIT: &str = r#"#!/bin/sh
trap '' TERM
case "$*" in
  *" add "*) : > .git/index.lock; sleep 30 ;;
  *rev-parse*) printf '.git/index.lock\n' ;;
esac
"#;

    /// Options whose PATH finds `script` as git first. A thread forking elsewhere can hold the
    /// script open for writing a moment longer, so it is probed until it no longer runs busy.
    pub(in crate::git) fn fake_git(repo: &Repo, script: &str) -> GitOptions {
        let bin = repo.scratch("bin");
        fs::create_dir(&bin).unwrap();
        fs::write(bin.join("git"), script).unwrap();
        fs::set_permissions(bin.join("git"), fs::Permissions::from_mode(0o755)).unwrap();
        let ready = (0..100).any(
            |_| match Command::new(bin.join("git")).arg("probe").status() {
                Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                    thread::sleep(Duration::from_millis(20));
                    false
                }
                other => {
                    other.unwrap();
                    true
                }
            },
        );
        assert!(ready, "the fake git stayed busy");
        repo.options
            .clone()
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
    }

    /// A git that ignores TERM and hangs in `commit` after creating `.git/<lock>` (none when
    /// empty). `symbolic-ref` names main, or fails when `detached`; each `--git-path X` maps to
    /// `.git/X`.
    fn stubborn_commit(lock: &str, detached: bool) -> String {
        let take = if lock.is_empty() {
            String::new()
        } else {
            format!(": > .git/{lock}; ")
        };
        let head = if detached {
            "exit 1"
        } else {
            "printf 'refs/heads/main\\n'"
        };
        format!(
            r#"#!/bin/sh
trap '' TERM
case "$*" in
  *" commit "*) {take}sleep 30 ;;
  *symbolic-ref*) {head} ;;
  *rev-parse*) prev=; for a in "$@"; do [ "$prev" = --git-path ] && printf '.git/%s\n' "$a"; prev=$a; done ;;
esac
"#
        )
    }

    /// Runs a killed write of `args` under `script`, checking it took no longer than the limit,
    /// the grace and both lock lookups.
    fn killed_write(repo: &Repo, script: &str, args: &[&str]) -> Result<Output, GitError> {
        let options = fake_git(repo, script);
        let started = Instant::now();
        let r = git_write(args, &[], &repo.root, &options);
        let bound = LIMIT + TERM_GRACE + 2 * LOOKUP_TIMEOUT + Duration::from_secs(1);
        assert!(started.elapsed() < bound, "took {:?}", started.elapsed());
        r
    }

    fn killed_commit(repo: &Repo, script: &str) -> Result<Output, GitError> {
        killed_write(repo, script, &["commit", "-q", "-m", "x"])
    }

    #[test]
    fn a_git_killed_after_the_grace_reports_its_lock_and_keeps_it() {
        let repo = Repo::new();
        let r = killed_write(&repo, STUBBORN_GIT, &["add", "-A", "--", "."]);
        match r {
            Err(GitError::Busy { lock }) => assert_eq!(lock, repo.lock()),
            other => panic!("expected Busy, got {:?}", other.err()),
        }
        assert!(repo.lock().exists(), "a lock was deleted");
    }

    #[test]
    fn a_killed_commit_reports_a_stale_head_ref_or_packed_refs_lock_and_keeps_it() {
        for name in ["HEAD.lock", "refs/heads/main.lock", "packed-refs.lock"] {
            let repo = Repo::new();
            let held = repo.root.join(".git").join(name);
            match killed_commit(&repo, &stubborn_commit(name, false)) {
                Err(GitError::Busy { lock }) => assert_eq!(lock, held, "{name}"),
                other => panic!("{name}: expected Busy, got {:?}", other.err()),
            }
            assert!(held.exists(), "{name} was deleted");
        }
    }

    #[test]
    fn a_killed_commit_on_a_detached_head_still_reports_head_lock() {
        let repo = Repo::new();
        let held = repo.root.join(".git/HEAD.lock");
        match killed_commit(&repo, &stubborn_commit("HEAD.lock", true)) {
            Err(GitError::Busy { lock }) => assert_eq!(lock, held),
            other => panic!("expected Busy, got {:?}", other.err()),
        }
        assert!(held.exists(), "HEAD.lock was deleted");
    }

    #[test]
    fn a_killed_add_ignores_a_foreign_head_lock() {
        let repo = Repo::new();
        let held = repo.root.join(".git/HEAD.lock");
        fs::write(&held, "").unwrap();
        let script = stubborn_commit("", false).replace(r#"*" commit "*)"#, r#"*" add "*)"#);
        let r = killed_write(&repo, &script, &["add", "-A", "--", "."]);
        assert!(matches!(r, Err(GitError::Timeout)), "{:?}", r.err());
        assert!(held.exists(), "HEAD.lock was deleted");
    }

    #[test]
    fn a_killed_commit_with_no_lock_left_is_a_timeout() {
        let repo = Repo::new();
        let r = killed_commit(&repo, &stubborn_commit("", false));
        assert!(matches!(r, Err(GitError::Timeout)), "{:?}", r.err());
    }

    /// A commit refused by a lock that exists before it runs is `Busy` naming that lock.
    fn assert_commit_busy(repo: &Repo, held: &Path) {
        fs::write(held, "").unwrap();
        match repo.write(&["commit", "-q", "-m", "x"]) {
            Err(GitError::Busy { lock }) => {
                assert_eq!(lock.canonicalize().unwrap(), held.canonicalize().unwrap())
            }
            other => panic!("expected Busy, got {:?}", other.map(|o| o.status)),
        }
        assert!(held.exists(), "the lock was deleted");
    }

    #[test]
    fn a_head_lock_held_before_a_commit_is_busy_and_kept() {
        let repo = Repo::new();
        repo.staged();
        assert_commit_busy(&repo, &repo.root.join(".git/HEAD.lock"));
    }

    #[test]
    fn a_branch_lock_held_before_a_commit_is_busy_and_kept() {
        let repo = Repo::new();
        repo.staged();
        repo.setup(&repo.root, &["commit", "-q", "-m", "first"]);
        fs::write(repo.root.join("a.txt"), "b\n").unwrap();
        repo.setup(&repo.root, &["add", "a.txt"]);
        assert_commit_busy(&repo, &repo.root.join(".git/refs/heads/main.lock"));
    }

    #[test]
    fn a_foreign_lock_taken_during_a_slow_post_commit_is_kept() {
        let repo = Repo::new();
        repo.staged();
        let started = repo.scratch("started");
        repo.hook(
            "post-commit",
            &format!("#!/bin/sh\n: > '{}'\nsleep 30\n", started.display()),
        );
        // Another git takes the lock once commit has released it and runs its post-commit hook.
        let lock = repo.lock();
        let taker = thread::spawn(move || {
            let seen = (0..LIMIT.as_millis() / 20).any(|_| {
                thread::sleep(Duration::from_millis(20));
                started.exists()
            });
            if seen {
                fs::write(&lock, "").unwrap();
            }
            seen
        });

        let r = repo.write(&["commit", "-q", "-m", "x"]);
        assert!(taker.join().unwrap(), "the post-commit hook never ran");
        assert!(
            matches!(r, Err(GitError::Timeout | GitError::Busy { .. })),
            "{:?}",
            r.err()
        );
        assert!(repo.lock().exists(), "a foreign lock was deleted");
    }

    #[test]
    fn a_lock_held_before_the_call_is_busy_and_kept() {
        let repo = Repo::new();
        fs::write(repo.root.join("a.txt"), "a\n").unwrap();
        fs::write(repo.lock(), "").unwrap();

        match repo.write(&["add", "-A", "--", "."]) {
            Err(GitError::Busy { lock }) => {
                assert_eq!(
                    lock.canonicalize().unwrap(),
                    repo.lock().canonicalize().unwrap()
                )
            }
            other => panic!("expected Busy, got {:?}", other.map(|o| o.status)),
        }
        assert!(repo.lock().exists());
    }
}
