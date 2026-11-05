//! A streaming git runner: no time limit, stderr delivered line by line, cancellable.

use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::io::{self, Read};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::GitOptions;
use super::run::{self, FIXED, PREFIX, SCRUBBED};

/// How long the reader threads get after the child exits to deliver its last lines.
const PIPE_GRACE: Duration = Duration::from_millis(250);

/// How many trailing stderr lines a finished call keeps for error messages.
const KEPT_LINES: usize = 50;

/// A shared cancel switch: every clone sees the same flag.
#[derive(Clone, Debug, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    /// A flag that is not set.
    pub fn new() -> CancelFlag {
        CancelFlag::default()
    }

    /// Set the flag; a running call that watches it stops as soon as it next looks.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Whether `cancel` has been called on this flag or any clone of it.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// The most stdout a capturing call keeps; the rest is read and dropped.
pub(crate) const STDOUT_CAP: usize = 64 * 1024;

/// One environment change: `None` removes the variable.
pub(crate) type EnvChange = (OsString, Option<OsString>);

/// What a finished (not cancelled) call produced.
#[derive(Debug)]
pub(crate) struct Streamed {
    pub(crate) success: bool,
    /// The last `KEPT_LINES` stderr lines joined by newlines.
    pub(crate) stderr: String,
    /// The first `STDOUT_CAP` bytes of stdout (lossy UTF-8); empty for the plain form.
    pub(crate) stdout: String,
}

/// Why a streamed call did not finish.
#[derive(Debug)]
pub(crate) enum StreamError {
    /// Stopped by the flag; `killed` when the child outlived SIGTERM, so its locks may remain.
    Cancelled {
        killed: bool,
    },
    Io(io::Error),
}

impl From<io::Error> for StreamError {
    fn from(e: io::Error) -> StreamError {
        StreamError::Io(e)
    }
}

/// Run `program` with `args` in `cwd`, handing each stderr line (split on `\r` and `\n`, blank
/// lines dropped) to `on_line` on this thread. The child leads its own process group so a cancel
/// takes its helpers down too. There is no time limit; only `cancel` stops it early.
pub(crate) fn stream_program(
    program: &OsStr,
    args: &[OsString],
    cwd: &Path,
    env: &[EnvChange],
    cancel: &CancelFlag,
    on_line: &mut dyn FnMut(&str),
) -> Result<Streamed, StreamError> {
    stream_body(program, args, cwd, env, cancel, on_line, false)
}

/// Like `stream_program`, but also keeps the first `STDOUT_CAP` bytes of stdout. Stdout is read
/// to the end even past the cap, so a chatty child never blocks on a full pipe.
pub(crate) fn stream_program_capture(
    program: &OsStr,
    args: &[OsString],
    cwd: &Path,
    env: &[EnvChange],
    cancel: &CancelFlag,
    on_line: &mut dyn FnMut(&str),
) -> Result<Streamed, StreamError> {
    stream_body(program, args, cwd, env, cancel, on_line, true)
}

fn stream_body(
    program: &OsStr,
    args: &[OsString],
    cwd: &Path,
    env: &[EnvChange],
    cancel: &CancelFlag,
    on_line: &mut dyn FnMut(&str),
    capture: bool,
) -> Result<Streamed, StreamError> {
    if cancel.is_cancelled() {
        return Err(StreamError::Cancelled { killed: false });
    }
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(if capture {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::piped());
    for (key, value) in env {
        match value {
            Some(v) => cmd.env(key, v),
            None => cmd.env_remove(key),
        };
    }
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);

    let mut child = cmd.spawn()?;
    let rx = read_lines(child.stderr.take());
    let out = capture.then(|| read_capped(child.stdout.take()));

    let mut tail: VecDeque<String> = VecDeque::new();
    let mut keep = |line: String, on_line: &mut dyn FnMut(&str)| {
        on_line(&line);
        if tail.len() == KEPT_LINES {
            tail.pop_front();
        }
        tail.push_back(line);
    };

    let status = loop {
        match rx.recv_timeout(Duration::from_millis(5)) {
            Ok(line) => keep(line, on_line),
            // The pipe is closed; the child is about to exit, so do not spin while waiting.
            Err(RecvTimeoutError::Disconnected) => thread::sleep(Duration::from_millis(5)),
            Err(RecvTimeoutError::Timeout) => {}
        }
        // Checked before the exit poll so a callback that sets the flag is honoured at once.
        if cancel.is_cancelled() {
            let killed = kill_tree(&mut child);
            return Err(StreamError::Cancelled { killed });
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
    };

    // The child is gone; collect what its readers still hold, but never wait on a grandchild
    // that keeps the pipe open.
    let deadline = Instant::now() + PIPE_GRACE;
    while let Ok(line) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        keep(line, on_line);
    }
    // Same deadline for stdout; the thread is not joined, the buffer is read as it stands.
    let stdout = out.map_or_else(String::new, |(buf, done)| {
        let _ = done.recv_timeout(deadline.saturating_duration_since(Instant::now()));
        let bytes = buf.lock().unwrap_or_else(|e| e.into_inner());
        String::from_utf8_lossy(&bytes).into_owned()
    });
    Ok(Streamed {
        success: status.success(),
        stderr: tail.into_iter().collect::<Vec<_>>().join("\n"),
        stdout,
    })
}

/// Read `pipe` to EOF on a thread, keeping the first `STDOUT_CAP` bytes. The receiver gets a
/// message (or disconnects) when the thread is done.
fn read_capped(
    pipe: Option<impl Read + Send + 'static>,
) -> (Arc<Mutex<Vec<u8>>>, mpsc::Receiver<()>) {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = mpsc::channel();
    if let Some(mut pipe) = pipe {
        let keep = Arc::clone(&buf);
        thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            while let Ok(n) = pipe.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                let mut held = keep.lock().unwrap_or_else(|e| e.into_inner());
                let room = STDOUT_CAP.saturating_sub(held.len());
                held.extend_from_slice(&chunk[..n.min(room)]);
            }
            let _ = tx.send(());
        });
    }
    (buf, rx)
}

/// Run `git` with `args` in `cwd` under the fixed flags and a prompt-closing environment, handing
/// each stderr line to `on_line`. Only `cancel` stops it early.
pub(crate) fn git_stream<S: AsRef<OsStr>>(
    args: &[S],
    cwd: &Path,
    options: &GitOptions,
    cancel: &CancelFlag,
    on_line: &mut dyn FnMut(&str),
) -> Result<Streamed, StreamError> {
    git_stream_body(args, cwd, options, cancel, on_line, false)
}

/// Like `git_stream`, but also returns the first `STDOUT_CAP` bytes of stdout.
pub(crate) fn git_stream_capture<S: AsRef<OsStr>>(
    args: &[S],
    cwd: &Path,
    options: &GitOptions,
    cancel: &CancelFlag,
    on_line: &mut dyn FnMut(&str),
) -> Result<Streamed, StreamError> {
    git_stream_body(args, cwd, options, cancel, on_line, true)
}

fn git_stream_body<S: AsRef<OsStr>>(
    args: &[S],
    cwd: &Path,
    options: &GitOptions,
    cancel: &CancelFlag,
    on_line: &mut dyn FnMut(&str),
    capture: bool,
) -> Result<Streamed, StreamError> {
    let argv: Vec<OsString> = PREFIX
        .iter()
        .map(OsString::from)
        .chain(args.iter().map(|a| a.as_ref().to_owned()))
        .collect();
    let configured =
        env_value(options, "GIT_SSH_COMMAND").or_else(|| global_ssh_command(cwd, options));
    let ssh = ssh_command(configured, env_value(options, "GIT_SSH").is_some());
    let env = hardened_env(options, ssh);
    let program = OsStr::new("git");
    if capture {
        stream_program_capture(program, &argv, cwd, &env, cancel, on_line)
    } else {
        stream_program(program, &argv, cwd, &env, cancel, on_line)
    }
}

/// The last `options.env` entry for `key`, else the process environment.
fn env_value(options: &GitOptions, key: &str) -> Option<String> {
    let set = options.env.iter().rev().find(|(k, _)| k == OsStr::new(key));
    match set {
        Some((_, v)) => Some(v.to_string_lossy().into_owned()),
        None => std::env::var(key).ok(),
    }
}

/// The first line of the user's global `core.sshCommand`; any failure means unset. Only
/// `--global` is read, so a repository's own config can never choose the command.
fn global_ssh_command(cwd: &Path, options: &GitOptions) -> Option<String> {
    let out = run::git(
        &["config", "--global", "--get", "core.sshCommand"],
        cwd,
        options,
    )
    .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines().next().map(str::to_owned)
}

/// The `GIT_SSH_COMMAND` that stops ssh prompting: the configured command plus `BatchMode`, plain
/// `ssh` when none is set, or `None` to leave a custom `GIT_SSH` program alone.
fn ssh_command(configured: Option<String>, git_ssh_set: bool) -> Option<String> {
    match configured.filter(|c| !c.trim().is_empty()) {
        Some(c) => Some(format!("{c} -o BatchMode=yes")),
        None if git_ssh_set => None,
        None => Some("ssh -o BatchMode=yes".to_owned()),
    }
}

/// The environment changes for a streamed call, in application order. The caller's entries come
/// first so the scrub, the fixed values and the prompt closers always win.
fn hardened_env(options: &GitOptions, ssh: Option<String>) -> Vec<EnvChange> {
    let mut env: Vec<EnvChange> = options
        .env
        .iter()
        .map(|(k, v)| (k.clone(), Some(v.clone())))
        .collect();
    env.extend(SCRUBBED.iter().map(|k| (OsString::from(k), None)));
    env.extend(
        FIXED
            .iter()
            .map(|(k, v)| (OsString::from(k), Some(OsString::from(v)))),
    );
    env.push(("GIT_ASKPASS".into(), Some("".into())));
    env.push(("SSH_ASKPASS_REQUIRE".into(), Some("never".into())));
    if let Some(ssh) = ssh {
        env.push(("GIT_SSH_COMMAND".into(), Some(ssh.into())));
    }
    env
}

/// Read `pipe` on a thread and send each non-blank line, split on `\r` and `\n`. The thread is
/// never joined: a grandchild may hold the pipe open long after the child is gone.
fn read_lines(pipe: Option<impl Read + Send + 'static>) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    let Some(mut pipe) = pipe else { return rx };
    thread::spawn(move || {
        let mut pending: Vec<u8> = Vec::new();
        let mut buf = [0u8; 4096];
        let send = |bytes: &mut Vec<u8>| -> bool {
            let ok =
                bytes.is_empty() || tx.send(String::from_utf8_lossy(bytes).into_owned()).is_ok();
            bytes.clear();
            ok
        };
        loop {
            match pipe.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    for &b in &buf[..n] {
                        if b == b'\r' || b == b'\n' {
                            if !send(&mut pending) {
                                return;
                            }
                        } else {
                            pending.push(b);
                        }
                    }
                }
            }
        }
        send(&mut pending);
    });
    rx
}

/// Stop the child and everything in its process group (TERM, then KILL to whatever of the group
/// outlives one shared grace, so git and its hooks can clean up), then reap it so the caller
/// may delete files the child held. True when the child itself needed KILL.
#[cfg(unix)]
fn kill_tree(child: &mut Child) -> bool {
    run::stop_group(child)
}

/// Kill the child's process tree, then reap it. Falls back to killing just the child if the
/// helper cannot run. Always true: nothing gets a graceful stop here.
#[cfg(windows)]
fn kill_tree(child: &mut Child) -> bool {
    let killed = Command::new("taskkill")
        .args(["/T", "/F", "/PID", &child.id().to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !killed {
        // Best effort: the child may have exited on its own already.
        let _ = child.kill();
    }
    let _ = child.wait();
    true
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Instant;

    fn sh(
        script: &str,
        args: &[&str],
        env: &[EnvChange],
        cancel: &CancelFlag,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<Streamed, StreamError> {
        let mut argv: Vec<OsString> = vec!["-c".into(), script.into(), "sh".into()];
        argv.extend(args.iter().map(OsString::from));
        stream_program(
            OsStr::new("sh"),
            &argv,
            Path::new("/"),
            env,
            cancel,
            on_line,
        )
    }

    #[test]
    fn splits_stderr_on_carriage_return_and_newline_and_drops_blank_lines() {
        let mut lines = Vec::new();
        let out = sh(
            r"printf 'a\rb\n\nc' >&2",
            &[],
            &[],
            &CancelFlag::new(),
            &mut |l| lines.push(l.to_owned()),
        )
        .unwrap();
        assert!(out.success);
        assert_eq!(lines, ["a", "b", "c"]);
    }

    #[test]
    fn a_failing_child_reports_failure_and_its_stderr() {
        let out = sh(
            "echo first >&2; echo second >&2; exit 3",
            &[],
            &[],
            &CancelFlag::new(),
            &mut |_| {},
        )
        .unwrap();
        assert!(!out.success);
        assert_eq!(out.stderr, "first\nsecond");
    }

    #[test]
    fn only_the_last_fifty_lines_are_kept() {
        let script = "i=0; while [ $i -lt 80 ]; do echo line$i >&2; i=$((i+1)); done";
        let out = sh(script, &[], &[], &CancelFlag::new(), &mut |_| {}).unwrap();
        let kept: Vec<&str> = out.stderr.lines().collect();
        assert_eq!((kept.len(), kept[0], kept[49]), (50, "line30", "line79"));
    }

    #[test]
    fn env_changes_set_and_remove_variables() {
        let env: Vec<EnvChange> = vec![
            ("APINATA_X".into(), Some("1".into())),
            ("HOME".into(), None),
        ];
        let mut lines = Vec::new();
        sh(
            r#"echo "x=$APINATA_X home=${HOME-unset}" >&2"#,
            &[],
            &env,
            &CancelFlag::new(),
            &mut |l| lines.push(l.to_owned()),
        )
        .unwrap();
        assert_eq!(lines, ["x=1 home=unset"]);
    }

    #[test]
    fn a_missing_program_is_io_not_found() {
        let r = stream_program(
            OsStr::new("no-such-program-apinata"),
            &[],
            Path::new("/"),
            &[],
            &CancelFlag::new(),
            &mut |_| {},
        );
        assert!(matches!(r, Err(StreamError::Io(e)) if e.kind() == io::ErrorKind::NotFound));
    }

    #[test]
    fn a_flag_set_before_the_call_cancels_without_waiting() {
        let cancel = CancelFlag::new();
        cancel.cancel();
        let started = Instant::now();
        let r = sh("sleep 30", &[], &[], &cancel, &mut |_| {});
        assert!(matches!(r, Err(StreamError::Cancelled { .. })));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    fn running(pid: u32) -> bool {
        match fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => !stat.rsplit(") ").next().unwrap_or("").starts_with('Z'),
            Err(_) => false,
        }
    }

    #[test]
    fn cancel_lets_the_child_run_its_term_trap() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("marker");
        let cancel = CancelFlag::new();
        let r = sh(
            r#"trap 'echo term > "$1"; exit 1' TERM; sleep 60 & echo started >&2; wait"#,
            &[marker.to_str().unwrap()],
            &[],
            &cancel,
            &mut |l| {
                if l == "started" {
                    cancel.cancel();
                }
            },
        );
        assert!(matches!(r, Err(StreamError::Cancelled { .. })));
        assert_eq!(fs::read_to_string(&marker).ok().as_deref(), Some("term\n"));
    }

    /// Cancels `script` once it prints "started" and returns the outcome.
    fn cancelled_on_start(script: &str) -> Result<Streamed, StreamError> {
        let cancel = CancelFlag::new();
        sh(script, &[], &[], &cancel, &mut |l| {
            if l == "started" {
                cancel.cancel();
            }
        })
    }

    #[test]
    fn a_cancel_that_needed_sigkill_says_so() {
        let r = cancelled_on_start("trap '' TERM; echo started >&2; sleep 30");
        assert!(
            matches!(r, Err(StreamError::Cancelled { killed: true })),
            "{r:?}"
        );
    }

    #[test]
    fn a_cancel_that_term_ended_says_nothing_was_killed() {
        let r = cancelled_on_start("echo started >&2; sleep 30");
        assert!(
            matches!(r, Err(StreamError::Cancelled { killed: false })),
            "{r:?}"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cancel_kills_a_grandchild_that_ignores_term() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        let cancel = CancelFlag::new();
        // The grandchild writes its pid and reports "started" only once TERM is ignored; the
        // leader dies on TERM.
        let r = sh(
            r#"sh -c 'trap "" TERM; echo $$ > "$0"; echo started >&2; exec sleep 60' "$1" & wait"#,
            &[pidfile.to_str().unwrap()],
            &[],
            &cancel,
            &mut |l| {
                if l == "started" {
                    cancel.cancel();
                }
            },
        );
        assert!(matches!(r, Err(StreamError::Cancelled { .. })));
        let pid: u32 = fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let gone = (0..100).any(|_| {
            thread::sleep(Duration::from_millis(50));
            !running(pid)
        });
        assert!(
            gone,
            "grandchild {pid} ignored TERM and survived the cancel"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cancel_kills_the_child_and_its_grandchildren() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        let cancel = CancelFlag::new();
        let started = Instant::now();
        // The grandchild inherits stderr, so only a group kill lets the runner finish.
        let r = sh(
            r#"sleep 60 & echo $! > "$1"; echo started >&2; wait"#,
            &[pidfile.to_str().unwrap()],
            &[],
            &cancel,
            &mut |l| {
                if l == "started" {
                    cancel.cancel();
                }
            },
        );
        assert!(matches!(r, Err(StreamError::Cancelled { .. })));
        assert!(started.elapsed() < Duration::from_secs(10));
        let pid: u32 = fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let gone = (0..100).any(|_| {
            thread::sleep(Duration::from_millis(50));
            !running(pid)
        });
        assert!(gone, "grandchild {pid} survived the cancel");
    }

    use super::super::GitOptions;
    use std::collections::HashMap;

    fn apply(env: &[EnvChange]) -> HashMap<OsString, OsString> {
        let mut map = HashMap::new();
        for (k, v) in env {
            match v {
                Some(v) => map.insert(k.clone(), v.clone()),
                None => map.remove(k),
            };
        }
        map
    }

    fn get(map: &HashMap<OsString, OsString>, key: &str) -> Option<String> {
        map.get(OsStr::new(key))
            .map(|v| v.to_string_lossy().into_owned())
    }

    #[test]
    fn ssh_command_wraps_what_is_configured_and_defaults_to_batch_ssh() {
        let some = |s: &str| Some(s.to_owned());
        assert_eq!(
            ssh_command(some("ssh -i k"), false),
            some("ssh -i k -o BatchMode=yes")
        );
        assert_eq!(
            ssh_command(some("ssh -i k"), true),
            some("ssh -i k -o BatchMode=yes")
        );
        assert_eq!(ssh_command(None, false), some("ssh -o BatchMode=yes"));
        assert_eq!(ssh_command(some("  "), false), some("ssh -o BatchMode=yes"));
        assert_eq!(ssh_command(None, true), None);
    }

    #[test]
    fn the_caller_cannot_reopen_a_prompt_path() {
        let options = GitOptions::new()
            .env("GIT_ASKPASS", "/bin/evil")
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("GIT_TERMINAL_PROMPT", "1")
            .env("GIT_SSH_COMMAND", "ssh -i k")
            .env("GIT_DIR", "/nowhere");
        let env = apply(&hardened_env(
            &options,
            Some("ssh -i k -o BatchMode=yes".into()),
        ));
        assert_eq!(get(&env, "GIT_ASKPASS").as_deref(), Some(""));
        assert_eq!(get(&env, "SSH_ASKPASS_REQUIRE").as_deref(), Some("never"));
        assert_eq!(get(&env, "GIT_TERMINAL_PROMPT").as_deref(), Some("0"));
        assert_eq!(
            get(&env, "GIT_SSH_COMMAND").as_deref(),
            Some("ssh -i k -o BatchMode=yes")
        );
        assert_eq!(get(&env, "GIT_DIR"), None);
        let none = apply(&hardened_env(&GitOptions::new(), None));
        assert_eq!(get(&none, "GIT_SSH_COMMAND"), None);
    }

    #[test]
    fn env_value_prefers_the_last_options_entry() {
        let options = GitOptions::new()
            .env("APINATA_K", "a")
            .env("APINATA_K", "b");
        assert_eq!(env_value(&options, "APINATA_K").as_deref(), Some("b"));
        assert_eq!(env_value(&options, "APINATA_UNSET_KEY"), None);
    }

    fn sh_capture(script: &str, on_line: &mut dyn FnMut(&str)) -> Result<Streamed, StreamError> {
        let argv: Vec<OsString> = vec!["-c".into(), script.into()];
        stream_program_capture(
            OsStr::new("sh"),
            &argv,
            Path::new("/"),
            &[],
            &CancelFlag::new(),
            on_line,
        )
    }

    #[test]
    fn the_capturing_form_keeps_stdout_and_still_streams_stderr() {
        let mut lines = Vec::new();
        let out = sh_capture("printf 'one\\ntwo\\n'; echo err >&2", &mut |l| {
            lines.push(l.to_owned())
        })
        .unwrap();
        assert!(out.success);
        assert_eq!(out.stdout, "one\ntwo\n");
        assert_eq!(lines, ["err"]);
        assert_eq!(out.stderr, "err");
    }

    #[test]
    fn captured_stdout_is_capped_and_a_large_write_never_blocks() {
        let started = Instant::now();
        let out = sh_capture(
            "head -c 300000 /dev/zero | tr '\\0' x; echo done >&2",
            &mut |_| {},
        )
        .unwrap();
        assert!(out.success);
        assert_eq!(out.stdout.len(), STDOUT_CAP);
        assert!(out.stdout.bytes().all(|b| b == b'x'));
        assert_eq!(out.stderr, "done");
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn the_plain_form_still_captures_nothing() {
        let out = sh("echo out", &[], &[], &CancelFlag::new(), &mut |_| {}).unwrap();
        assert!(out.success);
        assert_eq!(out.stdout, "");
    }

    #[test]
    fn a_grandchild_holding_stdout_does_not_hold_the_capturing_call() {
        let started = Instant::now();
        let out = sh_capture("echo out; sleep 5 &", &mut |_| {}).unwrap();
        assert_eq!(out.stdout, "out\n");
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn the_capturing_form_honours_a_flag_set_before_the_call() {
        let cancel = CancelFlag::new();
        cancel.cancel();
        let argv: Vec<OsString> = vec!["-c".into(), "sleep 30".into()];
        let r = stream_program_capture(
            OsStr::new("sh"),
            &argv,
            Path::new("/"),
            &[],
            &cancel,
            &mut |_| {},
        );
        assert!(
            matches!(r, Err(StreamError::Cancelled { killed: false })),
            "{r:?}"
        );
    }

    #[test]
    fn git_stream_capture_returns_what_git_printed() {
        let out = git_stream_capture(
            &["--version"],
            Path::new("/"),
            &GitOptions::new(),
            &CancelFlag::new(),
            &mut |_| {},
        )
        .unwrap();
        assert!(out.success);
        assert!(out.stdout.starts_with("git version "), "{}", out.stdout);
    }
}
