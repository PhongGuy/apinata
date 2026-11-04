//! Hostile OpenAPI documents end in notes or an error.

use crate::openapi_support;

use apinata_core::import::{DocumentError, GroupBy, NoteKind};
use openapi_support::{import, import_bytes, requests};
use serde_json::{Map, Value, json};

const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

fn fan_out(paths: usize) -> Value {
    let mut map = Map::new();
    for i in 0..paths {
        map.insert(
            format!("/p{i}"),
            json!({"$ref": "#/components/pathItems/all"}),
        );
    }
    let all: Map<String, Value> = METHODS.iter().map(|m| (m.to_string(), json!({}))).collect();
    json!({"openapi": "3.1.0", "servers": [{"url": "https://x.test"}], "paths": map, "components": {"pathItems": {"all": all}}})
}

#[test]
fn a_wide_path_item_fan_out_imports_within_the_budget() {
    // 20,000 refs to one eight-method path item: 160,000 requests, well under the budget.
    let c = import(&fan_out(20_000)).collection;
    assert_eq!(c.items.len(), 160_000);
    assert_eq!(requests(&c).len(), 160_000);
    assert!(c.unwritten.is_empty() && c.notes.is_empty());
}

#[test]
#[ignore = "builds a million requests"]
fn a_million_entries_is_too_large() {
    let doc = fan_out(130_000);
    let err = import_bytes(doc.to_string().as_bytes(), GroupBy::Tags).unwrap_err();
    assert_eq!(
        err,
        DocumentError::TooLarge {
            limit: 1_000_000,
            unit: "imported entries"
        }
    );
}

#[test]
fn path_item_ref_cycles_and_long_chains_import_nothing_with_notes() {
    let mut paths = Map::new();
    paths.insert("/a".into(), json!({"$ref": "#/paths/~1b"}));
    paths.insert("/b".into(), json!({"$ref": "#/paths/~1a"}));
    paths.insert("/self".into(), json!({"$ref": "#/paths/~1self"}));
    for i in 0..100 {
        paths.insert(
            format!("/c{i}"),
            json!({"$ref": format!("#/paths/~1c{}", i + 1)}),
        );
    }
    paths.insert("/c100".into(), json!({"get": {"summary": "end"}}));
    let doc = json!({"openapi": "3.1.0", "servers": [{"url": "https://x.test"}], "paths": paths});
    let c = import(&doc).collection;
    // /c36 to /c99 reach /c100 within 64 hops, /c0 to /c35 do not.
    assert_eq!(c.items.len(), 65);
    let converted = c
        .unwritten
        .iter()
        .filter(|n| n.kind == NoteKind::Converted)
        .count();
    assert_eq!(converted, 3 + 36);
}

#[test]
fn a_byte_over_each_cap_is_too_large() {
    let mut json = vec![b' '; (64 << 20) + 1];
    json[0] = b'{';
    assert_eq!(
        import_bytes(&json, GroupBy::Tags).unwrap_err(),
        DocumentError::TooLarge {
            limit: 64 << 20,
            unit: "bytes"
        }
    );
    let yaml = vec![b'a'; (16 << 20) + 1];
    assert_eq!(
        import_bytes(&yaml, GroupBy::Tags).unwrap_err(),
        DocumentError::TooLarge {
            limit: 16 << 20,
            unit: "bytes"
        }
    );
}

/// `paths` refs to one `get` whose description is `size` bytes.
fn described(paths: usize, size: usize) -> Value {
    let mut map = Map::new();
    for i in 0..paths {
        map.insert(
            format!("/p{i}"),
            json!({"$ref": "#/components/pathItems/item"}),
        );
    }
    let item = json!({"get": {"description": "d".repeat(size)}});
    json!({"openapi": "3.1.0", "servers": [{"url": "https://x.test"}], "paths": map, "components": {"pathItems": {"item": item}}})
}

#[test]
fn a_fanned_out_description_past_the_byte_budget_is_too_large() {
    // 160 copies of a 2 MiB description as name and docs pass 256 MiB about halfway.
    let doc = described(160, 2 << 20);
    assert_eq!(
        import_bytes(doc.to_string().as_bytes(), GroupBy::Tags).unwrap_err(),
        DocumentError::TooLarge {
            limit: 256 << 20,
            unit: "imported bytes"
        }
    );
}

#[test]
#[ignore = "the review probe"]
fn a_megabyte_description_behind_400_refs_is_too_large() {
    let doc = described(400, 1 << 20);
    let err = import_bytes(doc.to_string().as_bytes(), GroupBy::Tags).unwrap_err();
    assert_eq!(
        err,
        DocumentError::TooLarge {
            limit: 256 << 20,
            unit: "imported bytes"
        }
    );
}

#[test]
fn many_requests_with_one_name_are_renamed_in_linear_time() {
    let mut map = Map::new();
    for i in 0..20_000 {
        map.insert(format!("/p{i}"), json!({"get": {"summary": "same"}}));
    }
    let doc = json!({"openapi": "3.0.0", "servers": [{"url": "https://x.test"}], "paths": map});
    let c = import(&doc).collection;
    let names: Vec<&str> = requests(&c).iter().map(|(_, r)| r.name.as_str()).collect();
    assert_eq!(names[..3], ["same", "same (GET)", "same (1)"]);
    assert_eq!(names[19_999], "same (19998)");
}
