//! `graphql_schema`: a GraphQL request's schema fetched by introspection under the send rules,
//! a non-local host refused with an approval filed. No test reaches a non-local host.

use crate::common;

use common::{ROOT, TestServer};
use serde_json::json;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SCHEMA: &str = r#"{"data":{"__schema":{"queryType":{"name":"Query"},"mutationType":null,"subscriptionType":null,"types":[{"kind":"OBJECT","name":"Query","description":"DESC","fields":[{"name":"me","args":[],"type":{"kind":"SCALAR","name":"String"},"isDeprecated":false}]},{"kind":"SCALAR","name":"String"}]}}}"#;

fn graphql(url: &str, extra: &str) -> String {
    format!("info:\n  name: g\n  type: graphql\n  seq: 1\ngraphql:\n  url: \"{url}\"\n{extra}")
}

async fn upstream(response: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("IntrospectionQuery"))
        .respond_with(response)
        .mount(&server)
        .await;
    server
}

async fn start(url: &str) -> TestServer {
    TestServer::start(&[("opencollection.yml", ROOT), ("g.yml", &graphql(url, ""))]).await
}

#[tokio::test]
async fn the_schema_comes_back_as_sdl_with_its_roots() {
    let up =
        upstream(ResponseTemplate::new(200).set_body_string(SCHEMA.replace("DESC", "The root.")))
            .await;
    let server = start(&format!("{}/graphql", up.uri())).await;
    let got = server.ok("graphql_schema", json!({"path": "g.yml"})).await;
    assert_eq!(
        got,
        json!({
            "query_type": "Query",
            "type_count": 2,
            "format": "sdl",
            "text": "\"The root.\"\ntype Query {\n  me: String\n}\n",
            "total_bytes": 40,
            "truncated": false,
        })
    );
}

#[tokio::test]
async fn json_format_returns_the_answer_and_max_bytes_cuts_it() {
    let up = upstream(ResponseTemplate::new(200).set_body_string(SCHEMA)).await;
    let server = start(&format!("{}/graphql", up.uri())).await;
    let whole = server
        .ok("graphql_schema", json!({"path": "g.yml", "format": "json"}))
        .await;
    assert_eq!(whole["text"], SCHEMA);
    assert_eq!(whole["total_bytes"], SCHEMA.len());
    let cut = server
        .ok(
            "graphql_schema",
            json!({"path": "g.yml", "format": "json", "max_bytes": 10}),
        )
        .await;
    assert_eq!(cut["truncated"], true);
    assert!(cut["text"].as_str().unwrap().len() < SCHEMA.len(), "{cut}");
    let bad = server
        .err("graphql_schema", json!({"path": "g.yml", "format": "yaml"}))
        .await;
    assert!(bad.contains("`format`"), "{bad}");
}

#[tokio::test]
async fn an_untrusted_host_files_an_approval_and_sends_nothing() {
    let server = start("https://api.example.com/graphql").await;
    let message = server.err("graphql_schema", json!({"path": "g.yml"})).await;
    assert!(message.contains("`api.example.com`"), "{message}");
    assert!(
        message.contains("retry after the user approves"),
        "{message}"
    );
    let pending = server.inbox().pending().expect("inbox");
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0].host, "api.example.com");
    assert_eq!(pending[0].request, "g.yml");
}

#[tokio::test]
async fn a_denied_host_is_refused_without_an_approval() {
    let server = start("https://evil.example/graphql").await;
    server.deny("evil.example");
    let message = server.err("graphql_schema", json!({"path": "g.yml"})).await;
    assert!(message.contains("denied"), "{message}");
    assert!(server.inbox().pending().expect("inbox").is_empty());
}

#[tokio::test]
async fn graphql_errors_and_http_requests_are_errors() {
    let up = upstream(
        ResponseTemplate::new(200)
            .set_body_string(r#"{"errors":[{"message":"introspection is off"}]}"#),
    )
    .await;
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("g.yml", &graphql(&format!("{}/graphql", up.uri()), "")),
        (
            "h.yml",
            "info:\n  name: h\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: http://127.0.0.1:9/\n",
        ),
    ])
    .await;
    let errors = server.err("graphql_schema", json!({"path": "g.yml"})).await;
    assert!(
        errors.contains("GraphQL errors: introspection is off"),
        "{errors}"
    );
    let http = server.err("graphql_schema", json!({"path": "h.yml"})).await;
    assert!(
        http.contains("only a GraphQL request has a schema"),
        "{http}"
    );
    assert!(http.contains("nothing was sent"), "{http}");
}

#[tokio::test]
async fn a_secret_the_server_echoes_is_scrubbed() {
    let up = upstream(
        ResponseTemplate::new(200).set_body_string(SCHEMA.replace("DESC", "cli-Secret-5")),
    )
    .await;
    let header = "  headers:\n    - name: X-Key\n      value: \"{{key}}\"\n";
    let server = TestServer::start_with(
        &[
            ("opencollection.yml", ROOT),
            ("g.yml", &graphql(&format!("{}/graphql", up.uri()), header)),
        ],
        |o| o.env_var("key", "cli-Secret-5"),
    )
    .await;
    let got = server.ok("graphql_schema", json!({"path": "g.yml"})).await;
    let shown = serde_json::to_string(&got).unwrap();
    assert!(!shown.contains("cli-Secret-5"), "{shown}");
    let seen = up.received_requests().await.unwrap();
    assert_eq!(seen[0].headers.get("x-key").unwrap(), "cli-Secret-5");
}
