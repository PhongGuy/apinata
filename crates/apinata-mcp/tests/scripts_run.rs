//! run_folder runs only approved scripts, files one approval for the run, and reports each
//! request's tests and console.

use crate::common;

use apinata_core::script_hash;
use common::{ROOT, TestServer};
use serde_json::json;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

const A: &str = "test('a', () => expect(res.status).to.equal(200))";
const B: &str = "console.log('b ran')\ntest('b', () => {})";

fn req(name: &str, seq: u32, url: &str, kind: &str, code: &str) -> String {
    let body: String = code.lines().map(|l| format!("        {l}\n")).collect();
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{url}\"\nruntime:\n  scripts:\n    - type: {kind}\n      code: |-\n{body}"
    )
}

async fn mock(expected: u64) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .expect(expected)
        .mount(&s)
        .await;
    s
}

async fn start(s: &MockServer) -> TestServer {
    let a = req("a", 1, &format!("{}/a", s.uri()), "tests", A);
    let b = req("b", 2, &format!("{}/b", s.uri()), "tests", B);
    TestServer::start(&[("opencollection.yml", ROOT), ("a.yml", &a), ("b.yml", &b)]).await
}

#[tokio::test]
async fn an_unapproved_run_sends_nothing_and_files_one_approval_for_the_run_path() {
    let s = mock(0).await;
    let server = start(&s).await;
    let message = server.err("run_folder", json!({"path": ""})).await;
    assert!(message.contains("scripts need approval"), "{message}");
    let pending = server.inbox().pending_scripts().expect("inbox");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].request, "");
    let mut got = pending[0].hashes.clone();
    got.sort();
    let mut want = vec![script_hash(A), script_hash(B)];
    want.sort();
    assert_eq!(got, want);
}

#[tokio::test]
async fn an_approved_run_reports_tests_and_console_per_request() {
    let s = mock(2).await;
    let server = start(&s).await;
    server.approve_scripts(&[A, B]);
    let result = server.ok("run_folder", json!({"path": ""})).await;
    assert_eq!(result["results"][0]["tests"][0]["name"], "a");
    assert_eq!(result["results"][0]["tests"][0]["status"], "pass");
    assert_eq!(result["results"][1]["tests"][0]["status"], "pass");
    assert_eq!(result["results"][1]["console"][0]["text"], "b ran");
    assert_eq!(result["results"][1]["console"][0]["phase"], "tests");
}

// An error in a tests script names the tests, as the app and the command line do.
#[tokio::test]
async fn a_tests_script_error_names_the_tests() {
    const BAD: &str = "test('ok', () => {})\nthrow new Error('boom')";
    let s = mock(1).await;
    let a = req("a", 1, &format!("{}/a", s.uri()), "tests", BAD);
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("a.yml", &a)]).await;
    server.approve_scripts(&[BAD]);
    let result = server.ok("run_folder", json!({"path": ""})).await;
    assert_eq!(
        result["results"][0]["tests"][1]["message"],
        "request tests, line 2: Error: boom"
    );
}

#[tokio::test]
async fn skip_scripts_runs_without_them() {
    let s = mock(2).await;
    let server = start(&s).await;
    let result = server
        .ok("run_folder", json!({"path": "", "skip_scripts": true}))
        .await;
    assert_eq!(result["results"][0]["tests"][0]["status"], "skipped");
    assert_eq!(result["results"][1]["tests"][0]["status"], "skipped");
    assert!(server.inbox().pending_scripts().expect("inbox").is_empty());
}

#[tokio::test]
async fn a_script_set_host_header_is_refused_at_send_time() {
    let s = mock(0).await;
    let code = "req.setHeader('Host', 'evil.example')";
    let r = req("r", 1, &format!("{}/r", s.uri()), "before-request", code);
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", &r)]).await;
    server.approve_scripts(&[code]);
    let result = server.ok("run_folder", json!({"path": ""})).await;
    assert_eq!(result["results"][0]["outcome"], "errored");
    let error = result["results"][0]["error"].as_str().unwrap_or_default();
    assert!(error.contains("differs from the URL's host"), "{error}");
}

#[tokio::test]
async fn a_run_files_a_host_approval_for_a_script_send_to_an_untrusted_host() {
    let s = mock(1).await;
    let pre = "try { await bru.sendRequest({url: 'http://api.example.com/x'}) } catch (e) {}";
    let a = req("a", 1, &format!("{}/a", s.uri()), "before-request", pre);
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("a.yml", &a)]).await;
    server.approve_scripts(&[pre]);
    let result = server.ok("run_folder", json!({"path": ""})).await;
    assert_eq!(result["results"][0]["status"], 200, "{result}");
    let pending = server.inbox().pending().expect("inbox");
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0].host, "api.example.com");
    assert_eq!(pending[0].request, "a.yml");
}
