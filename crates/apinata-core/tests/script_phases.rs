//! Phase order, flow, the outcome table and limits with scripts on.

use crate::script_support;

use apinata_core::{Outcome, RunError, ScriptOptions, ScriptPhase, Warning};
use script_support::*;
use std::time::{Duration, Instant};
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

fn logs(tag: &str) -> String {
    script("before-request", &format!("console.log('pre {tag}')"))
        + &script("after-response", &format!("console.log('post {tag}')"))
        + &script("tests", &format!("console.log('tests {tag}')"))
}

async fn layered(flow: Option<&str>) -> Vec<String> {
    let s = server().await;
    let url = format!("{}/ok", s.uri());
    let dir = write_tree(&[
        ("opencollection.yml", &root(&logs("c"), flow)),
        ("f/folder.yml", &folder(&logs("f"))),
        ("f/r.yml", &scripted("R", &url, &logs("r"), "")),
    ]);
    let result = send(dir.path(), "f/r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    assert!(!result.warnings.contains(&Warning::ScriptsNotRun));
    assert_eq!(result.console[0].phase, ScriptPhase::PreRequest);
    assert_eq!(result.console[8].phase, ScriptPhase::Tests);
    console(&result)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn phases_run_in_sandwich_order() {
    let expected = [
        "pre c", "pre f", "pre r", "post r", "post f", "post c", "tests r", "tests f", "tests c",
    ];
    assert_eq!(layered(None).await, expected.map(|t| format!("log:{t}")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sequential_flow_runs_later_phases_outer_first() {
    let expected = [
        "pre c", "pre f", "pre r", "post c", "post f", "post r", "tests c", "tests f", "tests r",
    ];
    assert_eq!(
        layered(Some("sequential")).await,
        expected.map(|t| format!("log:{t}"))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_throwing_pre_request_script_sends_nothing() {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&s)
        .await;
    let pre = script(
        "before-request",
        "test('early', () => {});\nthrow new Error('stop')",
    );
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted("R", &format!("{}/ok", s.uri()), &pre, ""),
        ),
    ]);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Errored);
    assert!(result.response.is_none());
    match &result.error {
        Some(RunError::Script {
            phase: ScriptPhase::PreRequest,
            message,
            ..
        }) => {
            assert_eq!(message, "request script, line 2: Error: stop")
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(tests(&result), ["pre-request early: pass"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_throwing_post_response_script_fails_and_tests_still_run() {
    let s = server().await;
    let scripts = script("after-response", "throw new Error('bad')")
        + &script("tests", "test('t', () => {})");
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted("R", &format!("{}/ok", s.uri()), &scripts, ""),
        ),
    ]);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Failed);
    assert_eq!(
        tests(&result),
        [
            "post-response Post-response script error: fail request script, line 1: Error: bad",
            "tests t: pass"
        ]
    );
    assert!(result.tests[0].script_error);
    let report = run_dir(dir.path(), on()).await;
    assert_eq!(
        (report.summary.passed_tests, report.summary.failed_tests),
        (1, 0)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_throwing_tests_script_records_a_test_script_error() {
    let s = server().await;
    let scripts = script("tests", "null.x");
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted("R", &format!("{}/ok", s.uri()), &scripts, ""),
        ),
    ]);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Failed);
    assert_eq!(result.tests.len(), 1);
    assert_eq!(
        (result.tests[0].name.as_str(), result.tests[0].script_error),
        ("Test script error", true)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unsupported_member_in_a_later_phase_is_skipped_not_failed() {
    let s = server().await;
    let scripts = script("tests", "test('a', () => {});\nreq.onFail()");
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted("R", &format!("{}/ok", s.uri()), &scripts, ""),
        ),
    ]);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Passed);
    assert_eq!(
        tests(&result),
        [
            "tests a: pass",
            "tests Test script: skipped req.onFail is not supported by Apinata"
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unsupported_member_in_the_pre_request_script_errors_naming_it() {
    let s = server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/ok", s.uri()),
                &script("before-request", "req.onFail()"),
                "",
            ),
        ),
    ]);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Errored);
    let text = result
        .error
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default();
    assert!(
        text.contains("req.onFail is not supported by Apinata"),
        "{text}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failing_test_fails_the_request() {
    let s = server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/ok", s.uri()),
                &script("tests", "test('x', () => { throw new Error('no') })"),
                "",
            ),
        ),
    ]);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Failed);
    assert_eq!(tests(&result), ["tests x: fail no"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_runaway_pre_request_stops_at_the_limit_and_the_run_continues() {
    let s = server().await;
    let url = format!("{}/ok", s.uri());
    let mut second = scripted("S", &url, &script("tests", "test('s', () => {})"), "");
    second = second.replace("seq: 1", "seq: 2");
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &scripted("A", &url, &script("before-request", "while (true) {}"), ""),
        ),
        ("b.yml", &second),
    ]);
    let opts = apinata_core::RunOptions::new().scripts(
        ScriptOptions::new()
            .enabled(true)
            .timeout(Duration::from_millis(200)),
    );
    let start = Instant::now();
    let report = run_dir(dir.path(), opts).await;
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(report.results[0].outcome, Outcome::Errored);
    let text = report.results[0]
        .error
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default();
    assert!(text.contains("script exceeded the 0.2 s limit"), "{text}");
    assert_eq!(report.results[1].outcome, Outcome::Passed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_during_the_pre_request_phase_sends_nothing() {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&s)
        .await;
    let pre = script("before-request", "await new Promise(() => {})");
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted("R", &format!("{}/ok", s.uri()), &pre, ""),
        ),
    ]);
    let c = apinata_core::Collection::open(dir.path()).expect("opens");
    let ctx =
        apinata_core::VarContext::new(&c, apinata_core::ContextOptions::new()).expect("context");
    let exec = std::sync::Arc::new(
        apinata_core::Executor::new(apinata_core::ExecOptions::new()).expect("client"),
    );
    let request = apinata_core::RequestFile::load(dir.path().join("r.yml")).expect("loads");
    let cancel = apinata_core::CancelHandle::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        trigger.cancel();
    });
    let start = Instant::now();
    let item = apinata_core::ItemPath::new("r.yml").expect("item path");
    let result = apinata_core::send_one(item, &request, &ctx, &exec, &on(), &cancel).await;
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(result.outcome, Outcome::Cancelled);
    assert!(result.response.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_send_failure_skips_post_response_and_tests() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted("R", "http://127.0.0.1:1/", &logs("r"), ""),
        ),
    ]);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Errored);
    assert_eq!(console(&result), ["log:pre r"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_console_cap_spans_the_request() {
    let s = server().await;
    let scripts = script(
        "before-request",
        "for (let i = 0; i < 600; i++) console.log(i)",
    ) + &script("tests", "for (let i = 0; i < 600; i++) console.log(i)");
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted("R", &format!("{}/ok", s.uri()), &scripts, ""),
        ),
    ]);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.console.len(), 1_001);
    assert_eq!(
        console(&result)[1_000],
        "warn:200 more console entries were dropped"
    );
}

/// Sends a request whose before-request script is `code` and returns the error text.
async fn pre_request_error(code: &str) -> String {
    let s = server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/ok", s.uri()),
                &script("before-request", code),
                "",
            ),
        ),
    ]);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Errored);
    result
        .error
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_untranslated_pm_call_errors_naming_the_call() {
    let text = pre_request_error("var a = 1;\npm.visualizer.set(\"x\");").await;
    assert!(
        text.contains("pm.visualizer.set is not available; see the import report"),
        "{text}"
    );
    assert!(!text.contains("not defined"), "{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_error_names_the_untranslated_call_that_ran() {
    let code = "function later() {\n  pm.visualizer.set('x');\n}\nif (false) { pm.iterationData.get('a'); }\nvar n = pm.info.iteration;";
    let text = pre_request_error(code).await;
    assert!(
        text.contains("pm.info.iteration is not available; see the import report"),
        "{text}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn of_two_untranslated_calls_on_a_line_the_error_names_the_one_that_ran() {
    let text = pre_request_error("var a = 1;\nfalse && pm.a.b(); pm.c.d();").await;
    assert!(
        text.contains("pm.c.d is not available; see the import report"),
        "{text}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_untranslated_call_in_a_called_function_is_named() {
    let code = "if (false) { pm.x.y(); }\nfunction f() {\n  return pm.iterationData.get('a');\n}\nvar early = typeof pm;\nf();";
    let text = pre_request_error(code).await;
    assert!(
        text.contains("pm.iterationData.get is not available; see the import report"),
        "{text}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_untranslated_postman_global_errors_naming_the_call() {
    let text = pre_request_error("postman.setNextRequest('x');").await;
    assert!(
        text.contains("postman.setNextRequest is not available; see the import report"),
        "{text}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bare_pm_reference_without_a_chain_still_says_pm() {
    let text = pre_request_error("var p = pm;").await;
    assert!(
        text.contains("pm is not available; see the import report"),
        "{text}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_ordinary_reference_error_keeps_its_message() {
    let text = pre_request_error("nope.x").await;
    assert!(
        text.contains("ReferenceError: nope is not defined"),
        "{text}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_script_with_its_own_pm_is_unaffected() {
    let text = pre_request_error(
        "var pm = { visualizer: { set() {} } };\npm.visualizer.set('x');\nnope.x",
    )
    .await;
    assert!(
        text.contains("ReferenceError: nope is not defined"),
        "{text}"
    );
    assert!(!text.contains("import report"), "{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_undefined_cryptojs_reports_the_require_hint() {
    let text = pre_request_error("var h = CryptoJS.MD5('x');").await;
    assert!(
        text.contains("CryptoJS is not defined; use require('crypto-js')"),
        "{text}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_undefined_cheerio_or_xml2json_reports_a_hint() {
    for name in ["cheerio", "xml2Json"] {
        let text = pre_request_error(&format!("var d = {name}('x');")).await;
        assert!(
            text.contains(&format!("{name} is not available here")),
            "{text}"
        );
    }
}
