//! Folders from the first tag or from path segments.

use crate::openapi_support;

use apinata_core::import::{GroupBy, NoteKind};
use openapi_support::{folder, import, import_by, notes, request, tree};
use serde_json::json;

fn doc() -> serde_json::Value {
    json!({"openapi": "3.0.0", "servers": [{"url": "https://x.test"}],
    "tags": [{"name": "store", "description": "Orders"}, {"name": "pets", "description": "Pet ops"}],
    "paths": {
        "/": {"get": {"summary": "Home"}},
        "/pets": {"get": {"summary": "List", "tags": [" pets ", "animals", 7]}, "post": {"summary": "Add", "tags": ["pets"]}},
        "/pets/{id}": {"get": {"summary": "Show", "tags": ["pets"]}},
        "/store/orders": {"get": {"summary": "List", "tags": ["store"]}},
        "/misc": {"get": {"summary": "Untagged", "tags": []}, "put": {"summary": "Blank", "tags": ["  "]}}
    }})
}

#[test]
fn tags_group_by_first_tag_in_order_of_first_use_with_untagged_requests_last() {
    let c = import(&doc()).collection;
    assert_eq!(
        tree(&c),
        [
            "F 1 pets",
            "  R 1 List GET {{baseUrl}}/pets",
            "  R 2 Add POST {{baseUrl}}/pets",
            "  R 3 Show GET {{baseUrl}}/pets/:id",
            "F 2 store",
            "  R 1 List GET {{baseUrl}}/store/orders",
            "R 3 Home GET {{baseUrl}}/",
            "R 4 Untagged GET {{baseUrl}}/misc",
            "R 5 Blank PUT {{baseUrl}}/misc",
        ]
    );
    assert_eq!(
        (
            folder(&c, "pets").docs.as_str(),
            folder(&c, "store").docs.as_str()
        ),
        ("Pet ops", "Orders")
    );
    assert_eq!(request(&c, "pets/List").request.tags, ["pets", "animals"]);
    assert!(request(&c, "Blank").request.tags.is_empty());
}

#[test]
fn names_are_deduplicated_per_folder() {
    assert!(
        notes(&import(&doc()).collection)
            .iter()
            .all(|n| n.kind != NoteKind::Renamed)
    );
    let twice = json!({"openapi": "3.0.0", "paths": {
        "/a": {"get": {"summary": "X", "tags": ["t"]}}, "/b": {"get": {"summary": "X", "tags": ["t"]}}
    }});
    assert_eq!(
        tree(&import(&twice).collection),
        [
            "F 1 t",
            "  R 1 X GET {{baseUrl}}/a",
            "  R 2 X (GET) GET {{baseUrl}}/b"
        ]
    );
}

#[test]
fn paths_group_by_segment_with_requests_before_subfolders() {
    let c = import_by(&doc(), GroupBy::Path).collection;
    assert_eq!(
        tree(&c),
        [
            "F 1 Root",
            "  R 1 Home GET {{baseUrl}}/",
            "F 2 pets",
            "  R 1 List GET {{baseUrl}}/pets",
            "  R 2 Add POST {{baseUrl}}/pets",
            "  F 3 {id}",
            "    R 1 Show GET {{baseUrl}}/pets/:id",
            "F 3 store",
            "  F 1 orders",
            "    R 1 List GET {{baseUrl}}/store/orders",
            "F 4 misc",
            "  R 1 Untagged GET {{baseUrl}}/misc",
            "  R 2 Blank PUT {{baseUrl}}/misc",
        ]
    );
    assert!(folder(&c, "pets").docs.is_empty());
    assert_eq!(request(&c, "pets/List").request.tags, ["pets", "animals"]);
}

#[test]
fn swagger_2_groups_the_same_way() {
    let doc = json!({"swagger": "2.0", "host": "x.test", "tags": [{"name": "a", "description": "A"}],
        "paths": {"/x": {"get": {"summary": "One", "tags": ["a"]}}, "/y": {"get": {"summary": "Two"}}}});
    let c = import(&doc).collection;
    assert_eq!(
        tree(&c),
        [
            "F 1 a",
            "  R 1 One GET {{baseUrl}}/x",
            "R 2 Two GET {{baseUrl}}/y"
        ]
    );
    assert_eq!(folder(&c, "a").docs, "A");
}

#[test]
fn a_deep_path_stops_at_32_folders_and_many_paths_still_import() {
    let segments: Vec<String> = (0..5_000).map(|i| format!("s{i}")).collect();
    let deep = format!("/{}", segments.join("/"));
    let mut paths = serde_json::Map::new();
    paths.insert(deep.clone(), json!({"get": {"summary": "Deep"}}));
    let c = import_by(&json!({"openapi": "3.0.0", "paths": paths}), GroupBy::Path).collection;
    let lines = tree(&c);
    assert_eq!(lines.len(), 33);
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.trim_start().starts_with("F "))
            .count(),
        32
    );
    let last = segments[31..].join("/");
    assert_eq!(lines[31], format!("{}F 1 {last}", "  ".repeat(31)));
    assert_eq!(
        lines[32],
        format!("{}R 1 Deep GET {{{{baseUrl}}}}{deep}", "  ".repeat(32))
    );
    let converted: Vec<_> = c
        .unwritten
        .iter()
        .filter(|n| n.kind == NoteKind::Converted)
        .collect();
    assert_eq!(converted.len(), 1);
    assert_eq!(converted[0].source, format!("paths.{deep}"));
    assert!(
        converted[0].detail.contains("32"),
        "{}",
        converted[0].detail
    );

    let mut many = serde_json::Map::new();
    for i in 0..2_000 {
        many.insert(
            format!("/r{i}/{{id}}/items"),
            json!({"get": {"summary": "x"}}),
        );
    }
    let c = import_by(&json!({"openapi": "3.0.0", "paths": many}), GroupBy::Path).collection;
    assert_eq!(c.items.len(), 2_000);
    assert_eq!(tree(&c).len(), 2_000 * 4);
    assert!(c.unwritten.is_empty());
}
