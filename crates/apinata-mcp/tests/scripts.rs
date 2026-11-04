//! send_request runs only scripts the user approved by exact text, and reports their tests
//! and console redacted.

use crate::common;

use apinata_core::script_hash;
use common::{ROOT, TestServer};
use serde_json::json;
use wiremock::matchers::{header, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const DEV: &str = "name: dev\nvariables:\n  - name: token\n    secret: true\n";
const SECRET: &str = "Tk9-secret-value";
const PRE: &str = "req.setHeader('X-Sig', 'sig-' + bru.getEnvVar('token'))\nconsole.log('token', bru.getEnvVar('token'))\nbru.setVar('copy', 'x' + bru.getEnvVar('token'))";
const TESTS: &str = "test('status', () => expect(res.status).to.equal(200))\ntest('leak ' + bru.getEnvVar('token'), () => { throw new Error('bad ' + bru.getEnvVar('token')) })";

fn indent(code: &str) -> String {
    code.lines().map(|l| format!("        {l}\n")).collect()
}

fn scripted(url: &str, pre: &str) -> String {
    format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}/ok\"\nruntime:\n  scripts:\n    - type: before-request\n      code: |-\n{}    - type: tests\n      code: |-\n{}",
        indent(pre),
        indent(TESTS)
    )
}

async fn start(s: &MockServer) -> TestServer {
    let files = [
        ("opencollection.yml", ROOT.to_owned()),
        ("environments/dev.yml", DEV.to_owned()),
        ("r.yml", scripted(&s.uri(), PRE)),
    ];
    let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
    let server = TestServer::start_with(&files, |o| o.env("dev")).await;
    server.set_secret("dev", "token", SECRET);
    server
}

async fn mock(expected: u64) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .expect(expected)
        .mount(&s)
        .await;
    s
}

#[tokio::test]
async fn an_unapproved_script_refuses_the_send_and_files_one_approval() {
    let s = mock(0).await;
    let server = start(&s).await;
    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(
        message.contains("scripts need approval in the Apinata app; retry after the user approves"),
        "{message}"
    );
    let pending = server.inbox().pending_scripts().expect("inbox");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].request, "r.yml");
    let mut want = vec![script_hash(PRE), script_hash(TESTS)];
    want.sort();
    let mut got = pending[0].hashes.clone();
    got.sort();
    assert_eq!(got, want);
    server.err("send_request", json!({"path": "r.yml"})).await;
    assert_eq!(server.inbox().pending_scripts().expect("inbox").len(), 1);
    assert!(server.inbox().pending().expect("inbox").is_empty());
}

#[tokio::test]
async fn approved_scripts_run_and_report_tests_and_console_redacted() {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .and(header("x-sig", format!("sig-{SECRET}").as_str()))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&s)
        .await;
    let server = start(&s).await;
    server.approve_scripts(&[PRE, TESTS]);
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    assert_eq!(result["status"], 200);
    let tests = result["tests"].as_array().expect("tests");
    assert_eq!(tests.len(), 2, "{result}");
    assert_eq!(tests[0]["phase"], "tests");
    assert_eq!(tests[0]["name"], "status");
    assert_eq!(tests[0]["status"], "pass");
    assert_eq!(tests[1]["status"], "fail");
    assert_eq!(tests[1]["name"], "leak <secret>");
    assert_eq!(result["console"][0]["phase"], "pre-request");
    assert_eq!(result["console"][0]["level"], "log");
    assert_eq!(result["console"][0]["text"], "token [redacted]");
    let text = result.to_string();
    assert!(!text.contains(SECRET), "{text}");
    let vars = server
        .ok("list_runtime_variables", json!({}))
        .await
        .to_string();
    assert!(!vars.contains(SECRET), "{vars}");
    assert!(vars.contains("x<secret>"), "{vars}");
}

#[tokio::test]
async fn skip_scripts_sends_without_running_them() {
    let s = mock(1).await;
    let server = start(&s).await;
    let result = server
        .ok(
            "send_request",
            json!({"path": "r.yml", "skip_scripts": true}),
        )
        .await;
    assert_eq!(result["status"], 200);
    assert_eq!(result["tests"][0]["status"], "skipped");
    assert_eq!(result["tests"][0]["message"], "scripts are off");
    assert!(server.inbox().pending_scripts().expect("inbox").is_empty());
}

#[tokio::test]
async fn a_blocked_collection_refuses_without_filing() {
    let s = mock(0).await;
    let server = start(&s).await;
    server.block_scripts();
    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(
        message.contains("scripts are turned off for this collection"),
        "{message}"
    );
    assert!(server.inbox().pending_scripts().expect("inbox").is_empty());
}

#[tokio::test]
async fn a_changed_script_needs_approval_again() {
    let s = mock(0).await;
    let server = start(&s).await;
    server.approve_scripts(&[PRE, TESTS]);
    let changed = "bru.setVar('other', 1)";
    server.write("r.yml", &scripted(&s.uri(), changed));
    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(message.contains("scripts need approval"), "{message}");
    let pending = server.inbox().pending_scripts().expect("inbox");
    assert_eq!(pending[0].hashes, [script_hash(changed)]);
}

#[tokio::test]
async fn an_unreadable_trust_file_refuses_without_filing() {
    let s = mock(0).await;
    let server = start(&s).await;
    std::fs::write(server.script_trust_file(), "{ broken").expect("write");
    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(message.contains("nothing was sent"), "{message}");
    assert!(server.inbox().pending_scripts().expect("inbox").is_empty());
    assert_eq!(
        std::fs::read_to_string(server.script_trust_file()).expect("read"),
        "{ broken"
    );
}

#[tokio::test]
async fn a_send_cancelled_mid_script_keeps_a_copied_secret_out_of_runtime_variables() {
    let s = mock(0).await;
    let server = start(&s).await;
    let pre = "bru.setVar('copy', 'x' + bru.getEnvVar('token'))\nawait new Promise((done) => setTimeout(done, 3000))";
    server.write("r.yml", &scripted(&s.uri(), pre));
    server.approve_scripts(&[pre, TESTS]);
    let handle = server
        .start_call("send_request", json!({"path": "r.yml"}))
        .await;
    // Cancel while the pre-request script waits, before the request is resolved or guarded.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    handle.cancel(None).await.expect("cancel sent");
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let vars = server
        .ok("list_runtime_variables", json!({}))
        .await
        .to_string();
    assert!(vars.contains("copy"), "{vars}");
    assert!(!vars.contains(SECRET), "{vars}");
}

// Starts a send whose approved pre-request script copies the secret into `copy` and then waits,
// and returns once the copy is visible to other tool calls.
async fn copy_then_wait(
    server: &TestServer,
    url: &str,
) -> rmcp::service::RequestHandle<rmcp::RoleClient> {
    let pre = "bru.setVar('copy', 'x' + bru.getEnvVar('token'))\nawait new Promise((done) => setTimeout(done, 5000))";
    server.write("r.yml", &scripted(url, pre));
    server.approve_scripts(&[pre, TESTS]);
    let handle = server
        .start_call("send_request", json!({"path": "r.yml"}))
        .await;
    for _ in 0..200 {
        let vars = server.ok("list_runtime_variables", json!({})).await;
        if vars.to_string().contains("copy") {
            return handle;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("the script never set `copy`");
}

#[tokio::test]
async fn runtime_variables_listed_during_a_send_hide_a_copied_secret() {
    let s = mock(0).await;
    let server = start(&s).await;
    let handle = copy_then_wait(&server, &s.uri()).await;
    let vars = server
        .ok("list_runtime_variables", json!({}))
        .await
        .to_string();
    assert!(!vars.contains(SECRET), "{vars}");
    assert!(vars.contains("x<secret>"), "{vars}");
    handle.cancel(None).await.expect("cancel sent");
}

#[tokio::test]
async fn a_concurrent_send_using_a_copied_secret_is_redacted() {
    let s = mock(0).await;
    Mock::given(path("/echo"))
        .respond_with(ResponseTemplate::new(200).set_body_string(format!("got x{SECRET}")))
        .mount(&s)
        .await;
    let server = start(&s).await;
    let echo = format!(
        "info:\n  name: e\n  type: http\nhttp:\n  method: GET\n  url: \"{}/echo\"\n  headers:\n    - name: X-C\n      value: \"{{{{copy}}}}\"\n",
        s.uri()
    );
    server.write("e.yml", &echo);
    // An environment without the secret, so only the copy's taint can mark it sensitive.
    server.write("environments/prod.yml", "name: prod\nvariables: []\n");
    let handle = copy_then_wait(&server, &s.uri()).await;
    let result = server
        .ok(
            "send_request",
            json!({"path": "e.yml", "environment": "prod"}),
        )
        .await
        .to_string();
    assert!(!result.contains(SECRET), "{result}");
    assert!(result.contains("<secret>"), "{result}");
    handle.cancel(None).await.expect("cancel sent");
}

#[tokio::test]
async fn a_script_send_to_an_untrusted_host_files_a_host_approval() {
    let s = mock(2).await;
    let pre = "try { await bru.sendRequest({url: 'http://api.example.com/x'}) } catch (e) {}";
    let files = [
        ("opencollection.yml", ROOT.to_owned()),
        ("environments/dev.yml", DEV.to_owned()),
        ("r.yml", scripted(&s.uri(), pre)),
    ];
    let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
    let server = TestServer::start_with(&files, |o| o.env("dev")).await;
    server.set_secret("dev", "token", SECRET);
    server.approve_scripts(&[pre, TESTS]);
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    assert_eq!(result["status"], 200);
    let warnings = result["warnings"].to_string();
    assert!(
        warnings.contains("a script's own request was not sent: host `api.example.com`")
            && warnings.contains("Approval requested in the Apinata app"),
        "{warnings}"
    );
    let pending = server.inbox().pending().expect("inbox");
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0].host, "api.example.com");
    assert_eq!(pending[0].request, "r.yml");
    server.ok("send_request", json!({"path": "r.yml"})).await;
    assert_eq!(server.inbox().pending().expect("inbox").len(), 1);
}
