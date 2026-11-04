//! `RunOptions::fail_on_status`: a reply in the chosen range fails a request with no assertions.

use crate::script_support::{ROOT, run_dir, write_tree};

use apinata_core::{Outcome, RequestResult, ResultsReport, RunOptions, StatusRule};
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn request(url: &str, assertions: &str) -> String {
    let runtime = if assertions.is_empty() {
        String::new()
    } else {
        format!("runtime:\n  assertions:\n{assertions}")
    };
    format!(
        "info:\n  name: probe\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n{runtime}"
    )
}

async fn replying(status: u16) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(status))
        .mount(&s)
        .await;
    s
}

async fn one(status: u16, opts: RunOptions) -> RequestResult {
    let s = replying(status).await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request(&s.uri(), "")),
    ]);
    let report = run_dir(dir.path(), opts).await;
    (*report.results[0]).clone()
}

fn rule(r: StatusRule) -> RunOptions {
    RunOptions::new().fail_on_status(Some(r))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_200_passes_under_error() {
    let r = one(200, rule(StatusRule::Error)).await;
    assert_eq!(r.outcome, Outcome::Passed, "{r:?}");
    assert_eq!(r.status_failure, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_404_and_a_500_fail_under_error() {
    for code in [404, 500] {
        let r = one(code, rule(StatusRule::Error)).await;
        assert_eq!(r.outcome, Outcome::Failed, "{code}: {r:?}");
        assert_eq!(r.status_failure, Some(code));
        assert!(r.is_failure());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_rule_is_off_by_default() {
    let r = one(404, RunOptions::new()).await;
    assert_eq!(r.outcome, Outcome::Passed, "{r:?}");
    assert_eq!(r.status_failure, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_ignores_a_500_and_server_ignores_a_404() {
    assert_eq!(
        one(500, rule(StatusRule::Client)).await.outcome,
        Outcome::Passed
    );
    assert_eq!(
        one(404, rule(StatusRule::Client)).await.outcome,
        Outcome::Failed
    );
    assert_eq!(
        one(404, rule(StatusRule::Server)).await.outcome,
        Outcome::Passed
    );
    assert_eq!(
        one(500, rule(StatusRule::Server)).await.outcome,
        Outcome::Failed
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_503_then_200_with_retries_is_not_a_failure() {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&s)
        .await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request(&s.uri(), "")),
    ]);
    let opts = rule(StatusRule::Error).retries(2);
    let r = (*run_dir(dir.path(), opts).await.results[0]).clone();
    assert_eq!(r.outcome, Outcome::Passed, "{r:?}");
    assert_eq!(r.status_failure, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_503_forever_with_retries_fails_on_the_last_reply() {
    let r = one(503, rule(StatusRule::Error).retries(1)).await;
    assert_eq!(r.outcome, Outcome::Failed, "{r:?}");
    assert_eq!(r.status_failure, Some(503));
    assert_eq!(r.attempts(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_assertion_stays_failed_and_records_the_status() {
    let s = replying(404).await;
    let assertion = "    - expression: res.status\n      operator: eq\n      value: \"200\"\n";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request(&s.uri(), assertion)),
    ]);
    let r = (*run_dir(dir.path(), rule(StatusRule::Error)).await.results[0]).clone();
    assert_eq!(r.outcome, Outcome::Failed);
    assert_eq!(r.status_failure, Some(404));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_request_with_no_reply_is_still_errored() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request("http://127.0.0.1:1", "")),
    ]);
    let r = (*run_dir(dir.path(), rule(StatusRule::Error)).await.results[0]).clone();
    assert_eq!(r.outcome, Outcome::Errored, "{r:?}");
    assert_eq!(r.status_failure, None);
}

#[test]
fn rules_parse_from_their_names() {
    assert_eq!("error".parse::<StatusRule>(), Ok(StatusRule::Error));
    assert_eq!("4xx".parse::<StatusRule>(), Ok(StatusRule::Client));
    assert_eq!("5xx".parse::<StatusRule>(), Ok(StatusRule::Server));
    assert!("404".parse::<StatusRule>().is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_json_and_junit_reports_carry_the_reason() {
    let r = one(404, rule(StatusRule::Error)).await;
    let report = ResultsReport::new([], [&r]);
    let json: serde_json::Value = serde_json::from_str(&report.to_json()).expect("json");
    let reason = "status 404 is in the fail-on-status range (error)";
    assert_eq!(json["results"][0]["status_failure"], reason, "{json}");
    assert_eq!(json["results"][0]["outcome"], "failed");
    let xml = report.to_junit("2026-10-26T10:00:00.000", "host", false);
    assert!(
        xml.contains(&format!(r#"<failure type="failure" message="{reason}"/>"#)),
        "{xml}"
    );
    assert!(xml.contains(r#"failures="1""#), "{xml}");
    assert!(report.to_text().contains(reason), "{}", report.to_text());
    assert!(report.to_html("now").contains(reason));
    assert_eq!(report.exit_code(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_passing_row_has_no_status_failure_key() {
    let r = one(200, rule(StatusRule::Error)).await;
    let json: serde_json::Value =
        serde_json::from_str(&ResultsReport::new([], [&r]).to_json()).expect("json");
    assert!(json["results"][0].get("status_failure").is_none(), "{json}");
}
