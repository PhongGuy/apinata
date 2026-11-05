//! `apinata run --har FILE`: every exchange the run sent, written as one HAR 1.2 log with
//! secret values, credential headers and derived credentials replaced by `[redacted]`.

use crate::common;

use common::{ROOT, apinata, write_tree};
use serde_json::Value;
use std::path::Path;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const API_KEY: &str = "har-Key-5521";
const PASSWORD: &str = "har-Pass-8830";
const TOKEN: &str = "har-tok-7c19-SECRET";
const CLIENT_SECRET: &str = "har-cs-41aa-SECRET";
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

async fn server() -> MockServer {
    let s = MockServer::start().await;
    let token =
        serde_json::json!({"access_token": TOKEN, "token_type": "Bearer", "expires_in": 3600});
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

/// A basic-auth request sending the `apikey` secret in its query and a header, then an
/// OAuth 2.0 request whose token the server echoes back.
fn collection(url: &str) -> TempDir {
    let basic = format!(
        "info:\n  name: basic\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}/echo?key={{{{apikey}}}}\"\n  headers:\n    - name: X-Key\n      value: \"{{{{apikey}}}}\"\n  auth:\n    type: basic\n    username: bob\n    password: \"{{{{pw}}}}\"\n"
    );
    let oauth = format!(
        "info:\n  name: oauth\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: \"{url}/r\"\n  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{url}/token\"\n    credentials:\n      clientId: app\n      clientSecret: {CLIENT_SECRET}\n    scope: read\n    tokenConfig:\n      id: main\n      placement:\n        header: Bearer\n"
    );
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("basic.yml", &basic),
        ("oauth.yml", &oauth),
    ])
}

fn run(dir: &Path, cache: &Path, har: &Path) -> std::process::Output {
    apinata()
        .arg("run")
        .arg(dir)
        .args(["--no-keychain", "--env", "dev", "--env-var"])
        .arg(format!("apikey={API_KEY}"))
        .arg("--env-var")
        .arg(format!("pw={PASSWORD}"))
        .arg("--har")
        .arg(har)
        .env("APINATA_TEST_TOKEN_CACHE", cache)
        .env(
            "APINATA_TEST_TOKEN_KEY",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        )
        .output()
        .expect("runs")
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_har_writes_every_sent_exchange_redacted() {
    let s = server().await;
    let dir = collection(&s.uri());
    let out = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    let file = out.path().join("run.har");
    let ran = run(dir.path(), cache.path(), &file);
    assert_eq!(
        ran.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    let text = std::fs::read_to_string(&file).expect("the HAR file");
    let basic_b64 = base64_of(&format!("bob:{PASSWORD}"));
    for leak in [API_KEY, PASSWORD, TOKEN, CLIENT_SECRET, basic_b64.as_str()] {
        assert!(!text.contains(leak), "{leak} leaked: {text}");
    }
    assert!(!text.contains("{{apikey}}"), "written as sent: {text}");
    let doc: Value = serde_json::from_str(&text).expect("JSON");
    assert_eq!(doc["log"]["version"], "1.2");
    let entries = doc["log"]["entries"].as_array().expect("entries");
    assert_eq!(entries.len(), 2, "{text}");

    let basic = &entries[0];
    assert_eq!(
        basic["request"]["url"],
        format!("{}/echo?key=[redacted]", s.uri())
    );
    assert_eq!(header(&basic["request"]["headers"], "x-key"), "[redacted]");
    assert_eq!(
        header(&basic["request"]["headers"], "authorization"),
        "[redacted]"
    );
    assert_eq!(basic["response"]["status"], 200);
    assert_eq!(basic["response"]["statusText"], "OK");
    let body = basic["response"]["content"]["text"].as_str().expect("text");
    assert!(body.contains("[redacted]"), "{body}");

    let oauth = &entries[1];
    assert_eq!(oauth["request"]["url"], format!("{}/r", s.uri()));
    assert_eq!(
        header(&oauth["request"]["headers"], "authorization"),
        "[redacted]"
    );
    let body = oauth["response"]["content"]["text"].as_str().expect("text");
    assert!(body.starts_with("Bearer [redacted]"), "{body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_har_path_that_cannot_be_written_fails_the_run() {
    let s = server().await;
    let dir = collection(&s.uri());
    let cache = TempDir::new().unwrap();
    let missing = dir.path().join("no-such-dir").join("run.har");
    let ran = run(dir.path(), cache.path(), &missing);
    assert_ne!(ran.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&ran.stderr);
    assert!(stderr.contains("HAR"), "{stderr}");
}

fn base64_of(text: &str) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(text)
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

/// Echoes the X-Key header back as UTF-16LE then UTF-16BE, as a Windows-flavoured API might.
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_utf16_secret_in_either_body_is_redacted() {
    use base64::Engine as _;
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(WideEcho)
        .mount(&s)
        .await;
    let request = format!(
        "info:\n  name: wide\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: \"{}/wide\"\n  headers:\n    - name: X-Key\n      value: \"{{{{apikey}}}}\"\n  body:\n    type: file\n    data:\n      - filePath: \"wide.bin\"\n        contentType: application/octet-stream\n        selected: true\n",
        s.uri()
    );
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("wide.yml", &request),
    ]);
    let sent = [wide(API_KEY, false), wide(API_KEY, true)].concat();
    std::fs::write(dir.path().join("wide.bin"), sent).unwrap();
    let out = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    let file = out.path().join("wide.har");
    let ran = run(dir.path(), cache.path(), &file);
    assert_eq!(
        ran.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    let e = &doc["log"]["entries"][0];
    let content = &e["response"]["content"];
    let body = base64::engine::general_purpose::STANDARD
        .decode(content["text"].as_str().expect("the body"))
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
