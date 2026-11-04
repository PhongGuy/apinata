//! GraphQL request files: load, kind, accessors through the `graphql` key, collection membership.

use crate::tree_support;

use apinata_core::{AuthSpec, RequestFile, RequestKind};
use std::fs;
use std::path::Path;
use tempfile::TempDir;
use tree_support::{ROOT, ip, open, request};

fn full() -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/reference/2026-10-13-graphql-parity/normalized/synthetic/graphql-full.yml",
    ))
    .expect("corpus")
}

fn load_text(text: &str) -> (TempDir, Result<RequestFile, String>) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("r.yml");
    fs::write(&path, text).unwrap();
    let r = RequestFile::load(&path).map_err(|e| e.message().to_owned());
    (dir, r)
}

#[test]
fn a_graphql_file_reads_through_the_graphql_key() {
    let (_d, r) = load_text(&full());
    let r = r.expect("loads");
    assert_eq!(r.kind(), RequestKind::Graphql);
    assert_eq!(r.method(), "POST");
    assert_eq!(r.url(), "{{host}}/graphql?trace=1");
    assert_eq!(r.headers().len(), 2);
    assert_eq!(r.params().len(), 1);
    assert!(matches!(r.auth(), AuthSpec::Bearer { token } if token == "{{token}}"));
    assert_eq!(r.docs(), "# Search\nFinds users by term.");
    assert_eq!(r.assertions().len(), 1);
}

#[test]
fn a_graphql_file_without_a_method_is_a_post() {
    let text = "info:\n  name: g\n  type: graphql\ngraphql:\n  url: u\n";
    let (_d, r) = load_text(text);
    assert_eq!(r.expect("loads").method(), "POST");
    let (_d, h) = load_text("info:\n  name: h\n  type: http\nhttp:\n  method: GET\n  url: u\n");
    assert_eq!(h.expect("loads").kind(), RequestKind::Http);
}

#[test]
fn shapes_that_do_not_match_their_type_are_refused_by_name() {
    let (_d, r) = load_text("info:\n  name: g\n  type: graphql\nhttp:\n  url: u\n");
    assert_eq!(r.unwrap_err(), "the file has no `graphql` mapping");
    let (_d, r) = load_text("info:\n  name: g\n  type: grpc\ngrpc:\n  url: u\n");
    assert_eq!(
        r.unwrap_err(),
        "unsupported request type `grpc`, only `http` and `graphql` are supported"
    );
    let (_d, r) = load_text("info:\n  name: h\n  type: http\n");
    assert_eq!(r.unwrap_err(), "the file has no `http` mapping");
}

#[test]
fn a_collection_lists_graphql_files_as_requests() {
    let (_dir, c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("q.yml", &full()),
        (
            "broken.yml",
            "info:\n  name: b\n  type: graphql\n  seq: 3\n",
        ),
    ]);
    assert_eq!(
        c.request(&ip("q.yml")).expect("a request").kind(),
        RequestKind::Graphql
    );
    assert!(c.opaque_item(&ip("q.yml")).is_none());
    assert!(
        c.opaque_item(&ip("broken.yml")).is_some(),
        "a malformed file stays opaque"
    );
    assert_eq!(c.load_errors().len(), 1);
    assert_eq!(c.tree().requests().len(), 2);
}
