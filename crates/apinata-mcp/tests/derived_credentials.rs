//! Derived credentials never appear in a send_request or run_folder result.

use crate::common;

use common::{ROOT, TestServer};
use serde_json::json;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

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
            h("x-wsse"),
            req.url
        ))
    }
}

async fn echo() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(any()).respond_with(Echo).mount(&s).await;
    s
}

fn req(name: &str, seq: u32, url: &str, extra: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{url}\"\n{extra}"
    )
}

const OAUTH1_QUERY: &str = "  auth:\n    type: oauth1\n    consumerKey: ck\n    consumerSecret: cs-1\n    accessToken: tk\n    accessTokenSecret: ts-1\n    signatureMethod: HMAC-SHA1\n    placement: query\n";
const WSSE: &str = "  auth:\n    type: wsse\n    username: bob\n    password: wsse-pass-1\n";

/// The `oauth_signature` the server received, decoded.
async fn received_signature(s: &MockServer) -> String {
    let seen = s.received_requests().await.expect("recording");
    seen[0]
        .url
        .query_pairs()
        .find(|(k, _)| k == "oauth_signature")
        .map(|(_, v)| v.into_owned())
        .expect("signature in query")
}

fn forms(value: &str) -> Vec<String> {
    let form: String = url::form_urlencoded::byte_serialize(value.as_bytes()).collect();
    vec![value.to_owned(), form]
}

#[tokio::test]
async fn an_oauth1_query_signature_is_absent_from_the_send_result() {
    let s = echo().await;
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &req("r", 1, &format!("{}/v", s.uri()), OAUTH1_QUERY),
        ),
    ])
    .await;
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    let text = result.to_string();
    assert!(text.contains("oauth_signature="), "{text}");
    for form in forms(&received_signature(&s).await) {
        assert!(!text.contains(&form), "{form} in {text}");
    }
}

const PRINT: &str = "console.log(res.getBody());";

#[tokio::test]
async fn an_echoed_wsse_header_is_absent_from_body_console_and_run_assertions() {
    let s = echo().await;
    let fail = "runtime:\n  assertions:\n    - expression: res.body\n      operator: eq\n      value: \"nope\"\n";
    let script = format!(
        "runtime:\n  scripts:\n    - type: after-response\n      code: |-\n        {PRINT}\n"
    );
    let r = req(
        "r",
        1,
        &format!("{}/v", s.uri()),
        &format!("{WSSE}{script}"),
    );
    let q = req("q", 2, &format!("{}/v", s.uri()), &format!("{WSSE}{fail}"));
    let server =
        TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", &r), ("q.yml", &q)]).await;
    server.approve_scripts(&[PRINT]);
    let sent = server.ok("send_request", json!({"path": "r.yml"})).await;
    let seen = s.received_requests().await.expect("recording");
    let wsse = seen[0]
        .headers
        .get("x-wsse")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert!(wsse.starts_with("UsernameToken "));
    assert!(!sent.to_string().contains(&wsse), "{sent}");
    assert!(
        sent["console"][0]["text"]
            .as_str()
            .unwrap()
            .contains("[redacted]")
    );
    let run = server.ok("run_folder", json!({"path": ""})).await;
    let leaked: Vec<String> = s
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter_map(|r| {
            r.headers
                .get("x-wsse")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        })
        .collect();
    assert!(leaked.len() >= 3);
    for value in leaked {
        assert!(!run.to_string().contains(&value), "{value} in {run}");
    }
}

use wiremock::matchers::{body_string_contains, method, path};

/// Echo, with a digest challenge while no credential header is present.
struct Challenge;
impl Respond for Challenge {
    fn respond(&self, r: &Request) -> ResponseTemplate {
        if r.headers.get("authorization").is_none() && r.headers.get("x-wsse").is_none() {
            return ResponseTemplate::new(401).insert_header(
                "www-authenticate",
                r#"Digest realm="r", nonce="n0", qop="auth", opaque="op""#,
            );
        }
        Echo.respond(r)
    }
}

const OAUTH1_HEADER: &str = "  auth:\n    type: oauth1\n    consumerKey: ck\n    consumerSecret: cs-1\n    accessToken: tk\n    accessTokenSecret: ts-1\n    signatureMethod: HMAC-SHA1\n    placement: header\n";
const SIGV4: &str = "  auth:\n    type: awsv4\n    accessKeyId: AKIDTEST\n    secretAccessKey: secret-key\n    sessionToken: \"\"\n    service: execute-api\n    region: eu-west-1\n    profileName: \"\"\n";
const DIGEST: &str = "  auth:\n    type: digest\n    username: bob\n    password: digest-pass-1\n";
const EDGEGRID: &str = "  auth:\n    type: akamai-edgegrid\n    accessToken: akab-access-1\n    clientToken: akab-client-1\n    clientSecret: eg-secret-1\n    headersToSign: \"content-type,user-agent,accept\"\n";
const TOKEN: &str = "tok-4f1c-SECRET";

/// What the server received that the executor derived. OAuth 1.0 in a header derives only the
/// signature, so only the decoded `oauth_signature` is checked for it; every other mode derives
/// whole header values, so those and their 16+ char pieces (split at spaces, commas, quotes
/// and `=`) are checked, plus the decoded `oauth_signature` and `access` query values.
async fn received_credentials(s: &MockServer, mode: &str) -> Vec<String> {
    let mut out = Vec::new();
    for r in s.received_requests().await.expect("recording") {
        for name in ["authorization", "x-wsse"] {
            let Some(v) = r.headers.get(name).and_then(|v| v.to_str().ok()) else {
                continue;
            };
            if mode == "oauth1 header" {
                let signature = v
                    .split(", ")
                    .find_map(|p| p.strip_prefix("oauth_signature=\""))
                    .map(|p| p.trim_end_matches('"'));
                if let Some(signature) = signature {
                    out.push(
                        percent_encoding::percent_decode_str(signature)
                            .decode_utf8_lossy()
                            .into_owned(),
                    );
                }
                continue;
            }
            out.push(v.to_owned());
            out.extend(
                v.split([' ', ',', '"', '='])
                    .filter(|p| p.len() >= 16)
                    .map(str::to_owned),
            );
        }
        for (k, v) in r.url.query_pairs() {
            if k == "oauth_signature" || k == "access" {
                out.push(v.into_owned());
            }
        }
    }
    out
}

async fn provider() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=client_credentials"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"access_token": TOKEN, "token_type": "Bearer", "expires_in": 3600}),
        ))
        .mount(&s)
        .await;
    Mock::given(path("/v"))
        .respond_with(Challenge)
        .mount(&s)
        .await;
    s
}

fn oauth2(s: &MockServer, placement: &str) -> String {
    format!(
        "  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{}/token\"\n    credentials:\n      clientId: app\n      clientSecret: cs-77aa-SECRET\n      placement: body\n    scope: read\n    tokenConfig:\n      id: main\n      placement:\n        {placement}\n",
        s.uri()
    )
}

#[tokio::test]
async fn no_mode_leaves_a_received_credential_in_the_send_result() {
    for mode in [
        "oauth1 header",
        "oauth1 query",
        "sigv4",
        "wsse",
        "edgegrid",
        "digest",
        "oauth2 header",
        "oauth2 query",
    ] {
        let s = provider().await;
        let auth = match mode {
            "oauth1 header" => OAUTH1_HEADER.to_owned(),
            "oauth1 query" => OAUTH1_QUERY.to_owned(),
            "sigv4" => SIGV4.to_owned(),
            "wsse" => WSSE.to_owned(),
            "edgegrid" => EDGEGRID.to_owned(),
            "digest" => DIGEST.to_owned(),
            "oauth2 header" => oauth2(&s, "header: Bearer"),
            _ => oauth2(&s, "query: access"),
        };
        let server = TestServer::start(&[
            ("opencollection.yml", ROOT),
            ("r.yml", &req("r", 1, &format!("{}/v", s.uri()), &auth)),
        ])
        .await;
        let result = server.ok("send_request", json!({"path": "r.yml"})).await;
        let text = result.to_string();
        let received = received_credentials(&s, mode).await;
        assert!(!received.is_empty(), "{mode}: the server saw no credential");
        for value in received {
            for form in forms(&value) {
                assert!(!text.contains(&form), "{mode}: {form} in {text}");
            }
        }
    }
}

#[tokio::test]
async fn ntlm_values_are_absent_from_the_send_result() {
    use crate::ntlm_support::{Options, PASSWORD, USER, ntlm_server};
    let s = ntlm_server(Options {
        echo_auth: true,
        ..Options::default()
    })
    .await;
    let auth = format!(
        "  auth:\n    type: ntlm\n    username: {USER}\n    password: {PASSWORD}\n    domain: CORP\n"
    );
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &req("r", 1, &format!("{}/ok", s.origin()), &auth)),
    ])
    .await;
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    let text = result.to_string();
    let sent: Vec<String> = s.seen().into_iter().filter_map(|r| r.auth).collect();
    assert!(sent.iter().any(|a| a.starts_with("NTLM ")), "{sent:?}");
    for auth in sent {
        for piece in auth.split(' ').filter(|p| p.len() >= 16) {
            for form in forms(piece) {
                assert!(!text.contains(&form), "{form} in {text}");
            }
        }
    }
}
