//! `introspect`: a GraphQL request's schema fetched as Send would send it, answered with the
//! JSON text and the parsed schema, or a typed error.

use crate::common;
use crate::oauth2_support;

use std::sync::Arc;

use apinata_core::{
    CancelHandle, ExecError, IntrospectError, IntrospectOptions, OAuth2Error, Schema, SchemaError,
    introspect,
};
use common::setup;
use oauth2_support::{
    Fixture, ROOT, TOKEN, client_credentials, executor, on_grant, token_response,
};
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fixture_json() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/graphql/schema.json"
    ))
    .unwrap()
}

fn graphql_file(url: &str, auth: &str) -> String {
    format!("info:\n  name: g\n  type: graphql\n  seq: 1\ngraphql:\n  url: \"{url}\"\n{auth}")
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

/// Introspects `g.yml`, a GraphQL request at `url`, with `options`.
async fn fetch(
    url: &str,
    options: &IntrospectOptions,
) -> Result<apinata_core::Introspection, SchemaError> {
    let (_d, c, ctx) = setup(&[
        ("opencollection.yml", ROOT),
        ("g.yml", &graphql_file(url, "")),
    ]);
    let req = common::request(&c, "g.yml");
    introspect(&executor(), req, &ctx, options, &CancelHandle::new())
        .await
        .map_err(IntrospectError::into_error)
}

#[tokio::test]
async fn a_schema_answer_returns_its_text_and_the_parsed_schema() {
    let server = serve(ResponseTemplate::new(200).set_body_string(fixture_json())).await;
    let got = fetch(
        &format!("{}/graphql", server.uri()),
        &IntrospectOptions::new(),
    )
    .await
    .expect("a schema");
    assert_eq!(got.text(), fixture_json());
    assert_eq!(got.schema(), &Schema::from_json(&fixture_json()).unwrap());
    assert_eq!(got.into_schema().query_type(), Some("Query"));
}

#[tokio::test]
async fn an_http_error_status_is_a_status_error() {
    let server = serve(ResponseTemplate::new(503).set_body_string(fixture_json())).await;
    let err = fetch(
        &format!("{}/graphql", server.uri()),
        &IntrospectOptions::new(),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, SchemaError::Status { status: 503, .. }),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_graphql_errors_answer_names_each_message() {
    let body = r#"{"errors":[{"message":"introspection is disabled"},{"message":"try later"}]}"#;
    let server = serve(ResponseTemplate::new(200).set_body_string(body)).await;
    let err = fetch(
        &format!("{}/graphql", server.uri()),
        &IntrospectOptions::new(),
    )
    .await
    .unwrap_err();
    match &err {
        SchemaError::Graphql { messages } => {
            assert_eq!(messages, &["introspection is disabled", "try later"]);
        }
        other => panic!("expected GraphQL errors, got {other:?}"),
    }
    assert_eq!(
        err.to_string(),
        "the server answered with GraphQL errors: introspection is disabled; try later"
    );
}

#[tokio::test]
async fn a_non_json_answer_is_not_json() {
    let server = serve(ResponseTemplate::new(200).set_body_string("<html>hi</html>")).await;
    let err = fetch(
        &format!("{}/graphql", server.uri()),
        &IntrospectOptions::new(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, SchemaError::NotJson { .. }), "{err:?}");
}

#[tokio::test]
async fn a_closed_port_is_a_send_error() {
    let err = fetch("http://127.0.0.1:1/graphql", &IntrospectOptions::new())
        .await
        .unwrap_err();
    assert!(
        matches!(err, SchemaError::Send(ExecError::Connect { .. })),
        "{err:?}"
    );
}

#[tokio::test]
async fn an_http_request_has_no_schema() {
    let http = "info:\n  name: h\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: http://127.0.0.1:1/\n";
    let (_d, c, ctx) = setup(&[("opencollection.yml", ROOT), ("h.yml", http)]);
    let req = common::request(&c, "h.yml");
    let err = introspect(
        &executor(),
        req,
        &ctx,
        &IntrospectOptions::new(),
        &CancelHandle::new(),
    )
    .await
    .unwrap_err()
    .into_error();
    assert!(matches!(err, SchemaError::NotGraphql), "{err:?}");
}

#[tokio::test]
async fn a_guard_refusal_sends_nothing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture_json()))
        .expect(0)
        .mount(&server)
        .await;
    let options =
        IntrospectOptions::new().guard(Arc::new(|req| Some(format!("refused {}", req.method()))));
    let err = fetch(&format!("{}/graphql", server.uri()), &options)
        .await
        .unwrap_err();
    match err {
        SchemaError::Refused { message } => assert_eq!(message, "refused POST"),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn an_oauth2_request_fetches_its_token_first() {
    let server = MockServer::start().await;
    on_grant(
        &server,
        "client_credentials",
        token_response(json!({"expires_in": 3600})),
        1,
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture_json()))
        .expect(1)
        .mount(&server)
        .await;
    let auth = format!("  auth:\n{}", client_credentials(&server.uri(), ""));
    let file = graphql_file(&format!("{}/graphql", server.uri()), &auth);
    let f = Fixture::new(&[("opencollection.yml", ROOT), ("g.yml", &file)]);
    let got = introspect(
        &executor(),
        f.request("g.yml"),
        &f.ctx,
        &IntrospectOptions::new(),
        &CancelHandle::new(),
    )
    .await
    .expect("a schema");
    assert_eq!(got.schema().mutation_type(), Some("Mutation"));
}

#[tokio::test]
async fn a_failed_token_fetch_is_an_auth_error() {
    let server = MockServer::start().await;
    let refused = ResponseTemplate::new(401).set_body_json(json!({"error": "invalid_client"}));
    on_grant(&server, "client_credentials", refused, 1).await;
    let auth = format!("  auth:\n{}", client_credentials(&server.uri(), ""));
    let file = graphql_file(&format!("{}/graphql", server.uri()), &auth);
    let f = Fixture::new(&[("opencollection.yml", ROOT), ("g.yml", &file)]);
    let err = introspect(
        &executor(),
        f.request("g.yml"),
        &f.ctx,
        &IntrospectOptions::new(),
        &CancelHandle::new(),
    )
    .await
    .unwrap_err()
    .into_error();
    assert!(
        matches!(
            err,
            SchemaError::Auth(OAuth2Error::Provider { status: 401, .. })
        ),
        "{err:?}"
    );
}

/// A client-credentials GraphQL request on `server`, whose introspection gets `answer`.
async fn token_echo(server: &MockServer, answer: ResponseTemplate) -> Fixture {
    on_grant(
        server,
        "client_credentials",
        token_response(json!({"expires_in": 3600})),
        1,
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(answer)
        .mount(server)
        .await;
    let auth = format!("  auth:\n{}", client_credentials(&server.uri(), ""));
    let file = graphql_file(&format!("{}/graphql", server.uri()), &auth);
    Fixture::new(&[("opencollection.yml", ROOT), ("g.yml", &file)])
}

#[tokio::test]
async fn an_answer_carries_the_token_placed_on_the_request() {
    let server = MockServer::start().await;
    let f = token_echo(
        &server,
        ResponseTemplate::new(200).set_body_string(fixture_json()),
    )
    .await;
    let got = introspect(
        &executor(),
        f.request("g.yml"),
        &f.ctx,
        &IntrospectOptions::new(),
        &CancelHandle::new(),
    )
    .await
    .expect("a schema");
    assert!(
        got.derived_credentials().iter().any(|v| v == TOKEN),
        "token not carried"
    );
}

#[tokio::test]
async fn a_failed_answer_carries_the_token_too() {
    let server = MockServer::start().await;
    let echo = format!(r#"{{"errors":[{{"message":"bad token {TOKEN}"}}]}}"#);
    let f = token_echo(&server, ResponseTemplate::new(200).set_body_string(echo)).await;
    let err = introspect(
        &executor(),
        f.request("g.yml"),
        &f.ctx,
        &IntrospectOptions::new(),
        &CancelHandle::new(),
    )
    .await
    .unwrap_err();
    assert!(
        err.derived_credentials().iter().any(|v| v == TOKEN),
        "token not carried"
    );
    assert!(
        matches!(err.error(), SchemaError::Graphql { .. }),
        "{:?}",
        err.error()
    );
}

#[tokio::test]
async fn a_send_failure_carries_the_token_too() {
    let server = MockServer::start().await;
    on_grant(
        &server,
        "client_credentials",
        token_response(json!({"expires_in": 3600})),
        1,
    )
    .await;
    let auth = format!("  auth:\n{}", client_credentials(&server.uri(), ""));
    let file = graphql_file("http://127.0.0.1:1/graphql", &auth);
    let f = Fixture::new(&[("opencollection.yml", ROOT), ("g.yml", &file)]);
    let err = introspect(
        &executor(),
        f.request("g.yml"),
        &f.ctx,
        &IntrospectOptions::new(),
        &CancelHandle::new(),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err.error(), SchemaError::Send(_)),
        "{:?}",
        err.error()
    );
    assert!(
        err.derived_credentials().iter().any(|v| v == TOKEN),
        "token not carried"
    );
}
