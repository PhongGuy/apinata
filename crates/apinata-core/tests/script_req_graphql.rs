//! `req.getBody` and `req.setBody` on GraphQL requests, and the body that reaches the wire.

use crate::script_support;

use apinata_core::{Outcome, RequestResult, ResolveError, RunError};
use script_support::*;
use wiremock::matchers::{body_string, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const QUERY: &str = r#"{ user(id: \"{{id}}\") { name } }"#;
const SENT: &str = r#"{"query":"{ user(id: \"7\") { name } }","variables":{"id":"7"}}"#;

/// A GraphQL request to `{uri}/gql` with `variables` as written, a pre-request script setting
/// `id` to 7 before running `pre`, and a post-response script `post`.
fn graphql(uri: &str, variables: &str, pre: &str, post: &str) -> String {
    let scripts = script(
        "before-request",
        &format!("bru.setVar(\"id\", \"7\");\n{pre}"),
    ) + &script("after-response", post);
    format!(
        "info:\n  name: G\n  type: graphql\n  seq: 1\ngraphql:\n  method: POST\n  url: \"{uri}/gql\"\n  body:\n    query: \"{QUERY}\"\n    variables: '{variables}'\nruntime:\n  scripts:\n{scripts}"
    )
}

/// Sends the request, expecting exactly `sent` as a JSON body at `/gql`.
async fn run(variables: &str, pre: &str, post: &str, sent: &str) -> RequestResult {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/gql"))
        .and(header("content-type", "application/json"))
        .and(body_string(sent))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&s)
        .await;
    let text = graphql(&s.uri(), variables, pre, post);
    let dir = write_tree(&[("opencollection.yml", ROOT), ("g.yml", &text)]);
    let result = send(dir.path(), "g.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    s.verify().await;
    result
}

const SHOW: &str = "console.log(JSON.stringify(req.getBody()));";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_body_shows_the_raw_query_and_variables_text_before_the_send() {
    let pre = "console.log(JSON.stringify([req.getBody(), req.getBody({raw: true}), req.body]));";
    let result = run(r#"{"id": "{{id}}"}"#, pre, SHOW, SENT).await;
    let raw = r#"{"query":"{ user(id: \"{{id}}\") { name } }","variables":"{\"id\": \"{{id}}\"}"}"#;
    assert_eq!(
        console(&result),
        [format!("log:[{raw},{raw},{raw}]"), format!("log:{SENT}")]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_body_with_an_object_sends_its_json_as_is() {
    let pre = r#"req.setBody({query: "{ a(x: \"{{id}}\") }", variables: {n: 1}});
console.log(typeof req.getBody(), typeof req.getBody({raw: true}));"#;
    let sent = r#"{"query":"{ a(x: \"{{id}}\") }","variables":{"n":1}}"#;
    let result = run("{}", pre, SHOW, sent).await;
    assert_eq!(
        console(&result),
        ["log:object string".to_owned(), format!("log:{sent}")]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_body_with_a_string_sends_it_verbatim() {
    let pre = r#"req.setBody("not json {{id}}");
console.log(req.getBody());"#;
    let result = run("{}", pre, SHOW, "not json {{id}}").await;
    assert_eq!(
        console(&result),
        ["log:not json {{id}}", r#"log:"not json {{id}}""#]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_raw_object_is_interpolated_and_its_variables_parsed() {
    let pre = r#"const b = req.getBody(); b.operationName = "Q"; req.setBody(b, {raw: true});"#;
    let sent =
        r#"{"query":"{ user(id: \"7\") { name } }","variables":{"id":"7"},"operationName":"Q"}"#;
    let result = run(r#"{"id": "{{id}}"}"#, pre, SHOW, sent).await;
    assert_eq!(console(&result), [format!("log:{sent}")]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_round_trip_keeps_the_variables_as_text() {
    let pre =
        r#"const b = req.getBody(); b.query = b.query.replace("name", "id"); req.setBody(b);"#;
    let sent = r#"{"query":"{ user(id: \"{{id}}\") { id } }","variables":"{\"id\": \"{{id}}\"}"}"#;
    let result = run(r#"{"id": "{{id}}"}"#, pre, SHOW, sent).await;
    assert_eq!(console(&result), [format!("log:{sent}")]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_variables_fail_the_send_after_the_script_reads_them() {
    for pre in [
        "console.log(req.getBody().variables);",
        r#"req.setBody({query: "{ a }", variables: "{oops"}, {raw: true}); console.log(req.getBody().variables);"#,
    ] {
        let s = MockServer::start().await;
        let text = graphql(&s.uri(), "{oops", pre, SHOW);
        let dir = write_tree(&[("opencollection.yml", ROOT), ("g.yml", &text)]);
        let result = send(dir.path(), "g.yml", on()).await;
        assert!(
            matches!(
                result.error,
                Some(RunError::Resolve(ResolveError::GraphqlVariables { .. }))
            ),
            "{pre}: {:?}",
            result.error
        );
        assert_eq!(console(&result), ["log:{oops"], "{pre}");
        assert!(s.received_requests().await.expect("recorded").is_empty());
    }
}
