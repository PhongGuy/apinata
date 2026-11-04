//! Token fetches and schema fetches are logged under their own source, without secrets.

use crate::oauth2_support;
use crate::session_log_support;

use apinata_core::{
    AuthorizeOptions, CancelHandle, ContextOptions, ExecOptions, Executor, ItemPath, OAuth2Target,
    RunOptions, SendSource, SessionLog, TokenCache, fetch_schema, introspection_request, send_one,
};
use oauth2_support::{
    CLIENT_SECRET, FakeBrowser, Fixture, Play, REFRESH, ROOT, TOKEN, authorization_code, fixture,
    forms, on_grant, request, settings, single, token_response,
};
use serde_json::json;
use session_log_support::{assert_absent, entries, only};
use std::sync::Arc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ID_TOKEN: &str = "idt-31d0-SECRET";

fn recording() -> (Arc<Executor>, SessionLog) {
    let log = SessionLog::new();
    (
        Arc::new(Executor::new(ExecOptions::new().session_log(log.clone())).expect("client")),
        log,
    )
}

fn assert_no_token_secrets(log: &SessionLog) {
    for secret in [TOKEN, REFRESH, ID_TOKEN, CLIENT_SECRET] {
        assert_absent(log, secret);
    }
}

fn sources(log: &SessionLog) -> Vec<SendSource> {
    entries(log).iter().map(|e| e.source).collect()
}

/// A token answer that also issues a refresh token and an id token.
fn issued() -> ResponseTemplate {
    token_response(json!({"refresh_token": REFRESH, "id_token": ID_TOKEN}))
}

#[tokio::test]
async fn a_direct_token_fetch_is_published_at_once_as_oauth2() {
    let s = MockServer::start().await;
    on_grant(&s, "client_credentials", issued(), 2).await;
    let fx = fixture(&s, &[("r.yml", &settings(true, true))]);
    let (exec, log) = recording();
    exec.fetch_token(
        OAuth2Target::Request(fx.request("r.yml")),
        &fx.ctx,
        &AuthorizeOptions::new(),
        &CancelHandle::new(),
    )
    .await
    .expect("token");
    let e = only(&log);
    assert_eq!(
        (e.source, e.method.as_str(), e.status),
        (SendSource::OAuth2, "POST", Some(200))
    );
    assert!(e.response_body.text.contains("[redacted]"));
    assert_no_token_secrets(&log);
    let (exec, log) = recording();
    exec.prepare_auth(
        fx.request("r.yml"),
        &fx.ctx,
        &AuthorizeOptions::new().refetch(true),
        &CancelHandle::new(),
    )
    .await
    .expect("prepared");
    assert_eq!(sources(&log), [SendSource::OAuth2]);
    assert_no_token_secrets(&log);
}

#[tokio::test]
async fn a_runner_token_fetch_is_held_with_the_send() {
    let s = MockServer::start().await;
    on_grant(&s, "client_credentials", issued(), 1).await;
    Mock::given(path("/r"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(format!("hello {TOKEN} {REFRESH} {ID_TOKEN}")),
        )
        .mount(&s)
        .await;
    let fx = fixture(&s, &[("r.yml", &settings(true, true))]);
    let (exec, log) = recording();
    let item = ItemPath::new("r.yml").expect("path");
    send_one(
        item,
        fx.request("r.yml"),
        &fx.ctx,
        &exec,
        &RunOptions::new(),
        &CancelHandle::new(),
    )
    .await;
    assert_eq!(sources(&log), [SendSource::OAuth2, SendSource::Runner]);
    assert_no_token_secrets(&log);
}

#[tokio::test]
async fn a_later_send_with_a_cached_token_hides_every_issued_token() {
    let s = MockServer::start().await;
    on_grant(&s, "client_credentials", issued(), 1).await;
    Mock::given(path("/r"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(format!("hello {TOKEN} {REFRESH} {ID_TOKEN}")),
        )
        .mount(&s)
        .await;
    let fx = fixture(&s, &[("r.yml", &settings(true, true))]);
    let (exec, _) = recording();
    exec.prepare_auth(
        fx.request("r.yml"),
        &fx.ctx,
        &AuthorizeOptions::new(),
        &CancelHandle::new(),
    )
    .await
    .expect("prepared");
    let (exec, log) = recording();
    let item = ItemPath::new("r.yml").expect("path");
    send_one(
        item,
        fx.request("r.yml"),
        &fx.ctx,
        &exec,
        &RunOptions::new(),
        &CancelHandle::new(),
    )
    .await;
    assert_eq!(only(&log).source, SendSource::Runner);
    assert_no_token_secrets(&log);
}

#[tokio::test]
async fn a_schema_fetch_is_logged_without_its_secret_header() {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{\"echo\":\"gq-secret-9931\"}"))
        .mount(&s)
        .await;
    let g = format!(
        "info:\n  name: g\n  type: graphql\n  seq: 1\ngraphql:\n  url: \"{}/graphql\"\n  method: POST\n  headers:\n    - name: X-Key\n      value: \"{{{{tok}}}}\"\n",
        s.uri()
    );
    let fx = Fixture::with(
        &[("opencollection.yml", ROOT), ("g.yml", &g)],
        ContextOptions::new().sensitive_env_var("tok", "gq-secret-9931"),
        TokenCache::in_memory(),
    );
    let req = introspection_request(fx.request("g.yml"), &fx.ctx).expect("resolves");
    let (exec, log) = recording();
    let _ = fetch_schema(&exec, &req, &CancelHandle::new()).await;
    assert_eq!(only(&log).source, SendSource::GraphqlSchema);
    assert_absent(&log, "gq-secret-9931");
}

#[tokio::test]
async fn a_code_exchange_is_logged_as_oauth2_without_the_code_or_verifier() {
    let s = MockServer::start().await;
    on_grant(&s, "authorization_code", issued(), 1).await;
    let fx = single(&s, &authorization_code(&s.uri(), "    pkce: {}\n"));
    let browser = FakeBrowser::new(Play::Code("code-secret-5521"));
    let how = AuthorizeOptions::new().authorizer(browser.clone());
    let (exec, log) = recording();
    exec.prepare_auth(fx.request("a.yml"), &fx.ctx, &how, &CancelHandle::new())
        .await
        .expect("signed in");
    assert_eq!(only(&log).source, SendSource::OAuth2);
    assert_absent(&log, "code-secret-5521");
    let form = forms(&s, "authorization_code").await.remove(0);
    let verifier = form
        .iter()
        .find(|(k, _)| k == "code_verifier")
        .map(|(_, v)| v.clone())
        .expect("a verifier");
    assert_absent(&log, &verifier);
    assert_no_token_secrets(&log);
}

#[tokio::test]
async fn a_password_grant_is_logged_without_the_password() {
    let s = MockServer::start().await;
    on_grant(&s, "password", issued(), 1).await;
    let auth = format!(
        "    type: oauth2\n    flow: resource_owner_password_credentials\n    accessTokenUrl: \"{}/token\"\n    credentials:\n      clientId: app\n      clientSecret: {CLIENT_SECRET}\n    resourceOwner:\n      username: ann\n      password: pw-6e0b-SECRET\n",
        s.uri()
    );
    let text = request("p", 1, &format!("{}/r", s.uri()), &auth);
    let fx = Fixture::new(&[("opencollection.yml", ROOT), ("p.yml", &text)]);
    let (exec, log) = recording();
    exec.prepare_auth(
        fx.request("p.yml"),
        &fx.ctx,
        &AuthorizeOptions::new(),
        &CancelHandle::new(),
    )
    .await
    .expect("prepared");
    assert_eq!(only(&log).source, SendSource::OAuth2);
    assert_absent(&log, "pw-6e0b-SECRET");
    assert_no_token_secrets(&log);
}

#[tokio::test]
async fn a_form_encoded_token_answer_is_logged_without_its_tokens() {
    let s = MockServer::start().await;
    let body = format!(
        "access_token={TOKEN}&token_type=Bearer&refresh_token={REFRESH}&id_token={ID_TOKEN}"
    );
    let answer = ResponseTemplate::new(200)
        .insert_header("content-type", "application/x-www-form-urlencoded")
        .set_body_string(body);
    on_grant(&s, "client_credentials", answer, 1).await;
    let fx = fixture(&s, &[("r.yml", &settings(true, true))]);
    let (exec, log) = recording();
    let fetched = exec
        .fetch_token(
            OAuth2Target::Request(fx.request("r.yml")),
            &fx.ctx,
            &AuthorizeOptions::new(),
            &CancelHandle::new(),
        )
        .await;
    assert!(fetched.is_err(), "a form answer is not a token");
    assert_eq!(only(&log).source, SendSource::OAuth2);
    assert_no_token_secrets(&log);
}
