//! Hawk auth through Postman import and export, and its key through the keychain.

use crate::export_support::{exported, http, open};

use apinata_core::import::{
    ImportNote, ImportedCollection, ImportedItem, NoteKind, parse_postman, write_collection,
};
use apinata_core::{
    AuthSpec, Collection, ContextOptions, HawkSpec, MemoryStore, SecretStore, VarContext, resolve,
};
use serde_json::{Value, json};
use std::fs;
use std::sync::Arc;
use tempfile::TempDir;

const V21: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";
const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: c\n";

fn postman(entries: Value) -> ImportedCollection {
    let doc = json!({"info": {"name": "c", "schema": V21}, "item": [
        {"name": "r", "request": {"method": "GET", "url": "http://127.0.0.1:9/a",
            "auth": {"type": "hawk", "hawk": entries}}}]});
    parse_postman(doc.to_string().as_bytes()).expect("parses")
}

fn hawk_of(c: &ImportedCollection) -> HawkSpec {
    match c.items.as_slice() {
        [ImportedItem::Request(r)] => match &r.request.auth {
            AuthSpec::Hawk(spec) => spec.clone(),
            other => panic!("not hawk: {other:?}"),
        },
        other => panic!("one request: {other:?}"),
    }
}

fn notes_of(c: &ImportedCollection) -> Vec<ImportNote> {
    match c.items.as_slice() {
        [ImportedItem::Request(r)] => r.notes.clone(),
        other => panic!("one request: {other:?}"),
    }
}

fn kv(key: &str, value: Value) -> Value {
    json!({"key": key, "value": value})
}

#[test]
fn postman_hawk_maps_each_key_and_notes_a_user_it_drops() {
    let c = postman(json!([
        kv("authId", json!("id-1")),
        kv("authKey", json!("{{hawk_key}}")),
        kv("algorithm", json!("sha1")),
        kv("user", json!("alice")),
        kv("nonce", json!("n1")),
        kv("extraData", json!("x")),
        kv("app", json!("a")),
        kv("delegation", json!("d")),
        kv("timestamp", json!("1353832234")),
        kv("includePayloadHash", json!(true)),
    ]));
    let spec = hawk_of(&c);
    assert_eq!(spec.auth_id, "id-1");
    assert_eq!(spec.auth_key, "{{hawk_key}}");
    assert_eq!(spec.algorithm, "sha1");
    assert_eq!(spec.nonce, "n1");
    assert_eq!(spec.ext, "x");
    assert_eq!(spec.app, "a");
    assert_eq!(spec.dlg, "d");
    assert_eq!(spec.timestamp, "1353832234");
    assert!(spec.include_payload_hash);
    let notes = notes_of(&c);
    let dropped = notes
        .iter()
        .any(|n| n.kind == NoteKind::Dropped && n.detail.contains("user"));
    assert!(dropped, "{notes:#?}");
}

#[test]
fn postman_hawk_defaults_fill_what_is_missing() {
    let c = postman(json!([kv("authId", json!("id-1")), kv("user", json!(""))]));
    let spec = hawk_of(&c);
    assert_eq!(spec.algorithm, "sha256");
    assert!(!spec.include_payload_hash);
    let notes = notes_of(&c);
    assert!(
        notes.iter().all(|n| !n.detail.contains("user")),
        "{notes:#?}"
    );
}

#[test]
fn postman_export_maps_hawk_back() {
    let auth = "  auth:\n    type: hawk\n    authId: id-1\n    authKey: \"{{hawk_key}}\"\n    algorithm: sha1\n    ext: x\n    app: a\n    dlg: d\n    nonce: n1\n    timestamp: \"1353832234\"\n    includePayloadHash: true\n";
    let r = http("r", "GET", "http://x.test", auth);
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("r.yml", &r)]);
    let v = exported(&c);
    let got = &v["item"][0]["request"]["auth"];
    assert_eq!(got["type"], "hawk");
    let entry = |key: &str| {
        got["hawk"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["key"] == key)
            .map(|e| e["value"].clone())
            .unwrap_or_else(|| panic!("{key} in {got}"))
    };
    assert_eq!(entry("authId"), "id-1");
    assert_eq!(entry("authKey"), "{{hawk_key}}");
    assert_eq!(entry("algorithm"), "sha1");
    assert_eq!(entry("extraData"), "x");
    assert_eq!(entry("app"), "a");
    assert_eq!(entry("delegation"), "d");
    assert_eq!(entry("nonce"), "n1");
    assert_eq!(entry("timestamp"), "1353832234");
    assert_eq!(entry("includePayloadHash"), true);
    let back = parse_postman(v.to_string().as_bytes()).expect("parses");
    let spec = hawk_of(&back);
    assert_eq!(spec.ext, "x");
    assert_eq!(spec.dlg, "d");
    assert!(spec.include_payload_hash);
}

#[test]
fn an_imported_literal_key_goes_to_the_store_and_still_resolves() {
    let c = postman(json!([
        kv("authId", json!("id-1")),
        kv("authKey", json!("literal-hawk-key")),
    ]));
    let parent = TempDir::new().unwrap();
    let store = Arc::new(MemoryStore::new());
    let (written, _report) = write_collection(parent.path(), &c, &[], store.as_ref()).unwrap();
    let file = fs::read_to_string(written.root().join("r.yml")).unwrap();
    assert!(!file.contains("literal-hawk-key"), "{file}");
    let collection = Collection::open(written.root()).unwrap();
    let ctx = VarContext::new(
        &collection,
        ContextOptions::new()
            .env("Environment")
            .secrets(Arc::clone(&store) as Arc<dyn SecretStore>),
    )
    .unwrap();
    let r = resolve(&collection.tree().requests()[0], &ctx).unwrap();
    assert!(r.sensitive_values().iter().any(|v| v == "literal-hawk-key"));
}
