//! New GraphQL requests and drafts.

use crate::tree_support;

use apinata_core::{BodySpec, ItemPath, RequestKind};
use std::fs;
use tree_support::{ROOT, ip, new_request_bytes, open};

fn gql_new() -> String {
    new_request_bytes("gql-new")
}

#[test]
fn a_new_graphql_request_has_the_reference_bytes() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    let made = c
        .create_request_of_kind(&ItemPath::root(), "New GraphQL", RequestKind::Graphql)
        .expect("creates");
    assert_eq!(made, ip("New GraphQL.yml"));
    assert_eq!(
        fs::read_to_string(dir.path().join("New GraphQL.yml")).unwrap(),
        gql_new()
    );
    let r = c.request(&made).expect("a request");
    assert_eq!((r.kind(), r.method()), (RequestKind::Graphql, "POST"));
}

#[test]
fn create_request_still_makes_http() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    c.create_request(&ItemPath::root(), "Get users")
        .expect("creates");
    let text = fs::read_to_string(dir.path().join("Get users.yml")).unwrap();
    assert_eq!(
        text,
        new_request_bytes("new-request").replace("seq: 3", "seq: 1")
    );
}

#[test]
fn a_graphql_draft_is_a_graphql_request() {
    let (_dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    let draft = c
        .create_draft_of_kind(&ItemPath::root(), RequestKind::Graphql)
        .expect("drafts");
    let r = c.request(&draft).expect("a request");
    assert_eq!(r.kind(), RequestKind::Graphql);
    assert_eq!(r.method(), "POST");
    assert_eq!(
        r.body(),
        BodySpec::Graphql {
            query: String::new(),
            variables: String::new()
        }
    );
}

#[test]
fn a_saved_graphql_draft_writes_a_graphql_file() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    let draft = c
        .create_draft_of_kind(&ItemPath::root(), RequestKind::Graphql)
        .expect("drafts");
    let saved = c.save_draft(&draft, &ItemPath::root(), "Q").expect("saves");
    let text = fs::read_to_string(dir.path().join("Q.yml")).unwrap();
    let doc: yaml_serde::Value = yaml_serde::from_str(&text).expect("yaml");
    assert_eq!(doc["info"]["type"].as_str(), Some("graphql"), "{text}");
    assert!(doc["graphql"].is_mapping(), "{text}");
    assert!(doc.get("http").is_none(), "{text}");
    assert_eq!(
        c.request(&saved).expect("loaded").kind(),
        RequestKind::Graphql
    );
}
