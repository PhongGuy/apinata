//! JWT auth through Postman import and export, and its secrets through the keychain.

use crate::export_support::{exported, http, open};

use apinata_core::import::{ImportedCollection, ImportedItem, parse_postman, write_collection};
use apinata_core::{
    AuthSpec, Collection, ContextOptions, JwtAlgorithm, JwtPlacement, JwtSpec, MemoryStore,
    OAuth1KeyKind, OAuth1PrivateKey, SecretStore, VarContext, resolve,
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
            "auth": {"type": "jwt", "jwt": entries}}}]});
    parse_postman(doc.to_string().as_bytes()).expect("parses")
}

fn jwt_of(c: &ImportedCollection) -> JwtSpec {
    match c.items.as_slice() {
        [ImportedItem::Request(r)] => match &r.request.auth {
            AuthSpec::Jwt(spec) => spec.clone(),
            other => panic!("not jwt: {other:?}"),
        },
        other => panic!("one request: {other:?}"),
    }
}

fn kv(key: &str, value: Value) -> Value {
    json!({"key": key, "value": value})
}

#[test]
fn postman_jwt_maps_each_key() {
    let c = postman(json!([
        kv("algorithm", json!("ES256")),
        kv("secret", json!("")),
        kv("isSecretBase64Encoded", json!(true)),
        kv("privateKey", json!("{{pem}}")),
        kv("header", json!({"kid": "k1"})),
        kv("payload", json!("{\"sub\":\"{{u}}\"}")),
        kv("addTokenTo", json!("queryParam")),
        kv("queryParamKey", json!("jwt")),
        kv("headerPrefix", json!("")),
    ]));
    let spec = jwt_of(&c);
    assert_eq!(spec.algorithm, JwtAlgorithm::Es256);
    assert!(spec.secret_base64);
    assert_eq!(
        spec.private_key,
        Some(OAuth1PrivateKey::new(OAuth1KeyKind::Text, "{{pem}}"))
    );
    assert_eq!(spec.headers, "{\"kid\":\"k1\"}");
    assert_eq!(spec.payload, "{\"sub\":\"{{u}}\"}");
    assert_eq!(spec.placement, JwtPlacement::Query);
    assert_eq!(spec.query_param_key, "jwt");
    assert_eq!(spec.header_prefix.as_deref(), Some(""));
}

#[test]
fn postman_jwt_defaults_fill_what_is_missing() {
    let spec = jwt_of(&postman(json!([kv("secret", json!("s"))])));
    assert_eq!(spec.algorithm, JwtAlgorithm::Hs256);
    assert_eq!(spec.secret, "s");
    assert_eq!(spec.placement, JwtPlacement::Header);
    assert_eq!(spec.header_prefix.as_deref(), Some("Bearer"));
    assert_eq!(spec.query_param_key, "token");
    let text = jwt_of(&postman(json!([kv("header", json!("{\"a\":1}"))])));
    assert_eq!(text.headers, "{\"a\":1}");
}

#[test]
fn postman_export_maps_jwt_back() {
    let auth = "  auth:\n    type: jwt\n    algorithm: RS256\n    secret: \"\"\n    secretBase64: false\n    privateKey:\n      type: text\n      value: \"{{pem}}\"\n    headers: '{\"kid\":\"k1\"}'\n    payload: '{\"sub\":\"1\"}'\n    placement: query\n    headerPrefix: Bearer\n    queryParamKey: jwt\n";
    let r = http("r", "GET", "http://x.test", auth);
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("r.yml", &r)]);
    let v = exported(&c);
    let got = &v["item"][0]["request"]["auth"];
    assert_eq!(got["type"], "jwt");
    let entry = |key: &str| {
        got["jwt"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["key"] == key)
            .map(|e| e["value"].clone())
            .unwrap_or_else(|| panic!("{key} in {got}"))
    };
    assert_eq!(entry("algorithm"), "RS256");
    assert_eq!(entry("privateKey"), "{{pem}}");
    assert_eq!(entry("isSecretBase64Encoded"), false);
    assert_eq!(entry("header"), json!({"kid": "k1"}));
    assert_eq!(entry("payload"), "{\"sub\":\"1\"}");
    assert_eq!(entry("addTokenTo"), "queryParam");
    assert_eq!(entry("queryParamKey"), "jwt");
    assert_eq!(entry("headerPrefix"), "Bearer");
    let back = parse_postman(v.to_string().as_bytes()).expect("parses");
    let spec = jwt_of(&back);
    assert_eq!(spec.placement, JwtPlacement::Query);
    assert_eq!(spec.headers, "{\"kid\":\"k1\"}");
}

#[test]
fn imported_literals_go_to_the_store_and_still_sign() {
    let c = postman(json!([
        kv("algorithm", json!("HS256")),
        kv("secret", json!("literal-jwt-secret")),
        kv("payload", json!("{\"sub\":\"1\"}")),
    ]));
    let parent = TempDir::new().unwrap();
    let store = Arc::new(MemoryStore::new());
    let (written, _report) = write_collection(parent.path(), &c, &[], store.as_ref()).unwrap();
    let file = fs::read_to_string(written.root().join("r.yml")).unwrap();
    assert!(!file.contains("literal-jwt-secret"), "{file}");
    let collection = Collection::open(written.root()).unwrap();
    let ctx = VarContext::new(
        &collection,
        ContextOptions::new()
            .env("Environment")
            .secrets(Arc::clone(&store) as Arc<dyn SecretStore>),
    )
    .unwrap();
    let request = &collection.tree().requests()[0];
    let r = resolve(request, &ctx).unwrap();
    let auth = r
        .headers()
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("authorization"))
        .map(|(_, v)| v.clone());
    assert!(auth.is_some_and(|a| a.starts_with("Bearer ey")));
    assert!(
        r.sensitive_values()
            .iter()
            .any(|v| v == "literal-jwt-secret")
    );
}
