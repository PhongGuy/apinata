//! `Executor::prepare_auth`: the decision table row by row, the failures that happen before
//! any I/O, and a cache reopened from disk.

use crate::oauth2_support;

use crate::common::dead_port;
use apinata_core::{
    AuthorizeOptions, CancelHandle, ContextOptions, MemoryStore, OAuth2Error, ResolveError,
    SecretStore, TokenCache, VarContext,
};
use oauth2_support::*;
use serde_json::json;
use std::sync::Arc;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn how() -> AuthorizeOptions {
    AuthorizeOptions::new()
}

fn invalid_grant() -> ResponseTemplate {
    ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_grant"}))
}

/// A provider that must not be called at all.
async fn silent() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn a_fresh_token_is_fetched_once_and_then_used() {
    let server = MockServer::start().await;
    on_grant(
        &server,
        "client_credentials",
        token_response(json!({"expires_in": 3600})),
        1,
    )
    .await;
    let f = fixture(&server, &[("a.yml", "")]);
    f.prepare("a.yml", &how()).await.unwrap();
    f.prepare("a.yml", &how()).await.unwrap();
}

#[tokio::test]
async fn an_expired_token_is_refreshed_and_keeps_its_refresh_token() {
    let server = MockServer::start().await;
    // `expires_in: 1` is already expired under the 30 s margin.
    on_grant(
        &server,
        "client_credentials",
        token_response(json!({"expires_in": 1, "refresh_token": REFRESH})),
        1,
    )
    .await;
    on_grant(
        &server,
        "refresh_token",
        token_response(json!({"expires_in": 1})),
        2,
    )
    .await;
    let f = fixture(&server, &[("a.yml", &settings(true, true))]);
    for _ in 0..3 {
        f.prepare("a.yml", &how()).await.unwrap();
    }
    let refreshes = forms(&server, "refresh_token").await;
    assert_eq!(refreshes.len(), 2);
    for form in refreshes {
        assert!(
            form.contains(&("refresh_token".to_owned(), REFRESH.to_owned())),
            "{form:?}"
        );
    }
}

#[tokio::test]
async fn a_failed_refresh_clears_and_fetches_when_auto_fetch_is_on() {
    let server = MockServer::start().await;
    on_grant(
        &server,
        "client_credentials",
        token_response(json!({"expires_in": 1, "refresh_token": REFRESH})),
        2,
    )
    .await;
    on_grant(&server, "refresh_token", invalid_grant(), 1).await;
    let f = fixture(&server, &[("a.yml", &settings(true, true))]);
    f.prepare("a.yml", &how()).await.unwrap();
    f.prepare("a.yml", &how()).await.unwrap();
}

#[tokio::test]
async fn a_failed_refresh_without_auto_fetch_keeps_the_expired_token_and_stops_refreshing() {
    let server = MockServer::start().await;
    on_grant(
        &server,
        "client_credentials",
        token_response(json!({"expires_in": 1, "refresh_token": REFRESH})),
        1,
    )
    .await;
    on_grant(&server, "refresh_token", invalid_grant(), 1).await;
    // Same credentials: `a.yml` obtains the token, `b.yml` may only refresh it.
    let f = fixture(&server, &[("a.yml", ""), ("b.yml", &settings(false, true))]);
    f.prepare("a.yml", &how()).await.unwrap();
    f.prepare("b.yml", &how()).await.unwrap();
    f.prepare("b.yml", &how()).await.unwrap();
}

#[tokio::test]
async fn an_unreachable_refresh_endpoint_is_an_error_and_keeps_the_refresh_token() {
    let server = MockServer::start().await;
    on_grant(
        &server,
        "client_credentials",
        token_response(json!({"expires_in": 1, "refresh_token": REFRESH})),
        1,
    )
    .await;
    on_grant(
        &server,
        "refresh_token",
        token_response(json!({"expires_in": 3600})),
        1,
    )
    .await;
    // A port that refuses every connection.
    let closed = dead_port();
    let unreachable = format!(
        "{}    refreshTokenUrl: \"http://127.0.0.1:{closed}/token\"\n",
        settings(false, true)
    );
    // Same key and fingerprint: `a.yml` obtains the token, `b.yml` refreshes at a closed port,
    // `c.yml` refreshes at the provider.
    let f = fixture(
        &server,
        &[
            ("a.yml", ""),
            ("b.yml", &unreachable),
            ("c.yml", &settings(false, true)),
        ],
    );
    f.prepare("a.yml", &how()).await.unwrap();
    let err = f.prepare("b.yml", &how()).await.unwrap_err();
    assert!(matches!(err, OAuth2Error::Exec(_)), "{err:?}");
    f.prepare("c.yml", &how()).await.unwrap();
    let refreshes = forms(&server, "refresh_token").await;
    assert_eq!(refreshes.len(), 1);
    assert!(
        refreshes[0].contains(&("refresh_token".to_owned(), REFRESH.to_owned())),
        "{:?}",
        refreshes[0]
    );
}

#[tokio::test]
async fn an_expired_token_without_a_refresh_token_is_fetched_again() {
    let server = MockServer::start().await;
    on_grant(
        &server,
        "client_credentials",
        token_response(json!({"expires_in": 1})),
        2,
    )
    .await;
    let f = fixture(&server, &[("a.yml", &settings(true, true))]);
    f.prepare("a.yml", &how()).await.unwrap();
    f.prepare("a.yml", &how()).await.unwrap();
}

#[tokio::test]
async fn an_expired_token_is_kept_when_auto_fetch_is_off() {
    let server = MockServer::start().await;
    on_grant(
        &server,
        "client_credentials",
        token_response(json!({"expires_in": 1})),
        1,
    )
    .await;
    let f = fixture(
        &server,
        &[("a.yml", ""), ("b.yml", &settings(false, false))],
    );
    f.prepare("a.yml", &how()).await.unwrap();
    f.prepare("b.yml", &how()).await.unwrap();
    f.prepare("b.yml", &how()).await.unwrap();
}

#[tokio::test]
async fn no_token_without_auto_fetch_is_no_token() {
    let server = silent().await;
    let f = fixture(&server, &[("a.yml", &settings(false, false))]);
    let err = f.prepare("a.yml", &how()).await.unwrap_err();
    assert!(
        matches!(err, OAuth2Error::NoToken { ref credentials_id } if credentials_id == "main"),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_failed_fetch_is_an_error() {
    let server = MockServer::start().await;
    let refused = ResponseTemplate::new(401).set_body_json(json!({"error": "invalid_client"}));
    on_grant(&server, "client_credentials", refused, 1).await;
    let f = fixture(&server, &[("a.yml", "")]);
    let err = f.prepare("a.yml", &how()).await.unwrap_err();
    assert!(
        matches!(err, OAuth2Error::Provider { status: 401, .. }),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_browser_grant_without_a_token_needs_interaction() {
    let server = silent().await;
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
    let err = f.prepare("a.yml", &how()).await.unwrap_err();
    assert!(
        matches!(err, OAuth2Error::InteractionRequired { ref credentials_id } if credentials_id == "main"),
        "{err:?}"
    );
    assert_eq!(
        err.to_string(),
        "OAuth 2.0 token \"main\": this grant needs a browser sign-in; use Get token in the Apinata app, then run again"
    );
}

#[tokio::test]
async fn missing_fields_and_undefined_variables_fail_before_any_io() {
    let server = silent().await;
    let no_id = format!(
        "    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{}/token\"\n",
        server.uri()
    );
    let undefined = format!(
        "    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{}/{{{{nope}}}}\"\n    credentials:\n      clientId: app\n",
        server.uri()
    );
    let url = format!("{}/r", server.uri());
    let (a, b) = (
        request("a", 1, &url, &no_id),
        request("b", 2, &url, &undefined),
    );
    let f = Fixture::new(&[("opencollection.yml", ROOT), ("a.yml", &a), ("b.yml", &b)]);
    assert!(matches!(
        f.prepare("a.yml", &how()).await,
        Err(OAuth2Error::MissingField { field: "clientId" })
    ));
    let err = f.prepare("b.yml", &how()).await.unwrap_err();
    assert!(
        matches!(err, OAuth2Error::Resolve(ResolveError::UndefinedVariable { ref name, .. }) if name == "nope"),
        "{err:?}"
    );
}

#[tokio::test]
async fn other_auth_is_left_alone() {
    let digest = request(
        "a",
        1,
        "http://127.0.0.1:9/r",
        "    type: digest\n    username: u\n    password: p\n",
    );
    let none = "info:\n  name: b\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: http://127.0.0.1:9/r\n";
    let f = Fixture::new(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &digest),
        ("b.yml", none),
    ]);
    f.prepare("a.yml", &how()).await.unwrap();
    f.prepare("b.yml", &how()).await.unwrap();
}

#[tokio::test]
async fn a_changed_scope_reads_as_no_token() {
    let server = MockServer::start().await;
    on_grant(
        &server,
        "client_credentials",
        token_response(json!({"expires_in": 3600})),
        2,
    )
    .await;
    let url = format!("{}/r", server.uri());
    let read = request("a", 1, &url, &client_credentials(&server.uri(), ""));
    let write = request(
        "b",
        2,
        &url,
        &client_credentials(&server.uri(), "").replace("scope: read", "scope: write"),
    );
    let f = Fixture::new(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &read),
        ("b.yml", &write),
    ]);
    f.prepare("a.yml", &how()).await.unwrap();
    f.prepare("b.yml", &how()).await.unwrap();
}

#[tokio::test]
async fn a_cache_reopened_from_disk_still_holds_the_token() {
    let server = MockServer::start().await;
    on_grant(
        &server,
        "client_credentials",
        token_response(json!({"expires_in": 3600})),
        1,
    )
    .await;
    let state = tempfile::TempDir::new().unwrap();
    let file = state.path().join("oauth2-tokens.json");
    let keys: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
    let text = request(
        "a",
        1,
        &format!("{}/r", server.uri()),
        &client_credentials(&server.uri(), ""),
    );
    let f = Fixture::with(
        &[("opencollection.yml", ROOT), ("a.yml", &text)],
        ContextOptions::new(),
        TokenCache::open(file.clone(), keys.clone()),
    );
    f.prepare("a.yml", &how()).await.unwrap();
    // Another process: a new cache over the same file and keychain.
    let reopened = VarContext::new(
        &f.collection,
        ContextOptions::new().oauth2_tokens(TokenCache::open(file.clone(), keys)),
    )
    .unwrap();
    executor()
        .prepare_auth(f.request("a.yml"), &reopened, &how(), &CancelHandle::new())
        .await
        .unwrap();
    assert!(!String::from_utf8_lossy(&std::fs::read(&file).unwrap()).contains(TOKEN));
}

#[test]
fn the_prepare_future_is_send() {
    fn send<T: Send>(_: &T) {}
    let f = Fixture::new(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &request(
                "a",
                1,
                "http://127.0.0.1:9/r",
                &client_credentials("http://127.0.0.1:9", ""),
            ),
        ),
    ]);
    let (exec, how, cancel) = (executor(), how(), CancelHandle::new());
    let future = exec.prepare_auth(f.request("a.yml"), &f.ctx, &how, &cancel);
    send(&future);
}

// Errors name a setting by the label the editor shows next to it, never by its file key,
// and quote the token ID instead of wrapping it in backticks.
#[test]
fn oauth2_errors_use_the_editor_labels() {
    let cases = [
        ("authorizationUrl", "Authorization URL is empty"),
        ("accessTokenUrl", "Access token URL is empty"),
        ("callbackUrl", "Callback URL is empty"),
        ("clientId", "Client ID is empty"),
        ("username", "Username is empty"),
        (
            "tokenConfig.placement.query",
            "Query parameter name is empty",
        ),
    ];
    for (field, want) in cases {
        assert_eq!(OAuth2Error::MissingField { field }.to_string(), want);
        assert_eq!(ResolveError::OAuth2MissingField { field }.to_string(), want);
    }
    let no_token = "OAuth 2.0 token \"main\": no token yet; send the request or get a token first";
    let id = || "main".to_owned();
    assert_eq!(
        OAuth2Error::NoToken {
            credentials_id: id()
        }
        .to_string(),
        no_token
    );
    assert_eq!(
        ResolveError::OAuth2NoToken {
            credentials_id: id()
        }
        .to_string(),
        no_token
    );
    assert_eq!(
        OAuth2Error::InteractionRequired {
            credentials_id: id()
        }
        .to_string(),
        "OAuth 2.0 token \"main\": this grant needs a browser sign-in; use Get token in the Apinata app, then run again"
    );
}

#[test]
fn an_unknown_oauth2_field_reads_as_a_required_setting() {
    assert_eq!(
        apinata_core::oauth2_field_label("nope"),
        "A required OAuth 2.0 setting"
    );
    assert_eq!(
        OAuth2Error::MissingField { field: "nope" }.to_string(),
        "A required OAuth 2.0 setting is empty"
    );
}
