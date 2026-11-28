//! `run_folder` with `parallel`: the range and delay refusals, a refused plan sending and
//! filing nothing, overlapping sends reported in plan order, the host check on every send, and
//! the cookie warning.

use crate::common;

use std::time::Duration;

use common::{ROOT, TestServer};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const OK: &str = "runtime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n";

fn get(name: &str, seq: u32, url: &str, extra: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{url}\"\n{extra}"
    )
}

async fn echo_ok() -> MockServer {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;
    mock
}

#[tokio::test]
async fn a_degree_outside_1_to_64_is_a_tool_error() {
    let mock = echo_ok().await;
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &get("a", 1, &mock.uri(), "")),
    ])
    .await;
    for n in [0, 65] {
        let message = server
            .err("run_folder", json!({"path": "", "parallel": n}))
            .await;
        assert!(message.contains("1 to 64"), "{message}");
    }
    assert!(mock.received_requests().await.expect("recorded").is_empty());
}

#[tokio::test]
async fn parallel_with_a_delay_is_a_tool_error() {
    let mock = echo_ok().await;
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &get("a", 1, &mock.uri(), "")),
    ])
    .await;
    let message = server
        .err(
            "run_folder",
            json!({"path": "", "parallel": 2, "delay_ms": 10}),
        )
        .await;
    assert!(
        message.contains("cannot be combined with `delay_ms`"),
        "{message}"
    );
    assert!(mock.received_requests().await.expect("recorded").is_empty());
}

#[tokio::test]
async fn a_request_with_post_response_variables_refuses_the_run_before_anything_is_sent() {
    let mock = echo_ok().await;
    let capture = "runtime:\n  actions:\n    - type: set-variable\n      phase: after-response\n      selector:\n        expression: res.status\n        method: jsonq\n      variable:\n        name: code\n        scope: runtime\n";
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &get("a", 1, &mock.uri(), "")),
        ("b.yml", &get("b", 2, &mock.uri(), capture)),
    ])
    .await;
    let message = server
        .err("run_folder", json!({"path": "", "parallel": 2}))
        .await;
    assert!(message.contains("b.yml"), "{message}");
    assert!(message.contains("cannot run in parallel"), "{message}");
    assert!(mock.received_requests().await.expect("recorded").is_empty());
    assert!(server.inbox().pending().expect("inbox").is_empty());
}

// The server holds the first connection until the second arrives, so both pass only when the
// two sends overlap; a lone connection gets a 500 after a fallback wait instead of hanging.
#[tokio::test]
async fn two_sends_overlap_and_report_in_plan_order() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let url = format!("http://{}", listener.local_addr().expect("addr"));
    tokio::spawn(async move {
        let (first, _) = listener.accept().await.expect("accept");
        let second = tokio::time::timeout(Duration::from_secs(5), listener.accept()).await;
        let status = if second.is_ok() { "200 OK" } else { "500 Oops" };
        let reply = |mut socket: TcpStream| async move {
            let mut buf = [0u8; 4096];
            let _ = socket.read(&mut buf).await;
            let text =
                format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            let _ = socket.write_all(text.as_bytes()).await;
        };
        if let Ok(Ok((second, _))) = second {
            tokio::join!(reply(first), reply(second));
        } else {
            reply(first).await;
        }
    });
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &get("a", 1, &format!("{url}/a"), OK)),
        ("b.yml", &get("b", 2, &format!("{url}/b"), OK)),
    ])
    .await;
    let result = server
        .ok("run_folder", json!({"path": "", "parallel": 2}))
        .await;
    let paths: Vec<&str> = (result["results"].as_array().expect("results").iter())
        .map(|r| r["path"].as_str().expect("path"))
        .collect();
    assert_eq!(paths, ["a.yml", "b.yml"]);
    assert_eq!(result["summary"]["passed"], 2, "{result}");
    let progress = server.progress();
    assert_eq!(progress[0].message.as_deref(), Some("a.yml: passed"));
}

#[tokio::test]
async fn every_parallel_send_to_an_untrusted_host_is_refused_and_one_approval_is_filed() {
    let mock = echo_ok().await;
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &get("a", 1, "https://api.example.com/a", "")),
        ("b.yml", &get("b", 2, &mock.uri(), "")),
        ("c.yml", &get("c", 3, "https://api.example.com/c", "")),
    ])
    .await;
    let result = server
        .ok("run_folder", json!({"path": "", "parallel": 3}))
        .await;
    let outcomes: Vec<&str> = (result["results"].as_array().expect("results").iter())
        .map(|r| r["outcome"].as_str().expect("outcome"))
        .collect();
    assert_eq!(outcomes, ["errored", "passed", "errored"]);
    for ix in [0, 2] {
        let error = result["results"][ix]["error"].as_str().expect("error");
        assert!(error.contains("not trusted"), "{error}");
    }
    assert_eq!(server.inbox().pending().expect("inbox").len(), 1);
}

#[tokio::test]
async fn a_cookie_the_later_requests_could_not_see_is_warned_about() {
    let mock = MockServer::start().await;
    Mock::given(path("/login"))
        .respond_with(ResponseTemplate::new(200).append_header("set-cookie", "sid=one; Path=/"))
        .mount(&mock)
        .await;
    Mock::given(path("/me"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;
    let uri = mock.uri();
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &get("a", 1, &format!("{uri}/login"), "")),
        ("b.yml", &get("b", 2, &format!("{uri}/me"), "")),
    ])
    .await;
    let result = server
        .ok("run_folder", json!({"path": "", "parallel": 2}))
        .await;
    let warnings = result["results"][0]["warnings"].to_string();
    assert!(warnings.contains("running alongside"), "{result}");
    assert!(
        result["results"][1].get("warnings").is_none(),
        "the last request warns of nothing: {result}"
    );
}

#[tokio::test]
async fn a_script_that_sets_a_variable_refuses_the_run_and_files_no_script_approval() {
    let mock = echo_ok().await;
    let script = "runtime:\n  scripts:\n    - type: after-response\n      code: |-\n        bru.setVar('x', 1)\n";
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &get("a", 1, &mock.uri(), script)),
        ("b.yml", &get("b", 2, &mock.uri(), "")),
    ])
    .await;
    let message = server
        .err("run_folder", json!({"path": "", "parallel": 2}))
        .await;
    assert!(message.contains("a.yml"), "{message}");
    assert!(message.contains("setVar"), "{message}");
    assert!(message.contains("skip_scripts"), "{message}");
    assert!(server.inbox().pending_scripts().expect("inbox").is_empty());
    assert!(mock.received_requests().await.expect("recorded").is_empty());
    // Without scripts nothing is shared, so the same plan runs.
    let result = server
        .ok(
            "run_folder",
            json!({"path": "", "parallel": 2, "skip_scripts": true}),
        )
        .await;
    assert_eq!(result["summary"]["requests"], 2, "{result}");
}
