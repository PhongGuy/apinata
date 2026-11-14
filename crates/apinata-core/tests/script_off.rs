//! With scripts off: the warning, skipped tests entries, flow order and summary counts.

use crate::script_support;

use apinata_core::{Outcome, RunOptions, Warning};
use script_support::*;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn server() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    s
}

fn tree(url: &str, flow: Option<&str>) -> tempfile::TempDir {
    let tests = |code: &str| script("tests", code);
    write_tree(&[
        (
            "opencollection.yml",
            &root(&tests("test('c', () => {})"), flow),
        ),
        (
            "f/folder.yml",
            &folder(&(tests("x") + &script("before-request", "y"))),
        ),
        (
            "f/r.yml",
            &scripted(
                "R",
                url,
                &(script("before-request", "throw new Error('never')")
                    + &tests("test('r', () => {})")
                    + &tests("  ")),
                "",
            ),
        ),
    ])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scripts_off_warn_and_skip_each_tests_segment() {
    let s = server().await;
    let dir = tree(&format!("{}/ok", s.uri()), None);
    let result = send(dir.path(), "f/r.yml", RunOptions::new()).await;
    assert_eq!(result.outcome, Outcome::Passed);
    assert!(result.warnings.contains(&Warning::ScriptsNotRun));
    assert!(result.console.is_empty());
    assert_eq!(
        tests(&result),
        [
            "tests this request's tests: skipped scripts are off",
            "tests folder f tests: skipped scripts are off",
            "tests collection tests: skipped scripts are off",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sequential_flow_orders_tests_segments_outer_first() {
    let s = server().await;
    let dir = tree(&format!("{}/ok", s.uri()), Some("sequential"));
    let result = send(dir.path(), "f/r.yml", RunOptions::new()).await;
    assert_eq!(
        tests(&result),
        [
            "tests collection tests: skipped scripts are off",
            "tests folder f tests: skipped scripts are off",
            "tests this request's tests: skipped scripts are off",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_flow_means_sandwich() {
    let s = server().await;
    let dir = tree(&format!("{}/ok", s.uri()), Some("sideways"));
    let result = send(dir.path(), "f/r.yml", RunOptions::new()).await;
    assert_eq!(
        tests(&result)[0],
        "tests this request's tests: skipped scripts are off"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_summary_counts_skipped_tests_and_run_matches_send_one() {
    let s = server().await;
    let dir = tree(&format!("{}/ok", s.uri()), None);
    let report = run_dir(dir.path(), RunOptions::new()).await;
    assert_eq!(report.summary.skipped_tests, 3);
    assert_eq!(
        (report.summary.passed_tests, report.summary.failed_tests),
        (0, 0)
    );
    assert_eq!(report.results[0].tests.len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scripts_off_leave_a_request_untested_but_passed() {
    let s = server().await;
    let dir = tree(&format!("{}/ok", s.uri()), None);
    let result = send(dir.path(), "f/r.yml", RunOptions::new()).await;
    assert_eq!(result.outcome, Outcome::Passed);
    assert!(result.is_untested());
}
