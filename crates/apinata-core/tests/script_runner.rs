//! Runner control from scripts, and the test and assertion result getters.

use crate::script_support;

use apinata_core::{ExecutionMode, Outcome, RunReport, Warning};
use script_support::*;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn req(name: &str, seq: u32, url: &str, scripts: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{url}\"\nruntime:\n  scripts:\n{scripts}"
    )
}

fn names(report: &RunReport) -> Vec<String> {
    report
        .results
        .iter()
        .map(|r| format!("{}:{:?}", r.name, r.outcome))
        .collect()
}

async fn ok_server() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(path("/x"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    s
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_next_request_jumps_by_name_and_null_ends_the_run() {
    let s = ok_server().await;
    let u = format!("{}/x", s.uri());
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &req(
                "A",
                1,
                &u,
                &script("after-response", "bru.setNextRequest('C')"),
            ),
        ),
        ("b.yml", &req("B", 2, &u, "")),
        (
            "c.yml",
            &req(
                "C",
                3,
                &u,
                &script("tests", "bru.runner.setNextRequest(null)"),
            ),
        ),
        ("d.yml", &req("D", 4, &u, "")),
    ]);
    let report = run_dir(dir.path(), on()).await;
    assert_eq!(names(&report), ["A:Passed", "C:Passed", "D:Skipped"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_backward_jump_repeats_a_request() {
    let s = ok_server().await;
    let u = format!("{}/x", s.uri());
    let code = "const n = Number(bru.getVar('n') || 0) + 1; bru.setVar('n', n); if (n < 3) bru.setNextRequest('A');";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("A", 1, &u, &script("before-request", code))),
        ("b.yml", &req("B", 2, &u, "")),
    ]);
    let report = run_dir(dir.path(), on()).await;
    assert_eq!(
        names(&report),
        ["A:Passed", "A:Passed", "A:Passed", "B:Passed"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn skip_request_and_stop_execution() {
    let s = MockServer::start().await;
    Mock::given(path("/a"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&s)
        .await;
    Mock::given(path("/b"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&s)
        .await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &req(
                "A",
                1,
                &format!("{}/a", s.uri()),
                &script("before-request", "bru.runner.skipRequest()"),
            ),
        ),
        (
            "b.yml",
            &req(
                "B",
                2,
                &format!("{}/b", s.uri()),
                &script("after-response", "bru.runner.stopExecution()"),
            ),
        ),
        ("c.yml", &req("C", 3, &format!("{}/b", s.uri()), "")),
    ]);
    let report = run_dir(dir.path(), on()).await;
    assert_eq!(names(&report), ["A:Skipped", "B:Passed", "C:Skipped"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_next_request_ends_the_run_with_a_warning() {
    let s = ok_server().await;
    let u = format!("{}/x", s.uri());
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &req("A", 1, &u, &script("tests", "bru.setNextRequest('Nope')")),
        ),
        ("b.yml", &req("B", 2, &u, "")),
    ]);
    let report = run_dir(dir.path(), on()).await;
    assert_eq!(names(&report), ["A:Passed", "B:Skipped"]);
    assert!(
        report.results[0]
            .warnings
            .contains(&Warning::NextRequestNotFound)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runner_control_is_ignored_in_a_standalone_send() {
    let s = MockServer::start().await;
    Mock::given(path("/x"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&s)
        .await;
    let code = "bru.runner.skipRequest(); bru.runner.stopExecution(); bru.setNextRequest('Z');";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &req(
                "R",
                1,
                &format!("{}/x", s.uri()),
                &script("before-request", code),
            ),
        ),
    ]);
    let result = send(
        dir.path(),
        "r.yml",
        on().execution_mode(ExecutionMode::Standalone),
    )
    .await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    assert!(result.warnings.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn result_getters_report_tests_and_assertions() {
    let s = ok_server().await;
    let code = r#"test("a", () => {});
test("b", () => { throw new Error("no"); });
const t = await bru.getTestResults();
const a = await bru.getAssertionResults();
console.log(JSON.stringify(t.summary), t.results.map((r) => r.status + ":" + r.description).join(","), t.results[1].error);
console.log(JSON.stringify(a.summary), a.results.map((r) => [r.status, r.lhsExpr, r.operator, r.rhsOperand].join(" ")).join(","));"#;
    let extra = "  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n    - expression: res.status\n      operator: eq\n      value: \"500\"\n";
    let text = format!(
        "{}{extra}",
        req("R", 1, &format!("{}/x", s.uri()), &script("tests", code))
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(
        console(&result),
        [
            r#"log:{"total":2,"passed":1,"failed":1,"skipped":0} pass:a,fail:b no"#,
            r#"log:{"total":2,"passed":1,"failed":1,"skipped":0} pass res.status eq 200,fail res.status eq 500"#,
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_skipped_request_fetches_no_token() {
    let s = MockServer::start().await;
    let token = r#"{"access_token": "tok", "token_type": "Bearer", "expires_in": 3600}"#;
    Mock::given(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(token, "application/json"))
        .expect(0)
        .mount(&s)
        .await;
    Mock::given(path("/x"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&s)
        .await;
    let auth = format!(
        "  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{}/token\"\n    credentials:\n      clientId: app\n    scope: read\n    tokenConfig:\n      id: main\n      placement:\n        header: Bearer\n",
        s.uri()
    );
    let text = format!(
        "info:\n  name: A\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{}/x\"\n{auth}runtime:\n  scripts:\n{}",
        s.uri(),
        script("before-request", "bru.runner.skipRequest()")
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &text)]);
    let report = run_dir(dir.path(), on()).await;
    assert_eq!(names(&report), ["A:Skipped"]);
    assert!(report.results[0].error.is_none());
    assert!(s.received_requests().await.expect("recording").is_empty());
}
