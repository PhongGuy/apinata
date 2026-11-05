//! `apinata graphql schema`: a GraphQL request's schema fetched by introspection and printed.

use crate::common;

use common::{ROOT, apinata, dead_port, write_tree};
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SCHEMA: &str = r#"{"data":{"__schema":{"queryType":{"name":"Query"},"mutationType":null,"subscriptionType":null,"types":[{"kind":"OBJECT","name":"Query","description":"The root.","fields":[{"name":"me","args":[],"type":{"kind":"SCALAR","name":"String"},"isDeprecated":false}]},{"kind":"SCALAR","name":"String"}]}}}"#;

fn graphql(url: &str) -> String {
    format!("info:\n  name: g\n  type: graphql\n  seq: 1\ngraphql:\n  url: \"{url}\"\n")
}

async fn serve(response: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("IntrospectionQuery"))
        .respond_with(response)
        .mount(&server)
        .await;
    server
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn json_output_is_the_answer_as_received() {
    let s = serve(ResponseTemplate::new(200).set_body_string(SCHEMA)).await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("g.yml", &graphql(&format!("{}/graphql", s.uri()))),
    ]);
    apinata()
        .args(["graphql", "schema"])
        .arg(dir.path().join("g.yml"))
        .assert()
        .code(0)
        .stdout(format!("{SCHEMA}\n"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdl_output_prints_the_schema_document() {
    let s = serve(ResponseTemplate::new(200).set_body_string(SCHEMA)).await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("g.yml", &graphql(&format!("{}/graphql", s.uri()))),
    ]);
    apinata()
        .args(["graphql", "schema", "--format", "sdl"])
        .arg(dir.path().join("g.yml"))
        .assert()
        .code(0)
        .stdout("\"The root.\"\ntype Query {\n  me: String\n}\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_environment_and_env_vars_resolve_the_url_and_output_writes_a_file() {
    let s = serve(ResponseTemplate::new(200).set_body_string(SCHEMA)).await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            &format!(
                "name: dev\nvariables:\n  - name: base\n    value: {}\n",
                s.uri()
            ),
        ),
        ("g.yml", &graphql("{{base}}/{{route}}")),
    ]);
    let out = dir.path().join("schema.graphql");
    apinata()
        .args([
            "graphql",
            "schema",
            "--env",
            "dev",
            "--env-var",
            "route=graphql",
            "--format",
            "sdl",
        ])
        .arg("--output")
        .arg(&out)
        .arg(dir.path().join("g.yml"))
        .assert()
        .code(0)
        .stdout("");
    assert_eq!(
        std::fs::read_to_string(&out).unwrap(),
        "\"The root.\"\ntype Query {\n  me: String\n}\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_http_error_status_exits_1() {
    let s = serve(ResponseTemplate::new(500)).await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("g.yml", &graphql(&format!("{}/graphql", s.uri()))),
    ]);
    apinata()
        .args(["graphql", "schema"])
        .arg(dir.path().join("g.yml"))
        .assert()
        .code(1)
        .stdout("")
        .stderr(contains(
            "error: the server answered 500 Internal Server Error",
        ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn graphql_errors_exit_1_with_their_messages() {
    let body = r#"{"errors":[{"message":"introspection is off"}]}"#;
    let s = serve(ResponseTemplate::new(200).set_body_string(body)).await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("g.yml", &graphql(&format!("{}/graphql", s.uri()))),
    ]);
    apinata()
        .args(["graphql", "schema"])
        .arg(dir.path().join("g.yml"))
        .assert()
        .code(1)
        .stderr(contains("GraphQL errors: introspection is off"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_non_json_answer_exits_1() {
    let s = serve(ResponseTemplate::new(200).set_body_string("<html></html>")).await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("g.yml", &graphql(&format!("{}/graphql", s.uri()))),
    ]);
    apinata()
        .args(["graphql", "schema"])
        .arg(dir.path().join("g.yml"))
        .assert()
        .code(1)
        .stderr(contains("the text is not JSON"));
}

#[test]
fn no_answer_exits_2_without_the_secret_in_the_url() {
    let port = dead_port();
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: key\n    secret: true\n",
        ),
        (
            "g.yml",
            &graphql(&format!("http://127.0.0.1:{port}/graphql?key={{{{key}}}}")),
        ),
    ]);
    apinata()
        .args([
            "graphql",
            "schema",
            "--env",
            "dev",
            "--env-var",
            "key=Zq7Wk9Px-secret",
        ])
        .arg(dir.path().join("g.yml"))
        .assert()
        .code(2)
        .stderr(contains("error:").and(contains("Zq7Wk9Px").not()));
}

#[test]
fn an_http_request_exits_3() {
    let http = "info:\n  name: h\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: http://127.0.0.1:1/\n";
    let dir = write_tree(&[("opencollection.yml", ROOT), ("h.yml", http)]);
    apinata()
        .args(["graphql", "schema"])
        .arg(dir.path().join("h.yml"))
        .assert()
        .code(3)
        .stderr(contains("error: only a GraphQL request has a schema"));
}

#[test]
fn an_undefined_variable_exits_3_and_sends_nothing() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("g.yml", &graphql("{{host}}/graphql")),
    ]);
    apinata()
        .args(["graphql", "schema"])
        .arg(dir.path().join("g.yml"))
        .assert()
        .code(3)
        .stderr(contains("host"));
}

const TOKEN: &str = "tok-echo-4f1c-SECRET";
/// 32 zero bytes as base64: the key debug builds read instead of the keychain.
const KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

/// A provider on `server` answering token requests with `token`, and a GraphQL request there
/// that uses it.
async fn oauth_collection(server: &MockServer, token: ResponseTemplate) -> tempfile::TempDir {
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(token)
        .mount(server)
        .await;
    let auth = format!(
        "  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{}/token\"\n    credentials:\n      clientId: app\n      clientSecret: cs-77aa-SECRET\n    scope: read\n    tokenConfig:\n      id: main\n      placement:\n        header: Bearer\n",
        server.uri()
    );
    let file = format!(
        "info:\n  name: g\n  type: graphql\n  seq: 1\ngraphql:\n  url: \"{}/graphql\"\n{auth}",
        server.uri()
    );
    write_tree(&[("opencollection.yml", ROOT), ("g.yml", &file)])
}

fn token_ok() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(
        serde_json::json!({"access_token": TOKEN, "token_type": "Bearer", "expires_in": 3600}),
    )
}

/// `apinata graphql schema` on `dir`'s request with the token cache in `cache`.
fn schema_cmd(dir: &std::path::Path, cache: &std::path::Path) -> assert_cmd::Command {
    let mut cmd = apinata();
    cmd.args(["graphql", "schema"])
        .arg(dir.join("g.yml"))
        .env("APINATA_TEST_TOKEN_CACHE", cache)
        .env("APINATA_TEST_TOKEN_KEY", KEY);
    cmd
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_token_the_server_echoes_in_an_error_is_scrubbed() {
    let s = MockServer::start().await;
    let echo = format!(r#"{{"errors":[{{"message":"bad token {TOKEN}"}}]}}"#);
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(echo))
        .mount(&s)
        .await;
    let dir = oauth_collection(&s, token_ok()).await;
    let cache = tempfile::TempDir::new().unwrap();
    schema_cmd(dir.path(), cache.path())
        .assert()
        .code(1)
        .stderr(contains("bad token").and(contains(TOKEN).not()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_token_in_the_schema_text_is_scrubbed_from_stdout_but_kept_in_the_file() {
    let s = MockServer::start().await;
    let body = SCHEMA.replace("The root.", TOKEN);
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body.clone()))
        .mount(&s)
        .await;
    let dir = oauth_collection(&s, token_ok()).await;
    let cache = tempfile::TempDir::new().unwrap();
    schema_cmd(dir.path(), cache.path())
        .assert()
        .code(0)
        .stdout(contains("__schema").and(contains(TOKEN).not()));
    let out = dir.path().join("schema.json");
    schema_cmd(dir.path(), cache.path())
        .arg("--output")
        .arg(&out)
        .assert()
        .code(0);
    assert_eq!(std::fs::read_to_string(&out).unwrap(), format!("{body}\n"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_token_fetch_exits_2_like_run() {
    let s = MockServer::start().await;
    let refused =
        ResponseTemplate::new(401).set_body_json(serde_json::json!({"error": "invalid_client"}));
    let dir = oauth_collection(&s, refused).await;
    let cache = tempfile::TempDir::new().unwrap();
    schema_cmd(dir.path(), cache.path())
        .assert()
        .code(2)
        .stderr(contains("invalid_client"));
}
