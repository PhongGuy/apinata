//! `send_request` lists each hop's Set-Cookie verdicts by name, never by value.

use crate::common;

use common::{ROOT, TestServer, request};
use serde_json::{Value, json};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The cookie values the chain sets; none may reach the agent.
const VALUES: [&str; 3] = ["KeptVal-Q1x", "SecureVal-Q2x", "NextVal-Q3x"];

/// `app.test` (mapped to 127.0.0.1) answers `/start` with a stored cookie and a Secure one
/// rejected over plain http, then redirects to `/next`, which stores another.
async fn chain() -> (MockServer, TestServer) {
    chain_with("").await
}

/// [`chain`], with `runtime` appended to the request file as its top-level `runtime` YAML.
async fn chain_with(runtime: &str) -> (MockServer, TestServer) {
    let mock = MockServer::start().await;
    let port = common::port_of(&mock.uri());
    Mock::given(path("/start"))
        .respond_with(
            ResponseTemplate::new(302)
                .append_header("set-cookie", format!("kept={}; Path=/", VALUES[0]).as_str())
                .append_header(
                    "set-cookie",
                    format!("sec={}; Secure; Path=/", VALUES[1]).as_str(),
                )
                .insert_header("location", format!("http://app.test:{port}/next").as_str()),
        )
        .mount(&mock)
        .await;
    Mock::given(path("/next"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("set-cookie", format!("next={}; Path=/", VALUES[2]).as_str()),
        )
        .mount(&mock)
        .await;
    let root = format!(
        "{ROOT}extensions:\n  apinata:\n    hostOverrides:\n      - host: \"app.test\"\n        address: \"127.0.0.1\"\n"
    );
    let server = TestServer::start(&[
        ("opencollection.yml", &root),
        (
            "r.yml",
            &(request("r", 1, "GET", &format!("http://app.test:{port}/start")) + runtime),
        ),
    ])
    .await;
    (mock, server)
}

#[tokio::test]
async fn each_hop_lists_its_cookies_with_the_jars_verdict() {
    let (_mock, server) = chain().await;
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    let hops = result["timeline"].as_array().expect("timeline");
    assert_eq!(hops.len(), 2, "{result}");
    assert_eq!(
        hops[0]["cookies"],
        json!([
            {"name": "kept", "path": "/", "stored": true},
            {"name": "sec", "path": "/", "stored": false, "reason": "Secure cookie over http"},
        ]),
        "{result}"
    );
    assert_eq!(
        hops[1]["cookies"],
        json!([{"name": "next", "path": "/", "stored": true}]),
        "{result}"
    );
}

#[tokio::test]
async fn no_cookie_value_reaches_the_agent() {
    let (_mock, server) = chain().await;
    let whole = server.call("send_request", json!({"path": "r.yml"})).await;
    let shown = serde_json::to_string(&whole).expect("serialises");
    assert!(shown.contains("Secure cookie over http"), "{shown}");
    for value in VALUES {
        assert!(!shown.contains(value), "{value} leaked: {shown}");
    }
}

/// The values the pre-request script's cookies carry; none may reach the agent either.
const SCRIPT_VALUES: [&str; 2] = ["PreVal-Q4x", "EvilVal-Q5x"];

/// A pre-request script that stores one cookie and has one for another domain refused.
fn pre_script() -> String {
    format!(
        "await bru.cookies.add({{ key: \"pre\", value: \"{}\" }});\nawait bru.cookies.add({{ key: \"evil\", value: \"{}\", domain: \"evil.example\" }});",
        SCRIPT_VALUES[0], SCRIPT_VALUES[1]
    )
}

/// The chain, its request running [`pre_script`] approved, sent once; the whole result.
async fn send_scripted_chain() -> rmcp::model::CallToolResult {
    let code = pre_script();
    let body: String = code.lines().map(|l| format!("        {l}\n")).collect();
    let runtime =
        format!("runtime:\n  scripts:\n    - type: before-request\n      code: |-\n{body}");
    let (_mock, server) = chain_with(&runtime).await;
    server.approve_scripts(&[&code]);
    server.call("send_request", json!({"path": "r.yml"})).await
}

#[tokio::test]
async fn script_cookies_list_each_script_cookie_with_the_jars_verdict() {
    let whole = send_scripted_chain().await;
    let result = whole.structured_content.expect("structured");
    assert_eq!(
        result["script_cookies"],
        json!([
            {"name": "pre", "phase": "pre-request", "stored": true},
            {"name": "evil", "domain": "evil.example", "phase": "pre-request", "stored": false,
             "reason": "Domain does not match host"},
        ]),
        "{result}"
    );
    let hops = result["timeline"].as_array().expect("timeline");
    assert_eq!(
        hops[0]["cookies"].as_array().map(Vec::len),
        Some(2),
        "{result}"
    );
    assert_eq!(
        hops[1]["cookies"].as_array().map(Vec::len),
        Some(1),
        "{result}"
    );
}

#[tokio::test]
async fn no_script_cookie_value_reaches_the_agent() {
    let whole = send_scripted_chain().await;
    let shown = serde_json::to_string(&whole).expect("serialises");
    assert!(shown.contains("script_cookies"), "{shown}");
    for value in VALUES.iter().chain(&SCRIPT_VALUES) {
        assert!(!shown.contains(value), "{value} leaked: {shown}");
    }
}

#[tokio::test]
async fn a_send_without_script_cookies_has_no_script_cookies_field() {
    let (_mock, server) = chain().await;
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    assert_eq!(result.get("script_cookies"), None::<&Value>, "{result}");
}

#[tokio::test]
async fn a_hop_without_cookies_has_no_cookies_field() {
    let mock = MockServer::start().await;
    Mock::given(path("/plain"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &request("r", 1, "GET", &format!("{}/plain", mock.uri())),
        ),
    ])
    .await;
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    assert_eq!(
        result["timeline"][0].get("cookies"),
        None::<&Value>,
        "{result}"
    );
}

/// A server whose `/one` answers with the single Set-Cookie value `cookie`.
async fn setting(cookie: &str) -> MockServer {
    let mock = MockServer::start().await;
    Mock::given(path("/one"))
        .respond_with(ResponseTemplate::new(200).insert_header("set-cookie", cookie))
        .mount(&mock)
        .await;
    mock
}

#[tokio::test]
async fn an_unparseable_cookie_is_listed_without_a_name() {
    let mock = setting("NamelessVal-Z9; Path=/").await;
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &request("r", 1, "GET", &format!("{}/one", mock.uri())),
        ),
    ])
    .await;
    // Only the timeline is checked: the masked response headers are core's rule, not this one.
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    let timeline = result["timeline"].to_string();
    assert!(!timeline.contains("NamelessVal-Z9"), "{timeline}");
    assert_eq!(
        result["timeline"][0]["cookies"],
        json!([{"stored": false, "reason": "Could not be parsed"}]),
        "{result}"
    );
}

#[tokio::test]
async fn a_hop_url_never_shows_its_userinfo_password() {
    let mock = setting("a=1; Path=/").await;
    let port = common::port_of(&mock.uri());
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &request(
                "r",
                1,
                "GET",
                &format!("http://user:PassWd-Q7x@127.0.0.1:{port}/one"),
            ),
        ),
    ])
    .await;
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    let url = result["timeline"][0]["url"].as_str().expect("hop url");
    assert!(!url.contains("PassWd-Q7x"), "{result}");
    assert!(url.ends_with(&format!("127.0.0.1:{port}/one")), "{result}");
}

#[tokio::test]
async fn a_secret_used_as_a_cookie_name_or_in_the_hop_url_is_redacted() {
    let mock = setting("LeakyName9=v; Path=/").await;
    let server = TestServer::start_with(
        &[
            ("opencollection.yml", ROOT),
            (
                "r.yml",
                &request("r", 1, "GET", &format!("{}/one?k={{{{key}}}}", mock.uri())),
            ),
        ],
        |o| o.env_var("key", "LeakyName9"),
    )
    .await;
    let whole = server.call("send_request", json!({"path": "r.yml"})).await;
    let shown = serde_json::to_string(&whole).expect("serialises");
    assert!(!shown.contains("LeakyName9"), "{shown}");
    let result = whole.structured_content.expect("structured");
    let hop = &result["timeline"][0];
    assert!(hop["cookies"][0]["name"].is_string(), "{result}");
    assert!(hop["url"].as_str().expect("url").contains("k="), "{result}");
}

/// The whole `send_request` result for one GET of `url`, structured content and text blocks.
async fn whole_result_of(url: &str) -> (String, Value) {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request("r", 1, "GET", url)),
    ])
    .await;
    let whole = server.call("send_request", json!({"path": "r.yml"})).await;
    let shown = serde_json::to_string(&whole).expect("serialises");
    (shown, whole.structured_content.expect("structured"))
}

#[tokio::test]
async fn a_set_cookie_without_an_equals_sign_never_reaches_the_agent() {
    for cookie in ["sekrit-token; Path=/", "sekrit-token"] {
        let mock = setting(cookie).await;
        let (shown, _) = whole_result_of(&format!("{}/one", mock.uri())).await;
        assert!(!shown.contains("sekrit"), "{cookie:?} leaked: {shown}");
    }
}

#[tokio::test]
async fn the_top_level_url_never_shows_its_userinfo_password() {
    let mock = setting("a=1; Path=/").await;
    let port = common::port_of(&mock.uri());
    let (shown, result) =
        whole_result_of(&format!("http://user:pa55word-xyz@127.0.0.1:{port}/one")).await;
    assert!(!shown.contains("pa55word-xyz"), "{shown}");
    let url = result["url"].as_str().expect("url");
    assert!(!url.contains('@'), "{result}");
    for hop in result["timeline"].as_array().expect("timeline") {
        assert!(
            !hop["url"].as_str().expect("hop url").contains('@'),
            "{result}"
        );
    }
}

#[tokio::test]
async fn a_redirect_to_a_url_with_userinfo_never_shows_its_password() {
    let mock = MockServer::start().await;
    let port = common::port_of(&mock.uri());
    Mock::given(path("/start"))
        .respond_with(ResponseTemplate::new(302).insert_header(
            "location",
            format!("http://joe:hop-pa55-xyz@127.0.0.1:{port}/next").as_str(),
        ))
        .mount(&mock)
        .await;
    Mock::given(path("/next"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;
    let (shown, result) = whole_result_of(&format!("{}/start", mock.uri())).await;
    let hops = result["timeline"].as_array().expect("timeline");
    assert_eq!(hops.len(), 2, "{result}");
    assert!(!shown.contains("hop-pa55-xyz"), "{shown}");
    assert!(
        !result["url"].as_str().expect("url").contains('@'),
        "{result}"
    );
}

#[tokio::test]
async fn an_unfollowed_redirect_location_never_shows_its_password() {
    let mock = MockServer::start().await;
    let port = common::port_of(&mock.uri());
    Mock::given(path("/start"))
        .respond_with(ResponseTemplate::new(302).insert_header(
            "location",
            format!("http://joe:hop-pa55-xyz@127.0.0.1:{port}/next").as_str(),
        ))
        .mount(&mock)
        .await;
    let file = request("r", 1, "GET", &format!("{}/start", mock.uri()))
        + "settings:\n  followRedirects: false\n";
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", &file)]).await;
    let whole = server.call("send_request", json!({"path": "r.yml"})).await;
    let shown = serde_json::to_string(&whole).expect("serialises");
    assert!(shown.contains("302"), "{shown}");
    assert!(!shown.contains("hop-pa55-xyz"), "{shown}");
}

/// A send to a closed 127.0.0.1 port whose approved pre-request script stores one cookie; the
/// whole failed result.
async fn send_failed_scripted() -> rmcp::model::CallToolResult {
    let code = "await bru.cookies.add({ key: \"pre\", value: \"FailVal-Q9x\" });";
    let file = request("r", 1, "GET", "http://127.0.0.1:1/r")
        + &format!(
            "runtime:\n  scripts:\n    - type: before-request\n      code: |-\n        {code}\n"
        );
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", &file)]).await;
    server.approve_scripts(&[code]);
    server.call("send_request", json!({"path": "r.yml"})).await
}

#[tokio::test]
async fn a_failed_send_still_reports_its_pre_request_script_cookie() {
    let whole = send_failed_scripted().await;
    assert_eq!(whole.is_error, Some(true), "{whole:?}");
    let message = common::text(&whole);
    assert!(
        message
            .lines()
            .any(|l| l == "set-cookie pre: Stored (pre-request script)"),
        "{message}"
    );
    let shown = serde_json::to_string(&whole).expect("serialises");
    assert!(!shown.contains("FailVal-Q9x"), "{shown}");
}

#[tokio::test]
async fn a_failed_send_has_no_structured_content_and_one_value_free_cookie_line() {
    // The output schema is the success shape, which a send with no response cannot fill.
    let whole = send_failed_scripted().await;
    assert_eq!(whole.is_error, Some(true), "{whole:?}");
    assert_eq!(whole.structured_content, None, "{whole:?}");
    let message = common::text(&whole);
    let cookie_lines: Vec<&str> = message
        .lines()
        .filter(|l| l.starts_with("set-cookie "))
        .collect();
    assert_eq!(
        cookie_lines,
        ["set-cookie pre: Stored (pre-request script)"],
        "{message}"
    );
    assert!(!message.contains("FailVal-Q9x"), "{message}");
}
