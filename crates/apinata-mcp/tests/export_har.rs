//! `export_har`: the last exchange `send_request` made for a request, as a HAR 1.2 log with
//! secret values, credential headers and derived credentials replaced by `[redacted]`.

use crate::common;

use base64::Engine as _;
use common::{ROOT, TestServer};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const API_KEY: &str = "mcp-har-Key-5521";
const PASSWORD: &str = "mcp-har-Pass-8830";
const TOKEN: &str = "mcp-har-tok-7c19-SECRET";
const CLIENT_SECRET: &str = "mcp-har-cs-41aa-SECRET";
const DEV: &str =
    "name: dev\nvariables:\n  - name: apikey\n    secret: true\n  - name: pw\n    secret: true\n";

/// Echoes what a careless API might: the Authorization and X-Key headers and the URL.
struct Echo;
impl Respond for Echo {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let h = |n: &str| {
            req.headers
                .get(n)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_owned()
        };
        ResponseTemplate::new(200).set_body_string(format!(
            "{} | {} | {}",
            h("authorization"),
            h("x-key"),
            req.url
        ))
    }
}

async fn upstream() -> MockServer {
    let s = MockServer::start().await;
    let token = json!({"access_token": TOKEN, "token_type": "Bearer", "expires_in": 3600});
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(token))
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .respond_with(Echo)
        .mount(&s)
        .await;
    s
}

async fn start(url: &str) -> TestServer {
    let basic = format!(
        "info:\n  name: basic\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}/echo?key={{{{apikey}}}}\"\n  headers:\n    - name: X-Key\n      value: \"{{{{apikey}}}}\"\n  auth:\n    type: basic\n    username: bob\n    password: \"{{{{pw}}}}\"\n"
    );
    let oauth = format!(
        "info:\n  name: oauth\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: \"{url}/r\"\n  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{url}/token\"\n    credentials:\n      clientId: app\n      clientSecret: {CLIENT_SECRET}\n    scope: read\n    tokenConfig:\n      id: main\n      placement:\n        header: Bearer\n"
    );
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("basic.yml", &basic),
        ("oauth.yml", &oauth),
    ])
    .await;
    server.set_secret("dev", "apikey", API_KEY);
    server.set_secret("dev", "pw", PASSWORD);
    server
}

fn header<'a>(list: &'a Value, name: &str) -> &'a str {
    list.as_array()
        .expect("headers")
        .iter()
        .find(|h| {
            h["name"]
                .as_str()
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
        .and_then(|h| h["value"].as_str())
        .unwrap_or_else(|| panic!("no {name} header in {list}"))
}

/// Sends `item` under `dev`, then exports it and returns the parsed HAR and its text.
async fn sent_and_exported(server: &TestServer, item: &str) -> (Value, String) {
    server
        .ok("send_request", json!({"path": item, "environment": "dev"}))
        .await;
    let exported = server.ok("export_har", json!({"path": item})).await;
    assert_eq!(exported["path"], item);
    let text = exported["har"].as_str().expect("har text").to_owned();
    let basic_b64 = base64::engine::general_purpose::STANDARD.encode(format!("bob:{PASSWORD}"));
    for leak in [API_KEY, PASSWORD, TOKEN, CLIENT_SECRET, basic_b64.as_str()] {
        assert!(!text.contains(leak), "{leak} leaked: {text}");
    }
    (serde_json::from_str(&text).expect("JSON"), text)
}

#[tokio::test]
async fn a_basic_auth_send_exports_as_sent_and_redacted() {
    let s = upstream().await;
    let server = start(&s.uri()).await;
    let (doc, text) = sent_and_exported(&server, "basic.yml").await;
    assert!(!text.contains("{{apikey}}"), "written as sent: {text}");
    assert_eq!(doc["log"]["version"], "1.2");
    let e = &doc["log"]["entries"][0];
    assert_eq!(
        e["request"]["url"],
        format!("{}/echo?key=[redacted]", s.uri())
    );
    assert_eq!(header(&e["request"]["headers"], "x-key"), "[redacted]");
    assert_eq!(
        header(&e["request"]["headers"], "authorization"),
        "[redacted]"
    );
    assert_eq!(e["response"]["status"], 200);
    assert_eq!(e["response"]["statusText"], "OK");
}

#[tokio::test]
async fn an_oauth2_token_echoed_back_is_redacted() {
    let s = upstream().await;
    let server = start(&s.uri()).await;
    let (doc, _) = sent_and_exported(&server, "oauth.yml").await;
    let e = &doc["log"]["entries"][0];
    assert_eq!(
        header(&e["request"]["headers"], "authorization"),
        "[redacted]"
    );
    let body = e["response"]["content"]["text"].as_str().expect("text");
    assert!(body.starts_with("Bearer [redacted]"), "{body}");
}

#[tokio::test]
async fn exporting_without_a_prior_send_is_an_error() {
    let s = upstream().await;
    let server = start(&s.uri()).await;
    let message = server.err("export_har", json!({"path": "basic.yml"})).await;
    assert!(message.contains("send the request first"), "{message}");
}
