//! The runner with OAuth 2.0: one token for requests that share credentials, the fetch after
//! the pre-request script, and nothing sent when the fetch fails.

use crate::oauth2_support;

use apinata_core::{
    CancelHandle, OAuth2Error, Outcome, ResolveError, RunError, RunOptions, RunReport,
    ScriptOptions, run,
};
use oauth2_support::*;
use serde_json::json;
use std::sync::Arc;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn run_all(f: &Fixture, opts: RunOptions) -> RunReport {
    let plan = f.collection.plan(f.collection.root()).expect("plan");
    run(
        plan,
        f.ctx.clone(),
        Arc::new(executor()),
        opts,
        CancelHandle::new(),
        Box::new(|_| {}),
    )
    .await
}

/// The resource, answering only requests that carry the token, `times` of them.
async fn resource(server: &MockServer, times: u64) {
    Mock::given(method("GET"))
        .and(path("/r"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(ResponseTemplate::new(200))
        .expect(times)
        .mount(server)
        .await;
}

fn outcomes(report: &RunReport) -> Vec<Outcome> {
    report.results.iter().map(|r| r.outcome).collect()
}

#[tokio::test]
async fn requests_sharing_credentials_fetch_one_token() {
    let server = MockServer::start().await;
    on_grant(
        &server,
        "client_credentials",
        token_response(json!({"expires_in": 3600})),
        1,
    )
    .await;
    resource(&server, 2).await;
    let f = fixture(&server, &[("a.yml", ""), ("b.yml", "")]);
    let report = run_all(&f, RunOptions::new()).await;
    assert_eq!(outcomes(&report), [Outcome::Passed, Outcome::Passed]);
}

#[tokio::test]
async fn a_failed_fetch_errors_the_request_and_sends_nothing() {
    let server = MockServer::start().await;
    let refused = ResponseTemplate::new(401).set_body_json(json!({"error": "invalid_client"}));
    on_grant(&server, "client_credentials", refused, 1).await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let f = fixture(&server, &[("a.yml", "")]);
    let report = run_all(&f, RunOptions::new()).await;
    assert_eq!(outcomes(&report), [Outcome::Errored]);
    assert!(
        matches!(
            &report.results[0].error,
            Some(RunError::OAuth2(OAuth2Error::Provider { status: 401, .. }))
        ),
        "{:?}",
        report.results[0].error
    );
}

#[tokio::test]
async fn the_token_is_fetched_after_the_pre_request_script() {
    let server = MockServer::start().await;
    on_grant(
        &server,
        "client_credentials",
        token_response(json!({"expires_in": 3600})),
        1,
    )
    .await;
    resource(&server, 1).await;
    let p = server.uri();
    let auth = format!(
        "    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{p}{{{{tokenPath}}}}\"\n    credentials:\n      clientId: app\n    scope: read\n    tokenConfig:\n      id: main\n      placement:\n        header: Bearer\n"
    );
    let text = format!(
        "{}runtime:\n  scripts:\n    - type: before-request\n      code: |-\n        bru.setVar(\"tokenPath\", \"/token\");\n",
        request("a", 1, &format!("{p}/r"), &auth)
    );
    let f = Fixture::new(&[("opencollection.yml", ROOT), ("a.yml", &text)]);
    // Scripts off first: nothing defines the path, so the fetch fails before any I/O.
    let off = run_all(&f, RunOptions::new()).await;
    assert!(
        matches!(&off.results[0].error, Some(RunError::OAuth2(OAuth2Error::Resolve(ResolveError::UndefinedVariable { name, .. }))) if name == "tokenPath"),
        "{:?}",
        off.results[0].error
    );
    let on = run_all(
        &f,
        RunOptions::new().scripts(ScriptOptions::new().enabled(true)),
    )
    .await;
    assert_eq!(
        outcomes(&on),
        [Outcome::Passed],
        "{:?}",
        on.results[0].error
    );
}

#[tokio::test]
async fn a_browser_grant_errors_with_interaction_required() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let auth = format!(
        "    type: oauth2\n    flow: authorization_code\n    authorizationUrl: \"{0}/authorize\"\n    accessTokenUrl: \"{0}/token\"\n    credentials:\n      clientId: app\n    tokenConfig:\n      id: main\n",
        server.uri()
    );
    let f = Fixture::new(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &request("a", 1, &format!("{}/r", server.uri()), &auth),
        ),
    ]);
    let report = run_all(&f, RunOptions::new()).await;
    assert_eq!(outcomes(&report), [Outcome::Errored]);
    assert!(matches!(
        &report.results[0].error,
        Some(RunError::OAuth2(OAuth2Error::InteractionRequired { .. }))
    ));
}

#[tokio::test]
async fn the_run_wide_cap_bounds_the_token_fetch() {
    let server = MockServer::start().await;
    let slow = token_response(json!({})).set_delay(std::time::Duration::from_secs(3));
    on_grant(&server, "client_credentials", slow, 1).await;
    let f = fixture(&server, &[("a.yml", "")]);
    let capped =
        apinata_core::ExecOptions::new().timeout_cap(Some(std::time::Duration::from_millis(500)));
    let plan = f.collection.plan(f.collection.root()).expect("plan");
    let started = std::time::Instant::now();
    let report = run(
        plan,
        f.ctx.clone(),
        Arc::new(apinata_core::Executor::new(capped).expect("client")),
        RunOptions::new(),
        CancelHandle::new(),
        Box::new(|_| {}),
    )
    .await;
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    assert_eq!(outcomes(&report), [Outcome::Errored]);
    let error = report.results[0].error.as_ref().expect("an error");
    assert!(error.to_string().contains("run-wide timeout of"), "{error}");
}
