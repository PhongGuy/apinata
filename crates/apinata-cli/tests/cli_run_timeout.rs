//! `run --timeout`: a run-wide cap on every request's timeout.

use crate::common;

use common::{ROOT, apinata, run_json, write_tree};
use std::time::{Duration, Instant};
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn server(delay: Duration) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_delay(delay))
        .mount(&s)
        .await;
    s
}

fn request(url: &str, settings: &str) -> String {
    format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n{settings}"
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_cap_times_out_a_slow_request_and_names_itself() {
    let s = server(Duration::from_secs(3)).await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request(&s.uri(), "")),
    ]);
    let started = Instant::now();
    let (code, report) = run_json(dir.path(), &["--timeout", "1"]);
    assert_eq!(code, 2, "{report}");
    assert!(started.elapsed() < Duration::from_millis(2800));
    let text = report.to_string();
    assert!(
        text.contains("timed out (run-wide timeout of 1 s)"),
        "{text}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_shorter_own_timeout_keeps_its_plain_message() {
    let s = server(Duration::from_secs(3)).await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request(&s.uri(), "settings:\n  timeout: 300\n")),
    ]);
    let (code, report) = run_json(dir.path(), &["--timeout", "2"]);
    assert_eq!(code, 2, "{report}");
    let text = report.to_string();
    assert!(text.contains("timed out"), "{text}");
    assert!(!text.contains("run-wide"), "{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_fast_request_passes_under_the_cap() {
    let s = server(Duration::ZERO).await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request(&s.uri(), "")),
    ]);
    let (code, _) = run_json(dir.path(), &["--timeout", "5"]);
    assert_eq!(code, 0);
}

#[test]
fn a_zero_cap_is_a_usage_error() {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    apinata()
        .arg("run")
        .arg(dir.path())
        .args(["--timeout", "0"])
        .assert()
        .code(3)
        .stderr(predicates::str::contains("86400"));
}
