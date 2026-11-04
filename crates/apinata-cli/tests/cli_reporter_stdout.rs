//! `apinata run --reporter-*` with `-` writes that report to stdout.

use crate::common;

use common::{ROOT, apinata, check, get, write_tree};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A one-request passing collection; `/ok` must be hit `hits` times.
async fn one_ok(hits: u64) -> (MockServer, tempfile::TempDir) {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{\"id\":7}"))
        .expect(hits)
        .mount(&s)
        .await;
    let url = format!("{}/ok", s.uri());
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "pass.yml",
            &get("Pass", 1, &url, &check("res.status", "eq", "200")),
        ),
    ]);
    (s, dir)
}

/// The binary run in a scratch cwd with a scratch runtime dir.
fn run_in(cwd: &std::path::Path, coll: &std::path::Path, extra: &[&str]) -> std::process::Output {
    apinata()
        .current_dir(cwd)
        .env("XDG_RUNTIME_DIR", cwd)
        .arg("run")
        .arg(coll)
        .arg("--no-keychain")
        .args(extra)
        .output()
        .unwrap()
}

fn assert_human_on_stderr(out: &std::process::Output) {
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Requests: 1 (1 passed"), "{stderr}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn junit_to_stdout() {
    let (_s, dir) = one_ok(1).await;
    let cwd = tempfile::TempDir::new().unwrap();
    let out = run_in(cwd.path(), dir.path(), &["--reporter-junit", "-"]);
    assert_eq!(out.status.code(), Some(0), "{:?}", out);
    let xml = String::from_utf8(out.stdout.clone()).unwrap();
    assert!(
        xml.starts_with("<?xml version=\"1.0\"?>\n<testsuites>"),
        "{xml}"
    );
    assert!(xml.trim_end().ends_with("</testsuites>"), "{xml}");
    assert_human_on_stderr(&out);
    assert!(!cwd.path().join("-").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn json_to_stdout() {
    let (_s, dir) = one_ok(1).await;
    let cwd = tempfile::TempDir::new().unwrap();
    let out = run_in(cwd.path(), dir.path(), &["--reporter-json", "-"]);
    assert_eq!(out.status.code(), Some(0));
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["summary"]["requests"], 1);
    assert_human_on_stderr(&out);
    assert!(!cwd.path().join("-").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn html_to_stdout() {
    let (_s, dir) = one_ok(1).await;
    let cwd = tempfile::TempDir::new().unwrap();
    let out = run_in(cwd.path(), dir.path(), &["--reporter-html", "-"]);
    assert_eq!(out.status.code(), Some(0));
    let page = String::from_utf8(out.stdout.clone()).unwrap();
    assert!(page.starts_with("<!doctype html>"), "{page}");
    assert_human_on_stderr(&out);
    assert!(!cwd.path().join("-").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_stdout_reporters_exit_3_before_sending() {
    let (s, dir) = one_ok(0).await;
    let cwd = tempfile::TempDir::new().unwrap();
    let out = run_in(
        cwd.path(),
        dir.path(),
        &["--reporter-json", "-", "--reporter-junit", "-"],
    );
    assert_eq!(out.status.code(), Some(3));
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("only one reporter can write to stdout"),
        "{stderr}"
    );
    s.verify().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stdout_reporter_conflicts_exit_3_before_sending() {
    for flag in [
        &["--format", "json"][..],
        &["--body"],
        &["--show-response"],
        &["--watch"],
    ] {
        let (s, dir) = one_ok(0).await;
        let cwd = tempfile::TempDir::new().unwrap();
        let mut extra = vec!["--reporter-junit", "-"];
        extra.extend_from_slice(flag);
        let out = run_in(cwd.path(), dir.path(), &extra);
        assert_eq!(out.status.code(), Some(3), "{flag:?}");
        assert!(out.stdout.is_empty(), "{flag:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("cannot be combined"), "{flag:?}: {stderr}");
        s.verify().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stdout_reporter_combines_with_a_file_reporter() {
    let (_s, dir) = one_ok(1).await;
    let cwd = tempfile::TempDir::new().unwrap();
    let file = cwd.path().join("r.json");
    let out = run_in(
        cwd.path(),
        dir.path(),
        &[
            "--reporter-junit",
            "-",
            "--reporter-json",
            file.to_str().unwrap(),
        ],
    );
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8(out.stdout).unwrap().starts_with("<?xml"));
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(report["summary"]["requests"], 1);
}

#[test]
fn a_refused_run_writes_no_reporter_output() {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    let cwd = tempfile::TempDir::new().unwrap();
    let out = run_in(cwd.path(), dir.path(), &["--reporter-json", "-"]);
    assert_eq!(out.status.code(), Some(3));
    assert!(out.stdout.is_empty(), "{:?}", out.stdout);
    assert!(!cwd.path().join("-").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_two_stdout_error_names_both_flags() {
    let (_s, dir) = one_ok(0).await;
    let cwd = tempfile::TempDir::new().unwrap();
    let out = run_in(
        cwd.path(),
        dir.path(),
        &["--reporter-json", "-", "--reporter-html", "-"],
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--reporter-json") && stderr.contains("--reporter-html"),
        "{stderr}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dot_slash_dash_path_is_still_a_file() {
    let (_s, dir) = one_ok(1).await;
    let cwd = tempfile::TempDir::new().unwrap();
    let out = run_in(cwd.path(), dir.path(), &["--reporter-junit", "./-"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty() || !String::from_utf8_lossy(&out.stdout).starts_with("<?xml"));
    let xml = std::fs::read_to_string(cwd.path().join("-")).unwrap();
    assert!(xml.starts_with("<?xml"), "{xml}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_closed_stdout_pipe_exits_0_quietly() {
    let (_s, dir) = one_ok(1).await;
    let cwd = tempfile::TempDir::new().unwrap();
    let mut child = common::apinata_std();
    child
        .current_dir(cwd.path())
        .env("XDG_RUNTIME_DIR", cwd.path())
        .arg("run")
        .arg(dir.path())
        .args(["--no-keychain", "--reporter-junit", "-"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = child.spawn().unwrap();
    // The reader goes away before the report is written.
    drop(child.stdout.take());
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("cannot write"), "{stderr}");
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_full_stdout_exits_3_with_the_output_error() {
    let (_s, dir) = one_ok(1).await;
    let cwd = tempfile::TempDir::new().unwrap();
    let full = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .unwrap();
    let out = common::apinata_std()
        .current_dir(cwd.path())
        .env("XDG_RUNTIME_DIR", cwd.path())
        .arg("run")
        .arg(dir.path())
        .args(["--no-keychain", "--reporter-junit", "-"])
        .stdout(full)
        .stderr(std::process::Stdio::piped())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error: cannot write output"), "{stderr}");
}
