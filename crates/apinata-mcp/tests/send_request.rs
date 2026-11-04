//! `send_request` against local mock servers: the result shape, redaction of credentials and
//! sensitive values, and the refusals that send nothing.

use crate::common;

use common::{ROOT, TestServer};
use serde_json::{Value, json};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const DEV: &str = "name: dev\nvariables:\n  - name: token\n    secret: true\n";

/// A GET of `url` with `extra` appended under `http:` (headers, auth, body).
fn get(url: &str, extra: &str) -> String {
    format!("info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\n{extra}")
}

async fn mock(template: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(template)
        .mount(&server)
        .await;
    server
}

async fn start(request: &str) -> TestServer {
    TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("r.yml", request),
    ])
    .await
}

fn header<'a>(headers: &'a Value, name: &str) -> &'a str {
    headers
        .as_array()
        .expect("headers")
        .iter()
        .find(|h| {
            h["name"]
                .as_str()
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
        .and_then(|h| h["value"].as_str())
        .unwrap_or_else(|| panic!("no {name} in {headers}"))
}

#[tokio::test]
async fn reports_status_headers_body_and_timeline_with_credentials_redacted() {
    let mock = mock(
        ResponseTemplate::new(201)
            .insert_header("set-cookie", "sid=abc123; Path=/")
            .set_body_raw("{\"ok\":true}", "application/json"),
    )
    .await;
    let auth = "  auth:\n    type: bearer\n    token: plain-bearer\n";
    let server = start(&get(&format!("{}/users", mock.uri()), auth)).await;
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    assert_eq!(result["status"], 201);
    assert_eq!(result["body"]["text"], "{\"ok\":true}");
    assert_eq!(result["body"]["content_type"], "application/json");
    assert_eq!(result["body"]["truncated"], false);
    assert_eq!(
        header(&result["request_headers"], "authorization"),
        "<redacted>"
    );
    assert_eq!(
        header(&result["response_headers"], "set-cookie"),
        "sid=[redacted]"
    );
    assert_eq!(result["timeline"].as_array().expect("timeline").len(), 1);
    assert_eq!(result["timeline"][0]["status"], 201);
    assert!(result["url"].as_str().expect("url").ends_with("/users"));
}

#[tokio::test]
async fn a_host_header_naming_another_host_is_refused_before_sending() {
    let mock = mock(ResponseTemplate::new(200)).await;
    let headers = "  headers:\n    - name: Host\n      value: api.example.com\n";
    let server = start(&get(&mock.uri(), headers)).await;
    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(message.contains("Host header"), "{message}");
    assert!(mock.received_requests().await.expect("recorded").is_empty());
}

#[tokio::test]
async fn an_undefined_variable_is_named_and_nothing_is_sent() {
    let mock = mock(ResponseTemplate::new(200)).await;
    let server = start(&get(&format!("{}/{{{{nope}}}}", mock.uri()), "")).await;
    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(message.contains("nope"), "{message}");
    assert!(mock.received_requests().await.expect("recorded").is_empty());
}

#[tokio::test]
async fn a_secret_read_only_by_an_assertion_is_scrubbed_from_the_echoed_body() {
    let mock =
        mock(ResponseTemplate::new(200).set_body_json(json!({"token": "assert-Only-77"}))).await;
    let request = format!(
        "{}runtime:\n  assertions:\n    - expression: res.body.token\n      operator: eq\n      value: \"{{{{token}}}}\"\n",
        get(&mock.uri(), "")
    );
    let server = start(&request).await;
    server.set_secret("dev", "token", "assert-Only-77");
    let result = server
        .call(
            "send_request",
            json!({"path": "r.yml", "environment": "dev"}),
        )
        .await;
    let shown = serde_json::to_string(&result).expect("serialises");
    assert!(!shown.contains("assert-Only-77"), "{shown}");
    let body = &result.structured_content.expect("structured")["body"]["text"];
    assert_eq!(body, "{\"token\":\"<secret>\"}");
}

#[tokio::test]
async fn basic_credentials_written_in_the_url_are_scrubbed_from_an_echo() {
    // The server echoes the Authorization header reqwest builds from the URL's userinfo.
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(|req: &wiremock::Request| {
            let auth = req
                .headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("none")
                .to_owned();
            ResponseTemplate::new(200).set_body_string(auth)
        })
        .mount(&mock)
        .await;
    let url = mock.uri().replace("http://", "http://u:{{token}}@");
    let server = start(&get(&format!("{url}/"), "")).await;
    server.set_secret("dev", "token", "url-Pass-913");
    let result = server
        .ok(
            "send_request",
            json!({"path": "r.yml", "environment": "dev"}),
        )
        .await;
    assert_eq!(result["body"]["text"], "Basic <secret>", "{result}");
}

#[tokio::test]
async fn a_plaintext_oauth1_signature_is_scrubbed_from_the_timeline() {
    // PLAINTEXT signs with the percent-encoded secrets, which query placement form-encodes again.
    let mock = mock(ResponseTemplate::new(200)).await;
    let auth = "  auth:\n    type: oauth1\n    consumerKey: ck\n    consumerSecret: \"{{token}}\"\n    accessToken: tk\n    accessTokenSecret: 't:s%#y-tok'\n    signatureMethod: PLAINTEXT\n    placement: query\n";
    let server = start(&get(&format!("{}/", mock.uri()), auth)).await;
    server.set_secret("dev", "token", "c s&+/=?x-sec");
    let result = server
        .ok(
            "send_request",
            json!({"path": "r.yml", "environment": "dev"}),
        )
        .await;
    let text = result.to_string();
    assert!(text.contains("oauth_signature="), "{text}");
    for leaked in [
        "c%2520s%2526%252B%252F%253D%253Fx-sec",
        "t%253As%2525%2523y-tok",
    ] {
        assert!(!text.contains(leaked), "{leaked} in {text}");
    }
}

#[tokio::test]
async fn an_x_wsse_header_is_redacted_by_name_in_request_headers() {
    let mock = mock(ResponseTemplate::new(200)).await;
    let extra = "  headers:\n    - name: X-WSSE\n      value: literal-wsse-5531\n";
    let server = start(&get(&format!("{}/", mock.uri()), extra)).await;
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    assert_eq!(header(&result["request_headers"], "x-wsse"), "<redacted>");
}

const MISMATCH_WARNING: &str = "URL query and params disagree on z; the URL's query is sent";

#[tokio::test]
async fn a_url_and_params_query_mismatch_sends_the_urls_query_with_a_warning() {
    let mock = mock(ResponseTemplate::new(200)).await;
    let params = "  params:\n    - name: z\n      value: \"3\"\n      type: query\n";
    let server = start(&get(&format!("{}/m?z=6", mock.uri()), params)).await;
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    let received = mock.received_requests().await.expect("recorded");
    assert_eq!(received[0].url.query(), Some("z=6"));
    assert!(
        result["warnings"]
            .as_array()
            .expect("warnings")
            .contains(&json!(MISMATCH_WARNING)),
        "{result}"
    );
}

#[tokio::test]
async fn an_agreeing_send_has_no_query_mismatch_warning() {
    let mock = mock(ResponseTemplate::new(200)).await;
    let params = "  params:\n    - name: z\n      value: \"6\"\n      type: query\n";
    let server = start(&get(&format!("{}/m?z=6", mock.uri()), params)).await;
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    assert!(
        !result["warnings"].to_string().contains("disagree"),
        "{result}"
    );
}

#[tokio::test]
async fn send_requests_description_names_the_query_mismatch_warning() {
    let server = start(&get("http://localhost/", "")).await;
    let tools = server.tools().await;
    let tool = tools
        .iter()
        .find(|t| t.name == "send_request")
        .expect("send_request");
    assert!(
        tool.description
            .as_deref()
            .unwrap_or_default()
            .contains("query mismatch")
    );
}

#[tokio::test]
async fn a_zstd_reply_comes_back_decoded() {
    // `hello zstd` compressed by the `zstd` command line tool.
    let wire = [
        0x28, 0xb5, 0x2f, 0xfd, 0x04, 0x68, 0x51, 0x00, 0x00, 0x68, 0x65, 0x6c, 0x6c, 0x6f, 0x20,
        0x7a, 0x73, 0x74, 0x64, 0xcf, 0xdb, 0x60, 0x9c,
    ];
    let mock = mock(
        ResponseTemplate::new(200)
            .insert_header("content-encoding", "zstd")
            .set_body_raw(wire.to_vec(), "text/plain"),
    )
    .await;
    let server = start(&get(&format!("{}/zst", mock.uri()), "")).await;
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    assert_eq!(result["body"]["text"], "hello zstd");
}

#[tokio::test]
async fn a_url_username_is_absent_from_the_result_and_its_timeline() {
    // A username alone can be a token, so neither the request url nor a redirect's shows it.
    let mock = MockServer::start().await;
    let spliced = mock.uri().replace("http://", "http://acct-7781@");
    Mock::given(wiremock::matchers::path("/start"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", format!("{spliced}/next")),
        )
        .mount(&mock)
        .await;
    Mock::given(wiremock::matchers::path("/next"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;
    let server = start(&get(&format!("{spliced}/start"), "")).await;
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    assert_eq!(result["status"], 200, "{result}");
    assert!(!result.to_string().contains("acct-7781"), "{result}");
}

#[tokio::test]
async fn path_params_dropping_url_credentials_are_reported_in_warnings() {
    let mock = mock(ResponseTemplate::new(200)).await;
    let url = mock
        .uri()
        .replacen("http://", "http://acct-7781:pw-55xq@", 1);
    let params = "  params:\n    - name: id\n      value: \"7\"\n      type: path\n";
    let server = start(&get(&format!("{url}/users/:id"), params)).await;
    let whole = server.call("send_request", json!({"path": "r.yml"})).await;
    let result = whole.structured_content.clone().unwrap_or_default();
    assert_eq!(result["status"], 200, "{result}");
    let warnings = result["warnings"].as_array().expect("warnings");
    assert!(
        warnings.iter().any(
            |w| w == "credentials in the URL were not sent because the request has path params"
        ),
        "{warnings:?}"
    );
    let shown = serde_json::to_string(&whole).expect("serialises");
    for secret in ["acct-7781", "pw-55xq"] {
        assert!(!shown.contains(secret), "{shown}");
    }
}

#[tokio::test]
async fn the_send_request_description_covers_the_memory_jar_and_dropped_credentials() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let tools = server.tools().await;
    let tool = tools
        .iter()
        .find(|t| t.name == "send_request")
        .expect("listed");
    let description = tool.description.as_deref().unwrap_or_default();
    for phrase in [
        "The server's cookie jar is held in memory only: it lasts for the server process and is \
         never loaded from or saved to disk.",
        "`warnings` also reports when credentials in the URL were not sent because the request \
         has path params.",
    ] {
        assert!(description.contains(phrase), "{phrase}: {description}");
    }
}
