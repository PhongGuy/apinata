//! A failed write to stdout exits 3 with one `error:` line; a closed pipe exits 0 quietly.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::common::{self, ROOT, get, write_tree};
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The binary command with scratch HOME and XDG dirs and no session bus, so nothing reaches the
/// real keyring or config.
fn cli(dir: &Path, home: &Path, args: &[&str]) -> Command {
    let mut cmd = common::apinata_std();
    cmd.current_dir(dir)
        .args(args)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env_remove("XDG_RUNTIME_DIR")
        .env_remove("HTTP_PROXY")
        .env_remove("http_proxy")
        .stdin(Stdio::null())
        .stderr(Stdio::piped());
    cmd
}

fn finish(mut cmd: Command) -> std::process::Output {
    let child = cmd.spawn().expect("spawns");
    let started = std::time::Instant::now();
    let out = child.wait_with_output().expect("exits");
    assert!(started.elapsed() < Duration::from_secs(60));
    out
}

/// Stdout is /dev/full: every write fails with ENOSPC.
#[cfg(unix)]
fn assert_full_exits_3(dir: &Path, args: &[&str]) {
    let Ok(full) = std::fs::OpenOptions::new().write(true).open("/dev/full") else {
        return;
    };
    let home = tempfile::TempDir::new().expect("home");
    let mut cmd = cli(dir, home.path(), args);
    cmd.stdout(full);
    let out = finish(cmd);
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(3), "{args:?}: {err}");
    let lines: Vec<&str> = err.lines().collect();
    assert_eq!(lines.len(), 1, "{args:?}: {err}");
    assert!(lines[0].starts_with("error:"), "{args:?}: {err}");
}

/// The read end of stdout is closed before the child starts, so its first write is EPIPE.
#[cfg(unix)]
fn assert_closed_pipe_exits_0(dir: &Path, args: &[&str]) {
    let home = tempfile::TempDir::new().expect("home");
    let (reader, writer) = std::io::pipe().expect("pipe");
    drop(reader);
    let mut cmd = cli(dir, home.path(), args);
    cmd.stdout(writer);
    let out = finish(cmd);
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{args:?}: {err}");
    assert!(err.is_empty(), "{args:?}: {err}");
}

fn tree(url: &str) -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("needle.yml", &get("needle", 1, url, "")),
    ])
}

macro_rules! offline_case {
    ($full:ident, $pipe:ident, $args:expr) => {
        #[cfg(unix)]
        #[test]
        fn $full() {
            let dir = tree("http://x/");
            assert_full_exits_3(dir.path(), $args);
        }
        #[cfg(unix)]
        #[test]
        fn $pipe() {
            let dir = tree("http://x/");
            assert_closed_pipe_exits_0(dir.path(), $args);
        }
    };
}

offline_case!(ls_full_exits_3, ls_closed_pipe_exits_0, &["ls"]);
offline_case!(
    search_full_exits_3,
    search_closed_pipe_exits_0,
    &["search", "needle"]
);
offline_case!(check_full_exits_3, check_closed_pipe_exits_0, &["check"]);
offline_case!(
    check_json_full_exits_3,
    check_json_closed_pipe_exits_0,
    &["check", "--json"]
);
offline_case!(
    completions_full_exits_3,
    completions_closed_pipe_exits_0,
    &["completions", "bash"]
);

/// A collection whose one request hits a local server, and the server to keep alive.
#[cfg(unix)]
async fn served() -> (MockServer, tempfile::TempDir) {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let dir = tree(&format!("{}/x", s.uri()));
    (s, dir)
}

#[cfg(unix)]
async fn blocking(f: impl FnOnce() + Send + 'static) {
    tokio::task::spawn_blocking(f).await.expect("joins");
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_text_full_exits_3() {
    let (_s, dir) = served().await;
    blocking(move || assert_full_exits_3(dir.path(), &["run", "."])).await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_text_closed_pipe_exits_0() {
    let (_s, dir) = served().await;
    blocking(move || assert_closed_pipe_exits_0(dir.path(), &["run", "."])).await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_json_full_exits_3() {
    let (_s, dir) = served().await;
    blocking(move || assert_full_exits_3(dir.path(), &["run", ".", "--format", "json"])).await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_json_closed_pipe_exits_0() {
    let (_s, dir) = served().await;
    blocking(move || assert_closed_pipe_exits_0(dir.path(), &["run", ".", "--format", "json"]))
        .await;
}
