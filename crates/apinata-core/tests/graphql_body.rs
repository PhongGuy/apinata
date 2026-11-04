//! The GraphQL body: reading, mismatched modes, redaction and the example refusal.

use crate::tree_support;

use apinata_core::{BodySpec, ExampleResponse, RequestFile, TreeError};
use std::fs;
use std::path::Path;
use tempfile::TempDir;
use tree_support::{ROOT, ip, open};

fn full() -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/reference/2026-10-13-graphql-parity/normalized/synthetic/graphql-full.yml",
    ))
    .expect("corpus")
}

fn load_text(text: &str) -> (TempDir, RequestFile) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("r.yml");
    fs::write(&path, text).unwrap();
    let r = RequestFile::load(&path).expect("loads");
    (dir, r)
}

#[test]
fn the_body_holds_query_and_variables_as_written() {
    let (_d, r) = load_text(&full());
    let BodySpec::Graphql { query, variables } = r.body() else {
        panic!("{:?}", r.body())
    };
    assert!(query.starts_with("query Search($term: String!, $limit: Int) {\n"));
    assert!(variables.contains("// the search term"));
}

#[test]
fn defaults_and_mismatched_bodies() {
    let (_d, mut r) = load_text("info:\n  name: g\n  type: graphql\ngraphql:\n  url: u\n");
    assert_eq!(
        r.body(),
        BodySpec::Graphql {
            query: String::new(),
            variables: String::new()
        }
    );
    r.set_body(BodySpec::Json("{}".into()));
    assert!(!r.is_dirty(), "a json body on a graphql request is ignored");
    let (_d, mut h) = load_text("info:\n  name: h\n  type: http\nhttp:\n  method: GET\n  url: u\n");
    h.set_body(BodySpec::Graphql {
        query: "{ a }".into(),
        variables: String::new(),
    });
    assert!(
        !h.is_dirty(),
        "a graphql body on an http request is ignored"
    );
}

#[test]
fn debug_shows_lengths_not_text() {
    let shown = format!(
        "{:?}",
        BodySpec::Graphql {
            query: "secret-query".into(),
            variables: "secret-vars".into()
        }
    );
    assert!(shown.starts_with("Graphql"), "{shown}");
    assert!(!shown.contains("secret"), "{shown}");
}

#[test]
fn examples_are_refused_on_graphql_requests() {
    // The file holds one example, so rename and delete at index 0 would succeed on an HTTP request.
    let with_example = full().replace(
        "\ndocs: |-",
        "\nexamples:\n  - name: pong\n    request:\n      url: https://example.com/ping\n      method: GET\n    response:\n      status: 200\n\ndocs: |-",
    );
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("q.yml", &with_example)]);
    let q = ip("q.yml");
    assert!(matches!(
        c.add_example(&q, "e", "", &ExampleResponse::new(200)),
        Err(TreeError::NotFound { .. })
    ));
    assert!(matches!(
        c.rename_example(&q, 0, "x"),
        Err(TreeError::NotFound { .. })
    ));
    assert!(matches!(
        c.delete_example(&q, 0),
        Err(TreeError::NotFound { .. })
    ));
    assert_eq!(
        fs::read_to_string(dir.path().join("q.yml")).unwrap(),
        with_example
    );
}
