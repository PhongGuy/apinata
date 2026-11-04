//! Postman oauth2, awsv4 and digest auth, imported, written, loaded and sent against a mock
//! server on 127.0.0.1: the written files must carry credentials the executor can use.

use apinata_core::import::{ImportReport, NoteKind, parse_postman, write_collection};
use apinata_core::{
    AuthorizeOptions, CancelHandle, Collection, ContextOptions, ExecOptions, Executor, ItemPath,
    MemoryStore, TokenCache, VarContext, resolve,
};
use md5::{Digest as _, Md5};
use std::fs;
use std::path::Path;
use tempfile::TempDir;
use wiremock::matchers::{body_string_contains, header, header_regex, method, path, query_param};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// Import fixture `name` with its `HOST` placeholder pointing at `server`, write it and load it.
fn import(name: &str, server: &MockServer) -> (TempDir, Collection, ImportReport) {
    let text = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/postman/own/{name}")),
    )
    .expect("fixture")
    .replace("HOST", server.uri().trim_start_matches("http://"));
    let imported = parse_postman(text.as_bytes()).expect("parses");
    let parent = TempDir::new().expect("tempdir");
    let (written, report) =
        write_collection(parent.path(), &imported, &[], &MemoryStore::new()).expect("writes");
    let collection = Collection::open(written.root()).expect("opens");
    assert!(
        collection.load_errors().is_empty(),
        "{:?}",
        collection.load_errors()
    );
    (parent, collection, report)
}

fn context(c: &Collection) -> VarContext {
    VarContext::new(
        c,
        ContextOptions::new().oauth2_tokens(TokenCache::in_memory()),
    )
    .expect("context")
}

fn executor() -> Executor {
    Executor::new(ExecOptions::new()).expect("client")
}

#[tokio::test]
async fn imported_oauth2_client_credentials_fetches_a_token_and_sends_it() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=client_credentials"))
        .and(body_string_contains("client_id=app-id"))
        .and(body_string_contains("client_secret=app-secret"))
        .and(body_string_contains("audience=api"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({"access_token": "tok-123", "token_type": "Bearer", "expires_in": 3600}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(header("authorization", "Bearer tok-123"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let (_dir, c, report) = import("auth-oauth2.json", &server);
    assert!(
        report.notes.iter().any(|n| n.kind == NoteKind::Dropped),
        "the disabled param is noted: {:?}",
        report.notes
    );
    let ctx = context(&c);
    let file = c
        .request(&ItemPath::new("Client credentials.yml").expect("path"))
        .expect("request");
    executor()
        .prepare_auth(file, &ctx, &AuthorizeOptions::new(), &CancelHandle::new())
        .await
        .expect("token fetched");
    let res = executor()
        .send(&resolve(file, &ctx).expect("resolves"))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200);
    // The fixture's disabled `X-Old` header param must never reach the token endpoint.
    let sent = server.received_requests().await.expect("recorded");
    let token_calls: Vec<_> = sent.iter().filter(|r| r.url.path() == "/token").collect();
    assert_eq!(token_calls.len(), 1);
    assert!(token_calls[0].headers.get("x-old").is_none());
}

#[tokio::test]
async fn imported_oauth2_query_placement_sends_the_token_in_the_url() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({"access_token": "tok-q", "token_type": "Bearer", "expires_in": 3600}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api-query"))
        .and(query_param("access_token", "tok-q"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let (_dir, c, _) = import("auth-oauth2.json", &server);
    let ctx = context(&c);
    let file = c
        .request(&ItemPath::new("Client credentials query.yml").expect("path"))
        .expect("request");
    executor()
        .prepare_auth(file, &ctx, &AuthorizeOptions::new(), &CancelHandle::new())
        .await
        .expect("token fetched");
    let res = executor()
        .send(&resolve(file, &ctx).expect("resolves"))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn imported_awsv4_signs_the_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/signed"))
        .and(header_regex(
            "authorization",
            r"^AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/.*/eu-west-1/execute-api/aws4_request",
        ))
        .and(header("x-amz-security-token", "session-token-value"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let (_dir, c, _) = import("auth-awsv4.json", &server);
    let ctx = context(&c);
    let file = c
        .request(&ItemPath::new("Signed request.yml").expect("path"))
        .expect("request");
    let res = executor()
        .send(&resolve(file, &ctx).expect("resolves"))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200);
}

fn hex_md5(s: &str) -> String {
    Md5::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The value of `key` in a digest `Authorization` header.
fn field(auth: &str, key: &str) -> String {
    let start = auth.find(&format!("{key}=\"")).expect("field") + key.len() + 2;
    auth[start..].split('"').next().expect("value").to_owned()
}

/// Challenges, then answers 200 only when the response was computed from the imported password.
struct DigestServer;

impl Respond for DigestServer {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let challenge = ResponseTemplate::new(401).insert_header(
            "www-authenticate",
            r#"Digest realm="r", nonce="n0", qop="auth""#,
        );
        let Some(auth) = req
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .filter(|a| a.starts_with("Digest "))
        else {
            return challenge;
        };
        let ha1 = hex_md5("digest-user:r:digest-pass");
        let ha2 = hex_md5(&format!("{}:{}", req.method, field(auth, "uri")));
        let want = hex_md5(&format!(
            "{ha1}:n0:{}:{}:auth:{ha2}",
            field(auth, "nc"),
            field(auth, "cnonce")
        ));
        if field(auth, "username") == "digest-user" && field(auth, "response") == want {
            ResponseTemplate::new(200)
        } else {
            challenge
        }
    }
}

#[tokio::test]
async fn imported_digest_answers_the_challenge_with_the_imported_password() {
    let server = MockServer::start().await;
    Mock::given(path("/digest"))
        .respond_with(DigestServer)
        .mount(&server)
        .await;
    let (_dir, c, _) = import("auth-digest.json", &server);
    let ctx = context(&c);
    let file = c
        .request(&ItemPath::new("Digest protected.yml").expect("path"))
        .expect("request");
    let res = executor()
        .send(&resolve(file, &ctx).expect("resolves"))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200);
    let statuses: Vec<u16> = res.timeline().iter().map(|h| h.status).collect();
    assert_eq!(statuses, [401, 200]);
}
