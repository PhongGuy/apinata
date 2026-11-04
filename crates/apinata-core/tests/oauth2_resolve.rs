//! Placing a cached OAuth 2.0 token in `resolve`, the expired-token warning, and the
//! `$oauth2.<id>.<field>` variables.

use crate::oauth2_support;

use apinata_core::{AuthorizeOptions, ResolveError, VarScope, Warning, resolve};
use oauth2_support::*;
use serde_json::json;
use wiremock::MockServer;

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// A client-credentials block on `provider` with its own `tokenConfig` lines.
fn cc_with(provider: &str, token_config: &str) -> String {
    format!(
        "    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{provider}/token\"\n    credentials:\n      clientId: app\n    scope: read\n{token_config}"
    )
}

async fn provider(fields: serde_json::Value) -> MockServer {
    let server = MockServer::start().await;
    on_grant(&server, "client_credentials", token_response(fields), 1).await;
    server
}

#[tokio::test]
async fn a_header_placement_sends_the_prefixed_token_as_a_secret() {
    let server = provider(json!({"expires_in": 3600})).await;
    let f = fixture(&server, &[("a.yml", "")]);
    f.prepare("a.yml", &AuthorizeOptions::new()).await.unwrap();
    let r = resolve(f.request("a.yml"), &f.ctx).unwrap();
    assert_eq!(
        header(r.headers(), "authorization"),
        Some(format!("Bearer {TOKEN}").as_str())
    );
    assert!(r.sensitive_values().contains(&TOKEN.to_owned()));
    assert!(r.warnings().is_empty(), "{:?}", r.warnings());
}

#[tokio::test]
async fn unset_and_custom_prefixes_and_the_query_placement() {
    let server = provider(json!({})).await;
    let p = server.uri();
    let unset = cc_with(&p, "    tokenConfig:\n      id: main\n");
    let custom = cc_with(
        &p,
        "    tokenConfig:\n      id: main\n      placement:\n        header: Token\n",
    );
    let query = cc_with(
        &p,
        "    tokenConfig:\n      id: main\n      placement:\n        query: access\n",
    );
    let (u, c, q) = (
        request("u", 1, &format!("{p}/r"), &unset),
        request("c", 2, &format!("{p}/r"), &custom),
        request("q", 3, &format!("{p}/r?access=old&x=1"), &query),
    );
    let f = Fixture::new(&[
        ("opencollection.yml", ROOT),
        ("u.yml", &u),
        ("c.yml", &c),
        ("q.yml", &q),
    ]);
    f.prepare("u.yml", &AuthorizeOptions::new()).await.unwrap();
    let at = |rel: &str| resolve(f.request(rel), &f.ctx).unwrap();
    assert_eq!(header(at("u.yml").headers(), "authorization"), Some(TOKEN));
    assert_eq!(
        header(at("c.yml").headers(), "authorization"),
        Some(format!("Token {TOKEN}").as_str())
    );
    let placed = at("q.yml");
    assert_eq!(placed.url(), format!("{p}/r?access={TOKEN}&x=1"));
    assert_eq!(header(placed.headers(), "authorization"), None);
}

#[tokio::test]
async fn the_id_token_source_sends_the_id_token_and_needs_one() {
    let id_source = |p: &str| {
        cc_with(
            p,
            "    tokenConfig:\n      id: main\n      placement:\n        header: Bearer\n      source: id_token\n",
        )
    };
    let with_id = provider(json!({"id_token": "idt-1"})).await;
    let f = Fixture::new(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &request(
                "a",
                1,
                &format!("{}/r", with_id.uri()),
                &id_source(&with_id.uri()),
            ),
        ),
    ]);
    f.prepare("a.yml", &AuthorizeOptions::new()).await.unwrap();
    assert_eq!(
        header(
            resolve(f.request("a.yml"), &f.ctx).unwrap().headers(),
            "authorization"
        ),
        Some("Bearer idt-1")
    );
    let without = provider(json!({})).await;
    let g = Fixture::new(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &request(
                "a",
                1,
                &format!("{}/r", without.uri()),
                &id_source(&without.uri()),
            ),
        ),
    ]);
    g.prepare("a.yml", &AuthorizeOptions::new()).await.unwrap();
    assert_eq!(
        resolve(g.request("a.yml"), &g.ctx).unwrap_err(),
        ResolveError::OAuth2NoToken {
            credentials_id: "main".into()
        }
    );
}

#[test]
fn without_a_cached_token_resolve_fails() {
    let text = request(
        "a",
        1,
        "http://127.0.0.1:9/r",
        &client_credentials("http://127.0.0.1:9", ""),
    );
    let f = Fixture::new(&[("opencollection.yml", ROOT), ("a.yml", &text)]);
    assert_eq!(
        resolve(f.request("a.yml"), &f.ctx).unwrap_err(),
        ResolveError::OAuth2NoToken {
            credentials_id: "main".into()
        }
    );
}

#[test]
fn an_empty_query_key_is_reported_by_resolve() {
    let auth = cc_with(
        "http://127.0.0.1:9",
        "    tokenConfig:\n      placement:\n        query: \"\"\n",
    );
    let f = Fixture::new(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1, "http://127.0.0.1:9/r", &auth)),
    ]);
    assert_eq!(
        resolve(f.request("a.yml"), &f.ctx).unwrap_err(),
        ResolveError::OAuth2MissingField {
            field: "tokenConfig.placement.query"
        }
    );
}

#[tokio::test]
async fn an_expired_token_kept_by_policy_is_sent_with_a_warning() {
    let server = provider(json!({"expires_in": 1})).await;
    let f = fixture(
        &server,
        &[("a.yml", ""), ("b.yml", &settings(false, false))],
    );
    f.prepare("a.yml", &AuthorizeOptions::new()).await.unwrap();
    f.prepare("b.yml", &AuthorizeOptions::new()).await.unwrap();
    let r = resolve(f.request("b.yml"), &f.ctx).unwrap();
    assert_eq!(
        header(r.headers(), "authorization"),
        Some(format!("Bearer {TOKEN}").as_str())
    );
    assert!(
        r.warnings().contains(&Warning::OAuth2TokenExpired),
        "{:?}",
        r.warnings()
    );
}

#[tokio::test]
async fn an_inherited_oauth2_block_places_the_token() {
    let server = provider(json!({"expires_in": 3600})).await;
    let p = server.uri();
    let folder = format!(
        "info:\n  name: f\n  type: folder\nrequest:\n  auth:\n{}",
        client_credentials(&p, "")
    );
    let child = format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{p}/r\"\n  auth: inherit\n"
    );
    let f = Fixture::new(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder),
        ("f/r.yml", &child),
    ]);
    f.prepare("f/r.yml", &AuthorizeOptions::new())
        .await
        .unwrap();
    let r = resolve(f.request("f/r.yml"), &f.ctx).unwrap();
    assert_eq!(
        header(r.headers(), "authorization"),
        Some(format!("Bearer {TOKEN}").as_str())
    );
}

#[tokio::test]
async fn oauth2_variables_expose_the_cached_fields() {
    let server = provider(json!({"expires_in": 3600, "scope": "read"})).await;
    let p = server.uri();
    let plain = format!(
        "info:\n  name: v\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: \"{p}/v\"\n  headers:\n    - name: X-Token\n      value: \"{{{{$oauth2.main.access_token}}}}\"\n    - name: X-Type\n      value: \"{{{{$oauth2.main.token_type}}}}\"\n"
    );
    let a = request("a", 1, &format!("{p}/r"), &client_credentials(&p, ""));
    let f = Fixture::new(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &a),
        ("v.yml", &plain),
    ]);
    assert!(matches!(
        resolve(f.request("v.yml"), &f.ctx),
        Err(ResolveError::UndefinedVariable { .. })
    ));
    f.prepare("a.yml", &AuthorizeOptions::new()).await.unwrap();
    let r = resolve(f.request("v.yml"), &f.ctx).unwrap();
    assert_eq!(header(r.headers(), "x-token"), Some(TOKEN));
    assert_eq!(header(r.headers(), "x-type"), Some("Bearer"));
    assert_eq!(r.sensitive_values(), [TOKEN.to_owned()]);
    let vars = f.ctx.variables(f.request("v.yml"));
    let token = vars.get("$oauth2.main.access_token").expect("defined");
    assert_eq!(
        (token.scope(), token.is_secret(), token.value()),
        (VarScope::OAuth2, true, None)
    );
    let created = vars.get("$oauth2.main.created_at").expect("defined");
    assert_eq!(
        (created.scope(), created.is_secret()),
        (VarScope::OAuth2, false)
    );
    assert!(
        created.value().is_some_and(|v| v.parse::<u64>().is_ok()),
        "{created:?}"
    );
    assert!(
        vars.names()
            .contains(&"$oauth2.main.access_token".to_owned())
    );
}

#[tokio::test]
async fn a_secret_in_the_header_prefix_is_redacted() {
    let server = provider(json!({})).await;
    let p = server.uri();
    let auth = cc_with(
        &p,
        "    tokenConfig:\n      id: main\n      placement:\n        header: \"{{process.env.PFX}}\"\n",
    );
    let a = request("a", 1, &format!("{p}/r"), &auth);
    let f = Fixture::new(&[
        ("opencollection.yml", ROOT),
        (".env", "PFX=pfx-SECRET\n"),
        ("a.yml", &a),
    ]);
    f.prepare("a.yml", &AuthorizeOptions::new()).await.unwrap();
    let r = resolve(f.request("a.yml"), &f.ctx).unwrap();
    assert_eq!(
        header(r.headers(), "authorization"),
        Some(format!("pfx-SECRET {TOKEN}").as_str())
    );
    assert!(
        r.sensitive_values().contains(&"pfx-SECRET".to_owned()),
        "{:?}",
        r.sensitive_values()
    );
}
