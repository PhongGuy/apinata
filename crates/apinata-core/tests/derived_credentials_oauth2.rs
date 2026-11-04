//! OAuth 2.0 tokens are derived credentials: listed on the response, hidden in the report.

use crate::derived_support::{Echo, enc};
use crate::oauth2_support;

use apinata_core::{CancelHandle, ItemPath, Outcome, RequestResult, RunOptions, send_one};
use oauth2_support::{
    CLIENT_SECRET, TOKEN, client_credentials, executor, fixture, on_grant, settings, single,
    token_response,
};
use serde_json::json;
use std::sync::Arc;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer};

async fn provider() -> MockServer {
    let s = MockServer::start().await;
    on_grant(&s, "client_credentials", token_response(json!({})), 1).await;
    Mock::given(path("/r")).respond_with(Echo).mount(&s).await;
    s
}

async fn run(fx: &oauth2_support::Fixture, rel: &str) -> RequestResult {
    send_one(
        ItemPath::new(rel).expect("path"),
        fx.request(rel),
        &fx.ctx,
        &Arc::new(executor()),
        &RunOptions::new(),
        &CancelHandle::new(),
    )
    .await
}

#[tokio::test]
async fn a_header_token_is_derived_and_hidden_from_the_report() {
    let s = provider().await;
    let fx = fixture(&s, &[("r.yml", &settings(true, true))]);
    let result = run(&fx, "r.yml").await;
    let res = result.response.as_ref().expect("response");
    assert_eq!(result.derived_credentials(), [TOKEN]);
    assert_eq!(res.derived_credentials(), [TOKEN]);
    assert!(
        !result
            .derived_credentials()
            .iter()
            .any(|v| v == CLIENT_SECRET)
    );
    assert!(res.text().contains(TOKEN), "the server echoes it");
    assert!(!result.redact(res.text()).contains(TOKEN));
    assert_eq!(result.redact(&format!("x {TOKEN}")), "x [redacted]");
}

#[tokio::test]
async fn a_query_token_is_derived_and_hidden_from_the_sent_url() {
    let s = provider().await;
    let block = client_credentials(&s.uri(), &settings(true, true))
        .replace("        header: Bearer\n", "        query: access\n");
    let fx = single(&s, &block);
    let result = run(&fx, "a.yml").await;
    let res = result.response.as_ref().expect("response");
    assert_eq!(result.derived_credentials(), [TOKEN]);
    let url = &res.timeline()[0].url;
    assert!(url.contains(&format!("access={}", enc(TOKEN))), "{url}");
    assert!(!result.redact(url).contains(TOKEN));
}

#[tokio::test]
async fn a_result_without_a_response_derives_nothing() {
    let result = RequestResult::from_parts(
        ItemPath::new("a.yml").expect("path"),
        "A",
        Outcome::Errored,
        None,
        Vec::new(),
    );
    assert!(result.derived_credentials().is_empty());
}
