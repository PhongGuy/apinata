//! Status, Get token, Refresh and Clear for request, folder and collection auth.

use crate::oauth2_support;

use apinata_core::{
    AuthorizeOptions, CancelHandle, ItemPath, OAuth2Error, OAuth2Target, TokenStatus,
    clear_oauth2_token, is_oauth2, oauth2_status,
};
use oauth2_support::*;
use serde_json::json;
use std::time::{Duration, SystemTime};

fn how() -> AuthorizeOptions {
    AuthorizeOptions::new()
}
async fn get(f: &Fixture, target: OAuth2Target<'_>) -> Result<TokenStatus, OAuth2Error> {
    executor()
        .fetch_token(target, &f.ctx, &how(), &CancelHandle::new())
        .await
}

#[tokio::test]
async fn get_token_always_fetches_and_reports_the_status() {
    let server = wiremock::MockServer::start().await;
    on_grant(
        &server,
        "client_credentials",
        token_response(json!({"expires_in": 3600, "scope": "read write"})),
        2,
    )
    .await;
    let f = fixture(&server, &[("a.yml", "")]);
    let req = f.request("a.yml");
    assert_eq!(
        oauth2_status(OAuth2Target::Request(req), &f.ctx).unwrap(),
        None
    );
    f.prepare("a.yml", &how()).await.unwrap();
    let status = get(&f, OAuth2Target::Request(req)).await.unwrap();
    assert_eq!(
        status
            .expires_at
            .unwrap()
            .duration_since(status.obtained_at)
            .unwrap(),
        Duration::from_secs(3570)
    );
    assert!(!status.has_refresh_token);
    assert_eq!(status.scope.as_deref(), Some("read write"));
    assert!(!status.is_expired(SystemTime::now()));
    assert_eq!(
        oauth2_status(OAuth2Target::Request(req), &f.ctx).unwrap(),
        Some(status.clone())
    );
    let text = format!("{status:?}");
    assert!(!text.contains(TOKEN), "{text}");
}

#[tokio::test]
async fn other_auth_has_no_status_and_cannot_fetch() {
    let f = Fixture::new(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &request(
                "a",
                1,
                "https://api.test/r",
                "    type: bearer\n    token: x\n",
            ),
        ),
    ]);
    let req = f.request("a.yml");
    assert_eq!(
        oauth2_status(OAuth2Target::Request(req), &f.ctx).unwrap(),
        None
    );
    assert!(matches!(
        get(&f, OAuth2Target::Request(req)).await,
        Err(OAuth2Error::NotOAuth2)
    ));
}

#[tokio::test]
async fn folder_and_collection_targets_use_their_own_auth() {
    let server = wiremock::MockServer::start().await;
    on_grant(&server, "client_credentials", token_response(json!({})), 2).await;
    let uri = server.uri();
    let collection = format!(
        "{ROOT}request:\n  auth:\n{}",
        client_credentials(&uri, "").replace("id: main", "id: top")
    );
    let folder = format!(
        "info:\n  name: f\n  type: folder\nrequest:\n  auth:\n{}",
        client_credentials(&uri, "")
    );
    let inherit = "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: https://api.test/r\n  auth: inherit\n";
    let f = Fixture::new(&[
        ("opencollection.yml", collection.as_str()),
        ("f/folder.yml", folder.as_str()),
        ("f/r.yml", inherit),
    ]);
    let folder_item = ItemPath::new("f").unwrap();
    assert_eq!(
        oauth2_status(OAuth2Target::Folder(&folder_item), &f.ctx).unwrap(),
        None
    );
    get(&f, OAuth2Target::Folder(&folder_item)).await.unwrap();
    assert!(
        oauth2_status(OAuth2Target::Folder(&folder_item), &f.ctx)
            .unwrap()
            .is_some()
    );
    assert!(
        oauth2_status(OAuth2Target::Request(f.request("f/r.yml")), &f.ctx)
            .unwrap()
            .is_some(),
        "the request inherits the folder's token"
    );
    assert_eq!(
        oauth2_status(OAuth2Target::Collection, &f.ctx).unwrap(),
        None,
        "the collection auth has its own id"
    );
    get(&f, OAuth2Target::Collection).await.unwrap();
    assert!(
        oauth2_status(OAuth2Target::Collection, &f.ctx)
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn refresh_posts_a_refresh_and_keeps_the_refresh_token() {
    let server = wiremock::MockServer::start().await;
    on_grant(
        &server,
        "client_credentials",
        token_response(json!({"refresh_token": REFRESH, "expires_in": 3600})),
        1,
    )
    .await;
    on_grant(
        &server,
        "refresh_token",
        token_response(json!({"expires_in": 60})),
        1,
    )
    .await;
    let f = fixture(&server, &[("a.yml", "")]);
    let target = OAuth2Target::Request(f.request("a.yml"));
    get(&f, target).await.unwrap();
    let refreshed = executor()
        .refresh_token(target, &f.ctx, &CancelHandle::new())
        .await
        .unwrap();
    assert!(refreshed.has_refresh_token);
    assert_eq!(
        refreshed
            .expires_at
            .unwrap()
            .duration_since(refreshed.obtained_at)
            .unwrap(),
        Duration::from_secs(30)
    );
    let form = forms(&server, "refresh_token").await.remove(0);
    assert!(form.contains(&("refresh_token".to_owned(), REFRESH.to_owned())));
}

#[tokio::test]
async fn refresh_without_a_refresh_token_is_no_token_and_a_failure_keeps_the_cache() {
    let server = wiremock::MockServer::start().await;
    on_grant(
        &server,
        "client_credentials",
        token_response(json!({"refresh_token": REFRESH})),
        1,
    )
    .await;
    let refusal =
        wiremock::ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_grant"}));
    on_grant(&server, "refresh_token", refusal, 1).await;
    let f = fixture(&server, &[("a.yml", "")]);
    let target = OAuth2Target::Request(f.request("a.yml"));
    let none = executor()
        .refresh_token(target, &f.ctx, &CancelHandle::new())
        .await
        .unwrap_err();
    assert!(
        matches!(none, OAuth2Error::NoToken { ref credentials_id } if credentials_id == "main"),
        "{none:?}"
    );
    let before = get(&f, target).await.unwrap();
    let err = executor()
        .refresh_token(target, &f.ctx, &CancelHandle::new())
        .await
        .unwrap_err();
    assert!(
        matches!(err, OAuth2Error::Provider { status: 400, .. }),
        "{err:?}"
    );
    assert_eq!(oauth2_status(target, &f.ctx).unwrap(), Some(before));
}

#[tokio::test]
async fn clear_forgets_the_targets_token() {
    let server = wiremock::MockServer::start().await;
    on_grant(&server, "client_credentials", token_response(json!({})), 1).await;
    let f = fixture(&server, &[("a.yml", "")]);
    let target = OAuth2Target::Request(f.request("a.yml"));
    get(&f, target).await.unwrap();
    assert_eq!(clear_oauth2_token(target, &f.ctx).await.unwrap(), 1);
    assert_eq!(oauth2_status(target, &f.ctx).unwrap(), None);
}

#[tokio::test]
async fn get_token_signs_in_only_with_an_authorizer() {
    let server = wiremock::MockServer::start().await;
    on_grant(&server, "authorization_code", token_response(json!({})), 1).await;
    let f = single(&server, &authorization_code(&server.uri(), ""));
    let target = OAuth2Target::Request(f.request("a.yml"));
    assert!(matches!(
        get(&f, target).await,
        Err(OAuth2Error::InteractionRequired { .. })
    ));
    let browser = FakeBrowser::new(Play::Code("c0de"));
    let with = AuthorizeOptions::new().authorizer(browser.clone());
    executor()
        .fetch_token(target, &f.ctx, &with, &CancelHandle::new())
        .await
        .unwrap();
    assert_eq!(browser.prompts().len(), 1);
}

#[test]
fn a_clock_before_the_token_reads_as_expired() {
    let at = SystemTime::now();
    let s = TokenStatus::new(at, None, false, None);
    assert!(!s.is_expired(at + Duration::from_secs(1)));
    assert!(s.is_expired(at - Duration::from_secs(1)));
    let e = TokenStatus::new(at, Some(at + Duration::from_secs(5)), false, None);
    assert!(e.is_expired(at + Duration::from_secs(5)));
}

#[test]
fn is_oauth2_follows_the_effective_auth_of_each_target() {
    let uri = "http://127.0.0.1:9";
    let collection = format!("{ROOT}request:\n  auth:\n{}", client_credentials(uri, ""));
    let folder = format!(
        "info:\n  name: f\n  type: folder\nrequest:\n  auth:\n{}",
        client_credentials(uri, "")
    );
    let plain =
        "info:\n  name: p\n  type: folder\nrequest:\n  auth:\n    type: bearer\n    token: x\n";
    let inherit = "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: https://api.test/r\n  auth: inherit\n";
    let own = request("o", 2, "https://api.test/o", &client_credentials(uri, ""));
    let none = "info:\n  name: n\n  type: http\n  seq: 3\nhttp:\n  method: GET\n  url: https://api.test/n\n  auth:\n    type: none\n";
    let f = Fixture::new(&[
        ("opencollection.yml", collection.as_str()),
        ("f/folder.yml", folder.as_str()),
        ("f/r.yml", inherit),
        ("p/folder.yml", plain),
        ("p/o.yml", own.as_str()),
        ("p/n.yml", none),
    ]);
    let (f_item, p_item) = (ItemPath::new("f").unwrap(), ItemPath::new("p").unwrap());
    assert!(is_oauth2(OAuth2Target::Collection, &f.ctx));
    assert!(is_oauth2(OAuth2Target::Folder(&f_item), &f.ctx));
    assert!(is_oauth2(
        OAuth2Target::Request(f.request("f/r.yml")),
        &f.ctx
    ));
    assert!(is_oauth2(
        OAuth2Target::Request(f.request("p/o.yml")),
        &f.ctx
    ));
    assert!(!is_oauth2(OAuth2Target::Folder(&p_item), &f.ctx));
    assert!(!is_oauth2(
        OAuth2Target::Request(f.request("p/n.yml")),
        &f.ctx
    ));
}
