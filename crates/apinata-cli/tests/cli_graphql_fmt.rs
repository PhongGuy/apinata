//! `apinata graphql fmt` on query files and GraphQL requests.

use crate::common;

use apinata_core::{BodySpec, RequestFile, format_query};
use common::{ROOT, apinata, write_tree};
use predicates::str::contains;
use std::fs;

/// A GraphQL request file holding `query`.
pub(crate) fn request(query: &str) -> String {
    format!(
        "info:\n  name: g\n  type: graphql\n  seq: 1\ngraphql:\n  url: \"http://localhost/graphql\"\n  body:\n    query: {}\n",
        serde_json::to_string(query).unwrap()
    )
}

#[test]
fn fmt_rewrites_a_query_file_once() {
    let dir = write_tree(&[("q.graphql", "{ me { id, name } }")]);
    let path = dir.path().join("q.graphql");
    apinata()
        .args(["graphql", "fmt"])
        .arg(&path)
        .assert()
        .code(0);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "{\n  me {\n    id\n    name\n  }\n}\n"
    );
    apinata()
        .args(["graphql", "fmt", "--check"])
        .arg(&path)
        .assert()
        .code(0);
}

#[test]
fn fmt_check_exits_1_and_writes_nothing() {
    let dir = write_tree(&[("q.gql", "{ me { id } }")]);
    let path = dir.path().join("q.gql");
    apinata()
        .args(["graphql", "fmt", "--check"])
        .arg(&path)
        .assert()
        .code(1)
        .stderr(contains("would be reformatted"));
    assert_eq!(fs::read_to_string(&path).unwrap(), "{ me { id } }");
}

#[test]
fn fmt_on_a_request_writes_the_writer_bytes() {
    let text = request("{ me { id name } }");
    let dir = write_tree(&[("opencollection.yml", ROOT), ("g.yml", &text)]);
    let reference = write_tree(&[("g.yml", &text)]);
    apinata()
        .args(["graphql", "fmt"])
        .arg(dir.path().join("g.yml"))
        .assert()
        .code(0);
    let mut expected = RequestFile::load(reference.path().join("g.yml")).unwrap();
    let BodySpec::Graphql { query, variables } = expected.body() else {
        panic!("not graphql")
    };
    expected.set_body(BodySpec::Graphql {
        query: format_query(&query).unwrap(),
        variables,
    });
    expected.save().unwrap();
    let got = fs::read(dir.path().join("g.yml")).unwrap();
    assert_ne!(got, text.as_bytes());
    assert_eq!(got, fs::read(reference.path().join("g.yml")).unwrap());
}

#[test]
fn empty_and_comment_only_queries_exit_0_unchanged() {
    let empty = request("");
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("g.yml", &empty),
        ("c.graphql", "# nothing\n"),
    ]);
    apinata()
        .args(["graphql", "fmt"])
        .arg(dir.path().join("g.yml"))
        .assert()
        .code(0);
    apinata()
        .args(["graphql", "fmt"])
        .arg(dir.path().join("c.graphql"))
        .assert()
        .code(0);
    assert_eq!(fs::read_to_string(dir.path().join("g.yml")).unwrap(), empty);
    assert_eq!(
        fs::read_to_string(dir.path().join("c.graphql")).unwrap(),
        "# nothing\n"
    );
}

#[test]
fn fmt_syntax_errors_exit_1_with_the_position() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("q.graphql", "{\n  me {\n"),
        ("g.yml", &request("{ me {")),
    ]);
    apinata()
        .args(["graphql", "fmt"])
        .arg(dir.path().join("q.graphql"))
        .assert()
        .code(1)
        .stderr(contains("q.graphql:3:1: syntax error:"));
    apinata()
        .args(["graphql", "fmt"])
        .arg(dir.path().join("g.yml"))
        .assert()
        .code(1)
        .stderr(contains("g.yml (query) 1:7: syntax error:"));
}

#[test]
fn fmt_refuses_a_request_that_is_not_graphql_and_a_missing_file() {
    let http = "info:\n  name: h\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://localhost/h\"\n";
    let dir = write_tree(&[("opencollection.yml", ROOT), ("h.yml", http)]);
    apinata()
        .args(["graphql", "fmt"])
        .arg(dir.path().join("h.yml"))
        .assert()
        .code(3)
        .stderr(contains("not a GraphQL request"));
    apinata()
        .args(["graphql", "fmt"])
        .arg(dir.path().join("nope.graphql"))
        .assert()
        .code(3);
}
