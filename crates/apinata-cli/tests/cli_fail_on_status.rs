//! `run --fail-on-status`: a reply in range fails a request that has no assertions.

use crate::common;

use common::{ROOT, apinata, run_json, write_tree};
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn replying(status: u16) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(status))
        .mount(&s)
        .await;
    s
}

fn tree(url: &str) -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "probe.yml",
            &format!(
                "info:\n  name: probe\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n"
            ),
        ),
    ])
}

fn run(dir: &tempfile::TempDir, extra: &[&str]) -> std::process::Output {
    apinata()
        .arg("run")
        .arg(dir.path().join("probe.yml"))
        .args(extra)
        .arg("--no-keychain")
        .output()
        .expect("runs")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_200_exits_0_and_a_404_or_500_exits_1() {
    for (status, code) in [(200, 0), (404, 1), (500, 1)] {
        let s = replying(status).await;
        let dir = tree(&s.uri());
        let out = run(&dir, &["--fail-on-status"]);
        assert_eq!(out.status.code(), Some(code), "{status}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_the_flag_a_404_still_exits_0() {
    let s = replying(404).await;
    let dir = tree(&s.uri());
    assert_eq!(run(&dir, &[]).status.code(), Some(0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_value_picks_the_class() {
    let s = replying(500).await;
    let dir = tree(&s.uri());
    assert_eq!(run(&dir, &["--fail-on-status=4xx"]).status.code(), Some(0));
    assert_eq!(run(&dir, &["--fail-on-status=5xx"]).status.code(), Some(1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bad_value_is_a_usage_error() {
    let s = replying(200).await;
    let dir = tree(&s.uri());
    assert_eq!(run(&dir, &["--fail-on-status=404"]).status.code(), Some(3));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_text_summary_names_the_status() {
    let s = replying(404).await;
    let dir = tree(&s.uri());
    let out = run(&dir, &["--fail-on-status"]);
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        all.contains("status 404 is in the fail-on-status range (error)"),
        "{all}"
    );
    assert!(all.contains("1 failed"), "{all}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn json_and_junit_carry_the_status() {
    let s = replying(404).await;
    let dir = tree(&s.uri());
    let (code, report) = run_json(
        &dir.path().join("probe.yml"),
        &["--fail-on-status", "--no-keychain"],
    );
    assert_eq!(code, 1);
    assert_eq!(
        report["results"][0]["status_failure"], "status 404 is in the fail-on-status range (error)",
        "{report}"
    );
    let xml = dir.path().join("junit.xml");
    let out = run(
        &dir,
        &[
            "--reporter-junit",
            xml.to_str().expect("utf8"),
            "--fail-on-status",
        ],
    );
    assert_eq!(out.status.code(), Some(1));
    let text = std::fs::read_to_string(&xml).expect("junit");
    assert!(
        text.contains("status 404 is in the fail-on-status range"),
        "{text}"
    );
}

fn two_requests(url: &str) -> tempfile::TempDir {
    let req = |name: &str, seq: u32, path: &str| {
        format!(
            "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{url}/{path}\"\n"
        )
    };
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("one.yml", &req("one", 1, "a")),
        ("two.yml", &req("two", 2, "b")),
    ])
}

async fn served(s: &MockServer) -> usize {
    s.received_requests().await.expect("recorded").len()
}

fn run_dir(dir: &tempfile::TempDir, extra: &[&str]) -> std::process::Output {
    apinata()
        .arg("run")
        .arg(dir.path())
        .args(extra)
        .arg("--no-keychain")
        .output()
        .expect("runs")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_iteration_is_judged_and_the_run_exits_1() {
    let s = replying(404).await;
    let dir = two_requests(&s.uri());
    let out = run_dir(&dir, &["--iterations", "2", "--fail-on-status"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(served(&s).await, 4);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("4 failed"), "{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bail_stops_after_the_first_status_failure() {
    let s = replying(404).await;
    let dir = two_requests(&s.uri());
    let out = run_dir(&dir, &["--bail", "--fail-on-status"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(served(&s).await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bail_and_iterations_stop_the_whole_run() {
    let s = replying(500).await;
    let dir = two_requests(&s.uri());
    let out = run_dir(
        &dir,
        &["--iterations", "3", "--bail", "--fail-on-status=5xx"],
    );
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(served(&s).await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bare_flag_before_the_positional_takes_no_value() {
    let s = replying(404).await;
    let dir = tree(&s.uri());
    let out = apinata()
        .arg("run")
        .arg("--fail-on-status")
        .arg(dir.path().join("probe.yml"))
        .arg("--no-keychain")
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_junit_status_case_never_collides_with_an_assertion_case() {
    let s = replying(404).await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "probe.yml",
            &format!(
                "info:\n  name: probe\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{}\"\nruntime:\n  assertions:\n{}",
                s.uri(),
                common::check("res.status", "eq", "404")
            ),
        ),
    ]);
    let xml = dir.path().join("junit.xml");
    let out = run(
        &dir,
        &[
            "--reporter-junit",
            xml.to_str().expect("utf8"),
            "--fail-on-status",
        ],
    );
    assert_eq!(out.status.code(), Some(1));
    let text = std::fs::read_to_string(&xml).expect("junit");
    let names: Vec<&str> = text
        .split("<testcase name=\"")
        .skip(1)
        .filter_map(|c| c.split('"').next())
        .collect();
    assert!(names.contains(&"res.status eq 404"), "{names:?}");
    assert!(names.contains(&"[status]"), "{names:?}");
    assert!(!names.contains(&"res.status"), "{names:?}");
    let mut unique = names.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), names.len(), "{names:?}");
}
