//! `parse_postman`: schema and envelope, folders and requests, names and seq, URLs, headers and
//! bodies, as Bruno's `postman-to-bruno.js` maps them.

use apinata_core::import::{
    ImportedCollection, ImportedFolder, ImportedItem, ImportedRequestItem, NoteKind, PostmanError,
    parse_postman,
};
use apinata_core::{BodySpec, KeyValue, MultipartValue, ParamKind};
use std::fs;
use std::path::Path;

const V21: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";

fn fixture(name: &str) -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/postman/collections")
            .join(name),
    )
    .expect("fixture")
}

/// A v2.1 collection named `c` holding `items` (a JSON array).
fn collection(items: &str) -> ImportedCollection {
    let json = format!(r#"{{"info": {{"name": "c", "schema": "{V21}"}}, "item": {items}}}"#);
    parse_postman(json.as_bytes()).expect("parses")
}

fn folder<'a>(items: &'a [ImportedItem], name: &str) -> &'a ImportedFolder {
    items
        .iter()
        .find_map(|i| match i {
            ImportedItem::Folder(f) if f.name == name => Some(f),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no folder {name}"))
}

fn request<'a>(items: &'a [ImportedItem], name: &str) -> &'a ImportedRequestItem {
    items
        .iter()
        .find_map(|i| match i {
            ImportedItem::Request(r) if r.name == name => Some(r),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no request {name}"))
}

fn names(items: &[ImportedItem]) -> Vec<(String, u64)> {
    items
        .iter()
        .map(|i| match i {
            ImportedItem::Folder(f) => (format!("folder {}", f.name), f.seq),
            ImportedItem::Request(r) => (format!("request {}", r.name), r.seq),
            _ => unreachable!("only folders and requests exist"),
        })
        .collect()
}

#[test]
fn only_v2_schemas_are_accepted_bare_or_enveloped() {
    for schema in [
        V21,
        "https://schema.getpostman.com/json/collection/v2.0.0/collection.json",
        "https://schema.postman.com/json/collection/v2.1.0/collection.json",
        "https://schema.postman.com/json/collection/v2.0.0/collection.json",
    ] {
        let bare = format!(r#"{{"info": {{"name": "c", "schema": "{schema}"}}, "item": []}}"#);
        assert_eq!(parse_postman(bare.as_bytes()).expect("bare").name, "c");
        let wrapped = format!(r#"{{"collection": {bare}}}"#);
        assert_eq!(
            parse_postman(wrapped.as_bytes()).expect("wrapped").name,
            "c"
        );
    }
    for rejected in [
        "postman-invalid-schema.json",
        "postman-invalid-missing-info.json",
        "postman-malformed.json",
    ] {
        let result = parse_postman(&fixture(rejected));
        assert!(
            matches!(result, Err(PostmanError::UnsupportedSchema { .. })),
            "{rejected}: {result:?}"
        );
    }
    let v1 = br#"{"info": {"name": "c", "schema": "https://schema.getpostman.com/json/collection/v1.0.0/collection.json"}}"#;
    assert!(
        matches!(parse_postman(v1), Err(PostmanError::UnsupportedSchema { schema }) if schema.contains("v1.0.0"))
    );
}

#[test]
fn malformed_input_is_a_parse_error_with_a_byte_offset_or_invalid() {
    match parse_postman(b"{\n  \"info\": ,\n}") {
        Err(PostmanError::Parse { offset, .. }) => assert_eq!(offset, 12),
        other => panic!("expected a parse error, got {other:?}"),
    }
    let wrong_shape = format!(r#"{{"info": {{"name": "c", "schema": "{V21}"}}, "item": "no"}}"#);
    assert!(matches!(
        parse_postman(wrong_shape.as_bytes()),
        Err(PostmanError::Invalid { .. })
    ));
    let unnamed = format!(r#"{{"info": {{"schema": "{V21}"}}, "item": []}}"#);
    assert_eq!(
        parse_postman(unnamed.as_bytes()).expect("parses").name,
        "Untitled Collection"
    );
}

#[test]
fn folders_and_requests_keep_names_seq_and_brunos_dedupe() {
    let c = parse_postman(&fixture("apinata-edge-cases.json")).expect("parses");
    assert_eq!(c.name, "Edge: cases/1");
    assert_eq!(
        names(&c.items),
        [
            ("folder Users".to_owned(), 1),
            ("folder Bodies".to_owned(), 2),
            ("request Settings".to_owned(), 3),
            ("request Api key".to_owned(), 4),
            ("request No auth".to_owned(), 5)
        ]
    );
    assert_eq!(
        names(&folder(&c.items, "Users").items),
        [
            ("request Get user".to_owned(), 1),
            ("request Get user_1".to_owned(), 2),
            ("request a/b".to_owned(), 3),
            ("request a-b".to_owned(), 4)
        ]
    );
    let c = collection(
        r#"[{"name": "A", "item": []}, {"name": "A", "request": {"method": "GET", "url": ""}},
        {"name": "A", "item": []}, {"name": "A", "item": []}, {"item": []}, {"request": {"method": "get", "url": ""}}]"#,
    );
    assert_eq!(
        names(&c.items),
        [
            ("folder A".to_owned(), 1),
            ("request A".to_owned(), 2),
            ("folder A_1".to_owned(), 3),
            ("folder A_2".to_owned(), 4),
            ("folder Untitled Folder".to_owned(), 5),
            ("request Untitled Request".to_owned(), 6)
        ]
    );
}

#[test]
fn skipped_items_leave_seq_gaps_and_notes_without_files() {
    let c = collection(
        r#"[7, {"name": "No method", "request": {"url": "u"}},
        {"name": "Gql", "request": {"method": "POST", "url": "u", "body": {"mode": "graphql", "graphql": {"query": "{ a }"}}}},
        {"name": "Kept", "request": {"method": "GET", "url": "u"}}]"#,
    );
    assert_eq!(
        names(&c.items),
        [
            ("request Gql".to_owned(), 3),
            ("request Kept".to_owned(), 4)
        ]
    );
    let sources: Vec<(&str, NoteKind)> = c
        .unwritten
        .iter()
        .map(|n| (n.source.as_str(), n.kind))
        .collect();
    assert_eq!(
        sources,
        [
            ("Item 1", NoteKind::Dropped),
            ("No method", NoteKind::Dropped)
        ]
    );
    assert!(c.unwritten.iter().all(|n| n.file.is_none()));
}

#[test]
fn urls_params_and_headers_map_as_bruno_maps_them() {
    let c = parse_postman(&fixture("apinata-edge-cases.json")).expect("parses");
    let get = request(&folder(&c.items, "Users").items, "Get user");
    let r = &get.request;
    assert_eq!(r.method, "GET");
    assert_eq!(r.url, "https://{{host}}/users/:id?expand=true&debug=1");
    assert!(
        get.notes
            .iter()
            .any(|n| n.kind == NoteKind::Converted
                && n.source == "Users / Get user / request.url.raw")
    );
    let params: Vec<_> = r
        .params
        .iter()
        .map(|p| {
            (
                p.name.as_str(),
                p.value.as_str(),
                p.kind,
                p.enabled,
                p.description.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        params,
        [
            (
                "expand",
                "true",
                ParamKind::Query,
                true,
                Some("Expand relations")
            ),
            ("debug", "1", ParamKind::Query, false, None),
            ("id", "42", ParamKind::Path, true, Some("User id"))
        ]
    );
    let mut accept = KeyValue::new("Accept", "application/json");
    accept.description = Some("What we take".into());
    let mut off = KeyValue::new("X-Off", "1");
    off.enabled = false;
    assert_eq!(
        r.headers,
        [accept, off],
        "a blank description reads as none"
    );
    assert_eq!(r.docs, "Fetches one user");
    let form = &request(&folder(&c.items, "Bodies").items, "Form").request;
    assert_eq!(
        form.url, "https://api.example.com:8443/form?q=x",
        "built from parts"
    );
    assert_eq!(
        form.headers,
        [
            KeyValue::new("Content-Type", "application/x-www-form-urlencoded"),
            KeyValue::new("X-Trace", "on")
        ]
    );
}

#[test]
fn bodies_map_to_core_body_modes() {
    let c = parse_postman(&fixture("apinata-edge-cases.json")).expect("parses");
    let bodies = &folder(&c.items, "Bodies").items;
    let BodySpec::FormUrlEncoded(fields) = &request(bodies, "Form").request.body else {
        panic!("form")
    };
    assert_eq!(
        fields
            .iter()
            .map(|f| (f.name.as_str(), f.enabled, f.description.as_deref()))
            .collect::<Vec<_>>(),
        [("a", true, Some("First")), ("b", false, None)]
    );
    let BodySpec::Multipart(parts) = &request(bodies, "Multipart").request.body else {
        panic!("multipart")
    };
    assert_eq!(parts[0].value, MultipartValue::Text("text".into()));
    assert_eq!(parts[0].description.as_deref(), Some("Text part"));
    assert_eq!(
        parts[1].value,
        MultipartValue::Files(vec!["/tmp/a.png".into()])
    );
    assert_eq!(parts[1].content_type, "image/png");
    assert_eq!(
        parts[2].value,
        MultipartValue::Files(vec!["/tmp/b.txt".into()]),
        "default type with src is a file"
    );
    assert_eq!(
        request(bodies, "Guess JSON").request.body,
        BodySpec::Text("{\"a\": 1}".into()),
        "vnd.api+json fails Bruno's pattern"
    );
    assert_eq!(
        request(bodies, "Xml").request.body,
        BodySpec::Xml("<a/>".into())
    );
    let BodySpec::File(files) = &request(bodies, "Binary").request.body else {
        panic!("file")
    };
    assert_eq!(
        (
            files[0].path.as_str(),
            files[0].content_type.as_str(),
            files[0].selected
        ),
        ("/tmp/data.bin", "application/octet-stream", true)
    );
    let c = collection(
        r#"[
        {"name": "j", "request": {"method": "POST", "url": "u", "header": [{"key": "Content-Type", "value": "x/y", "disabled": true}, {"key": "content-type", "value": "application/hal+json"}], "body": {"mode": "raw", "raw": "{}"}}},
        {"name": "x", "request": {"method": "POST", "url": "u", "header": "Content-Type: text/xml", "body": {"mode": "raw", "raw": "<a/>"}}},
        {"name": "t", "request": {"method": "POST", "url": "u", "body": {"mode": "raw", "raw": "hi", "options": {"raw": {"language": "text"}}}}},
        {"name": "p", "request": {"method": "POST", "url": "u", "body": {"mode": "file", "file": {"src": "/a/b.PNG"}}}}]"#,
    );
    assert_eq!(
        request(&c.items, "j").request.body,
        BodySpec::Json("{}".into()),
        "the first enabled Content-Type decides"
    );
    assert_eq!(
        request(&c.items, "x").request.body,
        BodySpec::Xml("<a/>".into())
    );
    assert_eq!(
        request(&c.items, "t").request.body,
        BodySpec::Text("hi".into())
    );
    let BodySpec::File(files) = &request(&c.items, "p").request.body else {
        panic!("file")
    };
    assert_eq!(files[0].content_type, "image/png");
}
