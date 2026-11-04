//! `apinata run` and `apinata gen` with GraphQL requests against a local mock server.

use crate::common;

use common::{ROOT, apinata, column, run_json, write_tree};
use predicates::str::contains;
use serde_json::json;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const GQL: &str = r#"info:
  name: user
  type: graphql
  seq: 1
graphql:
  url: "HOST/graphql"
  body:
    query: "query ($id: ID!) { user(id: $id) { id } }"
    variables: '{"id": "{{uid}}"}'
runtime:
  variables:
    - name: uid
      value: "7"
  scripts:
    - type: before-request
      code: req.setHeader("x-script", "1");
  assertions:
    - expression: res.status
      operator: eq
      value: "200"
  actions:
    - type: set-variable
      phase: after-response
      selector:
        expression: res.body.data.user.id
        method: jsonq
      variable:
        name: userId
        scope: runtime
"#;

const NEXT: &str = "info:\n  name: next\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: \"HOST/users/{{userId}}\"\n";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_graphql_request_posts_json_runs_scripts_and_chains() {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(header("content-type", "application/json"))
        .and(header("x-script", "1"))
        .and(body_json(
            json!({"query": "query ($id: ID!) { user(id: $id) { id } }", "variables": {"id": "7"}}),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw("{\"data\":{\"user\":{\"id\":\"7\"}}}", "application/json"),
        )
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/users/7"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let gql = GQL.replace("HOST", &s.uri());
    let next = NEXT.replace("HOST", &s.uri());
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("user.yml", &gql),
        ("next.yml", &next),
    ]);
    let (code, report) = run_json(dir.path(), &[]);
    assert_eq!(code, 0, "{report}");
    assert_eq!(column(&report, "outcome"), ["passed", "passed"]);
    assert_eq!(report["results"][1]["status"], 200);
    assert_eq!(
        report["results"][0]["post_response_vars"],
        json!([{"name": "userId", "result": "set"}])
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unparseable_variables_are_that_requests_error() {
    let s = MockServer::start().await;
    let bad = format!(
        "info:\n  name: bad\n  type: graphql\n  seq: 1\ngraphql:\n  url: \"{}/graphql\"\n  body:\n    variables: \"{{not json\"\n",
        s.uri()
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("bad.yml", &bad)]);
    let (code, report) = run_json(dir.path(), &[]);
    assert_eq!(code, 2);
    assert!(
        column(&report, "error")[0].starts_with("Failed to parse GraphQL variables: "),
        "{report}"
    );
    assert!(s.received_requests().await.expect("recorded").is_empty());
}

#[test]
fn gen_prints_the_composed_body() {
    let file = "info:\n  name: g\n  type: graphql\n  seq: 1\ngraphql:\n  url: http://x.test/graphql\n  body:\n    query: \"{ me }\"\n";
    let dir = write_tree(&[("opencollection.yml", ROOT), ("g.yml", file)]);
    apinata()
        .args(["gen", "--target", "curl"])
        .arg(dir.path().join("g.yml"))
        .assert()
        .code(0)
        .stdout(contains(r#"{"query":"{ me }","variables":{}}"#))
        .stdout(contains("application/json"));
}
