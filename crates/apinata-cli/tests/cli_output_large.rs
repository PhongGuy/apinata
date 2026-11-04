//! `run -o` writes the complete body through core's file sink, whatever the body cap.

use crate::common::body_server::{Reply, Tail, body_server, pattern};
use crate::common::{ROOT, apinata, get, write_tree};
use std::path::Path;
use std::process::Output;

const MIB: usize = 1024 * 1024;

/// Runs with scratch HOME and XDG dirs and no session bus, so nothing reaches the real keyring
/// or config.
fn run(dir: &Path, args: &[&str]) -> Output {
    let home = tempfile::tempdir().expect("home");
    apinata()
        .arg("run")
        .arg(dir)
        .args(args)
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("XDG_CACHE_HOME", home.path().join("cache"))
        .env_remove("XDG_RUNTIME_DIR")
        .output()
        .expect("runs")
}

/// A request error: exit 2, and the report on stdout names the failed read.
fn assert_request_error(out: &Output) {
    let text = String::from_utf8_lossy(&out.stdout);
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{text} / {err}");
    assert!(text.contains("error reading a body"), "{text}");
    assert!(text.contains("1 errored"), "{text}");
}

/// Names in `dir`, so a stray temp file shows up.
fn names(dir: &Path) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();
    found
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_body_past_the_cap_is_saved_whole_and_stdout_stays_small() {
    let body = pattern(30 * MIB);
    let s = body_server(vec![("/big", Reply::plain(body.clone()))]).await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &get("A", 1, &s.url("/big"), "")),
    ]);
    let out_dir = tempfile::tempdir().unwrap();
    let file = out_dir.path().join("out.bin");
    let out = run(dir.path(), &["--body", "-o", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    assert!(std::fs::read(&file).unwrap() == body, "file differs");
    assert!(
        (MIB..12 * MIB).contains(&out.stdout.len()),
        "stdout {} bytes",
        out.stdout.len()
    );
    assert_eq!(names(out_dir.path()), ["out.bin"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_output_directory_gets_numbered_complete_files() {
    let a = pattern(5 * MIB);
    let b = pattern(7 * MIB + 3);
    let s = body_server(vec![
        ("/a", Reply::plain(a.clone())),
        ("/b", Reply::plain(b.clone())),
    ])
    .await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &get("A", 1, &s.url("/a"), "")),
        ("b.yml", &get("B", 2, &s.url("/b"), "")),
    ]);
    let out_dir = tempfile::tempdir().unwrap();
    let arg = format!("{}/", out_dir.path().join("d").display());
    let out = run(dir.path(), &["--max-body-bytes", "1024", "-o", &arg]);
    assert_eq!(out.status.code(), Some(0));
    let target = out_dir.path().join("d");
    assert!(std::fs::read(target.join("001-a.body")).unwrap() == a);
    assert!(std::fs::read(target.join("002-b.body")).unwrap() == b);
    assert_eq!(names(&target), ["001-a.body", "002-b.body"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_transfer_exits_nonzero_and_leaves_no_file() {
    let s = body_server(vec![(
        "/cut",
        Reply::short(pattern(MIB), 4 * MIB, Tail::Close),
    )])
    .await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &get("A", 1, &s.url("/cut"), "")),
    ]);
    let out_dir = tempfile::tempdir().unwrap();
    let file = out_dir.path().join("out.bin");
    let out = run(dir.path(), &["-o", file.to_str().unwrap()]);
    assert_request_error(&out);
    assert!(
        names(out_dir.path()).is_empty(),
        "{:?}",
        names(out_dir.path())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_second_request_leaves_the_first_file_intact() {
    let ok = pattern(2 * MIB);
    let s = body_server(vec![
        ("/ok", Reply::plain(ok.clone())),
        ("/cut", Reply::short(pattern(MIB), 4 * MIB, Tail::Close)),
    ])
    .await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &get("A", 1, &s.url("/ok"), "")),
        ("b.yml", &get("B", 2, &s.url("/cut"), "")),
    ]);
    let out_dir = tempfile::tempdir().unwrap();
    let file = out_dir.path().join("out.bin");
    let out = run(dir.path(), &["-o", file.to_str().unwrap()]);
    assert_request_error(&out);
    assert!(std::fs::read(&file).unwrap() == ok, "first file damaged");
    assert_eq!(names(out_dir.path()), ["out.bin"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn directory_numbers_count_failed_requests_and_every_iteration() {
    let s = body_server(vec![
        ("/ok", Reply::plain(b"fine".to_vec())),
        ("/cut", Reply::short(pattern(1024), 4096, Tail::Close)),
    ])
    .await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &get("A", 1, &s.url("/ok"), "")),
        ("b.yml", &get("B", 2, &s.url("/cut"), "")),
        ("c.yml", &get("C", 3, &s.url("/ok"), "")),
    ]);
    let out_dir = tempfile::tempdir().unwrap();
    let arg = format!("{}/", out_dir.path().display());
    let out = run(dir.path(), &["--iterations", "2", "-o", &arg]);
    // The failing middle request errors in both iterations.
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        names(out_dir.path()),
        ["001-a.body", "003-c.body", "004-a.body", "006-c.body"]
    );
}
