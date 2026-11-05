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

fn wide(value: &str, big: bool) -> Vec<u8> {
    value
        .encode_utf16()
        .flat_map(|u| {
            if big {
                u.to_be_bytes()
            } else {
                u.to_le_bytes()
            }
        })
        .collect()
}

fn holds(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// Echoes the X-Key header back as UTF-16LE then UTF-16BE.
struct WideEcho;
impl Respond for WideEcho {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let key = req
            .headers
            .get("x-key")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let body = [wide(key, false), wide(key, true)].concat();
        ResponseTemplate::new(200).set_body_raw(body, "application/octet-stream")
    }
}

/// A POST sending the `apikey` secret in a header and, as UTF-16, in its file body.
async fn wide_server(url: &str) -> TestServer {
    let request = format!(
        "info:\n  name: wide\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: \"{url}/wide\"\n  headers:\n    - name: X-Key\n      value: \"{{{{apikey}}}}\"\n  body:\n    type: file\n    data:\n      - filePath: \"wide.bin\"\n        contentType: application/octet-stream\n        selected: true\n"
    );
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("wide.yml", &request),
    ])
    .await;
    let sent = [wide(API_KEY, false), wide(API_KEY, true)].concat();
    std::fs::write(server.root().join("wide.bin"), sent).unwrap();
    server.set_secret("dev", "apikey", API_KEY);
    server
}

#[tokio::test]
async fn a_utf16_secret_in_either_body_is_redacted() {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(WideEcho)
        .mount(&s)
        .await;
    let server = wide_server(&s.uri()).await;
    server
        .ok(
            "send_request",
            json!({"path": "wide.yml", "environment": "dev"}),
        )
        .await;
    // Asked for more than the default budget, so the binary body comes back whole.
    let exported = server
        .ok(
            "export_har",
            json!({"path": "wide.yml", "max_body_bytes": 65536}),
        )
        .await;
    let doc: Value = serde_json::from_str(exported["har"].as_str().unwrap()).unwrap();
    let e = &doc["log"]["entries"][0];
    let body = base64::engine::general_purpose::STANDARD
        .decode(e["response"]["content"]["text"].as_str().expect("the body"))
        .unwrap();
    let posted = e["request"]["postData"]["text"].as_str().expect("posted");
    for big in [false, true] {
        let leak = wide(API_KEY, big);
        assert!(!holds(&body, &leak), "response body, big endian {big}");
        assert!(
            !holds(posted.as_bytes(), &leak),
            "request body, big endian {big}"
        );
        assert!(holds(&body, &wide("[redacted]", big)), "big endian {big}");
    }
}

#[tokio::test]
async fn bodies_are_cut_at_max_body_bytes_and_binary_gives_type_and_size() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/big"))
        .respond_with(ResponseTemplate::new(200).set_body_string("x".repeat(40 * 1024)))
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/bin"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(vec![0x89, 0xff, 0x00, 0x01], "image/png"),
        )
        .mount(&s)
        .await;
    let get = |name: &str| {
        format!(
            "info:\n  name: {name}\n  type: http\nhttp:\n  method: GET\n  url: \"{}/{name}\"\n",
            s.uri()
        )
    };
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("big.yml", &get("big")),
        ("bin.yml", &get("bin")),
    ])
    .await;
    let export = |item: &'static str, args: Value| {
        let server = &server;
        async move {
            server.ok("send_request", json!({"path": item})).await;
            let out = server.ok("export_har", args).await;
            let doc: Value = serde_json::from_str(out["har"].as_str().unwrap()).unwrap();
            doc["log"]["entries"][0].clone()
        }
    };
    let e = export("big.yml", json!({"path": "big.yml"})).await;
    let content = &e["response"]["content"];
    assert_eq!(
        content["text"].as_str().unwrap().len(),
        32 * 1024,
        "the default budget"
    );
    assert_eq!(content["size"], 40 * 1024);
    assert!(e["comment"].as_str().unwrap().contains("cut"), "{e}");
    let e = export("big.yml", json!({"path": "big.yml", "max_body_bytes": 100})).await;
    assert_eq!(
        e["response"]["content"]["text"].as_str().unwrap().len(),
        100
    );

    let e = export("bin.yml", json!({"path": "bin.yml"})).await;
    let content = &e["response"]["content"];
    assert!(content.get("text").is_none(), "{e}");
    assert_eq!(content["size"], 4);
    assert_eq!(content["mimeType"], "image/png");
    let e = export(
        "bin.yml",
        json!({"path": "bin.yml", "max_body_bytes": 65536}),
    )
    .await;
    assert_eq!(e["response"]["content"]["encoding"], "base64");
}
