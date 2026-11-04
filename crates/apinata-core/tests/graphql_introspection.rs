//! The introspection request: resolved like Send, the literal query POSTed, the answer judged.

use crate::common;

use std::time::Duration;

use apinata_core::{
    Body, CancelHandle, ExecError, ExecOptions, Executor, INTROSPECTION_QUERY, ResolveError,
    ResolvedRequest, SCHEMA_BODY_LIMIT, SCHEMA_TIMEOUT, SchemaError, fetch_schema,
    introspection_request,
};
use common::{request, setup};
use sha2::{Digest, Sha256};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nrequest:\n  headers:\n    - name: X-C\n      value: c\n";
const FOLDER: &str = "info:\n  name: f\n  type: folder\n  seq: 1\nrequest:\n  headers:\n    - name: X-F\n      value: f\n";
/// Headers the forced ones replace, bearer auth, and a body that fails if it is read.
const UNDER: &str = "  method: GET\n  headers:\n    - name: content-type\n      value: text/plain\n    - name: ACCEPT\n      value: text/html\n    - name: X-R\n      value: r\n  auth:\n    type: bearer\n    token: t0k\n  body:\n    query: '{ me(id: \"{{nope}}\") }'\n    variables: '{not json'\n";

fn fixture() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/graphql/schema.json"
    ))
    .unwrap()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn expected_body() -> String {
    format!(
        "{{\"query\":{}}}",
        serde_json::to_string(INTROSPECTION_QUERY).unwrap()
    )
}

fn executor(limit: u64) -> Executor {
    Executor::new(ExecOptions::new().max_body_bytes(Some(limit))).expect("client")
}

/// Resolves `f/g.yml`, a GraphQL request at `url` inside a folder with its own header.
fn introspect(url: &str, under: &str, top: &str) -> Result<ResolvedRequest, SchemaError> {
    let file = format!(
        "info:\n  name: g\n  type: graphql\n  seq: 1\ngraphql:\n  url: \"{url}\"\n{under}{top}"
    );
    let (_d, c, ctx) = setup(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", FOLDER),
        ("f/g.yml", &file),
    ]);
    introspection_request(request(&c, "f/g.yml"), &ctx)
}

fn values<'a>(r: &'a ResolvedRequest, name: &str) -> Vec<&'a str> {
    r.headers()
        .iter()
        .filter(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
        .collect()
}

#[test]
fn the_query_is_the_graphql_js_default_text() {
    assert_eq!(INTROSPECTION_QUERY.len(), 1905);
    assert!(INTROSPECTION_QUERY.starts_with("\n    query IntrospectionQuery {\n"));
    assert_eq!(
        hex(&Sha256::digest(INTROSPECTION_QUERY.as_bytes())),
        "463001cc6b5737dd586d4295e25644c7d59b199c2a1a6ea6be8f0fe907e413d5"
    );
}

#[test]
fn the_request_is_a_post_of_the_literal_query_with_forced_json_headers() {
    let r = introspect("http://x.test/graphql", UNDER, "").expect("resolves");
    assert_eq!(r.method(), "POST");
    assert_eq!(r.body(), &Body::Text(expected_body()));
    assert_eq!(expected_body().len(), 2023);
    assert_eq!(values(&r, "content-type"), ["application/json"]);
    assert_eq!(values(&r, "accept"), ["application/json"]);
    assert_eq!(values(&r, "x-c"), ["c"]);
    assert_eq!(values(&r, "x-f"), ["f"]);
    assert_eq!(values(&r, "x-r"), ["r"]);
    assert_eq!(values(&r, "authorization"), ["Bearer t0k"]);
    assert_eq!(r.timeout(), Some(SCHEMA_TIMEOUT));
    let own = introspect("http://x.test/graphql", "", "settings:\n  timeout: 2500\n").unwrap();
    assert_eq!(own.timeout(), Some(Duration::from_millis(2500)));
}

#[test]
fn unresolvable_urls_http_requests_and_missing_tokens_fail_before_sending() {
    let undefined = introspect("{{host}}/graphql", "", "").unwrap_err();
    assert!(
        matches!(undefined, SchemaError::Resolve(ResolveError::UndefinedVariable { ref name, .. }) if name == "host")
    );
    let oauth2 = "  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"http://127.0.0.1:9/token\"\n    credentials:\n      clientId: app\n      clientSecret: s\n      placement: body\n    scope: read\n    tokenConfig:\n      id: main\n      placement:\n        header: Bearer\n";
    let no_token = introspect("http://x.test/graphql", oauth2, "").unwrap_err();
    assert!(
        matches!(no_token, SchemaError::Resolve(ResolveError::OAuth2NoToken { ref credentials_id }) if credentials_id == "main")
    );
    let http = "info:\n  name: h\n  type: http\nhttp:\n  method: GET\n  url: http://x.test/\n";
    let (_d, c, ctx) = setup(&[("opencollection.yml", ROOT), ("h.yml", http)]);
    assert!(matches!(
        introspection_request(request(&c, "h.yml"), &ctx),
        Err(SchemaError::NotGraphql)
    ));
}

async fn serve(response: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(response)
        .mount(&server)
        .await;
    server
}

async fn load(
    server: &MockServer,
    top: &str,
    limit: u64,
) -> Result<apinata_core::Schema, SchemaError> {
    let r = introspect(&format!("{}/graphql", server.uri()), UNDER, top).expect("resolves");
    fetch_schema(&executor(limit), &r, &CancelHandle::new()).await
}

#[tokio::test]
async fn the_server_sees_exactly_the_body_and_headers() {
    let server = serve(ResponseTemplate::new(200).set_body_string(fixture())).await;
    let schema = load(&server, "", SCHEMA_BODY_LIMIT)
        .await
        .expect("a schema");
    assert_eq!(schema.query_type(), Some("Query"));
    let seen = server.received_requests().await.unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].body, expected_body().into_bytes());
    let body: serde_json::Value = serde_json::from_slice(&seen[0].body).unwrap();
    assert_eq!(
        body.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["query"]
    );
    let header = |name: &str| {
        seen[0]
            .headers
            .get(name)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(header("content-type"), "application/json");
    assert_eq!(header("accept"), "application/json");
    assert_eq!(header("x-c"), "c");
    assert_eq!(header("x-f"), "f");
    assert_eq!(header("authorization"), "Bearer t0k");
}

#[tokio::test]
async fn bad_answers_map_to_their_errors() {
    let status = serve(ResponseTemplate::new(500).set_body_string(fixture())).await;
    let err = load(&status, "", SCHEMA_BODY_LIMIT).await.unwrap_err();
    assert!(matches!(err, SchemaError::Status { status: 500, .. }));
    assert_eq!(
        err.to_string(),
        "the server answered 500 Internal Server Error"
    );
    let html = serve(ResponseTemplate::new(200).set_body_string("<html></html>")).await;
    assert!(matches!(
        load(&html, "", SCHEMA_BODY_LIMIT).await,
        Err(SchemaError::NotJson { .. })
    ));
    let errors =
        serve(ResponseTemplate::new(200).set_body_string("{\"errors\":[{\"message\":\"no\"}]}"))
            .await;
    assert!(matches!(
        load(&errors, "", SCHEMA_BODY_LIMIT).await,
        Err(SchemaError::NoData)
    ));
    let big = serve(ResponseTemplate::new(200).set_body_string(fixture())).await;
    assert!(
        matches!(load(&big, "", 64).await, Err(SchemaError::TooLarge { limit }) if limit == SCHEMA_BODY_LIMIT)
    );
}

#[tokio::test]
async fn a_slow_server_times_out() {
    let slow = serve(
        ResponseTemplate::new(200)
            .set_body_string(fixture())
            .set_delay(Duration::from_secs(3)),
    )
    .await;
    let err = load(&slow, "settings:\n  timeout: 200\n", SCHEMA_BODY_LIMIT)
        .await
        .unwrap_err();
    assert!(
        matches!(err, SchemaError::Send(ExecError::Timeout { .. })),
        "{err:?}"
    );
}

#[test]
fn schema_loads_ignore_the_context_default() {
    let inherit =
        "info:\n  name: g\n  type: graphql\n  seq: 1\ngraphql:\n  url: \"http://x.test/graphql\"\n";
    let own = format!("{inherit}settings:\n  timeout: 2500\n");
    let (_d, c, ctx) = setup(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", FOLDER),
        ("f/g.yml", inherit),
        ("f/own.yml", &own),
    ]);
    let ctx = ctx.with_default_timeout(Some(Duration::from_secs(5)));
    let r = introspection_request(request(&c, "f/g.yml"), &ctx).expect("resolves");
    assert_eq!(r.timeout(), Some(SCHEMA_TIMEOUT));
    let r = introspection_request(request(&c, "f/own.yml"), &ctx).expect("resolves");
    assert_eq!(r.timeout(), Some(Duration::from_millis(2500)));
}
