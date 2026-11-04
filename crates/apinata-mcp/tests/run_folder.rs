//! `run_folder` over the wire: outcomes without bodies, progress per finished request,
//! cancellation from the client, and the run-level refusals.

use crate::common;

use std::time::Duration;

use common::{ROOT, TestServer};
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const DEV: &str = "name: dev\nvariables:\n  - name: token\n    secret: true\n";

fn get(name: &str, seq: u32, url: &str, extra: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{url}\"\n{extra}"
    )
}

async fn echo_ok() -> MockServer {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&mock)
        .await;
    mock
}

#[tokio::test]
async fn reports_each_outcome_without_bodies_and_one_progress_per_request() {
    let mock = echo_ok().await;
    let status = "runtime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"201\"\n";
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("api/a.yml", &get("a", 1, &mock.uri(), "")),
        ("api/b.yml", &get("b", 2, &mock.uri(), status)),
        ("api/c.yml", &get("c", 3, "{{nope}}", "")),
    ])
    .await;
    let result = server.ok("run_folder", json!({"path": "api"})).await;
    assert_eq!(
        result["summary"],
        json!({"requests": 3, "passed": 1, "unverified": 0, "no_tests": 1, "failed": 1, "errored": 1, "skipped": 0, "cancelled": 0})
    );
    let outcomes: Vec<&str> = result["results"]
        .as_array()
        .expect("results")
        .iter()
        .map(|r| r["outcome"].as_str().expect("outcome"))
        .collect();
    assert_eq!(outcomes, ["passed", "failed", "errored"]);
    assert_eq!(result["results"][0]["status"], 200);
    assert!(
        result["results"][2]["error"]
            .as_str()
            .expect("error")
            .contains("nope")
    );
    assert!(
        !result.to_string().contains("\"ok\""),
        "bodies are never returned: {result}"
    );
    let progress = server.progress();
    assert_eq!(progress.len(), 3, "{progress:?}");
    assert_eq!(progress[2].progress, 3.0);
    assert_eq!(progress[2].total, Some(3.0));
    assert_eq!(progress[0].message.as_deref(), Some("api/a.yml: passed"));
}

#[tokio::test]
async fn bail_skips_the_rest_after_a_failure() {
    let mock = echo_ok().await;
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &get("a", 1, "{{nope}}", "")),
        ("b.yml", &get("b", 2, &mock.uri(), "")),
    ])
    .await;
    let result = server
        .ok("run_folder", json!({"path": "", "bail": true}))
        .await;
    assert_eq!(result["summary"]["errored"], 1);
    assert_eq!(result["summary"]["skipped"], 1);
    assert!(mock.received_requests().await.expect("recorded").is_empty());
}

#[tokio::test]
async fn an_untrusted_host_in_a_run_files_an_approval_and_errors_that_request() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &get("a", 1, "https://api.example.com/", "")),
    ])
    .await;
    let result = server.ok("run_folder", json!({"path": ""})).await;
    let error = result["results"][0]["error"].as_str().expect("error");
    assert!(error.contains("retry after the user approves"), "{error}");
    assert_eq!(server.inbox().pending().expect("inbox").len(), 1);
}

#[tokio::test]
async fn a_host_header_in_any_planned_request_refuses_the_whole_run() {
    let mock = echo_ok().await;
    let fronted = "  headers:\n    - name: Host\n      value: api.example.com\n";
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &get("a", 1, &mock.uri(), "")),
        ("b.yml", &get("b", 2, &mock.uri(), fronted)),
    ])
    .await;
    let message = server.err("run_folder", json!({"path": ""})).await;
    assert!(message.contains("b.yml"), "{message}");
    assert!(mock.received_requests().await.expect("recorded").is_empty());
}

#[tokio::test]
async fn progress_and_errors_are_scrubbed_of_the_run_sensitive_set() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        (
            "a.yml",
            &get("a", 1, "http://127.0.0.1:9/{{token}}/{{missing}}", ""),
        ),
        ("b.yml", &get("b", 2, "http://127.0.0.1:9/{{token}}", "")),
    ])
    .await;
    server.set_secret("dev", "token", "run-Secret-42");
    let result = server
        .call("run_folder", json!({"path": "", "environment": "dev"}))
        .await;
    let shown = format!(
        "{} {:?}",
        serde_json::to_string(&result).expect("serialises"),
        server.progress()
    );
    assert!(!shown.contains("run-Secret-42"), "{shown}");
}

#[tokio::test]
async fn a_client_cancel_aborts_the_send_in_flight_and_skips_the_rest() {
    let fast = echo_ok().await;
    // A listener that never answers, and reports when the client side closes the connection.
    let slow = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let slow_url = format!("http://{}/", slow.local_addr().expect("addr"));
    let (closed_tx, closed_rx) = oneshot::channel();
    tokio::spawn(async move {
        let (mut socket, _) = slow.accept().await.expect("accept");
        let mut buf = [0u8; 1024];
        while socket.read(&mut buf).await.is_ok_and(|n| n > 0) {}
        let _ = closed_tx.send(());
    });
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &get("a", 1, &fast.uri(), "")),
        ("b.yml", &get("b", 2, &slow_url, "")),
        ("c.yml", &get("c", 3, &fast.uri(), "")),
    ])
    .await;
    let handle = server.start_call("run_folder", json!({"path": ""})).await;
    // Wait for `a` to finish, so `b` is the send in flight.
    for _ in 0..100 {
        if !server.progress().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(server.progress().len(), 1);
    tokio::time::sleep(Duration::from_millis(100)).await;
    handle.cancel(None).await.expect("cancel sent");
    tokio::time::timeout(Duration::from_secs(5), closed_rx)
        .await
        .expect("the in-flight send was dropped")
        .expect("listener reported");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        fast.received_requests().await.expect("recorded").len(),
        1,
        "c never sent"
    );
}

#[tokio::test]
async fn a_secret_host_header_refuses_the_run_and_is_scrubbed() {
    let mock = echo_ok().await;
    let fronted = "  headers:\n    - name: Host\n      value: \"{{token}}\"\n";
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("a.yml", &get("a", 1, &mock.uri(), fronted)),
    ])
    .await;
    server.set_secret("dev", "token", "Fronted-Host-7");
    let result = server
        .call("run_folder", json!({"path": "", "environment": "dev"}))
        .await;
    assert_eq!(result.is_error, Some(true));
    let shown = serde_json::to_string(&result).expect("serialises");
    assert!(shown.contains("Host header `<secret>`"), "{shown}");
    assert!(!shown.contains("Fronted-Host-7"), "{shown}");
    assert!(mock.received_requests().await.expect("recorded").is_empty());
}

fn tagged(name: &str, seq: u32, url: &str, tag: &str, extra: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\n  tags:\n    - {tag}\nhttp:\n  method: GET\n  url: \"{url}\"\n{extra}"
    )
}

#[tokio::test]
async fn tags_filter_the_run_before_the_host_check() {
    let mock = echo_ok().await;
    let fronted = "  headers:\n    - name: Host\n      value: api.example.com\n";
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &tagged("a", 1, &mock.uri(), "smoke", "")),
        ("b.yml", &tagged("b", 2, &mock.uri(), "slow", fronted)),
    ])
    .await;
    for args in [
        json!({"path": "", "tags": ["smoke"]}),
        json!({"path": "", "exclude_tags": [" slow "]}),
    ] {
        let result = server.ok("run_folder", args).await;
        let paths: Vec<&str> = result["results"]
            .as_array()
            .expect("results")
            .iter()
            .map(|r| r["path"].as_str().expect("path"))
            .collect();
        assert_eq!(paths, ["a.yml"]);
    }
    assert_eq!(mock.received_requests().await.expect("recorded").len(), 2);
}

#[tokio::test]
async fn a_run_that_matches_no_request_is_a_tool_error() {
    let mock = echo_ok().await;
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &tagged("a", 1, &mock.uri(), "smoke", "")),
    ])
    .await;
    let result = server
        .call("run_folder", json!({"path": "", "tags": ["nope"]}))
        .await;
    assert_eq!(result.is_error, Some(true));
    let shown = serde_json::to_string(&result).expect("serialises");
    assert!(shown.contains("no requests matched: tags nope"), "{shown}");
    assert!(mock.received_requests().await.expect("recorded").is_empty());
}

#[tokio::test]
async fn delay_ms_spaces_the_requests() {
    let mock = echo_ok().await;
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &get("a", 1, &mock.uri(), "")),
        ("b.yml", &get("b", 2, &mock.uri(), "")),
    ])
    .await;
    let t = std::time::Instant::now();
    let result = server
        .ok("run_folder", json!({"path": "", "delay_ms": 400}))
        .await;
    assert_eq!(result["summary"]["passed"], 2);
    assert!(t.elapsed() >= Duration::from_millis(400));
}

fn item_urls(requests: &[wiremock::Request]) -> Vec<String> {
    requests.iter().map(|r| r.url.path().to_owned()).collect()
}

#[tokio::test]
async fn inline_data_runs_the_folder_once_per_row_and_groups_results_by_iteration() {
    let mock = echo_ok().await;
    let url = format!("{}/item/{{{{id}}}}", mock.uri());
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("api/a.yml", &get("a", 1, &url, "")),
    ])
    .await;
    let result = server
        .ok(
            "run_folder",
            json!({"path": "api", "data": [{"id": "1", "tag": null}, {"id": 2}]}),
        )
        .await;
    let seen = mock.received_requests().await.expect("recorded");
    assert_eq!(item_urls(&seen), ["/item/1", "/item/2"]);
    let iterations = result["iterations"].as_array().expect("iterations");
    assert_eq!(iterations.len(), 2, "{result}");
    assert_eq!(iterations[0]["index"], 0);
    assert_eq!(iterations[0]["data"], json!({"id": "1", "tag": ""}));
    assert_eq!(iterations[1]["data"], json!({"id": "2"}));
    assert_eq!(iterations[1]["results"][0]["outcome"], "passed");
    assert_eq!(result["summary"]["requests"], 2);
}

#[tokio::test]
async fn iterations_without_data_runs_that_many_passes() {
    let mock = echo_ok().await;
    let url = format!("{}/n/{{{{iteration}}}}", mock.uri());
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &get("a", 1, &url, "")),
    ])
    .await;
    let result = server
        .ok("run_folder", json!({"path": "", "iterations": 3}))
        .await;
    let seen = mock.received_requests().await.expect("recorded");
    assert_eq!(item_urls(&seen), ["/n/0", "/n/1", "/n/2"]);
    let iterations = result["iterations"].as_array().expect("iterations");
    assert_eq!(iterations.len(), 3);
    assert_eq!(iterations[2]["data"], json!({}));
}

#[tokio::test]
async fn a_run_without_iterations_keeps_the_flat_shape() {
    let mock = echo_ok().await;
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &get("a", 1, &mock.uri(), "")),
    ])
    .await;
    let result = server.ok("run_folder", json!({"path": ""})).await;
    assert_eq!(result["results"].as_array().expect("results").len(), 1);
    assert!(result.get("iterations").is_none(), "{result}");
}

#[tokio::test]
async fn the_iteration_cap_and_bad_counts_are_clear_errors_and_nothing_is_sent() {
    let mock = echo_ok().await;
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &get("a", 1, &mock.uri(), "")),
    ])
    .await;
    let over = server
        .err("run_folder", json!({"path": "", "iterations": 101}))
        .await;
    assert!(over.contains("100"), "{over}");
    let zero = server
        .err("run_folder", json!({"path": "", "iterations": 0}))
        .await;
    assert!(zero.contains("at least 1"), "{zero}");
    let rows: Vec<_> = (0..101).map(|i| json!({"id": i})).collect();
    let many = server
        .err("run_folder", json!({"path": "", "data": rows}))
        .await;
    assert!(many.contains("100"), "{many}");
    let empty = server
        .err("run_folder", json!({"path": "", "data": []}))
        .await;
    assert!(empty.contains("no rows"), "{empty}");
    assert!(mock.received_requests().await.expect("recorded").is_empty());
}

#[tokio::test]
async fn disable_cookies_neither_stores_nor_sends_them_and_the_next_run_is_unaffected() {
    let mock = MockServer::start().await;
    Mock::given(wiremock::matchers::path("/set"))
        .respond_with(ResponseTemplate::new(200).append_header("Set-Cookie", "sid=abc; Path=/"))
        .mount(&mock)
        .await;
    Mock::given(wiremock::matchers::path("/check"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &get("a", 1, &format!("{}/set", mock.uri()), "")),
        ("b.yml", &get("b", 2, &format!("{}/check", mock.uri()), "")),
    ])
    .await;
    let cookie_on_check = |seen: &[wiremock::Request]| {
        seen.iter()
            .filter(|r| r.url.path() == "/check")
            .map(|r| r.headers.contains_key("cookie"))
            .collect::<Vec<_>>()
    };
    let cookie_count = |s: &Value| s["cookies"].as_array().expect("cookies").len();
    // A disabled run stores nothing, even though /set answers with a cookie.
    server
        .ok("run_folder", json!({"path": "", "disable_cookies": true}))
        .await;
    assert_eq!(cookie_count(&server.ok("list_cookies", json!({})).await), 0);
    // The switch is undone afterwards: a normal run stores, and its /check sends the cookie.
    server.ok("run_folder", json!({"path": ""})).await;
    assert_eq!(cookie_count(&server.ok("list_cookies", json!({})).await), 1);
    // With a cookie in the jar, a disabled run still sends none.
    server
        .ok("run_folder", json!({"path": "", "disable_cookies": true}))
        .await;
    let seen = mock.received_requests().await.expect("recorded");
    assert_eq!(cookie_on_check(&seen), [false, true, false]);
}

fn cookie_mock_files(mock: &MockServer) -> Vec<(String, String)> {
    let at = |name: &str, seq: u32, path: &str| {
        (
            format!("{name}.yml"),
            get(name, seq, &format!("{}{path}", mock.uri()), ""),
        )
    };
    vec![
        ("opencollection.yml".to_owned(), ROOT.to_owned()),
        at("set", 1, "/set"),
        at("check", 2, "/check"),
        (
            "slow/s.yml".to_owned(),
            get("s", 1, &format!("{}/slow", mock.uri()), ""),
        ),
    ]
}

async fn cookie_mock() -> MockServer {
    let mock = MockServer::start().await;
    Mock::given(wiremock::matchers::path("/set"))
        .respond_with(ResponseTemplate::new(200).append_header("Set-Cookie", "sid=abc; Path=/"))
        .mount(&mock)
        .await;
    Mock::given(wiremock::matchers::path("/check"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;
    Mock::given(wiremock::matchers::path("/slow"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(400)))
        .mount(&mock)
        .await;
    mock
}

#[tokio::test]
async fn a_send_during_a_disable_cookies_run_still_gets_cookies() {
    let mock = cookie_mock().await;
    let server = TestServer::start_owned(&cookie_mock_files(&mock)).await;
    server.ok("send_request", json!({"path": "set.yml"})).await;
    let run = server
        .start_call(
            "run_folder",
            json!({"path": "slow", "disable_cookies": true}),
        )
        .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    server
        .ok("send_request", json!({"path": "check.yml"}))
        .await;
    run.await_response().await.expect("run finished");
    let seen = mock.received_requests().await.expect("recorded");
    let check = seen
        .iter()
        .find(|r| r.url.path() == "/check")
        .expect("sent");
    assert!(check.headers.contains_key("cookie"), "{:?}", check.headers);
}

#[tokio::test]
async fn overlapping_disable_cookies_runs_leave_cookies_on() {
    let mock = cookie_mock().await;
    let server = TestServer::start_owned(&cookie_mock_files(&mock)).await;
    let args = json!({"path": "slow", "disable_cookies": true});
    let first = server.start_call("run_folder", args.clone()).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let second = server.start_call("run_folder", args).await;
    first.await_response().await.expect("first finished");
    second.await_response().await.expect("second finished");
    server.ok("send_request", json!({"path": "set.yml"})).await;
    let listed = server.ok("list_cookies", json!({})).await;
    assert_eq!(listed["cookies"].as_array().expect("cookies").len(), 1);
}

#[tokio::test]
async fn data_and_iterations_combine_as_the_smaller_and_each_row_sends() {
    let mock = echo_ok().await;
    let url = format!("{}/item/{{{{id}}}}", mock.uri());
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &get("a", 1, &url, "")),
    ])
    .await;
    let result = server
        .ok(
            "run_folder",
            json!({"path": "", "iterations": 2, "data": [{"id": "x"}, {"id": "y"}, {"id": "z"}]}),
        )
        .await;
    assert_eq!(result["iterations"].as_array().expect("passes").len(), 2);
    assert_eq!(result["summary"]["passed"], 2);
    let seen = mock.received_requests().await.expect("recorded");
    assert_eq!(item_urls(&seen), ["/item/x", "/item/y"]);
}

#[tokio::test]
async fn a_secret_equal_to_a_data_value_is_masked_in_the_iteration_output() {
    let mock = echo_ok().await;
    let url = format!("{}/x?t={{{{token}}}}", mock.uri());
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("a.yml", &get("a", 1, &url, "")),
    ])
    .await;
    server.set_secret("dev", "token", "Data-Secret-91");
    let result = server
        .call(
            "run_folder",
            json!({"path": "", "environment": "dev", "data": [{"v": "Data-Secret-91"}]}),
        )
        .await;
    let shown = serde_json::to_string(&result).expect("serialises");
    assert!(shown.contains("\"iterations\""), "{shown}");
    assert!(!shown.contains("Data-Secret-91"), "{shown}");
}

#[tokio::test]
async fn the_run_folder_schema_has_no_insecure_option() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let tools = server.tools().await;
    let tool = tools
        .iter()
        .find(|t| t.name == "run_folder")
        .expect("listed");
    let schema = Value::Object((*tool.input_schema).clone());
    let properties = schema["properties"].as_object().expect("properties");
    assert!(!properties.contains_key("insecure"), "{properties:?}");
    for wanted in ["iterations", "data", "disable_cookies"] {
        assert!(properties.contains_key(wanted), "{wanted}");
    }
}

#[tokio::test]
async fn a_request_whose_url_and_params_disagree_reports_query_mismatch() {
    let mock = echo_ok().await;
    let mismatched = get(
        "m",
        1,
        &format!("{}/m?z=6", mock.uri()),
        "  params:\n    - name: z\n      value: \"3\"\n      type: query\n",
    );
    let agreeing = get(
        "a",
        2,
        &format!("{}/a?z=3", mock.uri()),
        "  params:\n    - name: z\n      value: \"3\"\n      type: query\n",
    );
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("m.yml", &mismatched),
        ("a.yml", &agreeing),
    ])
    .await;
    let result = server.ok("run_folder", json!({"path": ""})).await;
    assert_eq!(
        result["results"][0]["query_mismatch"],
        json!({"names": ["z"], "note": "the URL's query is sent"})
    );
    assert!(
        result["results"][1].get("query_mismatch").is_none(),
        "{result}"
    );
    let tools = server.tools().await;
    let tool = tools
        .iter()
        .find(|t| t.name == "run_folder")
        .expect("listed");
    let description = tool.description.as_deref().unwrap_or_default();
    assert!(description.contains("query_mismatch"), "{description}");
}
