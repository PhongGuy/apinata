//! Post-response variables over MCP: captures persist across calls, are listed, and chain in runs.

use crate::common;

use common::{ROOT, TestServer};
use serde_json::json;
use wiremock::matchers::{header, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn action(name: &str, expression: &str) -> String {
    format!(
        "    - type: set-variable\n      phase: after-response\n      selector:\n        expression: {expression}\n        method: jsonq\n      variable:\n        name: {name}\n        scope: runtime\n"
    )
}

async fn mock() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(path("/login"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw("{\"token\":\"abc\"}", "application/json"),
        )
        .mount(&s)
        .await;
    Mock::given(path("/me"))
        .and(header("authorization", "Bearer abc"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    Mock::given(path("/me"))
        .respond_with(ResponseTemplate::new(401))
        .with_priority(10)
        .mount(&s)
        .await;
    s
}

fn files(uri: &str) -> Vec<(String, String)> {
    let login = format!(
        "info:\n  name: login\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: \"{uri}/login\"\nruntime:\n  actions:\n{}{}",
        action("token", "res.body.token"),
        action("len", "res.body.token.length")
    );
    let me = format!(
        "info:\n  name: me\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: \"{uri}/me\"\n  headers:\n    - name: Authorization\n      value: \"Bearer {{{{token}}}}\"\n"
    );
    vec![
        ("opencollection.yml".to_owned(), ROOT.to_owned()),
        ("login.yml".to_owned(), login),
        ("me.yml".to_owned(), me),
    ]
}

#[tokio::test]
async fn captures_persist_across_calls_and_are_listed() {
    let s = mock().await;
    let server = TestServer::start_owned(&files(&s.uri())).await;
    assert_eq!(
        server.ok("list_runtime_variables", json!({})).await,
        json!({"variables": []})
    );
    let first = server
        .ok("send_request", json!({"path": "login.yml"}))
        .await;
    assert_eq!(
        first["post_response_vars"],
        json!([
            {"name": "token", "result": "set"},
            {"name": "len", "result": "set"}
        ])
    );
    let second = server.ok("send_request", json!({"path": "me.yml"})).await;
    assert_eq!(second["status"], 200);
    assert_eq!(
        server.ok("list_runtime_variables", json!({})).await,
        json!({"variables": [{"name": "len", "value": "3"}, {"name": "token", "value": "abc"}]})
    );
}

#[tokio::test]
async fn a_run_chains_within_itself() {
    let s = mock().await;
    let server = TestServer::start_owned(&files(&s.uri())).await;
    let result = server.ok("run_folder", json!({"path": ""})).await;
    assert_eq!(
        result["results"][0]["post_response_vars"][0],
        json!({"name": "token", "result": "set"})
    );
    assert_eq!(result["results"][1]["status"], 200);
}

#[tokio::test]
async fn a_run_refuses_a_host_header_it_cannot_check_beforehand() {
    let s = mock().await;
    let mut tree = files(&s.uri());
    tree.push((
        "fronted.yml".to_owned(),
        "info:\n  name: fronted\n  type: http\n  seq: 3\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:9/\"\n  headers:\n    - name: host\n      value: \"{{later}}\"\n".to_owned(),
    ));
    let server = TestServer::start_owned(&tree).await;
    let error = server.err("run_folder", json!({"path": ""})).await;
    assert!(
        error.contains(
            "`fronted.yml`: the Host header could not be checked before the run; nothing was sent"
        ),
        "{error}"
    );
    assert!(s.received_requests().await.unwrap_or_default().is_empty());
}

#[tokio::test]
async fn a_run_with_a_host_header_that_resolves_beforehand_is_sent() {
    let s = mock().await;
    let host = s.uri().trim_start_matches("http://").to_owned();
    let mut tree = files(&s.uri());
    tree.push((
        "hosted.yml".to_owned(),
        format!("info:\n  name: hosted\n  type: http\n  seq: 3\nhttp:\n  method: GET\n  url: \"{}/login\"\n  headers:\n    - name: Host\n      value: \"{host}\"\n", s.uri()),
    ));
    let server = TestServer::start_owned(&tree).await;
    let result = server.ok("run_folder", json!({"path": ""})).await;
    assert_eq!(result["summary"]["requests"], 3);
    assert_eq!(result["results"][2]["status"], 200);
}

/// Whether `s` received any request whose Host header is `host`.
async fn received_host(s: &MockServer, host: &str) -> bool {
    s.received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .any(|r| r.headers.get("host").is_some_and(|v| v == host))
}

#[tokio::test]
async fn a_host_header_a_capture_changes_is_refused_when_it_is_sent() {
    let s = mock().await;
    let host = s.uri().trim_start_matches("http://").to_owned();
    let mut tree = files(&s.uri());
    // login also captures `h`, which the steered request's Host header reads.
    tree[1].1.push_str(&action("h", "res.body.token"));
    tree.push((
        "environments/dev.yml".to_owned(),
        format!("name: dev\nvariables:\n  - name: h\n    value: \"{host}\"\n"),
    ));
    tree.push((
        "steered.yml".to_owned(),
        format!("info:\n  name: steered\n  type: http\n  seq: 3\nhttp:\n  method: GET\n  url: \"{}/login\"\n  headers:\n    - name: Host\n      value: \"{{{{h}}}}\"\n", s.uri()),
    ));
    let server = TestServer::start_owned(&tree).await;
    let result = server
        .ok("run_folder", json!({"path": "", "environment": "dev"}))
        .await;
    assert_eq!(result["results"][0]["status"], 200);
    assert_eq!(result["results"][1]["status"], 200);
    assert_eq!(result["results"][2]["outcome"], "errored");
    assert_eq!(
        result["results"][2]["error"],
        "the Host header `abc` differs from the URL's host; nothing was sent"
    );
    assert_eq!(s.received_requests().await.unwrap_or_default().len(), 2);
    assert!(!received_host(&s, "abc").await);
}

#[tokio::test]
async fn a_header_name_a_capture_turns_into_host_is_refused_when_it_is_sent() {
    let s = mock().await;
    Mock::given(path("/name"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw("{\"n\":\"Host\"}", "application/json"),
        )
        .mount(&s)
        .await;
    let mut tree = files(&s.uri());
    tree.push((
        "environments/dev.yml".to_owned(),
        "name: dev\nvariables:\n  - name: n\n    value: X-Other\n".to_owned(),
    ));
    tree.push((
        "name.yml".to_owned(),
        format!("info:\n  name: name\n  type: http\n  seq: 3\nhttp:\n  method: GET\n  url: \"{}/name\"\nruntime:\n  actions:\n{}", s.uri(), action("n", "res.body.n")),
    ));
    tree.push((
        "named.yml".to_owned(),
        format!("info:\n  name: named\n  type: http\n  seq: 4\nhttp:\n  method: GET\n  url: \"{}/login\"\n  headers:\n    - name: \"{{{{n}}}}\"\n      value: evil.example.com\n", s.uri()),
    ));
    let server = TestServer::start_owned(&tree).await;
    let result = server
        .ok("run_folder", json!({"path": "", "environment": "dev"}))
        .await;
    assert_eq!(
        result["results"][2]["post_response_vars"][0],
        json!({"name": "n", "result": "set"})
    );
    assert_eq!(result["results"][3]["outcome"], "errored");
    assert_eq!(
        result["results"][3]["error"],
        "the Host header `evil.example.com` differs from the URL's host; nothing was sent"
    );
    assert!(!received_host(&s, "evil.example.com").await);
}

#[tokio::test]
async fn a_run_refuses_an_inherited_host_header_it_cannot_check_beforehand() {
    let s = mock().await;
    let mut tree = files(&s.uri());
    tree.push((
        "api/folder.yml".to_owned(),
        "info:\n  name: api\nrequest:\n  headers:\n    - name: Host\n      value: \"{{later}}\"\n"
            .to_owned(),
    ));
    tree.push((
        "api/inner.yml".to_owned(),
        format!("info:\n  name: inner\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{}/login\"\n", s.uri()),
    ));
    let server = TestServer::start_owned(&tree).await;
    let error = server.err("run_folder", json!({"path": ""})).await;
    assert!(
        error.contains(
            "`api/inner.yml`: the Host header could not be checked before the run; nothing was sent"
        ),
        "{error}"
    );
    assert!(s.received_requests().await.unwrap_or_default().is_empty());
}

#[tokio::test]
async fn a_secret_in_a_url_that_resolves_only_during_the_run_is_scrubbed() {
    const SECRET: &str = "Path-Secret-93";
    let s = mock().await;
    let mut tree = files(&s.uri());
    // login also captures `later`, which completes a URL that holds the secret in its path.
    tree[1].1.push_str(&action("later", "res.body.token"));
    tree.push((
        "environments/dev.yml".to_owned(),
        "name: dev\nvariables:\n  - name: key\n    secret: true\n".to_owned(),
    ));
    tree.push((
        "late.yml".to_owned(),
        "info:\n  name: late\n  type: http\n  seq: 3\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:9/{{key}}/{{later}}\"\n".to_owned(),
    ));
    let server = TestServer::start_owned(&tree).await;
    server.set_secret("dev", "key", SECRET);
    let result = server
        .ok("run_folder", json!({"path": "", "environment": "dev"}))
        .await;
    assert_eq!(result["results"][2]["outcome"], "errored");
    let shown = result.to_string();
    assert!(!shown.contains(SECRET), "{shown}");
}
