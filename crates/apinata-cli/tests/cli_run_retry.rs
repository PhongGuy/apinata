//! `run --retries`, `--retry-delay` and `--retry-unsafe`.

use crate::common;

use common::{ROOT, apinata, run_json, write_tree};
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn flaky(status: u16, fail: u64) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(status))
        .up_to_n_times(fail)
        .mount(&s)
        .await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    s
}

fn request(method: &str, url: &str) -> String {
    format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: {method}\n  url: \"{url}\"\nruntime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n"
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retries_turn_two_503s_into_a_pass_and_the_json_reports_three_attempts() {
    let s = flaky(503, 2).await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request("GET", &s.uri())),
    ]);
    let (code, report) = run_json(dir.path(), &["--retries", "2", "--no-keychain"]);
    assert_eq!(code, 0, "{report}");
    assert_eq!(report["results"][0]["attempts"], 3, "{report}");
    assert_eq!(s.received_requests().await.expect("rec").len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_single_attempt_leaves_the_json_unchanged() {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request("GET", &s.uri())),
    ]);
    let (code, report) = run_json(dir.path(), &["--retries", "2", "--no-keychain"]);
    assert_eq!(code, 0, "{report}");
    assert!(report["results"][0].get("attempts").is_none(), "{report}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_console_names_the_attempts() {
    let s = flaky(503, 2).await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request("GET", &s.uri())),
    ]);
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .args(["--retries", "2", "--no-keychain"])
        .output()
        .expect("runs");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("(3 attempts)"), "{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn junit_notes_the_attempts() {
    let s = flaky(503, 2).await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request("GET", &s.uri())),
    ]);
    let file = dir.path().join("junit.xml");
    apinata()
        .arg("run")
        .arg(dir.path())
        .args(["--retries", "2", "--no-keychain", "--reporter-junit"])
        .arg(&file)
        .output()
        .expect("runs");
    let text = std::fs::read_to_string(&file).expect("report");
    assert!(
        text.contains("<system-out>attempts: 3</system-out>"),
        "{text}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_needs_retry_unsafe() {
    let s = flaky(503, 2).await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request("POST", &s.uri())),
    ]);
    let (code, _) = run_json(dir.path(), &["--retries", "2", "--no-keychain"]);
    assert_eq!(code, 1);
    assert_eq!(s.received_requests().await.expect("rec").len(), 1);

    let s = flaky(503, 2).await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request("POST", &s.uri())),
    ]);
    let (code, report) = run_json(
        dir.path(),
        &["--retries", "2", "--retry-unsafe", "--no-keychain"],
    );
    assert_eq!(code, 0, "{report}");
    assert_eq!(report["results"][0]["attempts"], 3, "{report}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_default_sends_once() {
    let s = flaky(503, 2).await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request("GET", &s.uri())),
    ]);
    let (code, _) = run_json(dir.path(), &["--no-keychain"]);
    assert_eq!(code, 1);
    assert_eq!(s.received_requests().await.expect("rec").len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_help_records_the_rules() {
    let out = apinata().args(["run", "--help"]).output().expect("runs");
    let text = String::from_utf8_lossy(&out.stdout);
    for flag in ["--retries", "--retry-delay", "--retry-unsafe"] {
        assert!(text.contains(flag), "{text}");
    }
    assert!(text.contains("502"), "{text}");
    assert!(text.contains("Retry-After"), "{text}");
}

async fn slow(settings: &str) -> (MockServer, tempfile::TempDir) {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(3)))
        .mount(&s)
        .await;
    let text = format!("{}{settings}", request("GET", &s.uri()));
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    (s, dir)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_request_timeout_applies_to_each_attempt() {
    let (_s, dir) = slow("settings:\n  timeout: 300\n").await;
    let started = std::time::Instant::now();
    let (code, report) = run_json(dir.path(), &["--retries", "1", "--no-keychain"]);
    assert_eq!(code, 2, "{report}");
    assert_eq!(report["results"][0]["attempts"], 2, "{report}");
    assert!(started.elapsed() < std::time::Duration::from_millis(2500));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_run_wide_timeout_is_not_retried() {
    let (_s, dir) = slow("").await;
    let (code, report) = run_json(
        dir.path(),
        &["--retries", "2", "--timeout", "1", "--no-keychain"],
    );
    assert_eq!(code, 2, "{report}");
    assert!(report["results"][0].get("attempts").is_none(), "{report}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_console_announces_each_retry_without_the_url() {
    let s = flaky(503, 2).await;
    let url = format!("{}/?token=hunter2", s.uri());
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request("GET", &url)),
    ]);
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .args(["--retries", "2", "--retry-delay", "5", "--no-keychain"])
        .output()
        .expect("runs");
    let err = String::from_utf8_lossy(&out.stderr);
    let lines: Vec<&str> = err.lines().filter(|l| l.starts_with("retrying ")).collect();
    assert_eq!(
        lines,
        [
            "retrying r.yml (attempt 2 of 3) in 5 ms",
            "retrying r.yml (attempt 3 of 3) in 5 ms"
        ],
        "{err}"
    );
    assert!(!err.contains("hunter2"), "{err}");
}
