//! A script's engine errors quote the failing source line.

use crate::script_support;

use apinata_core::{ContextOptions, ExecOptions};
use script_support::*;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn server() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    s
}

async fn fail(kind: &str, code: &str, ctx: ContextOptions) -> apinata_core::RequestResult {
    let s = server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted("R", &format!("{}/ok", s.uri()), &script(kind, code), ""),
        ),
    ]);
    send_full(dir.path(), "r.yml", on(), ctx, ExecOptions::new())
        .await
        .0
}

fn post_message(result: &apinata_core::RequestResult) -> String {
    tests(result)[0].clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pre_request_not_a_function_quotes_its_line() {
    let r = fail(
        "before-request",
        "const o = {};\nlet a = 1;\n  o.baz();  \n",
        ContextOptions::new(),
    )
    .await;
    assert_eq!(
        script_error(&r),
        "request script, line 3: TypeError: not a function in \"o.baz();\""
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_post_response_not_a_function_quotes_its_line() {
    let r = fail(
        "after-response",
        "const o = {};\no.baz();",
        ContextOptions::new(),
    )
    .await;
    let line = post_message(&r);
    assert!(
        line.ends_with("request script, line 2: TypeError: not a function in \"o.baz();\""),
        "{line}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reference_error_quotes_its_line() {
    let r = fail(
        "before-request",
        "let a = 1;\nconsole.log(nope + a);",
        ContextOptions::new(),
    )
    .await;
    assert_eq!(
        script_error(&r),
        "request script, line 2: ReferenceError: nope is not defined in \"console.log(nope + a);\""
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_user_throw_is_not_echoed() {
    let r = fail(
        "before-request",
        "throw new Error('x')",
        ContextOptions::new(),
    )
    .await;
    assert_eq!(script_error(&r), "request script, line 1: Error: x");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_long_line_is_cut_with_an_ellipsis() {
    let pad = "a".repeat(200);
    let code = format!("const o = {{}}; /* {pad} */ o.baz();");
    let r = fail("before-request", &code, ContextOptions::new()).await;
    let message = script_error(&r);
    let quoted = message.split(" in \"").nth(1).expect(&message);
    assert!(
        quoted.ends_with("...\"") || quoted.ends_with("\u{2026}\""),
        "{message}"
    );
    assert!(quoted.chars().count() <= 125, "{message}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_secret_on_the_failing_line_is_redacted_when_shown() {
    let secret = "S3cret-tok-77";
    let code = format!("const o = {{}};\no.baz('{secret}');");
    let ctx = ContextOptions::new().sensitive_env_var("tok", secret);
    let r = fail("before-request", &code, ctx).await;
    let shown = r.redact(&script_error(&r));
    assert!(!shown.contains(secret), "{shown}");
    assert!(shown.contains("in \"o.baz("), "{shown}");
}
