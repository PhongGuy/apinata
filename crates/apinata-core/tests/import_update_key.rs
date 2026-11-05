//! The operation key and the canonical field hashes.

use apinata_core::import::{
    ChangedField, ImportedRequest, add_request, file_field_hashes, operation_key, spec_field_hashes,
};
use apinata_core::{
    BodySpec, Collection, ItemPath, KeyValue, Param, ParamKind, VarValue, Variable,
};
use tempfile::TempDir;

#[test]
fn the_key_drops_the_base_prefix_query_and_trailing_slash() {
    assert_eq!(
        operation_key("get", "{{baseUrl}}/users/:id?x=1"),
        "GET /users/{}"
    );
    assert_eq!(
        operation_key("GET", "{{host}}/users/{id}/"),
        "GET /users/{}"
    );
    assert_eq!(
        operation_key("GET", "{{baseUrl}}//users//:uid"),
        "GET /users/{}"
    );
    assert_eq!(
        operation_key("GET", "{{a}}/v1/x"),
        operation_key("GET", "{{b}}/v1/x")
    );
    assert_eq!(
        operation_key("POST", "{{baseUrl}}/users#top"),
        "POST /users"
    );
}

#[test]
fn a_partial_parameter_segment_is_compared_literally() {
    assert_eq!(operation_key("GET", "{{b}}/f/:id.json"), "GET /f/:id.json");
    assert_ne!(
        operation_key("GET", "{{b}}/f/:id.json"),
        operation_key("GET", "{{b}}/f/:id")
    );
}

#[test]
fn the_method_is_part_of_the_key() {
    assert_ne!(
        operation_key("GET", "{{b}}/u"),
        operation_key("POST", "{{b}}/u")
    );
}

fn rich_request() -> ImportedRequest {
    let mut r = ImportedRequest::new("GET", "{{baseUrl}}/users/:id?expand=1");
    r.params = vec![
        Param::new("expand", "1", ParamKind::Query),
        Param::new("id", "", ParamKind::Path),
    ];
    let mut off = KeyValue::new("X-Off", "1");
    off.enabled = false;
    off.description = Some("disabled on purpose".into());
    r.headers = vec![KeyValue::new("Accept", "application/json"), off];
    r.body = BodySpec::Json("{\"a\": 1}".into());
    r.docs = "Fetch a user".into();
    // Whitespace and a duplicate: the tag setter normalises these.
    r.tags = vec![" people ".into(), "people".into(), "read".into()];
    r.vars = vec![Variable::new("k", VarValue::Text("v".into()))];
    r
}

#[test]
fn spec_side_and_file_side_hashes_agree() {
    let dir = TempDir::new().unwrap();
    let mut c = Collection::create(&dir.path().join("c"), "c").unwrap();
    for request in [
        rich_request(),
        ImportedRequest::new("DELETE", "{{baseUrl}}/x"),
    ] {
        let path = add_request(&mut c, &ItemPath::root(), "r", &request).unwrap();
        let on_file = file_field_hashes(c.request(&path).unwrap());
        assert_eq!(spec_field_hashes(&request).unwrap(), on_file);
    }
}

#[test]
fn hashes_are_16_lowercase_hex_and_differ_only_in_the_changed_field() {
    let a = spec_field_hashes(&rich_request()).unwrap();
    let mut changed = rich_request();
    changed.body = BodySpec::Json("{\"a\": 2}".into());
    let b = spec_field_hashes(&changed).unwrap();
    for (field, hash) in a.iter() {
        assert_eq!(hash.len(), 16, "{field:?}");
        assert!(
            hash.bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        );
        assert_eq!(
            hash == b.get(field).unwrap(),
            field != ChangedField::Body,
            "{field:?}"
        );
    }
    assert_eq!(a.iter().count(), 8);
}

#[test]
fn an_absent_field_hashes_the_same_whatever_its_spelling() {
    let none = ImportedRequest::new("GET", "{{b}}/x");
    let mut empty = none.clone();
    empty.docs = String::new();
    empty.tags = vec!["  ".into()];
    assert_eq!(
        spec_field_hashes(&none).unwrap(),
        spec_field_hashes(&empty).unwrap()
    );
}
