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
    run_typed(variables, pre, post, sent, "application/json", |t| t).await
}

/// Like `run`, with the file text passed through `edit` and `content_type` expected.
async fn run_typed(
    variables: &str,
    pre: &str,
    post: &str,
    sent: &str,
    content_type: &str,
    edit: impl Fn(String) -> String,
) -> RequestResult {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/gql"))
        .and(header("content-type", content_type))
        .and(body_string(sent))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&s)
        .await;
    let text = edit(graphql(&s.uri(), variables, pre, post));
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
async fn set_body_with_an_object_sends_its_json_text_expanded() {
    let pre = r#"req.setBody({query: "{ a(x: \"{{id}}\") }", variables: {n: 1}});
console.log(typeof req.getBody(), typeof req.getBody({raw: true}));"#;
    let set = r#"{"query":"{ a(x: \"{{id}}\") }","variables":{"n":1}}"#;
    let sent = r#"{"query":"{ a(x: \"7\") }","variables":{"n":1}}"#;
    let result = run("{}", pre, SHOW, sent).await;
    assert_eq!(
        console(&result),
        ["log:object string".to_owned(), format!("log:{set}")]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_body_with_a_string_sends_it_expanded() {
    let pre = r#"req.setBody("not json {{id}}");
console.log(req.getBody());"#;
    let result = run("{}", pre, SHOW, "not json 7").await;
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
    let set = r#"{"query":"{ user(id: \"{{id}}\") { id } }","variables":"{\"id\": \"{{id}}\"}"}"#;
    let sent = r#"{"query":"{ user(id: \"7\") { id } }","variables":"{\"id\": \"7\"}"}"#;
    let result = run(r#"{"id": "{{id}}"}"#, pre, SHOW, sent).await;
    assert_eq!(console(&result), [format!("log:{set}")]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn changing_the_got_body_without_set_body_changes_nothing() {
    let pre = r#"const b = req.getBody(); b.query = "{ changed }"; b.variables = "{}";"#;
    let result = run(r#"{"id": "{{id}}"}"#, pre, SHOW, SENT).await;
    assert_eq!(console(&result), [format!("log:{SENT}")]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_object_under_another_content_type_stays_an_object() {
    let pre = r#"req.setBody({query: "{ a(x: \"{{id}}\") }", variables: "{\"n\": {{id}}}"});
console.log(typeof req.getBody());"#;
    let sent = r#"{"query":"{ a(x: \"7\") }","variables":{"n":7}}"#;
    let plain = |t: String| {
        t.replace(
            "  body:\n",
            "  headers:\n    - name: Content-Type\n      value: text/plain\n  body:\n",
        )
    };
    let result = run_typed("{}", pre, SHOW, sent, "text/plain", plain).await;
    assert_eq!(
        console(&result),
        ["log:object".to_owned(), format!("log:{sent}")]
    );
}

/// Sends the request to a server that must see nothing, returning the result.
async fn refused(variables: &str, pre: &str) -> RequestResult {
    let s = MockServer::start().await;
    let text = graphql(&s.uri(), variables, pre, SHOW);
    let dir = write_tree(&[("opencollection.yml", ROOT), ("g.yml", &text)]);
    let result = send(dir.path(), "g.yml", on()).await;
    assert!(s.received_requests().await.expect("recorded").is_empty());
    result
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_name_in_a_set_body_fails_the_send() {
    for pre in [
        r#"req.setBody("{{nope}}");"#,
        r#"req.setBody({query: "{{nope}}"});"#,
    ] {
        let result = refused("{}", pre).await;
        assert!(
            matches!(
                result.error,
                Some(RunError::Resolve(ResolveError::UndefinedVariable { .. }))
            ),
            "{pre}: {:?}",
            result.error
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_variables_fail_the_send_after_the_script_reads_them() {
    for pre in [
        "console.log(req.getBody().variables);",
        r#"req.setBody({query: "{ a }", variables: "{oops"}, {raw: true}); console.log(req.getBody().variables);"#,
    ] {
        let result = refused("{oops", pre).await;
        assert!(
            matches!(
                result.error,
                Some(RunError::Resolve(ResolveError::GraphqlVariables { .. }))
            ),
            "{pre}: {:?}",
            result.error
        );
        assert_eq!(console(&result), ["log:{oops"], "{pre}");
    }
}
