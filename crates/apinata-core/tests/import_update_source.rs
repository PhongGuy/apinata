//! Only an OpenAPI import is marked as updatable.

use crate::import_update_support::{bundle, spec};
use apinata_core::import::{CollectionFormat, CollectionOptions, parse_collections};

#[test]
fn an_openapi_import_is_marked_as_coming_from_a_spec() {
    assert!(bundle(&spec("update-v1.json")).collection.spec_source);
}

#[test]
fn other_formats_are_not() {
    let har = br#"{"log":{"version":"1.2","entries":[]}}"#;
    let mut options = CollectionOptions::default();
    options.name = Some("h".into());
    let b = parse_collections(Some(CollectionFormat::Har), har, &options)
        .unwrap()
        .remove(0);
    assert!(!b.collection.spec_source);
    let postman = br#"{"info":{"name":"P","schema":"https://schema.getpostman.com/json/collection/v2.1.0/collection.json"},"item":[]}"#;
    let b = parse_collections(
        Some(CollectionFormat::Postman),
        postman,
        &CollectionOptions::default(),
    )
    .unwrap()
    .remove(0);
    assert!(!b.collection.spec_source);
}
