//! ASAP auth through Postman import, Postman, HTML and OpenAPI export, auth JSON,
//! and its private key through the keychain.

use crate::export_support::{exported, http, open};

use apinata_core::import::{
    ImportedCollection, ImportedItem, auth_secret_fields, parse_postman, replace_auth_secret,
    write_collection,
};
use apinata_core::{
    AsapSpec, AuthSpec, Collection, ContextOptions, MemoryStore, SecretStore, VarContext,
    parse_auth_json, resolve,
};
use serde_json::{Value, json};
use std::fs;
use std::sync::Arc;
use tempfile::TempDir;

const V21: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";
const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: c\n";
const RSA: &str = include_str!("fixtures/jwt/rsa.pem");

fn postman(entries: Value) -> ImportedCollection {
    let doc = json!({"info": {"name": "c", "schema": V21}, "item": [
        {"name": "r", "request": {"method": "GET", "url": "http://127.0.0.1:9/a",
            "auth": {"type": "asap", "asap": entries}}}]});
    parse_postman(doc.to_string().as_bytes()).expect("parses")
}

fn only(c: &ImportedCollection) -> &apinata_core::import::ImportedRequestItem {
    match c.items.as_slice() {
        [ImportedItem::Request(r)] => r,
        other => panic!("one request: {other:?}"),
    }
}

fn asap_of(c: &ImportedCollection) -> AsapSpec {
    match &only(c).request.auth {
        AuthSpec::Asap(spec) => spec.clone(),
        other => panic!("not asap: {other:?}"),
    }
}

fn kv(key: &str, value: Value) -> Value {
    json!({"key": key, "value": value})
}

#[test]
fn postman_asap_maps_each_key_and_serializes_object_claims() {
    let c = postman(json!([
        kv("alg", json!("PS256")),
        kv("kid", json!("svc/a/key1")),
        kv("iss", json!("svc/a")),
        kv("aud", json!("svc/b")),
        kv("sub", json!("user-1")),
        kv("exp", json!("600")),
        kv("privateKey", json!("{{asap_key}}")),
        kv("claims", json!({"scope": "read"})),
    ]));
    let spec = asap_of(&c);
    assert_eq!(spec.algorithm, "PS256");
    assert_eq!(spec.key_id, "svc/a/key1");
    assert_eq!(spec.issuer, "svc/a");
    assert_eq!(spec.audience, "svc/b");
    assert_eq!(spec.subject, "user-1");
    assert_eq!(spec.expires_in, "600");
    assert_eq!(spec.private_key, "{{asap_key}}");
    assert_eq!(spec.additional_claims, r#"{"scope":"read"}"#);
}

#[test]
fn postman_asap_defaults_fill_what_is_missing() {
    let spec = asap_of(&postman(json!([kv("iss", json!("i"))])));
    assert_eq!(spec.algorithm, "RS256");
    assert_eq!(spec.expires_in, "");
    assert_eq!(spec.additional_claims, "");
}

#[test]
fn postman_export_maps_asap_back() {
    let auth = "  auth:\n    type: asap\n    algorithm: RS512\n    issuer: svc/a\n    audience: svc/b\n    keyId: k1\n    subject: s\n    expiresIn: \"60\"\n    additionalClaims: '{\"x\":1}'\n    privateKey: \"{{asap_key}}\"\n";
    let r = http("r", "GET", "http://x.test", auth);
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("r.yml", &r)]);
    let v = exported(&c);
    let got = &v["item"][0]["request"]["auth"];
    assert_eq!(got["type"], "asap");
    let entry = |key: &str| {
        got["asap"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["key"] == key)
            .map(|e| e["value"].clone())
            .unwrap_or_else(|| panic!("{key} in {got}"))
    };
    assert_eq!(entry("alg"), "RS512");
    assert_eq!(entry("iss"), "svc/a");
    assert_eq!(entry("aud"), "svc/b");
    assert_eq!(entry("kid"), "k1");
    assert_eq!(entry("sub"), "s");
    assert_eq!(entry("exp"), "60");
    assert_eq!(entry("claims"), "{\"x\":1}");
    assert_eq!(entry("privateKey"), "{{asap_key}}");
    let back = asap_of(&parse_postman(v.to_string().as_bytes()).expect("parses"));
    assert_eq!(back.algorithm, "RS512");
    assert_eq!(back.additional_claims, "{\"x\":1}");
    assert_eq!(back.expires_in, "60");
}

#[test]
fn html_and_openapi_name_asap() {
    let auth = "  auth:\n    type: asap\n    issuer: i\n    privateKey: k-secret\n";
    let r = http("r", "GET", "http://x.test", auth);
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("r.yml", &r)]);
    let page = crate::export_support::html_page(&c);
    assert!(page.contains("ASAP"), "the label");
    assert!(!page.contains("k-secret"), "the key stays out");
    let notes = crate::export_support::openapi_notes(&c);
    assert!(
        notes.contains(&(
            "r / auth".to_owned(),
            "the ASAP authentication cannot be exported".to_owned()
        )),
        "{notes:?}"
    );
}

#[test]
fn an_imported_literal_key_goes_to_the_store_and_still_resolves() {
    let one_line = RSA.trim().replace('\n', "\\n");
    let c = postman(json!([
        kv("iss", json!("i")),
        kv("aud", json!("a")),
        kv("kid", json!("k")),
        kv("privateKey", json!(one_line)),
    ]));
    let parent = TempDir::new().unwrap();
    let store = Arc::new(MemoryStore::new());
    let (written, _report) = write_collection(parent.path(), &c, &[], store.as_ref()).unwrap();
    let file = fs::read_to_string(written.root().join("r.yml")).unwrap();
    let body: String = RSA.lines().nth(1).unwrap().to_owned();
    assert!(!file.contains(&body), "{file}");
    assert!(!file.contains("PRIVATE KEY"), "{file}");
    let collection = Collection::open(written.root()).unwrap();
    let ctx = VarContext::new(
        &collection,
        ContextOptions::new()
            .env("Environment")
            .secrets(Arc::clone(&store) as Arc<dyn SecretStore>),
    )
    .unwrap();
    let r = resolve(&collection.tree().requests()[0], &ctx).unwrap();
    assert!(r.sensitive_values().iter().any(|v| v.contains(&body)));
}

#[test]
fn move_to_a_secret_finds_and_replaces_only_the_key() {
    let mut spec = AsapSpec::default();
    spec.issuer = "literal-asap-key".into();
    spec.private_key = "literal-asap-key".into();
    let mut auth = AuthSpec::Asap(spec);
    assert_eq!(
        auth_secret_fields(&auth),
        [("private_key", "literal-asap-key".to_owned())]
    );
    replace_auth_secret(&mut auth, "literal-asap-key", "{{asap_key}}");
    let AuthSpec::Asap(moved) = auth else {
        panic!("not asap")
    };
    assert_eq!(moved.private_key, "{{asap_key}}");
    assert_eq!(moved.issuer, "literal-asap-key");
}

/// An ASAP document with every field given.
const ASAP: &str = r#"{"type":"asap","algorithm":"ES256","issuer":"i","audience":"a",
    "key_id":"k","subject":"s","expires_in":"60","additional_claims":"{}",
    "private_key":"{{asap_key}}"}"#;

#[test]
fn auth_json_reads_every_asap_field_in_snake_case() {
    let AuthSpec::Asap(got) = parse_auth_json(ASAP).expect("parses") else {
        panic!("asap")
    };
    let mut want = AsapSpec::default();
    want.algorithm = "ES256".into();
    want.issuer = "i".into();
    want.audience = "a".into();
    want.key_id = "k".into();
    want.subject = "s".into();
    want.expires_in = "60".into();
    want.additional_claims = "{}".into();
    want.private_key = "{{asap_key}}".into();
    assert!(got == want, "{got:?}");
}

#[test]
fn a_bad_asap_document_names_its_field_and_never_the_key() {
    for (json, field) in [
        (ASAP.replace(r#""subject":"s","#, ""), "`subject`"),
        (
            ASAP.replace(r#""issuer":"i""#, r#""issuer":"i","iss":"x""#),
            "`iss`",
        ),
    ] {
        let err = parse_auth_json(&json).expect_err(&json);
        let text = err.to_string();
        assert!(text.contains(field), "{field}: {text}");
        assert!(!text.contains("asap_key"), "{text}");
    }
}
