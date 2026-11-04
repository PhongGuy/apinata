//! Scripts in `apinata run`: report rows, --no-scripts, --script-timeout and exit codes.

use crate::common;

use common::{ROOT, apinata, run_json, write_tree};
use serde_json::json;
use std::time::{Duration, Instant};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn script(kind: &str, code: &str) -> String {
    let body: String = code.lines().map(|l| format!("        {l}\n")).collect();
    format!("    - type: {kind}\n      code: |-\n{body}")
}

fn scripted(url: &str, scripts: &str) -> String {
    format!(
        "info:\n  name: R\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\nruntime:\n  scripts:\n{scripts}"
    )
}

async fn server() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    s
}

fn tests_and_log() -> String {
    script(
        "before-request",
        "console.log('hi', req.getExecutionMode())",
    ) + &script(
        "tests",
        "test('ok', () => expect(res.status).to.equal(200));\ntest('bad', () => expect(res.status).to.equal(500));",
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tests_and_console_appear_in_the_json_report() {
    let s = server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted(&format!("{}/ok", s.uri()), &tests_and_log()),
        ),
    ]);
    let (code, report) = run_json(dir.path(), &[]);
    assert_eq!(code, 1);
    assert_eq!(
        report["results"][0]["tests"],
        json!([
            {"phase": "tests", "name": "ok", "status": "pass"},
            {"phase": "tests", "name": "bad", "status": "fail", "message": "expected 200 to equal 500"}
        ])
    );
    assert_eq!(
        report["results"][0]["console"],
        json!([{"phase": "pre-request", "level": "log", "text": "hi cli"}])
    );
    let summary = &report["summary"];
    assert_eq!(
        (
            summary["passed_tests"].as_u64(),
            summary["failed_tests"].as_u64(),
            summary["skipped_tests"].as_u64()
        ),
        (Some(1), Some(1), Some(0))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_scripts_skips_tests_and_exits_0() {
    let s = server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted(&format!("{}/ok", s.uri()), &tests_and_log()),
        ),
    ]);
    let (code, report) = run_json(dir.path(), &["--no-scripts"]);
    assert_eq!(code, 0);
    assert_eq!(
        report["results"][0]["tests"],
        json!([{"phase": "tests", "name": "request tests", "status": "skipped", "message": "scripts are off"}])
    );
    assert_eq!(report["results"][0]["console"], json!([]));
    assert_eq!(report["summary"]["skipped_tests"], 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_script_error_entry_is_marked_and_exits_1() {
    let s = server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted(
                &format!("{}/ok", s.uri()),
                &script("after-response", "throw new Error('x')"),
            ),
        ),
    ]);
    let (code, report) = run_json(dir.path(), &[]);
    assert_eq!(code, 1);
    assert_eq!(
        report["results"][0]["tests"],
        json!([{"phase": "post-response", "name": "Post-response script error", "status": "fail", "message": "request script, line 1: Error: x", "scriptError": true}])
    );
    assert_eq!(report["summary"]["failed_tests"], 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pre_request_error_exits_2_and_the_timeout_flag_sets_the_limit() {
    let s = server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted(
                &format!("{}/ok", s.uri()),
                &script("before-request", "while (true) {}"),
            ),
        ),
    ]);
    let start = Instant::now();
    let (code, report) = run_json(dir.path(), &["--script-timeout", "1"]);
    assert!(
        start.elapsed() < Duration::from_secs(10),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(code, 2);
    assert_eq!(report["results"][0]["outcome"], "errored");
    let error = report["results"][0]["error"].as_str().unwrap_or_default();
    assert!(error.contains("script exceeded the 1 s limit"), "{error}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_text_report_lists_tests_console_and_totals() {
    let s = server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted(&format!("{}/ok", s.uri()), &tests_and_log()),
        ),
    ]);
    let out = apinata().arg("run").arg(dir.path()).output().expect("runs");
    let text = String::from_utf8_lossy(&out.stdout);
    for line in [
        "   ✓ ok",
        "   ✕ bad (expected 200 to equal 500)",
        "   [log] hi cli",
        "Tests: 1 passed, 1 failed, 0 skipped",
    ] {
        assert!(text.contains(line), "missing {line:?} in\n{text}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_expression_reads_as_failed_with_its_error() {
    let s = server().await;
    let text = format!(
        "info:\n  name: R\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{}/ok\"\nruntime:\n  actions:\n    - type: set-variable\n      phase: after-response\n      selector:\n        expression: \"nope.x\"\n        method: jsonq\n      variable:\n        name: bad\n        scope: runtime\n",
        s.uri()
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    let (_, report) = run_json(dir.path(), &[]);
    let row = &report["results"][0]["post_response_vars"][0];
    assert_eq!(row["result"], "failed");
    assert!(
        row["reason"]
            .as_str()
            .is_some_and(|r| r.contains("ReferenceError")),
        "{row}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_expression_is_listed_in_the_text_report() {
    let s = server().await;
    let text = format!(
        "info:\n  name: R\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{}/ok\"\nruntime:\n  actions:\n    - type: set-variable\n      phase: after-response\n      selector:\n        expression: \"nope.x\"\n        method: jsonq\n      variable:\n        name: bad\n        scope: runtime\n",
        s.uri()
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    let out = apinata().arg("run").arg(dir.path()).output().expect("runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("! post-response variable `bad` not set: ReferenceError"),
        "{stdout}"
    );
}
