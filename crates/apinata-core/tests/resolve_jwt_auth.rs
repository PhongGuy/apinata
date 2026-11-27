//! JWT auth through resolve and send: placement, inheritance, keys, errors and hiding.

use crate::common;
use crate::derived_support::Echo;

use apinata_core::{
    CancelHandle, ExecOptions, Executor, ItemPath, ResolveError, ResolvedRequest, RunOptions,
    Target, generate, send_one,
};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use common::{at, request, setup};
use serde_json::Value;
use std::sync::Arc;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nrequest:\n  variables:\n    - name: user\n      value: alice\n";
const RSA_PKCS1: &str = include_str!("fixtures/jwt/rsa-pkcs1.pem");
const EC256_SEC1: &str = include_str!("fixtures/jwt/ec256-sec1.pem");
const SECRET: &str = "jwt-secret-value-1";

fn http(url: &str, auth: &str) -> String {
    format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\n  auth:\n{auth}"
    )
}

fn hs(extra: &str) -> String {
    format!(
        "    type: jwt\n    algorithm: HS256\n    secret: {SECRET}\n    payload: '{{\"sub\":\"{{{{user}}}}\"}}'\n{extra}"
    )
}

fn resolve_with(files: &[(&str, &str)]) -> Result<ResolvedRequest, ResolveError> {
    let (_d, c, ctx) = setup(files);
    at(&c, &ctx, "r.yml")
}

fn one(auth: &str) -> Result<ResolvedRequest, ResolveError> {
    let r = http("https://x.test/a?keep=1", auth);
    resolve_with(&[("opencollection.yml", ROOT), ("r.yml", &r)])
}

fn authorization(r: &ResolvedRequest) -> Option<String> {
    r.headers()
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("authorization"))
        .map(|(_, v)| v.clone())
}

/// The decoded header and payload of a token.
fn parts(token: &str) -> (Value, Value) {
    let mut it = token.split('.');
    let mut part = || -> Value {
        let bytes = URL_SAFE_NO_PAD.decode(it.next().unwrap()).unwrap();
        serde_json::from_slice(&bytes).unwrap()
    };
    let header = part();
    (header, part())
}

fn reason(auth: &str) -> String {
    match one(auth) {
        Err(ResolveError::Jwt { reason }) => reason,
        other => panic!("expected a JWT error: {other:?}"),
    }
}

#[test]
fn the_header_carries_the_prefix_and_a_token_over_the_filled_payload() {
    let r = one(&hs("")).unwrap();
    let value = authorization(&r).unwrap();
    let token = value.strip_prefix("Bearer ").expect("default prefix");
    let (header, payload) = parts(token);
    assert_eq!(header.to_string(), "{\"alg\":\"HS256\"}");
    assert_eq!(payload.to_string(), "{\"sub\":\"alice\"}");
    let custom = one(&hs("    headerPrefix: JWT\n")).unwrap();
    assert!(authorization(&custom).unwrap().starts_with("JWT ey"));
}

#[test]
fn an_empty_prefix_sends_the_bare_token() {
    let r = one(&hs("    headerPrefix: \"\"\n")).unwrap();
    let value = authorization(&r).unwrap();
    assert!(value.starts_with("ey") && !value.contains(' '), "{value}");
}

#[test]
fn query_placement_replaces_a_parameter_of_the_same_name() {
    let r = http(
        "https://x.test/a?jwt=old&keep=1",
        &hs("    placement: query\n    queryParamKey: jwt\n"),
    );
    let r = resolve_with(&[("opencollection.yml", ROOT), ("r.yml", &r)]).unwrap();
    assert_eq!(authorization(&r), None);
    let url = url::Url::parse(r.url()).unwrap();
    let pairs: Vec<(String, String)> = url.query_pairs().into_owned().collect();
    assert_eq!(pairs.len(), 2, "{pairs:?}");
    assert_eq!(pairs[0].0, "jwt");
    assert!(pairs[0].1.starts_with("ey"));
    let default = one(&hs("    placement: query\n")).unwrap();
    assert!(default.url().contains("token=ey"), "{}", default.url());
}

#[test]
fn a_request_inherits_jwt_from_its_folder() {
    let folder = format!(
        "info:\n  name: f\n  type: folder\nrequest:\n  auth:\n{}",
        hs("")
    );
    let r = "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: https://x.test/\n  auth: inherit\n";
    let (_d, c, ctx) = setup(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder),
        ("f/r.yml", r),
    ]);
    let got = at(&c, &ctx, "f/r.yml").unwrap();
    assert!(authorization(&got).unwrap().starts_with("Bearer ey"));
}

#[test]
fn an_undefined_variable_fails_as_undefined() {
    for auth in [
        "    type: jwt\n    secret: s\n    payload: '{\"a\":\"{{nope}}\"}'\n",
        "    type: jwt\n    secret: \"{{nope}}\"\n",
        "    type: jwt\n    algorithm: RS256\n    privateKey:\n      type: text\n      value: \"{{nope}}\"\n",
    ] {
        let got = one(auth);
        assert!(
            matches!(&got, Err(ResolveError::UndefinedVariable { name, .. }) if name == "nope"),
            "{auth}: {got:?}"
        );
    }
}

#[test]
fn keys_load_from_pkcs1_files_and_sec1_text() {
    let file = http(
        "https://x.test/",
        "    type: jwt\n    algorithm: RS256\n    privateKey:\n      type: file\n      value: keys/k.pem\n",
    );
    let r = resolve_with(&[
        ("opencollection.yml", ROOT),
        ("keys/k.pem", RSA_PKCS1),
        ("r.yml", &file),
    ])
    .unwrap();
    let (header, _) = parts(authorization(&r).unwrap().strip_prefix("Bearer ").unwrap());
    assert_eq!(header["alg"], "RS256");
    let escaped = EC256_SEC1.trim().replace('\n', "\\n");
    let text = format!(
        "    type: jwt\n    algorithm: ES256\n    privateKey:\n      type: text\n      value: '{escaped}'\n"
    );
    assert!(authorization(&one(&text).unwrap()).is_some());
}

#[test]
fn bad_keys_payloads_and_names_are_errors_that_echo_nothing() {
    let key = |alg: &str| {
        let escaped = EC256_SEC1.trim().replace('\n', "\\n");
        format!(
            "    type: jwt\n    algorithm: {alg}\n    privateKey:\n      type: text\n      value: '{escaped}'\n"
        )
    };
    let cases = [
        (key("RS256"), "does not match RS256"),
        (key("ES512"), "does not match ES512"),
        (
            "    type: jwt\n    algorithm: RS256\n".to_owned(),
            "needs a private key",
        ),
        (
            "    type: jwt\n    secret: \"\"\n".to_owned(),
            "needs a secret",
        ),
        (
            "    type: jwt\n    algorithm: none\n    secret: s\n".to_owned(),
            "unknown algorithm",
        ),
        (
            format!("    type: jwt\n    secret: {SECRET}\n    payload: '[\"leak-claim\"]'\n"),
            "payload is not a JSON object",
        ),
        (
            format!("    type: jwt\n    secret: {SECRET}\n    placement: cookie\n"),
            "unknown placement",
        ),
        (
            format!("    type: jwt\n    secret: {SECRET}\n    expiresIn: soon\n"),
            "expiresIn",
        ),
        (
            "    type: jwt\n    secret: \"%%%\"\n    secretBase64: true\n".to_owned(),
            "base64",
        ),
    ];
    for (auth, want) in cases {
        let got = reason(&auth);
        assert!(got.contains(want), "{want}: {got}");
        let shown = ResolveError::Jwt { reason: got }.to_string();
        assert!(shown.starts_with("JWT: "), "{shown}");
        for leak in [SECRET, "leak-claim", "BEGIN", "%%%"] {
            assert!(!shown.contains(leak), "{shown}");
        }
    }
}

#[test]
fn the_secret_key_and_token_are_sensitive_values() {
    let r = one(&hs("")).unwrap();
    let token = authorization(&r).unwrap().replace("Bearer ", "");
    assert!(r.sensitive_values().iter().any(|v| v == SECRET));
    assert!(r.sensitive_values().contains(&token));
    let escaped = EC256_SEC1.trim().replace('\n', "\\n");
    let text = format!(
        "    type: jwt\n    algorithm: ES256\n    privateKey:\n      type: text\n      value: '{escaped}'\n"
    );
    let r = one(&text).unwrap();
    assert!(r.sensitive_values().contains(&escaped));
}

#[test]
fn a_snippet_carries_the_token_as_a_sensitive_value() {
    let r = one(&hs("")).unwrap();
    let token = authorization(&r).unwrap().replace("Bearer ", "");
    let code = generate(&r, Target::Curl);
    assert!(code.contains(&token), "{code}");
    assert!(!code.contains("not included"), "{code}");
}

#[tokio::test]
async fn sends_hide_the_token_and_secret_from_the_result() {
    let s = MockServer::start().await;
    Mock::given(path("/r")).respond_with(Echo).mount(&s).await;
    let r = http(&format!("{}/r", s.uri()), &hs(""));
    let (_d, c, ctx) = setup(&[("opencollection.yml", ROOT), ("r.yml", &r)]);
    let result = send_one(
        ItemPath::new("r.yml").unwrap(),
        request(&c, "r.yml"),
        &ctx,
        &Arc::new(Executor::new(ExecOptions::new()).unwrap()),
        &RunOptions::new(),
        &CancelHandle::new(),
    )
    .await;
    let res = result.response.as_ref().expect("response");
    let sent = res.text().split(" | ").next().unwrap().to_owned();
    let token = sent.strip_prefix("Bearer ").expect("echoed").to_owned();
    assert!(result.derived_credentials().contains(&token));
    assert!(!result.redact(res.text()).contains(&token));
    for hop in res.timeline() {
        for (name, value) in &hop.request_headers {
            let shown = result.redact_header(name, value);
            assert!(
                !shown.contains(&token) && !shown.contains(SECRET),
                "{shown}"
            );
        }
    }
}
