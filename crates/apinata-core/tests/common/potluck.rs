//! A stub of the Potluck JSON API: canned objects and mounting helpers.

use apinata_core::GitOptions;
use apinata_core::team::{HttpTeamClient, Identity, ServerUrl, TeamAuth, TeamToken, team_options};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

pub fn tok() -> String {
    format!("plk_{}", "A".repeat(43))
}

pub fn token() -> TeamToken {
    TeamToken::parse(&tok()).unwrap()
}

/// `base` with the team git env for `server`, as the app builds it for ada.
pub fn team_opts(base: GitOptions, server: &ServerUrl) -> GitOptions {
    let identity = Identity::new("Ada Lovelace", "ada@users.noreply.127.0.0.1");
    team_options(base, &TeamAuth::new(server.as_str(), tok()), &identity)
}

pub fn client(s: &MockServer) -> HttpTeamClient {
    HttpTeamClient::new(ServerUrl::parse(&s.uri()).unwrap()).unwrap()
}

pub fn user(name: &str, admin: bool) -> Value {
    json!({"username": name, "display_name": "Ada Lovelace", "email": null,
           "server_admin": admin, "disabled": false, "created_at": "2026-11-15T10:00:00Z"})
}

pub fn identity() -> Value {
    json!({"name": "Ada Lovelace", "email": "ada@users.noreply.127.0.0.1"})
}

pub fn token_meta(id: i64) -> Value {
    json!({"id": id, "name": "Apinata", "prefix": "plk_AAAA",
           "created_at": "2026-11-15T10:00:00Z", "last_used_at": null, "expires_at": null})
}

pub fn workspace(base: &str, team: &str, slug: &str, kind: &str, role: &str) -> Value {
    json!({"team": team, "slug": slug, "name": format!("{team} {slug}"), "kind": kind,
           "default_branch": "main", "role": role,
           "git_url": format!("{base}/git/{team}/{slug}.git"),
           "events_url": format!("{base}/api/v1/workspaces/{team}/{slug}/events"),
           "policy": {"later": true}})
}

pub fn error(code: &str, message: &str, field: Option<&str>) -> Value {
    json!({"error": {"code": code, "message": message, "field": field}})
}

pub fn info(password: bool) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(
        json!({"version": "0.1.0", "api": 1, "auth": {"password": password, "oidc": false}}),
    )
}

pub fn session(id: i64, admin: bool) -> ResponseTemplate {
    ResponseTemplate::new(201).set_body_json(
        json!({"token": tok(), "token_meta": token_meta(id), "user": user("ada", admin)}),
    )
}

pub fn me(admin: bool, workspaces: Vec<Value>) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(
        json!({"user": user("ada", admin), "identity": identity(), "workspaces": workspaces}),
    )
}

pub async fn answer(s: &MockServer, verb: &str, at: &str, response: ResponseTemplate) {
    Mock::given(method(verb))
        .and(path(at))
        .respond_with(response)
        .mount(s)
        .await;
}

/// How many requests reached `verb at`.
pub async fn hits(s: &MockServer, verb: &str, at: &str) -> usize {
    let all = s.received_requests().await.unwrap();
    all.iter()
        .filter(|r| r.method.as_str() == verb && r.url.path() == at)
        .count()
}
