//! A pasted curl command whose body is a GraphQL payload offers itself as a GraphQL request.

use apinata_core::import::{ImportedRequest, parse_curl};
use apinata_core::{BodySpec, RequestKind};

fn request(command: &str) -> ImportedRequest {
    parse_curl(command)
        .unwrap_or_else(|e| panic!("{command}: {e}"))
        .request
}

#[test]
fn a_json_query_and_variables_body_is_graphql() {
    let r = request(
        r#"curl https://x.test/api -H 'Content-Type: application/json' --data-raw '{"query":"{ me { id } }","variables":{"id":1},"operationName":null}'"#,
    );
    // Parsing alone never changes the kind: pasting onto an HTTP request keeps it HTTP.
    assert_eq!(r.kind, RequestKind::Http);
    let g = r.as_graphql().expect("a GraphQL payload");
    assert_eq!(g.kind, RequestKind::Graphql);
    assert_eq!(
        g.body,
        BodySpec::Graphql {
            query: "{ me { id } }".into(),
            variables: "{\n  \"id\": 1\n}".into(),
        }
    );
    assert_eq!(
        (g.method.as_str(), g.url.as_str()),
        ("POST", "https://x.test/api")
    );
    assert_eq!(g.headers, r.headers);
}

#[test]
fn an_application_graphql_body_is_the_query_itself() {
    let r = request(
        "curl https://x.test/q -H 'Content-Type: application/graphql' -H 'X-A: 1' -d '{ me { id } }'",
    );
    let g = r.as_graphql().expect("a GraphQL body");
    assert_eq!(
        g.body,
        BodySpec::Graphql {
            query: "{ me { id } }".into(),
            variables: String::new(),
        }
    );
    // The query now goes out as JSON, so the raw-query content type would mislabel it.
    let names: Vec<&str> = g.headers.iter().map(|h| h.name.as_str()).collect();
    assert_eq!(names, ["X-A"]);
}

#[test]
fn other_json_bodies_stay_http() {
    for data in [
        r#"{"query":"shoes","page":2}"#,
        r#"{"a":1}"#,
        r#"{"query":7}"#,
        r#"[{"query":"{ me }"}]"#,
    ] {
        let r = request(&format!(
            "curl https://x.test -H 'Content-Type: application/json' --data-raw '{data}'"
        ));
        assert!(r.as_graphql().is_none(), "{data}");
    }
    assert!(
        request("curl https://x.test/graphql")
            .as_graphql()
            .is_none()
    );
}
