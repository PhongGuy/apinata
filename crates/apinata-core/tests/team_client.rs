//! The team client over a stub server on 127.0.0.1: requests, parsing and error mapping.

use crate::common::dead_port;
use crate::common::potluck::*;
use apinata_core::team::{HttpTeamClient, NewWorkspace, Role, ServerUrl, TeamApi, TeamError};
use serde_json::json;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn bearer() -> String {
    format!("Bearer {}", tok())
}

#[tokio::test]
async fn the_server_info_says_whether_password_sign_in_is_offered() {
    let s = MockServer::start().await;
    answer(&s, "GET", "/api/v1/server", info(false)).await;
    let got = client(&s).server_info().await.unwrap();
    assert_eq!(
        (got.version.as_str(), got.api, got.password_sign_in),
        ("0.1.0", 1, false)
    );
    let sent = s.received_requests().await.unwrap();
    let ua = format!("apinata/{}", env!("CARGO_PKG_VERSION"));
    assert_eq!(
        sent[0].headers.get("user-agent").unwrap().to_str().unwrap(),
        ua
    );
}

#[tokio::test]
async fn sign_in_posts_the_password_and_returns_the_token_and_user() {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/session"))
        .and(body_json(
            json!({"username": "ada", "password": "hunter2", "token_name": "Apinata"}),
        ))
        .respond_with(session(7, true))
        .mount(&s)
        .await;
    let signed = client(&s).sign_in("ada", "hunter2").await.unwrap();
    assert_eq!(signed.token.expose(), tok());
    assert_eq!(signed.token_id, "7");
    assert_eq!(
        (signed.user.username.as_str(), signed.user.server_admin),
        ("ada", true)
    );
    assert!(!format!("{signed:?}").contains(&tok()[4..]));
}

#[tokio::test]
async fn a_malformed_token_in_the_answer_is_a_protocol_error() {
    let s = MockServer::start().await;
    let body =
        json!({"token": "plk_short", "token_meta": token_meta(7), "user": user("ada", false)});
    answer(
        &s,
        "POST",
        "/api/v1/session",
        ResponseTemplate::new(201).set_body_json(body),
    )
    .await;
    let e = client(&s).sign_in("ada", "hunter2").await.unwrap_err();
    assert!(matches!(e, TeamError::Protocol(_)), "{e:?}");
}

#[tokio::test]
async fn me_sends_the_bearer_token_and_lists_only_apinata_workspaces_sorted() {
    let s = MockServer::start().await;
    let base = s.uri();
    let listed = vec![
        workspace(&base, "web", "site", "apinata", "write"),
        workspace(&base, "data", "pg", "dbento", "admin"),
        workspace(&base, "core", "api", "apinata", "read"),
        workspace(&base, "core", "ops", "apinata", "owner"),
    ];
    Mock::given(method("GET"))
        .and(path("/api/v1/me"))
        .and(header("authorization", bearer().as_str()))
        .respond_with(me(false, listed))
        .mount(&s)
        .await;
    let got = client(&s).me(&token()).await.unwrap();
    assert_eq!(got.identity.name(), "Ada Lovelace");
    assert_eq!(got.workspaces.len(), 4);
    let rows: Vec<(String, Role)> = got
        .apinata_workspaces()
        .iter()
        .map(|w| (w.full_name(), w.role))
        .collect();
    assert_eq!(
        rows,
        [
            ("core/api".to_owned(), Role::Read),
            ("core/ops".to_owned(), Role::Read),
            ("web/site".to_owned(), Role::Write)
        ]
    );
}

#[tokio::test]
async fn each_error_code_maps_to_its_own_variant() {
    let s = MockServer::start().await;
    let cases: [(u16, serde_json::Value, TeamError); 9] = [
        (
            401,
            error("unauthenticated", "Sign in again.", None),
            TeamError::Unauthenticated,
        ),
        (
            403,
            error("forbidden", "Admins only.", None),
            TeamError::Forbidden("Admins only.".into()),
        ),
        (
            404,
            error("not_found", "No such workspace.", None),
            TeamError::NotFound("No such workspace.".into()),
        ),
        (
            409,
            error("conflict", "Taken.", None),
            TeamError::Conflict("Taken.".into()),
        ),
        (
            422,
            error("invalid_request", "Too long.", Some("slug")),
            TeamError::Invalid {
                field: Some("slug".into()),
                message: "Too long.".into(),
            },
        ),
        (
            400,
            error("invalid_request", "Not JSON.", None),
            TeamError::Invalid {
                field: None,
                message: "Not JSON.".into(),
            },
        ),
        (
            503,
            error("unavailable", "Busy.", None),
            TeamError::Unavailable,
        ),
        (
            500,
            error("internal", "Oops.", None),
            TeamError::Server {
                status: 500,
                code: "internal".into(),
                message: "Oops.".into(),
            },
        ),
        (
            418,
            error("teapot", "Short.", None),
            TeamError::Server {
                status: 418,
                code: "teapot".into(),
                message: "Short.".into(),
            },
        ),
    ];
    for (status, body, want) in cases {
        s.reset().await;
        answer(
            &s,
            "GET",
            "/api/v1/me",
            ResponseTemplate::new(status).set_body_json(body),
        )
        .await;
        assert_eq!(
            client(&s).me(&token()).await.unwrap_err(),
            want,
            "HTTP {status}"
        );
    }
}

#[tokio::test]
async fn a_rate_limit_carries_retry_after_and_a_bare_error_keeps_its_status() {
    let s = MockServer::start().await;
    let limited = ResponseTemplate::new(429)
        .insert_header("retry-after", "7")
        .set_body_json(error("rate_limited", "Slow down.", None));
    answer(&s, "POST", "/api/v1/session", limited).await;
    assert_eq!(
        client(&s).sign_in("ada", "x").await.unwrap_err(),
        TeamError::RateLimited {
            retry_after_secs: Some(7)
        }
    );
    answer(
        &s,
        "GET",
        "/api/v1/me",
        ResponseTemplate::new(502).set_body_string("<html>bad gateway</html>"),
    )
    .await;
    assert_eq!(
        client(&s).me(&token()).await.unwrap_err(),
        TeamError::Server {
            status: 502,
            code: String::new(),
            message: String::new()
        }
    );
}

#[tokio::test]
async fn a_rate_limit_reads_retry_after_given_as_an_http_date() {
    let s = MockServer::start().await;
    let at = chrono::Utc::now() + chrono::Duration::seconds(120);
    let date = at.format("%a, %d %b %Y %H:%M:%S GMT").to_string();
    let limited = ResponseTemplate::new(429)
        .insert_header("retry-after", date.as_str())
        .set_body_json(error("rate_limited", "Slow down.", None));
    answer(&s, "POST", "/api/v1/session", limited).await;
    let e = client(&s).sign_in("ada", "x").await.unwrap_err();
    let TeamError::RateLimited {
        retry_after_secs: Some(secs),
    } = e
    else {
        panic!("{e:?}");
    };
    assert!((100..=120).contains(&secs), "{secs}");
}

#[tokio::test]
async fn a_redirect_is_not_followed_so_the_token_stays_on_this_server() {
    let s = MockServer::start().await;
    let other = MockServer::start().await;
    answer(&other, "GET", "/api/v1/me", me(false, vec![])).await;
    let moved = ResponseTemplate::new(302)
        .insert_header("location", format!("{}/api/v1/me", other.uri()).as_str());
    answer(&s, "GET", "/api/v1/me", moved).await;
    let e = client(&s).me(&token()).await.unwrap_err();
    assert!(matches!(e, TeamError::Server { status: 302, .. }), "{e:?}");
    assert_eq!(hits(&other, "GET", "/api/v1/me").await, 0);
}

#[tokio::test]
async fn an_unreachable_server_is_offline_and_the_error_holds_no_token() {
    let server = ServerUrl::parse(&format!("http://127.0.0.1:{}", dead_port())).unwrap();
    let e = HttpTeamClient::new(server)
        .unwrap()
        .me(&token())
        .await
        .unwrap_err();
    assert!(matches!(e, TeamError::Offline(_)), "{e:?}");
    assert!(!format!("{e:?} {e}").contains("plk_"));
}

#[tokio::test]
async fn an_oversized_answer_is_refused() {
    let s = MockServer::start().await;
    answer(
        &s,
        "GET",
        "/api/v1/me",
        ResponseTemplate::new(200).set_body_string("x".repeat(2 * 1024 * 1024)),
    )
    .await;
    assert!(matches!(
        client(&s).me(&token()).await,
        Err(TeamError::Protocol(_))
    ));
}

#[tokio::test]
async fn revoke_deletes_the_token_by_id() {
    let s = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/tokens/7"))
        .and(header("authorization", bearer().as_str()))
        .respond_with(ResponseTemplate::new(204))
        .mount(&s)
        .await;
    client(&s).revoke(&token(), "7").await.unwrap();
    assert_eq!(hits(&s, "DELETE", "/api/v1/tokens/7").await, 1);
}

#[tokio::test]
async fn teams_and_create_workspace_use_the_admin_endpoints() {
    let s = MockServer::start().await;
    let teams = json!([{"slug": "core", "name": "Core", "members": ["ada"]}]);
    answer(
        &s,
        "GET",
        "/api/v1/teams",
        ResponseTemplate::new(200).set_body_json(teams),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/teams/core/workspaces"))
        .and(body_json(json!({"slug": "api-tests", "name": "API tests", "kind": "apinata", "default_branch": "main", "seed": true})))
        .respond_with(ResponseTemplate::new(201).set_body_json(workspace(&s.uri(), "core", "api-tests", "apinata", "admin")))
        .mount(&s)
        .await;
    let api = client(&s);
    let got = api.teams(&token()).await.unwrap();
    assert_eq!((got[0].slug.as_str(), got[0].members.len()), ("core", 1));
    let new = NewWorkspace::new("core", "api-tests", "API tests").unwrap();
    let made = api.create_workspace(&token(), &new).await.unwrap();
    assert_eq!(
        (made.full_name(), made.role),
        ("core/api-tests".to_owned(), Role::Admin)
    );
}

#[tokio::test]
async fn a_server_under_a_path_prefix_gets_its_api_under_that_prefix() {
    let s = MockServer::start().await;
    answer(&s, "GET", "/potluck/api/v1/server", info(true)).await;
    let api =
        HttpTeamClient::new(ServerUrl::parse(&format!("{}/potluck", s.uri())).unwrap()).unwrap();
    assert!(api.server_info().await.unwrap().password_sign_in);
}
