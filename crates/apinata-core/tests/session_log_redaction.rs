//! No secret reaches a published entry of a direct send, in any field or wire form.

use crate::common;
use crate::session_log_support;

use apinata_core::{Collection, ContextOptions, ExecOptions, ResolvedRequest, VarContext};
use common::{at, resolved_one, setup, write_tree};
use session_log_support::{assert_absent, entries, header, logged, only, texts};
use wiremock::matchers::{any, header_exists, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";
const SECRET: &str = "S3cr/et val-77";
/// Every signer secret below, which the echo server repeats in its body and a header.
const SIGNER_SECRETS: [&str; 8] = [
    "dg-pass-5521",
    "ws-pass-2210",
    "cs-oauth-3391",
    "ts-oauth-7713",
    "tk-oauth-1180",
    "eg-secret-6612",
    "akab-access-9",
    "akab-client-9",
];
const DIGEST: &str =
    "  auth:\n    type: digest\n    username: \"u\"\n    password: \"dg-pass-5521\"\n";
const WSSE: &str = "  auth:\n    type: wsse\n    username: bob\n    password: ws-pass-2210\n";
const EDGEGRID: &str = "  auth:\n    type: akamai-edgegrid\n    accessToken: akab-access-9\n    clientToken: akab-client-9\n    clientSecret: eg-secret-6612\n    headersToSign: \"user-agent\"\n    nonce: n0nce\n    timestamp: 20140321T19:34:21+0000\n";

fn oauth1(placement: &str) -> String {
    format!(
        "  auth:\n    type: oauth1\n    consumerKey: ck\n    consumerSecret: cs-oauth-3391\n    accessToken: tk-oauth-1180\n    accessTokenSecret: ts-oauth-7713\n    signatureMethod: HMAC-SHA1\n    placement: {placement}\n"
    )
}

/// `r.yml` with `under_http` under `http:`, `tok` = `secret` given as a sensitive override.
fn resolved_secret(method: &str, url: &str, under_http: &str, secret: &str) -> ResolvedRequest {
    let text = format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: {method}\n  url: \"{url}\"\n{under_http}"
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    let c = Collection::open(dir.path()).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new().sensitive_env_var("tok", secret))
        .expect("context");
    at(&c, &ctx, "r.yml").expect("resolves")
}

/// Answers every request 200, repeating `text` in an `x-echo` header and a JSON body.
async fn echo(text: &str) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-echo", text)
                .set_body_string(format!("{{\"echo\":\"{text}\"}}")),
        )
        .mount(&s)
        .await;
    s
}

/// Fails unless every `oauth_signature=` in `text` is followed by the placeholder.
fn signature_hidden(text: &str) {
    for (i, _) in text.match_indices("oauth_signature=") {
        assert!(
            text[i + "oauth_signature=".len()..].starts_with("[redacted]"),
            "{text}"
        );
    }
}

#[tokio::test]
async fn a_secret_in_the_query_a_header_and_a_json_body_is_redacted() {
    let s = echo(SECRET).await;
    let under = "  headers:\n    - name: X-Key\n      value: \"{{tok}}\"\n  body:\n    type: json\n    data: '{\"k\":\"{{tok}}\"}'\n";
    let (exec, log) = logged(ExecOptions::new());
    exec.send(&resolved_secret(
        "POST",
        &format!("{}/q?key={{{{tok}}}}", s.uri()),
        under,
        SECRET,
    ))
    .await
    .expect("sent");
    assert_absent(&log, SECRET);
    let e = only(&log);
    assert!(e.url.contains("key=[redacted]"), "{}", e.url);
    assert_eq!(header(&e.request_headers, "x-key"), Some("[redacted]"));
    assert!(
        e.request_body.text.contains("[redacted]") && e.response_body.text.contains("[redacted]")
    );
    assert_eq!(
        header(&e.hops[0].response_headers, "x-echo"),
        Some("[redacted]")
    );
}

#[tokio::test]
async fn a_secret_with_a_space_and_a_plus_in_the_query_and_path_is_redacted() {
    let secret = "Sp ace+plus/77";
    let s = echo("ok").await;
    let (exec, log) = logged(ExecOptions::new());
    let url = format!("{}/p/{{{{tok}}}}?key={{{{tok}}}}", s.uri());
    exec.send(&resolved_secret("GET", &url, "", secret))
        .await
        .expect("sent");
    assert_absent(&log, secret);
    let e = only(&log);
    for text in [&e.url, &e.hops[0].url] {
        assert!(
            !text.contains("ace+plus") && !text.contains("Sp%20ace"),
            "{text}"
        );
    }
}

#[tokio::test]
async fn basic_auth_and_cookies_are_redacted() {
    let s = MockServer::start().await;
    Mock::given(path("/b"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("set-cookie", "sid=cookie-77; Path=/")
                .set_body_string("pw was pw-basic-4471"),
        )
        .mount(&s)
        .await;
    let auth = "  auth:\n    type: basic\n    username: u\n    password: pw-basic-4471\n";
    let req = resolved_one("GET", &format!("{}/b", s.uri()), auth, "");
    let (exec, log) = logged(ExecOptions::new());
    exec.send(&req).await.expect("first");
    exec.send(&req).await.expect("second carries the cookie");
    assert_absent(&log, "pw-basic-4471");
    assert_absent(&log, "cookie-77");
    let all = entries(&log);
    assert_eq!(
        header(&all[0].request_headers, "authorization"),
        Some("[redacted]")
    );
    assert_eq!(
        header(&all[0].hops[0].response_headers, "set-cookie"),
        Some("[redacted]")
    );
    assert_eq!(
        header(&all[1].hops[0].request_headers, "cookie"),
        Some("[redacted]")
    );
}

#[tokio::test]
async fn a_bearer_token_echoed_in_another_case_is_redacted() {
    let s = echo("BEARER-TOK-6631").await;
    let auth = "  auth:\n    type: bearer\n    token: bearer-tok-6631\n";
    let (exec, log) = logged(ExecOptions::new());
    exec.send(&resolved_one("GET", &format!("{}/t", s.uri()), auth, ""))
        .await
        .expect("sent");
    assert_absent(&log, "bearer-tok-6631");
    assert_eq!(
        header(&only(&log).request_headers, "authorization"),
        Some("[redacted]")
    );
}

#[tokio::test]
async fn a_sigv4_session_token_is_redacted() {
    let s = echo("sess-tok-8812").await;
    let auth = "  auth:\n    type: awsv4\n    accessKeyId: AKIDTEST\n    secretAccessKey: secret-key\n    sessionToken: \"sess-tok-8812\"\n    service: execute-api\n    region: eu-west-1\n    profileName: \"\"\n";
    let (exec, log) = logged(ExecOptions::new());
    exec.send(&resolved_one("GET", &format!("{}/v", s.uri()), auth, ""))
        .await
        .expect("sent");
    assert_absent(&log, "sess-tok-8812");
    assert_eq!(
        header(&only(&log).hops[0].request_headers, "x-amz-security-token"),
        Some("[redacted]")
    );
}

#[tokio::test]
async fn a_literal_sigv4_secret_key_is_redacted() {
    let s = echo("sk-literal-4417").await;
    let auth = "  auth:\n    type: awsv4\n    accessKeyId: AKIDTEST\n    secretAccessKey: sk-literal-4417\n    sessionToken: \"\"\n    service: execute-api\n    region: eu-west-1\n    profileName: \"\"\n";
    let (exec, log) = logged(ExecOptions::new());
    exec.send(&resolved_one("GET", &format!("{}/v", s.uri()), auth, ""))
        .await
        .expect("sent");
    assert_absent(&log, "sk-literal-4417");
}

#[tokio::test]
async fn an_inherited_api_key_is_redacted_in_header_and_query_placement() {
    // Each path echoes only the key its own send carries; a send cannot know another's secret.
    let s = MockServer::start().await;
    for (p, key) in [("/h", "ak-root-4410"), ("/r", "ak-folder-5520")] {
        Mock::given(path(p))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-echo", key)
                    .set_body_string(format!("{{\"echo\":\"{key}\"}}")),
            )
            .mount(&s)
            .await;
    }
    let root = format!(
        "{ROOT}request:\n  auth:\n    type: apikey\n    key: X-Api-Key\n    value: ak-root-4410\n    placement: header\n"
    );
    let folder = "info:\n  name: q\n  type: folder\nrequest:\n  auth:\n    type: apikey\n    key: api_key\n    value: ak-folder-5520\n    placement: query\n";
    let req = |name: &str| {
        format!(
            "info:\n  name: {name}\n  type: http\nhttp:\n  method: GET\n  url: \"{}/{name}\"\n  auth: inherit\n",
            s.uri()
        )
    };
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", &root),
        ("q/folder.yml", folder),
        ("h.yml", &req("h")),
        ("q/r.yml", &req("r")),
    ]);
    let (exec, log) = logged(ExecOptions::new());
    for rel in ["h.yml", "q/r.yml"] {
        exec.send(&at(&c, &ctx, rel).expect("resolves"))
            .await
            .expect("sent");
    }
    let all = entries(&log);
    assert_eq!(all.len(), 2);
    assert_eq!(
        header(&all[0].request_headers, "x-api-key"),
        Some("[redacted]")
    );
    assert!(all[1].url.contains("api_key=[redacted]"), "{}", all[1].url);
    assert_absent(&log, "ak-root-4410");
    assert_absent(&log, "ak-folder-5520");
}

#[tokio::test]
async fn a_digest_retry_hop_and_its_echoed_password_are_redacted() {
    let s = MockServer::start().await;
    let echoed = SIGNER_SECRETS.join(" ");
    Mock::given(path("/d"))
        .and(header_exists("authorization"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-echo", echoed.as_str())
                .set_body_string(echoed.clone()),
        )
        .with_priority(1)
        .mount(&s)
        .await;
    Mock::given(path("/d"))
        .respond_with(ResponseTemplate::new(401).insert_header(
            "www-authenticate",
            r#"Digest realm="r", nonce="n0", qop="auth""#,
        ))
        .with_priority(2)
        .mount(&s)
        .await;
    let (exec, log) = logged(ExecOptions::new());
    exec.send(&resolved_one("GET", &format!("{}/d", s.uri()), DIGEST, ""))
        .await
        .expect("sent");
    let e = only(&log);
    assert_eq!(
        e.hops.iter().map(|h| h.status).collect::<Vec<_>>(),
        [401, 200]
    );
    assert_eq!(
        header(&e.hops[1].request_headers, "authorization"),
        Some("[redacted]")
    );
    assert_absent(&log, "dg-pass-5521");
}

#[tokio::test]
async fn signer_secrets_echoed_by_the_server_never_reach_the_log() {
    let echoed = SIGNER_SECRETS.join(" ");
    let s = echo(&echoed).await;
    for auth in [WSSE.to_owned(), oauth1("header"), EDGEGRID.to_owned()] {
        let (exec, log) = logged(ExecOptions::new());
        exec.send(&resolved_one("GET", &format!("{}/e", s.uri()), &auth, ""))
            .await
            .expect("sent");
        let e = only(&log);
        assert_eq!(
            header(&e.hops[0].request_headers, "authorization")
                .or(header(&e.hops[0].request_headers, "x-wsse")),
            Some("[redacted]"),
            "{auth}"
        );
        for secret in SIGNER_SECRETS.iter().filter(|v| auth.contains(*v)) {
            assert_absent(&log, secret);
        }
    }
}

#[tokio::test]
async fn oauth1_in_the_query_and_the_body_hides_its_token_and_signature() {
    let s = echo(&SIGNER_SECRETS.join(" ")).await;
    let form = "  body:\n    type: form-urlencoded\n    data:\n      - name: b\n        value: x\n";
    for (method, under) in [
        ("GET", oauth1("query")),
        ("POST", format!("{}{form}", oauth1("body"))),
    ] {
        let (exec, log) = logged(ExecOptions::new());
        exec.send(&resolved_one(method, &format!("{}/o", s.uri()), &under, ""))
            .await
            .expect("sent");
        let e = only(&log);
        assert!(
            texts(&e).iter().any(|t| t.contains("oauth_signature=")),
            "{method}: the signature is placed somewhere"
        );
        for text in texts(&e) {
            signature_hidden(&text);
        }
        for secret in ["tk-oauth-1180", "cs-oauth-3391", "ts-oauth-7713"] {
            assert_absent(&log, secret);
        }
    }
}

async fn redirecting() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(path("/start"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/end"))
        .mount(&s)
        .await;
    Mock::given(path("/end"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    s
}

#[tokio::test]
async fn oauth1_and_edgegrid_redirect_hops_have_their_authorization_redacted() {
    for auth in [oauth1("header"), EDGEGRID.to_owned()] {
        let s = redirecting().await;
        let (exec, log) = logged(ExecOptions::new());
        exec.send(&resolved_one(
            "GET",
            &format!("{}/start", s.uri()),
            &auth,
            "",
        ))
        .await
        .expect("sent");
        let e = only(&log);
        assert_eq!(e.hops.len(), 2);
        for hop in &e.hops {
            assert_eq!(
                header(&hop.request_headers, "authorization"),
                Some("[redacted]"),
                "{auth}"
            );
        }
    }
}
