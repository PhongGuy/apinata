//! A GraphQL request resolves to a JSON text body.

use crate::common;

use apinata_core::{Body, ResolveError};
use common::{at, pairs, setup};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nrequest:\n  variables:\n    - name: n\n      value:\n        type: number\n        data: \"42\"\n    - name: id\n      value: u1\n";

/// A GraphQL file without a method; `under` is indented under `graphql:`.
fn gql(under: &str) -> String {
    format!("info:\n  name: g\n  type: graphql\ngraphql:\n  url: https://x.test/graphql\n{under}")
}

#[test]
fn query_and_variables_become_one_json_body() {
    let full = gql(
        "  body:\n    query: |-\n      { user(id: \"{{id}}\") { name } }\n    variables: |-\n      {\n        // the user\n        \"id\": \"{{id}}\",\n        \"n\": {{n}},\n        \"f\": 1.50\n      }\n",
    );
    let bare = gql("  body:\n    query: \"{ me }\"\n");
    let none = gql("");
    let typed = gql(
        "  headers:\n    - name: Content-Type\n      value: application/graphql+json\n  body:\n    query: \"{ me }\"\n",
    );
    let commented = gql(
        "  body:\n    variables: |-\n      {\n        // {{nope}}\n        \"a\": 1\n      }\n",
    );
    let (_d, c, ctx) = setup(&[
        ("opencollection.yml", ROOT),
        ("full.yml", &full),
        ("bare.yml", &bare),
        ("none.yml", &none),
        ("typed.yml", &typed),
        ("commented.yml", &commented),
    ]);
    let check = |rel: &str, body: &str, headers: &[(&str, &str)]| {
        let r = at(&c, &ctx, rel).expect(rel);
        assert_eq!(r.method(), "POST", "{rel}");
        assert_eq!(r.body(), &Body::Text(body.to_owned()), "{rel}");
        assert_eq!(r.headers().to_vec(), pairs(headers), "{rel}");
    };
    let json = [("content-type", "application/json")];
    check(
        "full.yml",
        r#"{"query":"{ user(id: \"u1\") { name } }","variables":{"id":"u1","n":42,"f":1.5}}"#,
        &json,
    );
    check("bare.yml", r#"{"query":"{ me }","variables":{}}"#, &json);
    check("none.yml", r#"{"query":"","variables":{}}"#, &json);
    check(
        "typed.yml",
        r#"{"query":"{ me }","variables":{}}"#,
        &[("Content-Type", "application/graphql+json")],
    );
    check(
        "commented.yml",
        r#"{"query":"","variables":{"a":1}}"#,
        &json,
    );
}

#[test]
fn bad_variables_and_undefined_names_stop_the_request() {
    let bad = gql("  body:\n    variables: \"{not json\"\n");
    let undefined = gql("  body:\n    query: \"{ a(x: {{missing}}) }\"\n");
    let (_d, c, ctx) = setup(&[
        ("opencollection.yml", ROOT),
        ("bad.yml", &bad),
        ("undefined.yml", &undefined),
    ]);
    let err = at(&c, &ctx, "bad.yml").expect_err("unparseable");
    assert!(
        matches!(err, ResolveError::GraphqlVariables { .. }),
        "{err:?}"
    );
    assert!(
        err.to_string()
            .starts_with("Failed to parse GraphQL variables: "),
        "{err}"
    );
    let err = at(&c, &ctx, "undefined.yml").expect_err("undefined");
    assert!(
        matches!(err, ResolveError::UndefinedVariable { .. }),
        "{err:?}"
    );
}

#[test]
fn variables_that_are_only_a_comment_do_not_parse() {
    let comment = gql("  body:\n    query: \"{ me }\"\n    variables: \"// only a comment\"\n");
    let empty = gql("  body:\n    query: \"{ me }\"\n    variables: \"\"\n");
    let (_d, c, ctx) = setup(&[
        ("opencollection.yml", ROOT),
        ("comment.yml", &comment),
        ("empty.yml", &empty),
    ]);
    let err = at(&c, &ctx, "comment.yml").expect_err("nothing to parse");
    assert!(
        matches!(err, ResolveError::GraphqlVariables { .. }),
        "{err:?}"
    );
    let r = at(&c, &ctx, "empty.yml").expect("empty is {}");
    assert_eq!(
        r.body(),
        &Body::Text(r#"{"query":"{ me }","variables":{}}"#.to_owned())
    );
}
