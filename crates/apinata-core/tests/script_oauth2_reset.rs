//! `bru.resetOauth2Credential`: forgetting a credentials id now and in the persisted cache.

use crate::script_support;

use apinata_core::{
    Collection, ContextOptions, ExecOptions, MemoryStore, Outcome, RequestFile, SecretStore,
    TokenCache, VarContext,
};
use script_support::*;
use std::sync::Arc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "tok-58-SECRET";

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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reset_forces_a_new_token() {
    // Without the reset the second request would reuse the cached token: one fetch, not two.
    let s = provider(2).await;
    let post = "bru.resetOauth2Credential('main');\nbru.setVar('after', String(bru.getOauth2CredentialVar('$oauth2.main.access_token')));";
    let tests_code = "test('gone', () => expect(bru.getVar('after')).to.equal('undefined'));";
    let scripts = script("after-response", post) + &script("tests", tests_code);
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &oauth("A", 1, &s.uri(), "main", &scripts)),
        ("b.yml", &oauth("B", 2, &s.uri(), "main", "")),
    ]);
    let report = run_dir(dir.path(), on()).await;
    let outcomes: Vec<Outcome> = report.results.iter().map(|r| r.outcome).collect();
    assert_eq!(outcomes, [Outcome::Passed, Outcome::Passed]);
    assert_eq!(tests(&report.results[0]), ["tests gone: pass"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bad_credentials_id_throws() {
    let s = MockServer::start().await;
    Mock::given(path("/r"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let code = "for (const v of ['', undefined, 5]) {\n  try { bru.resetOauth2Credential(v); console.log('kept'); } catch (e) { console.log(e instanceof Error, e.message); }\n}";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/r", s.uri()),
                &script("before-request", code),
                "",
            ),
        ),
    ]);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    let line = "log:true bru.resetOauth2Credential: credentialsId must be a non-empty string";
    assert_eq!(console(&result), [line, line, line]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reset_is_persisted() {
    let s = provider(2).await;
    let keys: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
    let store = tempfile::TempDir::new().unwrap();
    let file = store.path().join("tokens.json");
    let cache = TokenCache::open(file.clone(), keys.clone());
    let reset = script("after-response", "bru.resetOauth2Credential('main');");
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("keep.yml", &oauth("K", 1, &s.uri(), "keep", "")),
        ("r.yml", &oauth("R", 2, &s.uri(), "main", &reset)),
    ]);
    for rel in ["keep.yml", "r.yml"] {
        let ctx = ContextOptions::new().oauth2_tokens(cache.clone());
        let (result, _) = send_full(dir.path(), rel, on(), ctx, ExecOptions::new()).await;
        assert_eq!(result.outcome, Outcome::Passed, "{rel}: {:?}", result.error);
    }
    // A fresh cache over the same file sees the kept id and not the reset one.
    let again = TokenCache::open(file, keys);
    again.load().await;
    let c = Collection::open(dir.path()).unwrap();
    let ctx = VarContext::new(&c, ContextOptions::new().oauth2_tokens(again)).unwrap();
    let req = RequestFile::load(dir.path().join("r.yml")).unwrap();
    let vars = ctx.variables(&req);
    assert!(vars.get("$oauth2.keep.access_token").is_some());
    assert!(vars.get("$oauth2.main.access_token").is_none());
}
