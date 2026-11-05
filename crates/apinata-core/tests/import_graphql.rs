//! GraphQL requests from Insomnia and Hoppscotch exports, imported and written as GraphQL.

use apinata_core::import::{
    CollectionFormat, CollectionOptions, ImportedCollection, ImportedItem, ImportedRequestItem,
    NoteKind, parse_collections, write_collection,
};
use apinata_core::{AuthSpec, BodySpec, KeyValue, MemoryStore, RequestKind};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/graphql");
    fs::read(path.join(name)).expect("fixture")
}

fn import(format: CollectionFormat, name: &str) -> ImportedCollection {
    let mut bundles =
        parse_collections(Some(format), &fixture(name), &CollectionOptions::default())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
    assert_eq!(bundles.len(), 1, "{name}");
    bundles.remove(0).collection
}

/// The request at `path`, folder names then the request name.
fn find<'a>(items: &'a [ImportedItem], path: &[&str]) -> &'a ImportedRequestItem {
    let (last, folders) = path.split_last().expect("a path");
    let mut items = items;
    for name in folders {
        items = items
            .iter()
            .find_map(|i| match i {
                ImportedItem::Folder(f) if f.name == *name => Some(&f.items),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no folder {name}"));
    }
    items
        .iter()
        .find_map(|i| match i {
            ImportedItem::Request(r) if r.name == *last => Some(r),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no request {last}"))
}

fn graphql(query: &str, variables: &str) -> BodySpec {
    BodySpec::Graphql {
        query: query.into(),
        variables: variables.into(),
    }
}

/// Every written file under the collection root, then the report's notes.
fn written(c: &ImportedCollection) -> String {
    let dir = TempDir::new().expect("tempdir");
    let (collection, report) =
        write_collection(dir.path(), c, &[], &MemoryStore::new()).expect("writes");
    let mut files = BTreeMap::new();
    let mut stack = vec![collection.root().to_path_buf()];
    while let Some(at) = stack.pop() {
        for entry in fs::read_dir(&at).expect("dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path.strip_prefix(collection.root()).expect("under root");
                let rel = rel.to_string_lossy().replace('\\', "/");
                files.insert(rel, fs::read_to_string(&path).expect("utf-8"));
            }
        }
    }
    let mut out = String::new();
    for (path, text) in files {
        out.push_str(&format!("== {path}\n{text}"));
    }
    out.push_str("== report\n");
    for n in &report.notes {
        out.push_str(&format!("{:?} {}: {}\n", n.kind, n.source, n.detail));
    }
    out
}

#[test]
fn insomnia_v4_graphql_requests_keep_query_variables_headers_and_auth() {
    let c = import(CollectionFormat::Insomnia, "insomnia-v4-graphql.json");
    assert!(c.unwritten.is_empty(), "{:#?}", c.unwritten);
    let r = find(&c.items, &["Users", "User by id"]);
    assert_eq!(r.request.kind, RequestKind::Graphql);
    assert_eq!(r.request.method, "POST");
    assert_eq!(r.request.url, "{{base_url}}/graphql");
    assert_eq!(
        r.request.body,
        graphql(
            "query User($id: ID!) { user(id: $id) { id name } }",
            "{\n  \"id\": \"{{user_id}}\"\n}"
        )
    );
    let mut trace = KeyValue::new("X-Trace", "on");
    trace.enabled = false;
    assert_eq!(
        r.request.headers,
        [KeyValue::new("Content-Type", "application/json"), trace]
    );
    assert_eq!(
        r.request.auth,
        AuthSpec::Bearer {
            token: "{{token}}".into()
        }
    );
    // The operation name has no place in the request file, so it is noted.
    assert!(
        r.notes.iter().any(|n| n.kind == NoteKind::Dropped
            && n.source == "Users / User by id / body.text"
            && n.detail.contains("operation name `User`")),
        "{:#?}",
        r.notes
    );
    let plain = find(&c.items, &["Plain JSON"]);
    assert_eq!(plain.request.kind, RequestKind::Http);
    assert_eq!(plain.request.body, BodySpec::Json("{\"a\": 1}".into()));
}

#[test]
fn an_insomnia_graphql_body_that_is_not_json_imports_empty_with_a_note() {
    let c = import(CollectionFormat::Insomnia, "insomnia-v4-graphql.json");
    let r = find(&c.items, &["Broken body"]);
    assert_eq!(r.request.kind, RequestKind::Graphql);
    assert_eq!(r.request.body, graphql("", ""));
    assert!(
        r.notes.iter().any(|n| n.kind == NoteKind::Dropped
            && n.source == "Broken body / body.text"
            && n.detail.contains("not JSON")),
        "{:#?}",
        r.notes
    );
    for text in ["null", "[1]", "\"q\"", "7", ""] {
        let input = format!(
            r#"{{"_type": "export", "__export_format": 4, "resources": [
            {{"_id": "wrk", "_type": "workspace", "parentId": null, "name": "w"}},
            {{"_id": "req", "_type": "request", "parentId": "wrk", "name": "R", "method": "POST",
              "url": "u", "body": {{"mimeType": "application/graphql", "text": {}}}}}]}}"#,
            serde_json::to_string(text).unwrap()
        );
        let mut bundles = parse_collections(
            Some(CollectionFormat::Insomnia),
            input.as_bytes(),
            &CollectionOptions::default(),
        )
        .expect("parses");
        let c = bundles.remove(0).collection;
        assert_eq!(
            find(&c.items, &["R"]).request.body,
            graphql("", ""),
            "{text}"
        );
    }
}

#[test]
fn insomnia_v5_graphql_requests_import() {
    let c = import(CollectionFormat::Insomnia, "insomnia-v5-graphql.yaml");
    assert!(c.unwritten.is_empty(), "{:#?}", c.unwritten);
    let r = find(&c.items, &["Catalog", "Products"]);
    assert_eq!(r.request.kind, RequestKind::Graphql);
    assert_eq!(r.request.url, "{{base_url}}/graphql");
    assert_eq!(
        r.request.body,
        graphql("{ products(first: 2) { sku } }", "")
    );
    assert_eq!(
        r.request.auth,
        AuthSpec::Basic {
            username: "ann".into(),
            password: "{{password}}".into()
        }
    );
    assert_eq!(
        r.request.headers,
        [KeyValue::new("Accept", "application/json")]
    );
    let count = find(&c.items, &["Count"]);
    assert_eq!(
        count.request.body,
        graphql("query Count($n: Int) { count(n: $n) }", "{\n  \"n\": 3\n}")
    );
    // An empty operation name loses nothing.
    assert!(count.notes.is_empty(), "{:#?}", count.notes);
}

#[test]
fn hoppscotch_graphql_requests_import_with_their_folders() {
    let c = import(CollectionFormat::Hoppscotch, "hoppscotch-graphql-coll.json");
    assert!(c.unwritten.is_empty(), "{:#?}", c.unwritten);
    let r = find(&c.items, &["Users", "User by id"]);
    assert_eq!(r.request.kind, RequestKind::Graphql);
    assert_eq!(r.request.method, "POST");
    assert_eq!(r.request.url, "{{baseUrl}}/graphql");
    assert_eq!(
        r.request.body,
        graphql(
            "query User($id: ID!) { user(id: $id) { id name } }",
            "{\n  \"id\": \"{{userId}}\"\n}"
        )
    );
    let mut trace = KeyValue::new("X-Trace", "on");
    trace.enabled = false;
    assert_eq!(r.request.headers, [trace]);
    assert_eq!(
        r.request.auth,
        AuthSpec::Bearer {
            token: "{{token}}".into()
        }
    );
    assert_eq!(r.request.docs, "Looks up one user");

    // A legacy entry has no auth, which its migration reads as none.
    let legacy = find(&c.items, &["Legacy query"]);
    assert_eq!(legacy.request.kind, RequestKind::Graphql);
    assert_eq!(legacy.request.body, graphql("{ status }", ""));
    assert_eq!(legacy.request.auth, AuthSpec::None);
    assert_eq!(
        legacy.request.headers,
        [KeyValue::new("Accept", "application/json")]
    );

    let scripted = find(&c.items, &["Scripted"]);
    assert_eq!(
        scripted.request.auth,
        AuthSpec::Basic {
            username: "ann".into(),
            password: "{{pw}}".into()
        }
    );
    assert!(
        scripted
            .notes
            .iter()
            .any(|n| n.kind == NoteKind::Dropped && n.source == "Scripted / preRequestScript"),
        "{:#?}",
        scripted.notes
    );
    let seqs: Vec<u64> = c
        .items
        .iter()
        .map(|i| match i {
            ImportedItem::Folder(f) => f.seq,
            ImportedItem::Request(r) => r.seq,
            _ => unreachable!("only folders and requests exist"),
        })
        .collect();
    assert_eq!(seqs, [1, 2, 3]);
}

#[test]
fn hoppscotch_graphql_unknown_members_are_noted() {
    let input = br#"{"v": 12, "name": "c", "folders": [], "requests": [
        {"v": 10, "name": "q", "url": "u", "query": "{ a }", "variables": "", "zzProbe": 1}]}"#;
    let mut bundles = parse_collections(
        Some(CollectionFormat::Hoppscotch),
        input,
        &CollectionOptions::default(),
    )
    .expect("parses");
    let c = bundles.remove(0).collection;
    let r = find(&c.items, &["q"]);
    assert!(
        r.notes
            .iter()
            .any(|n| n.kind == NoteKind::Dropped && n.source == "q / zzProbe"),
        "{:#?}",
        r.notes
    );
}

#[test]
fn written_graphql_imports_match_their_snapshots() {
    let cases = [
        (CollectionFormat::Insomnia, "insomnia-v4-graphql.json"),
        (CollectionFormat::Insomnia, "insomnia-v5-graphql.yaml"),
        (CollectionFormat::Hoppscotch, "hoppscotch-graphql-coll.json"),
    ];
    for (format, name) in cases {
        let text = written(&import(format, name));
        assert!(text.contains("type: graphql"), "{name}:\n{text}");
        insta::assert_snapshot!(name.replace('.', "_"), text);
    }
}

fn insomnia_one(request: &str) -> ImportedRequestItem {
    let input = format!(
        r#"{{"_type": "export", "__export_format": 4, "resources": [
        {{"_id": "wrk", "_type": "workspace", "parentId": null, "name": "w"}},
        {{"_id": "req", "_type": "request", "parentId": "wrk", "name": "R", "method": "POST",
          "url": "u", {request}}}]}}"#
    );
    let mut bundles = parse_collections(
        Some(CollectionFormat::Insomnia),
        input.as_bytes(),
        &CollectionOptions::default(),
    )
    .expect("parses");
    find(&bundles.remove(0).collection.items, &["R"]).clone()
}

fn hoppscotch_one(request: &str) -> ImportedCollection {
    let input = format!(r#"{{"v": 12, "name": "c", "folders": [], "requests": [{request}]}}"#);
    let mut bundles = parse_collections(
        Some(CollectionFormat::Hoppscotch),
        input.as_bytes(),
        &CollectionOptions::default(),
    )
    .expect("parses");
    bundles.remove(0).collection
}

#[test]
fn an_insomnia_graphql_content_type_header_is_sent_as_json() {
    let r = insomnia_one(
        r#""headers": [{"name": "content-type", "value": "application/graphql; charset=utf-8"},
            {"name": "Accept", "value": "application/graphql"}],
          "body": {"mimeType": "application/graphql", "text": "{\"query\": \"{ a }\"}"}"#,
    );
    assert_eq!(
        r.request.headers,
        [
            KeyValue::new("content-type", "application/json"),
            KeyValue::new("Accept", "application/graphql")
        ]
    );
    assert!(
        r.notes.iter().any(|n| n.kind == NoteKind::Converted
            && n.source == "R / headers[0].value"
            && n.detail.contains("application/json")),
        "{:#?}",
        r.notes
    );
    // An HTTP request keeps the header as written.
    let http = insomnia_one(
        r#""headers": [{"name": "Content-Type", "value": "application/graphql"}],
          "body": {"mimeType": "text/plain", "text": "{ a }"}"#,
    );
    assert_eq!(
        http.request.headers,
        [KeyValue::new("Content-Type", "application/graphql")]
    );
}

#[test]
fn an_insomnia_graphql_object_without_query_or_variables_is_noted() {
    let r = insomnia_one(r#""body": {"mimeType": "application/graphql", "text": "{}"}"#);
    assert_eq!(r.request.body, graphql("", ""));
    assert!(
        r.notes.iter().any(|n| n.kind == NoteKind::Dropped
            && n.source == "R / body.text"
            && n.detail.contains("not JSON")),
        "{:#?}",
        r.notes
    );
}

#[test]
fn a_newer_hoppscotch_graphql_version_is_noted() {
    let c = hoppscotch_one(r#"{"v": 11, "name": "q", "url": "u", "query": "{ a }"}"#);
    let r = find(&c.items, &["q"]);
    assert_eq!(r.request.kind, RequestKind::Graphql);
    assert!(
        r.notes
            .iter()
            .any(|n| n.kind == NoteKind::Converted && n.source == "q / v"),
        "{:#?}",
        r.notes
    );
    let known = hoppscotch_one(r#"{"v": 10, "name": "q", "url": "u", "query": "{ a }"}"#);
    assert!(find(&known.items, &["q"]).notes.is_empty());
    let odd = hoppscotch_one(r#"{"v": 1.5, "name": "q", "url": "u", "query": "{ a }"}"#);
    assert!(odd.items.is_empty());
    assert!(
        odd.unwritten
            .iter()
            .any(|n| n.kind == NoteKind::Dropped && n.source == "q"),
        "{:#?}",
        odd.unwritten
    );
}

#[test]
fn a_graphql_request_can_inherit_its_auth() {
    let c = hoppscotch_one(
        r#"{"v": 10, "name": "q", "url": "u", "query": "{ a }",
            "auth": {"authType": "inherit", "authActive": true}}"#,
    );
    assert_eq!(find(&c.items, &["q"]).request.auth, AuthSpec::Inherit);
    let r = insomnia_one(
        r#""authentication": {}, "body": {"mimeType": "application/graphql", "text": "{\"query\": \"{ a }\"}"}"#,
    );
    assert_eq!(r.request.kind, RequestKind::Graphql);
    assert_eq!(r.request.auth, AuthSpec::Inherit);
}
