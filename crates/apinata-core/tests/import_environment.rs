//! `parse_postman_environment` and `add_environment` against Bruno's bytes for
//! the import-parity corpus's `env-apinata-env-edge-cases`.

use apinata_core::import::{NoteKind, PostmanError, add_environment, parse_postman_environment};
use apinata_core::{Collection, MemoryStore, SecretError, SecretKey, SecretStore, VarValue};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn fixture() -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/postman/environments/apinata-env-edge-cases.json"),
    )
    .expect("fixture")
}

/// A store whose writes all fail, as a locked keychain does.
struct Locked;

impl SecretStore for Locked {
    fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
        Ok(None)
    }
    fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
        Err(SecretError::Unavailable {
            message: "locked".into(),
        })
    }
    fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
        Ok(())
    }
}

#[test]
fn values_are_named_stringified_and_secrets_deduplicated() {
    let env = parse_postman_environment(&fixture()).expect("parses");
    assert_eq!(env.name, "Staging: EU");
    let vars: Vec<_> = env
        .variables
        .iter()
        .map(|v| (v.name.as_str(), v.secret, v.enabled))
        .collect();
    assert_eq!(
        vars,
        [
            ("host", false, true),
            ("port", false, true),
            ("obj", false, true),
            ("token", true, true),
            ("off_key", false, false)
        ]
    );
    // VarValue has no Debug, so values are compared with plain asserts.
    assert!(env.variables[1].value == VarValue::Text("8443".into()));
    let pretty = "{\n  \"a\": 1,\n  \"b\": [\n    true,\n    null\n  ]\n}";
    assert!(env.variables[2].value == VarValue::Text(pretty.into()));
    assert!(
        env.variables[3].value == VarValue::Text("s3cret".into()),
        "the twin with a value survives"
    );
    assert!(
        env.notes
            .iter()
            .any(|n| n.kind == NoteKind::Dropped && n.source == "values[3]"),
        "{:?}",
        env.notes
    );
    assert!(
        env.notes
            .iter()
            .any(|n| n.kind == NoteKind::Converted && n.source == "values[5].key"),
        "{:?}",
        env.notes
    );
}

#[test]
fn names_follow_coerce_env_name_and_values_may_be_absent() {
    assert_eq!(
        parse_postman_environment(br#"{"id": "x", "name": 42, "values": []}"#)
            .expect("parses")
            .name,
        "42"
    );
    assert_eq!(
        parse_postman_environment(br#"{"name": " dev ", "values": []}"#)
            .expect("parses")
            .name,
        " dev "
    );
    assert!(
        parse_postman_environment(br#"{"name": "dev"}"#)
            .expect("parses")
            .variables
            .is_empty()
    );
    for bad in [
        &br#"{"name": "  ", "values": []}"#[..],
        br#"{"values": []}"#,
        br#"{"name": {}, "values": []}"#,
        br#"{"name": "dev", "values": "x"}"#,
    ] {
        assert!(matches!(
            parse_postman_environment(bad),
            Err(PostmanError::Invalid { .. })
        ));
    }
    assert!(matches!(
        parse_postman_environment(b"{"),
        Err(PostmanError::Parse { .. })
    ));
}

#[test]
fn add_environment_writes_brunos_bytes_and_stores_secrets() {
    let dir = TempDir::new().expect("tempdir");
    let mut collection = Collection::create(&dir.path().join("c"), "c").expect("create");
    let store = MemoryStore::new();
    let env = parse_postman_environment(&fixture()).expect("parses");
    let (name, report) = add_environment(&mut collection, &env, &store).expect("adds");
    assert_eq!(name, "Staging- EU");
    assert_eq!(report.environments, ["Staging- EU"]);
    let expected = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/reference/2026-10-09-import-parity/expected/env-apinata-env-edge-cases/environments/Staging- EU.yml")).expect("expected");
    assert_eq!(
        fs::read_to_string(collection.root().join("environments/Staging- EU.yml"))
            .expect("written"),
        expected
    );
    assert_eq!(
        store
            .get(&collection.secret_key("Staging- EU", "token"))
            .expect("reads"),
        Some("s3cret".into())
    );
    assert!(report.secrets_to_enter.is_empty());
    assert!(
        report
            .notes
            .iter()
            .all(|n| n.file.as_deref() == Some("environments/Staging- EU.yml"))
    );
    let (again, _) = add_environment(&mut collection, &env, &store).expect("adds again");
    assert_eq!(again, "Staging- EU copy");
}

#[test]
fn a_secret_the_store_refuses_or_that_has_no_value_needs_input() {
    let dir = TempDir::new().expect("tempdir");
    let mut collection = Collection::create(&dir.path().join("c"), "c").expect("create");
    let env = parse_postman_environment(&fixture()).expect("parses");
    let (name, report) = add_environment(&mut collection, &env, &Locked).expect("the file is kept");
    assert!(
        collection
            .root()
            .join(format!("environments/{name}.yml"))
            .exists()
    );
    assert_eq!(
        report.secrets_to_enter,
        [(name.clone(), "token".to_owned())]
    );
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.kind == NoteKind::NeedsInput && n.source == "values[4]"),
        "{:?}",
        report.notes
    );
    let blank = parse_postman_environment(
        br#"{"name": "b", "values": [{"key": "s", "value": "", "type": "secret"}]}"#,
    )
    .expect("parses");
    let store = MemoryStore::new();
    let (name, report) = add_environment(&mut collection, &blank, &store).expect("adds");
    assert_eq!(report.secrets_to_enter, [(name, "s".to_owned())]);
    assert_eq!(
        store.get(&collection.secret_key("b", "s")).expect("reads"),
        None,
        "nothing to store"
    );
}

#[test]
fn unknown_environment_and_entry_members_are_reported() {
    let json = br#"{"id": "x", "name": "e", "owner": "me", "_postman_variable_scope": "environment",
        "values": [{"key": "a", "value": "1", "enabled": true, "type": "default", "description": "d"}, 7]}"#;
    let env = parse_postman_environment(json).expect("parses");
    let mut sources: Vec<&str> = env.notes.iter().map(|n| n.source.as_str()).collect();
    sources.sort_unstable();
    assert_eq!(sources, ["owner", "values[0].description", "values[1]"]);
    assert!(env.notes.iter().all(|n| n.kind == NoteKind::Dropped));
}
