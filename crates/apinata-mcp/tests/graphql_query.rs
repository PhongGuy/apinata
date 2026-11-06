//! `graphql_format` and `graphql_validate`: by query and by path, the one-of rule, the cap.

use crate::common;

use apinata_core::{BodySpec, RequestFile};
use common::{ROOT, TestServer, request};
use serde_json::{Value, json};
use std::fs;

const SDL: &str = "type Query { me: User }\ntype User { id: ID! name: String old: String @deprecated(reason: \"use name\") }\n";

fn graphql(query: &str) -> String {
    format!(
        "info:\n  name: g\n  type: graphql\n  seq: 1\ngraphql:\n  url: \"http://localhost/graphql\"\n  body:\n    query: {}\n",
        serde_json::to_string(query).unwrap()
    )
}

fn saved_query(server: &TestServer) -> String {
    let BodySpec::Graphql { query, .. } = RequestFile::load(server.root().join("g.yml"))
        .unwrap()
        .body()
    else {
        panic!("not graphql")
    };
    query
}

#[tokio::test]
async fn format_by_query_returns_the_text() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let got = server
        .ok("graphql_format", json!({"query": "{ me { id } }"}))
        .await;
    assert_eq!(got, json!({"query": "{\n  me {\n    id\n  }\n}\n"}));
    let blank = server
        .ok("graphql_format", json!({"query": "# nothing\n"}))
        .await;
    assert_eq!(blank, json!({"query": "# nothing\n"}));
}

#[tokio::test]
async fn format_by_path_saves_and_reports_a_change() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("g.yml", &graphql("{ me { id } }")),
    ])
    .await;
    assert_eq!(
        server.ok("graphql_format", json!({"path": "g.yml"})).await,
        json!({"changed": true})
    );
    assert_eq!(saved_query(&server), "{\n  me {\n    id\n  }\n}\n");
    assert_eq!(
        server.ok("graphql_format", json!({"path": "g.yml"})).await,
        json!({"changed": false})
    );
}

#[tokio::test]
async fn an_empty_request_query_is_left_alone() {
    let text = graphql("");
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("g.yml", &text)]).await;
    assert_eq!(
        server.ok("graphql_format", json!({"path": "g.yml"})).await,
        json!({"changed": false})
    );
    assert_eq!(
        fs::read_to_string(server.root().join("g.yml")).unwrap(),
        text
    );
}

#[tokio::test]
async fn exactly_one_of_path_and_query() {
    let server =
        TestServer::start(&[("opencollection.yml", ROOT), ("g.yml", &graphql("{ me }"))]).await;
    for tool in ["graphql_format", "graphql_validate"] {
        let none = server.err(tool, json!({})).await;
        assert!(none.contains("exactly one of `path` and `query`"), "{none}");
        let both = server
            .err(tool, json!({"path": "g.yml", "query": "{ me }"}))
            .await;
        assert!(both.contains("exactly one of `path` and `query`"), "{both}");
    }
}

#[tokio::test]
async fn a_syntax_error_writes_nothing() {
    let text = graphql("{ me {");
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("g.yml", &text)]).await;
    let err = server.err("graphql_format", json!({"path": "g.yml"})).await;
    assert!(
        err.contains("syntax error at 1:7") && err.contains("nothing was written"),
        "{err}"
    );
    assert_eq!(
        fs::read_to_string(server.root().join("g.yml")).unwrap(),
        text
    );
}

#[tokio::test]
async fn a_request_that_is_not_graphql_is_refused() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("h.yml", &request("h", 1, "GET", "http://localhost/h")),
    ])
    .await;
    let err = server.err("graphql_format", json!({"path": "h.yml"})).await;
    assert!(err.contains("is not a GraphQL request"), "{err}");
}

#[tokio::test]
async fn validate_without_a_schema_checks_syntax_only() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    assert_eq!(
        server
            .ok("graphql_validate", json!({"query": "{ me { nmae } }"}))
            .await,
        json!({"schema_checked": false, "diagnostics": []})
    );
    let got = server
        .ok("graphql_validate", json!({"query": "{ me { id }"}))
        .await;
    assert_eq!(
        got["diagnostics"],
        json!([{"start": 11, "end": 11, "line": 1, "column": 12, "severity": "error", "message": "syntax error: expected R_CURLY, got EOF"}])
    );
}

#[tokio::test]
async fn validate_with_a_schema_by_query_and_by_path() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("g.yml", &graphql("{ me { nmae old } }")),
    ])
    .await;
    let expected = json!({"schema_checked": true, "diagnostics": [
        {"start": 7, "end": 11, "line": 1, "column": 8, "severity": "error", "message": "type `User` does not have a field `nmae`"},
        {"start": 12, "end": 15, "line": 1, "column": 13, "severity": "warning", "message": "`User.old` is deprecated: use name"}
    ]});
    assert_eq!(
        server
            .ok(
                "graphql_validate",
                json!({"query": "{ me { nmae old } }", "schema": SDL})
            )
            .await,
        expected
    );
    assert_eq!(
        server
            .ok("graphql_validate", json!({"path": "g.yml", "schema": SDL}))
            .await,
        expected
    );
}

#[tokio::test]
async fn validate_caps_the_list_and_refuses_a_bad_schema() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let got = server
        .ok("graphql_validate", json!({"query": "} ".repeat(300)}))
        .await;
    let list = got["diagnostics"].as_array().unwrap();
    assert_eq!(list.len(), 201);
    assert_eq!(list[200]["message"], Value::from("and 100 more"));
    assert_eq!(list[200]["severity"], Value::from("info"));
    let err = server
        .err(
            "graphql_validate",
            json!({"query": "{ me }", "schema": "{ not json"}),
        )
        .await;
    assert!(err.contains("`schema`"), "{err}");
}
