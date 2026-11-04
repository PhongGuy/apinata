//! `bru.getOauth2CredentialVar`: the `$oauth2` values a script reads.

use crate::script_support;

use apinata_core::{ContextOptions, ExecOptions, Outcome, VarContext};
use script_support::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "tok-31-SECRET";

/// A client-credentials request on `p`'s `/token` under credentials id `id`.
fn oauth(name: &str, seq: u32, p: &str, id: &str, scripts: &str) -> String {
    let runtime = if scripts.is_empty() {
        String::new()
    } else {
        format!("runtime:\n  scripts:\n{scripts}")
    };
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{p}/api\"\n  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{p}/token\"\n    credentials:\n      clientId: app\n    scope: read\n    tokenConfig:\n      id: {id}\n      placement:\n        header: Bearer\n{runtime}"
    )
}

async fn provider(times: u64) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": TOKEN, "token_type": "Bearer", "expires_in": 3600
        })))
        .expect(times)
        .mount(&s)
        .await;
    Mock::given(path("/api"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    s
}

fn var(ctx: &VarContext, name: &str) -> Option<String> {
    ctx.runtime_vars()
        .entries()
        .iter()
        .find(|v| v.name() == name)
        .and_then(|v| v.value().map(str::to_owned))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_script_reads_the_cached_token_fields() {
    let s = provider(1).await;
    let pre = "bru.setVar('pre', String(bru.getOauth2CredentialVar('$oauth2.main.access_token')));";
    let post = "bru.setVar('tok', bru.getOauth2CredentialVar('$oauth2.main.access_token'));\nbru.setVar('type', bru.getOauth2CredentialVar('$oauth2.main.token_type'));\nbru.setVar('exp', typeof bru.getOauth2CredentialVar('$oauth2.main.expires_in'));\nbru.setVar('other', String(bru.getOauth2CredentialVar('$oauth2.other.access_token')));\nbru.setVar('bare', String(bru.getOauth2CredentialVar('access_token')));";
    let scripts = script("before-request", pre) + &script("after-response", post);
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &oauth("R", 1, &s.uri(), "main", &scripts)),
    ]);
    let (result, ctx) = send_full(
        dir.path(),
        "r.yml",
        on(),
        ContextOptions::new(),
        ExecOptions::new(),
    )
    .await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    // Before the token is fetched there is nothing to read.
    assert_eq!(var(&ctx, "pre").as_deref(), Some("undefined"));
    assert_eq!(var(&ctx, "tok").as_deref(), Some(TOKEN));
    assert_eq!(var(&ctx, "type").as_deref(), Some("Bearer"));
    assert_eq!(var(&ctx, "exp").as_deref(), Some("number"));
    assert_eq!(var(&ctx, "other").as_deref(), Some("undefined"));
    assert_eq!(var(&ctx, "bare").as_deref(), Some("undefined"));
    let sensitive = result.script_sensitive_values();
    assert!(sensitive.contains(&TOKEN.to_owned()), "{sensitive:?}");
    assert!(!sensitive.contains(&"Bearer".to_owned()), "{sensitive:?}");
}
