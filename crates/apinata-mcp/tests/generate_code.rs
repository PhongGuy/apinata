//! `generate_code`: a request as code in every target, without sending it and without
//! revealing a secret.

use crate::common;

use apinata_core::{Collection, ContextOptions, Target, VarContext, generate, resolve};
use base64::Engine;
use common::{ROOT, TestServer};
use serde_json::json;

const DEV: &str = "name: dev\nvariables:\n  - name: token\n    secret: true\n";
const PLAIN: &str = "info:\n  name: r\n  type: http\nhttp:\n  method: POST\n  url: \"http://127.0.0.1:9/users?a=1\"\n  headers:\n    - name: X-Trace\n      value: abc\n  body:\n    type: json\n    data: '{\"k\": 1}'\n";

async fn start(request: &str) -> TestServer {
    TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("r.yml", request),
    ])
    .await
}

#[tokio::test]
async fn every_cli_target_is_accepted() {
    let server = start(PLAIN).await;
    for target in Target::ALL {
        let result = server
            .ok(
                "generate_code",
                json!({"path": "r.yml", "target": target.id()}),
            )
            .await;
        assert_eq!(result["target"], target.id());
        assert!(
            result["code"]
                .as_str()
                .is_some_and(|c| c.contains("127.0.0.1")),
            "{}: {result}",
            target.id()
        );
    }
}

#[tokio::test]
async fn an_unknown_target_lists_the_valid_ones() {
    let server = start(PLAIN).await;
    let err = server
        .err("generate_code", json!({"path": "r.yml", "target": "cobol"}))
        .await;
    for target in Target::ALL {
        assert!(err.contains(target.id()), "{err}");
    }
}

#[tokio::test]
async fn a_secret_value_never_appears_in_the_code() {
    let request = "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:9/x\"\n  headers:\n    - name: X-Key\n      value: \"{{token}}\"\n  auth:\n    type: bearer\n    token: \"{{token}}\"\n";
    let server = start(request).await;
    server.set_secret("dev", "token", "s3cr3t-Value-91");
    for target in Target::ALL {
        let result = server
            .ok(
                "generate_code",
                json!({"path": "r.yml", "target": target.id(), "environment": "dev"}),
            )
            .await;
        let code = result["code"].as_str().expect("code");
        assert!(!code.contains("s3cr3t-Value-91"), "{}: {code}", target.id());
        assert!(code.contains("<secret>"), "{}: {code}", target.id());
    }
}

#[tokio::test]
async fn an_undefined_variable_is_an_error_not_a_literal() {
    let request = "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:9/{{missing}}\"\n";
    let server = start(request).await;
    let err = server
        .err("generate_code", json!({"path": "r.yml", "target": "curl"}))
        .await;
    assert!(err.contains("missing"), "{err}");
}

#[tokio::test]
async fn a_var_override_reaches_the_code() {
    let request = "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:9/{{id}}\"\n";
    let server = start(request).await;
    let result = server
        .ok(
            "generate_code",
            json!({"path": "r.yml", "target": "curl", "vars": {"id": "42"}}),
        )
        .await;
    assert!(result["code"].as_str().expect("code").contains("/42"));
}

#[tokio::test]
async fn matches_core_output_when_no_secret_is_involved() {
    let server = start(PLAIN).await;
    let c = Collection::open(server.root()).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("ctx");
    let file = c
        .request(&apinata_core::ItemPath::new("r.yml").expect("path"))
        .expect("request");
    let resolved = resolve(file, &ctx).expect("resolves");
    for target in Target::ALL {
        let result = server
            .ok(
                "generate_code",
                json!({"path": "r.yml", "target": target.id()}),
            )
            .await;
        assert_eq!(
            result["code"],
            generate(&resolved, *target),
            "{}",
            target.id()
        );
    }
}

const SECRET: &str = "s3cr3t-Value-91";

/// A request with the secret variable `token` at one location, named for failure messages.
fn locations() -> Vec<(&'static str, String)> {
    let req = |url: &str, rest: &str| {
        let head = "info:\n  name: r\n  type: http\nhttp:\n  method: POST\n";
        format!("{head}  url: \"{url}\"\n{rest}")
    };
    let plain = "http://127.0.0.1:9/x";
    vec![
        ("url userinfo", req("http://bob:{{token}}@127.0.0.1:9/x", "")),
        (
            "basic auth",
            req(plain, "  auth:\n    type: basic\n    username: bob\n    password: \"{{token}}\"\n"),
        ),
        (
            "digest auth",
            req(plain, "  auth:\n    type: digest\n    username: bob\n    password: \"{{token}}\"\n"),
        ),
        (
            "bearer auth",
            req(plain, "  auth:\n    type: bearer\n    token: \"{{token}}\"\n"),
        ),
        ("query param", req("http://127.0.0.1:9/x?api_key={{token}}", "")),
        (
            "form body",
            req(plain, "  body:\n    type: form-urlencoded\n    data:\n      - name: pw\n        value: \"{{token}}\"\n"),
        ),
        (
            "text body",
            req(plain, "  body:\n    type: text\n    data: \"pw={{token}}\"\n"),
        ),
        (
            "graphql variables",
            "info:\n  name: r\n  type: graphql\ngraphql:\n  method: POST\n  url: \"http://127.0.0.1:9/x\"\n  body:\n    query: \"query($p: String) { a(p: $p) }\"\n    variables: '{\"p\": \"{{token}}\"}'\n"
                .to_owned(),
        ),
    ]
}

#[tokio::test]
async fn no_target_leaks_a_secret_from_any_location() {
    let b64 = |text: &str| base64::engine::general_purpose::STANDARD.encode(text);
    // Some targets escape or drop the padding, so the unpadded text is what is looked for.
    let userpass = b64(&format!("bob:{SECRET}"))
        .trim_end_matches('=')
        .to_owned();
    let forms = [
        ("raw", SECRET.to_owned()),
        ("base64 of the secret", b64(SECRET)),
        ("base64 of user:secret", userpass.clone()),
    ];
    for (location, request) in locations() {
        let server = start(&request).await;
        server.set_secret("dev", "token", SECRET);
        // The same request resolved in core with the secret known, to prove it is really there
        // before scrubbing; a scrub test over an empty or errored result would prove nothing.
        let c = Collection::open(server.root()).expect("opens");
        let ctx = VarContext::new(
            &c,
            ContextOptions::new()
                .env("dev")
                .sensitive_env_var("token", SECRET),
        )
        .expect("ctx");
        let file = c
            .request(&apinata_core::ItemPath::new("r.yml").expect("path"))
            .expect("request");
        let resolved = resolve(file, &ctx).expect("resolves");
        assert!(
            resolved.sensitive_values().iter().any(|v| v == SECRET),
            "location {location}: the secret is not among the sensitive values"
        );
        for target in Target::ALL {
            let raw = generate(&resolved, *target);
            // Digest answers a challenge, so a snippet may not carry the password at all.
            let carried = raw.contains(SECRET) || raw.contains(&userpass);
            assert!(
                carried || location == "digest auth",
                "target {} at location {location}: core output lacks the secret: {raw}",
                target.id()
            );
            let result = server
                .ok(
                    "generate_code",
                    json!({"path": "r.yml", "target": target.id(), "environment": "dev"}),
                )
                .await;
            let shown = result.to_string();
            if carried {
                assert!(
                    result["code"]
                        .as_str()
                        .is_some_and(|c| c.contains("<secret>")),
                    "target {} at location {location} shows no <secret>: {shown}",
                    target.id()
                );
            }
            for (form, value) in &forms {
                assert!(
                    !shown.contains(value.as_str()),
                    "target {} leaks the {form} at location {location}: {shown}",
                    target.id()
                );
            }
        }
    }
}

const CACHED_TOKEN: &str = "tok-cached-SECRET";

#[tokio::test]
async fn a_cached_oauth2_token_is_used_and_scrubbed_but_never_fetched() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let p = MockServer::start().await;
    let body = json!({"access_token": CACHED_TOKEN, "token_type": "Bearer", "expires_in": 3600});
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&p)
        .await;
    Mock::given(method("GET"))
        .and(path("/r"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&p)
        .await;
    let uri = p.uri();
    let request = format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{uri}/r\"\n  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{uri}/token\"\n    credentials:\n      clientId: app\n      clientSecret: cs-77aa-SECRET\n    tokenConfig:\n      id: main\n      placement:\n        header: Bearer\n"
    );
    let server = start(&request).await;
    let token_fetches = || async {
        p.received_requests()
            .await
            .expect("recorded")
            .iter()
            .filter(|r| r.url.path() == "/token")
            .count()
    };
    // With nothing cached, generating fails and never reaches the token host.
    let err = server
        .err("generate_code", json!({"path": "r.yml", "target": "curl"}))
        .await;
    assert!(!err.contains(CACHED_TOKEN), "{err}");
    assert_eq!(token_fetches().await, 0, "generate_code fetched a token");
    // A send caches the token in memory; the snippet then carries it as `<secret>`.
    server.ok("send_request", json!({"path": "r.yml"})).await;
    assert_eq!(token_fetches().await, 1);
    for target in Target::ALL {
        let result = server
            .ok(
                "generate_code",
                json!({"path": "r.yml", "target": target.id()}),
            )
            .await
            .to_string();
        assert!(!result.contains(CACHED_TOKEN), "{}: {result}", target.id());
        assert!(
            !result.contains("cs-77aa-SECRET"),
            "{}: {result}",
            target.id()
        );
        assert!(result.contains("<secret>"), "{}: {result}", target.id());
    }
    assert_eq!(token_fetches().await, 1, "generate_code fetched a token");
}

#[tokio::test]
async fn a_request_warning_is_returned_and_otherwise_the_list_is_empty() {
    let warned = "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:9/x\"\nsettings:\n  encodeUrl: false\n";
    let server = start(warned).await;
    let result = server
        .ok("generate_code", json!({"path": "r.yml", "target": "curl"}))
        .await;
    let warnings = result["warnings"].as_array().expect("warnings array");
    assert_eq!(warnings.len(), 1, "{result}");
    assert!(
        warnings[0]
            .as_str()
            .is_some_and(|w| w.contains("encodeUrl")),
        "{result}"
    );
    assert!(result["code"].is_string() && result["target"] == "curl");

    let server = start(PLAIN).await;
    let result = server
        .ok("generate_code", json!({"path": "r.yml", "target": "curl"}))
        .await;
    assert_eq!(result["warnings"], json!([]), "{result}");
}
