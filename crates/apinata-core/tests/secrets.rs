//! The secret store contract, the in-memory store, keys and warnings. The keychain test touches
//! the real platform keychain, so it is ignored and run by hand.

use apinata_core::{Collection, EnvWarning, MemoryStore, SecretError, SecretKey, SecretStore};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;

/// A store implemented outside the crate, as shells and tests do.
struct Refusing;

impl SecretStore for Refusing {
    fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
        Err(SecretError::Unavailable {
            message: "locked".into(),
        })
    }
    fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
        Err(SecretError::Unavailable {
            message: "locked".into(),
        })
    }
    fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
        Err(SecretError::Unavailable {
            message: "locked".into(),
        })
    }
}

fn key(variable: &str) -> SecretKey {
    SecretKey::new("/c", "dev", variable)
}

#[test]
fn the_memory_store_sets_gets_replaces_and_deletes() {
    let store = MemoryStore::new();
    assert_eq!(store.get(&key("t")), Ok(None));
    store.set(&key("t"), "one").expect("set");
    store.set(&key("t"), "two").expect("replace");
    assert_eq!(store.get(&key("t")), Ok(Some("two".into())));
    assert_eq!(store.get(&key("other")), Ok(None));
    store.delete(&key("t")).expect("delete");
    assert_eq!(store.get(&key("t")), Ok(None));
    store
        .delete(&key("t"))
        .expect("deleting a missing value succeeds");
}

#[test]
fn stores_are_usable_as_shared_trait_objects() {
    let stores: Vec<Arc<dyn SecretStore>> = vec![Arc::new(MemoryStore::new()), Arc::new(Refusing)];
    let results: Vec<bool> = stores
        .iter()
        .map(|s| s.set(&key("t"), "v").is_ok())
        .collect();
    assert_eq!(results, [true, false]);
}

#[test]
fn debug_and_errors_never_show_values() {
    let store = MemoryStore::new();
    store.set(&key("t"), "hunter2").expect("set");
    assert_eq!(format!("{store:?}"), "MemoryStore { entries: 1 }");
    let warning = EnvWarning::Secret {
        environment: "dev".into(),
        variable: "t".into(),
        error: SecretError::Unavailable {
            message: "locked".into(),
        },
    };
    assert_eq!(
        warning.to_string(),
        "secret `t` of environment `dev`: secret store unavailable: locked"
    );
    assert!(!format!("{warning:?}").contains("hunter2"));
}

#[test]
fn the_collection_key_uses_the_canonical_root() {
    let dir = TempDir::new().expect("tempdir");
    fs::write(
        dir.path().join("opencollection.yml"),
        "opencollection: 1.0.0\ninfo:\n  name: c\n",
    )
    .expect("root");
    fs::create_dir(dir.path().join("sub")).expect("mkdir");
    let c = Collection::open(dir.path().join("sub/..")).expect("opens");
    let key = c.secret_key("dev", "token");
    assert_eq!(
        key.collection,
        fs::canonicalize(dir.path()).expect("canonical")
    );
    assert_eq!(
        (key.environment.as_str(), key.variable.as_str()),
        ("dev", "token")
    );
    assert_eq!(
        key,
        SecretKey::new(fs::canonicalize(dir.path()).unwrap(), "dev", "token")
    );
}

/// Run by hand: `cargo test -p apinata-core --test secrets -- --ignored`.
#[cfg(feature = "keychain")]
#[test]
#[ignore = "touches the real platform keychain"]
fn the_keychain_round_trips_a_value() {
    let store = apinata_core::KeychainStore::new().expect("keychain available");
    let key = SecretKey::new("/apinata/keychain-test", "dev", "token");
    store.set(&key, "s3cret").expect("set");
    assert_eq!(store.get(&key), Ok(Some("s3cret".into())));
    store.delete(&key).expect("delete");
    assert_eq!(store.get(&key), Ok(None));
    store
        .delete(&key)
        .expect("deleting a missing value succeeds");
    assert_eq!(format!("{store:?}"), "KeychainStore { .. }");
}

#[test]
fn a_memory_store_lists_only_the_collection_asked_for() {
    let s = MemoryStore::new();
    s.set(&SecretKey::new("/a", "dev", "x"), "1").unwrap();
    s.set(&SecretKey::new("/a", "prod", "y"), "2").unwrap();
    s.set(&SecretKey::new("/b", "dev", "z"), "3").unwrap();
    s.set(&SecretKey::new("", "", "oauth2-token-key"), "k")
        .unwrap();
    let mut got = s.list(Path::new("/a")).unwrap();
    got.sort_by(|p, q| p.variable.cmp(&q.variable));
    assert_eq!(
        got,
        [
            SecretKey::new("/a", "dev", "x"),
            SecretKey::new("/a", "prod", "y")
        ]
    );
    assert!(
        s.list(Path::new("")).unwrap().len() == 1,
        "only the empty path holds that key"
    );
}

#[test]
fn a_store_without_list_reports_it_rather_than_answering_empty() {
    struct Plain;
    impl SecretStore for Plain {
        fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
            Ok(None)
        }
        fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
            Ok(())
        }
        fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
            Ok(())
        }
    }
    assert!(matches!(
        Plain.list(Path::new("/a")),
        Err(SecretError::Failed { .. })
    ));
}
