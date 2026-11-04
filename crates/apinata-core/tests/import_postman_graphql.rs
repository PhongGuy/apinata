//! Postman GraphQL bodies, as the reference converter maps them.

use apinata_core::import::{ImportedItem, ImportedRequest, parse_postman, write_collection};
use apinata_core::{BodySpec, ItemPath, MemoryStore, RequestKind};
use tempfile::TempDir;

const V21: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";

fn import(item: &str) -> (Vec<ImportedItem>, usize) {
    let json = r#"{"info": {"name": "c", "schema": "V21"}, "item": [ITEM]}"#
        .replace("V21", V21)
        .replace("ITEM", item);
    let c = parse_postman(json.as_bytes()).expect("parses");
    (c.items, c.unwritten.len())
}

fn only(items: &[ImportedItem]) -> &ImportedRequest {
    match items {
        [ImportedItem::Request(r)] => &r.request,
        other => panic!("{} items", other.len()),
    }
}

fn graphql(query: &str, variables: &str) -> BodySpec {
    BodySpec::Graphql {
        query: query.into(),
        variables: variables.into(),
    }
}

#[test]
fn a_graphql_body_makes_a_graphql_request() {
    let (items, dropped) = import(
        r#"{"name": "Users", "request": {"method": "POST", "url": "https://x.test/graphql",
            "header": [{"key": "x-a", "value": "1"}],
            "body": {"mode": "graphql", "graphql": {"query": "{ users { id } }", "variables": "{\"limit\": 10}"}}}}"#,
    );
    assert_eq!(dropped, 0);
    let r = only(&items);
    assert_eq!(r.kind, RequestKind::Graphql);
    assert_eq!(r.method, "POST");
    assert_eq!(r.url, "https://x.test/graphql");
    assert_eq!(r.headers.len(), 1);
    assert_eq!(r.body, graphql("{ users { id } }", "{\"limit\": 10}"));
}

#[test]
fn graphql_values_follow_the_reference_converter() {
    let body = |graphql: &str| {
        let item = format!(
            r#"{{"name": "g", "request": {{"method": "POST", "url": "u", "body": {{"mode": "graphql"{graphql}}}}}}}"#
        );
        only(&import(&item).0).body.clone()
    };
    assert_eq!(
        body(r#", "graphql": "{\"query\": \"{ a }\", \"variables\": \"{}\"}""#),
        graphql("{ a }", "{}")
    );
    assert_eq!(body(r#", "graphql": "not json""#), graphql("", ""));
    assert_eq!(body(""), graphql("", ""));
    assert_eq!(
        body(r#", "graphql": {"query": "{ a }", "variables": {"a": 1}}"#),
        graphql("{ a }", "{\n  \"a\": 1\n}")
    );
    assert_eq!(
        body(r#", "graphql": {"query": "", "variables": ""}"#),
        graphql("", "")
    );
}

#[test]
fn an_imported_graphql_request_is_written_as_one() {
    let json = r#"{"info": {"name": "c", "schema": "V21"}, "item": [
        {"name": "Users", "request": {"method": "POST", "url": "https://x.test/graphql",
         "body": {"mode": "graphql", "graphql": {"query": "{ users { id } }", "variables": "{\"limit\": 10}"}}}}]}"#
        .replace("V21", V21);
    let imported = parse_postman(json.as_bytes()).expect("parses");
    let parent = TempDir::new().expect("tempdir");
    let (collection, _report) =
        write_collection(parent.path(), &imported, &[], &MemoryStore::new()).expect("writes");
    let r = collection
        .request(&ItemPath::new("Users.yml").unwrap())
        .expect("written");
    assert_eq!(r.kind(), RequestKind::Graphql);
    assert_eq!(r.body(), graphql("{ users { id } }", "{\"limit\": 10}"));
    let text = std::fs::read_to_string(r.path()).unwrap();
    assert!(
        text.starts_with("info:\n  name: Users\n  type: graphql\n"),
        "{text}"
    );
}
