//! `prepare_auth` options and concurrency: one fetch for concurrent callers, `refetch` once
//! per key, and `cache_only` never touching the provider.

use crate::oauth2_support;

use apinata_core::{AuthorizeOptions, OAuth2Error};
use oauth2_support::*;
use serde_json::json;
use std::time::Duration;
use wiremock::MockServer;

fn how() -> AuthorizeOptions {
    AuthorizeOptions::new()
}

#[tokio::test]
async fn concurrent_callers_share_one_fetch() {
    let server = MockServer::start().await;
    let slow = token_response(json!({"expires_in": 3600})).set_delay(Duration::from_millis(200));
    on_grant(&server, "client_credentials", slow, 1).await;
    let f = fixture(&server, &[("a.yml", ""), ("b.yml", "")]);
    let how = how();
    let (a, b) = tokio::join!(f.prepare("a.yml", &how), f.prepare("b.yml", &how));
    a.unwrap();
    b.unwrap();
}

#[tokio::test]
async fn refetch_ignores_the_cache_once_per_key() {
    let server = MockServer::start().await;
    on_grant(
        &server,
        "client_credentials",
        token_response(json!({"expires_in": 3600})),
        2,
    )
    .await;
    let f = fixture(&server, &[("a.yml", ""), ("b.yml", "")]);
    f.prepare("a.yml", &how()).await.unwrap();
    let again = AuthorizeOptions::new().refetch(true);
    f.prepare("a.yml", &again).await.unwrap();
    f.prepare("b.yml", &again.clone()).await.unwrap();
}

#[tokio::test]
async fn cache_only_never_fetches_or_refreshes() {
    let server = MockServer::start().await;
    on_grant(
        &server,
        "client_credentials",
        token_response(json!({"expires_in": 1, "refresh_token": REFRESH})),
        1,
    )
    .await;
    on_grant(&server, "refresh_token", token_response(json!({})), 0).await;
    let f = fixture(&server, &[("a.yml", &settings(true, true))]);
    let cache_only = AuthorizeOptions::new().cache_only(true);
    assert!(matches!(
        f.prepare("a.yml", &cache_only).await,
        Err(OAuth2Error::NoToken { .. })
    ));
    f.prepare("a.yml", &how()).await.unwrap();
    f.prepare("a.yml", &cache_only).await.unwrap();
}
