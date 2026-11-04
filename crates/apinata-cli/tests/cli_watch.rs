//! `apinata run --watch`: reruns on edits, one run per burst, no loops, clean exit on SIGINT.

use crate::common;

use common::{ROOT, apinata, check, get, write_tree};
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const WAIT: Duration = Duration::from_secs(20);

/// A spawned `apinata run --watch`, killed by its recorded PID when dropped.
struct Watch {
    child: Child,
    pid: u32,
    lines: Receiver<String>,
    seen: Vec<String>,
    _home: tempfile::TempDir,
}

fn spawn_reader(stream: impl std::io::Read + Send + 'static, tx: std::sync::mpsc::Sender<String>) {
    std::thread::spawn(move || {
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
}

/// The stderr line printed once a run's output and report files are complete.
const DONE: &str = "waiting for changes";

impl Watch {
    fn start(dir: &std::path::Path, extra: &[&str]) -> Watch {
        let home = tempfile::TempDir::new().expect("home");
        let mut cmd = crate::common::apinata_std();
        cmd.arg("run")
            .arg(dir)
            .arg("--watch")
            .args(extra)
            .env("HOME", home.path())
            .env("XDG_CONFIG_HOME", home.path().join("config"))
            .env("XDG_DATA_HOME", home.path().join("data"))
            .env_remove("HTTP_PROXY")
            .env_remove("http_proxy")
            .env_remove("ALL_PROXY")
            .env_remove("all_proxy")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().expect("spawn");
        let pid = child.id();
        let (tx, lines) = channel();
        // Both streams feed one channel; a run is over at its stderr "waiting" line.
        let out = child.stdout.take().expect("stdout");
        let err = child.stderr.take().expect("stderr");
        spawn_reader(out, tx.clone());
        spawn_reader(err, tx);
        Watch {
            child,
            pid,
            lines,
            seen: Vec::new(),
            _home: home,
        }
    }

    /// Wait until `n` lines containing `needle` have been seen in all; panics on timeout.
    fn wait_for(&mut self, needle: &str, n: usize) {
        let deadline = Instant::now() + WAIT;
        while self.seen.iter().filter(|l| l.contains(needle)).count() < n {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(line) => self.seen.push(line),
                Err(_) => panic!("no {n}th {needle:?} in {:#?}", self.seen),
            }
        }
    }

    /// Wait until `n` runs are over: stderr says so, and stdout, read on its own thread, has
    /// delivered each run's last report line too.
    fn wait_run(&mut self, n: usize) {
        self.wait_for(DONE, n);
        self.wait_for("Tests:", n);
    }

    /// Whether any new line arrives within `quiet`; lines are recorded.
    fn quiet_for(&mut self, quiet: Duration) -> bool {
        match self.lines.recv_timeout(quiet) {
            Ok(line) => {
                self.seen.push(line);
                false
            }
            Err(_) => true,
        }
    }

    /// Finished runs, counted by their `Tests:` line.
    fn runs(&self) -> usize {
        self.seen.iter().filter(|l| l.starts_with("Tests:")).count()
    }

    /// Send SIGINT and return the exit code.
    fn interrupt(&mut self) -> Option<i32> {
        let status = Command::new("kill")
            .args(["-INT", &self.pid.to_string()])
            .status()
            .expect("kill");
        assert!(status.success());
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(status) = self.child.try_wait().expect("wait") {
                return status.code();
            }
            assert!(Instant::now() < deadline, "did not exit");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        // Only the recorded child is killed, and only if it is still ours.
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = Command::new("kill")
                .args(["-KILL", &self.pid.to_string()])
                .status();
            let _ = self.child.wait();
        }
    }
}

async fn setup(expect: &str) -> (MockServer, tempfile::TempDir) {
    let server = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let url = format!("{}/ok", server.uri());
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &get("A", 1, &url, &check("res.status", "eq", expect)),
        ),
    ]);
    (server, dir)
}

fn rewrite(dir: &std::path::Path, server: &MockServer, expect: &str) {
    let url = format!("{}/ok", server.uri());
    std::fs::write(
        dir.join("a.yml"),
        get("A", 1, &url, &check("res.status", "eq", expect)),
    )
    .expect("write");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_edit_reruns_with_the_new_result_and_sigint_keeps_the_last_code() {
    let (server, dir) = setup("200").await;
    let mut w = Watch::start(dir.path(), &[]);
    w.wait_run(1);
    assert!(w.seen.iter().any(|l| l.contains("Requests: 1 (1 passed")));
    rewrite(dir.path(), &server, "201");
    w.wait_run(2);
    assert!(w.seen.iter().any(|l| l.contains("--- rerun at")));
    assert!(w.seen.iter().any(|l| l.contains("1 failed")));
    assert_eq!(w.interrupt(), Some(1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sigint_after_a_passing_run_exits_0() {
    let (_server, dir) = setup("200").await;
    let mut w = Watch::start(dir.path(), &[]);
    w.wait_run(1);
    assert_eq!(w.interrupt(), Some(0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_quick_edits_cause_one_rerun() {
    let (server, dir) = setup("200").await;
    let mut w = Watch::start(dir.path(), &[]);
    w.wait_run(1);
    rewrite(dir.path(), &server, "201");
    rewrite(dir.path(), &server, "202");
    w.wait_run(2);
    assert!(w.quiet_for(Duration::from_millis(2500)), "{:#?}", w.seen);
    assert_eq!(w.runs(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_report_written_inside_the_collection_does_not_loop() {
    let (server, dir) = setup("200").await;
    let report = dir.path().join("report.json");
    let mut w = Watch::start(
        dir.path(),
        &["--reporter-json", report.to_str().expect("utf-8")],
    );
    w.wait_run(1);
    assert!(w.quiet_for(Duration::from_millis(2500)), "{:#?}", w.seen);
    assert_eq!(w.runs(), 1);
    // A real edit still reruns.
    rewrite(dir.path(), &server, "201");
    w.wait_run(2);
}

#[test]
fn watch_with_json_format_is_refused() {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .args(["--watch", "--format", "json"])
        .timeout(Duration::from_secs(20))
        .assert()
        .code(3);
    let err = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(err.contains("--watch cannot be combined"), "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cookie_jar_inside_the_collection_does_not_loop() {
    let server = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200).insert_header("set-cookie", "sid=abc; Path=/"))
        .mount(&server)
        .await;
    let url = format!("{}/ok", server.uri());
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &get("A", 1, &url, &check("res.status", "eq", "200")),
        ),
    ]);
    let jar = dir.path().join("jar.json");
    let mut w = Watch::start(dir.path(), &["--cookie-jar", jar.to_str().expect("utf-8")]);
    w.wait_run(1);
    assert!(jar.is_file());
    assert!(w.quiet_for(Duration::from_millis(2500)), "{:#?}", w.seen);
    assert_eq!(w.runs(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_body_file_inside_the_collection_does_not_loop() {
    let (_server, dir) = setup("200").await;
    let out = dir.path().join("body.bin");
    let mut w = Watch::start(dir.path(), &["-o", out.to_str().expect("utf-8")]);
    w.wait_run(1);
    assert!(w.quiet_for(Duration::from_millis(2500)), "{:#?}", w.seen);
    assert_eq!(w.runs(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_report_in_a_subfolder_does_not_loop() {
    let (server, dir) = setup("200").await;
    let url = format!("{}/ok", server.uri());
    let sub = dir.path().join("sub");
    std::fs::create_dir(&sub).expect("mkdir");
    std::fs::write(
        sub.join("b.yml"),
        get("B", 2, &url, &check("res.status", "eq", "200")),
    )
    .expect("write");
    let report = sub.join("report.xml");
    let mut w = Watch::start(
        dir.path(),
        &["--reporter-junit", report.to_str().expect("utf-8")],
    );
    w.wait_run(1);
    assert!(report.is_file());
    assert!(w.quiet_for(Duration::from_millis(2500)), "{:#?}", w.seen);
    assert_eq!(w.runs(), 1);
}
