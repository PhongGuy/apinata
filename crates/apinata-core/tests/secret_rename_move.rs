//! Moving one stored value to its renamed key, and finishing a value typed again.

use crate::common::write_tree;

use std::sync::atomic::{AtomicBool, Ordering};

use apinata_core::{
    Collection, EnvError, MemoryStore, RenameRefusal, SecretError, SecretKey, SecretName,
    SecretStore,
};
use tempfile::TempDir;

const STAGING: &str = "name: staging\nvariables:\n  - name: api_token\n    secret: true\n";

/// `staging` declares `api_token`; `dev` has no file, so its keys are fileless.
fn tree() -> (TempDir, Collection) {
    let dir = write_tree(&[
        (
            "opencollection.yml",
            "opencollection: 1.0.0\ninfo:\n  name: c\n",
        ),
        ("environments/staging.yml", STAGING),
    ]);
    let c = Collection::open(dir.path()).unwrap();
    (dir, c)
}

fn n(env: &str, var: &str) -> SecretName {
    SecretName::new(env, var)
}

/// A store that fails the calls it is told to, or keeps a changed value on `set`.
#[derive(Default)]
struct Faulty {
    inner: MemoryStore,
    fail_set: AtomicBool,
    fail_delete: AtomicBool,
    mangle: AtomicBool,
}

impl SecretStore for Faulty {
    fn get(&self, key: &SecretKey) -> Result<Option<String>, SecretError> {
        self.inner.get(key)
    }
    fn set(&self, key: &SecretKey, value: &str) -> Result<(), SecretError> {
        if self.fail_set.load(Ordering::SeqCst) {
            return Err(SecretError::Failed {
                message: "set refused".into(),
            });
        }
        let kept = if self.mangle.load(Ordering::SeqCst) {
            format!("{value}!")
        } else {
            value.to_owned()
        };
        self.inner.set(key, &kept)
    }
    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        if self.fail_delete.load(Ordering::SeqCst) {
            return Err(SecretError::Failed {
                message: "delete refused".into(),
            });
        }
        self.inner.delete(key)
    }
    fn list(&self, collection: &std::path::Path) -> Result<Vec<SecretKey>, SecretError> {
        self.inner.list(collection)
    }
}

// A stored value no temp path or message holds by chance, so a no-value check fails only on a leak.
const TOKEN: &str = "secret-value-token-7f3a";

fn get(c: &Collection, store: &dyn SecretStore, env: &str, var: &str) -> Option<String> {
    store.get(&c.secret_key(env, var)).unwrap()
}

fn seeded(c: &Collection) -> Faulty {
    let store = Faulty::default();
    store
        .inner
        .set(&c.secret_key("dev", "token"), TOKEN)
        .unwrap();
    store
}

fn refused(got: Result<(), EnvError>) -> RenameRefusal {
    match got {
        Err(EnvError::RenameRefused { reason, .. }) => reason,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn moves_a_free_target_and_removes_the_old_key() {
    let (_d, c) = tree();
    let store = seeded(&c);
    c.move_renamed_secret(&n("dev", "token"), &n("staging", "api_token"), &store)
        .unwrap();
    assert_eq!(
        get(&c, &store, "staging", "api_token").as_deref(),
        Some(TOKEN)
    );
    assert_eq!(get(&c, &store, "dev", "token"), None);
}

#[test]
fn a_same_target_only_removes_the_old_key() {
    let (_d, c) = tree();
    let store = seeded(&c);
    store
        .inner
        .set(&c.secret_key("staging", "api_token"), TOKEN)
        .unwrap();
    store.fail_set.store(true, Ordering::SeqCst);
    c.move_renamed_secret(&n("dev", "token"), &n("staging", "api_token"), &store)
        .unwrap();
    assert_eq!(get(&c, &store, "dev", "token"), None);
}

#[test]
fn a_taken_target_is_refused_and_both_values_stay() {
    let (_d, c) = tree();
    let store = seeded(&c);
    store
        .inner
        .set(&c.secret_key("staging", "api_token"), "other")
        .unwrap();
    let got = c.move_renamed_secret(&n("dev", "token"), &n("staging", "api_token"), &store);
    assert_eq!(refused(got), RenameRefusal::Taken);
    assert_eq!(get(&c, &store, "dev", "token").as_deref(), Some(TOKEN));
    assert_eq!(
        get(&c, &store, "staging", "api_token").as_deref(),
        Some("other")
    );
}

#[test]
fn a_live_old_key_is_refused() {
    let (dir, _) = tree();
    std::fs::write(
        dir.path().join("environments/dev.yml"),
        "name: dev\nvariables:\n  - name: token\n    secret: true\n",
    )
    .unwrap();
    let c = Collection::open(dir.path()).unwrap();
    let store = seeded(&c);
    let got = c.move_renamed_secret(&n("dev", "token"), &n("staging", "api_token"), &store);
    assert_eq!(refused(got), RenameRefusal::NotOrphaned);
    assert_eq!(get(&c, &store, "staging", "api_token"), None);
}

#[test]
fn a_target_not_declared_secret_is_refused() {
    let (_d, c) = tree();
    let store = seeded(&c);
    let got = c.move_renamed_secret(&n("dev", "token"), &n("staging", "nope"), &store);
    assert_eq!(refused(got), RenameRefusal::NotDeclared);
}

#[test]
fn nothing_stored_is_refused() {
    let (_d, c) = tree();
    let store = Faulty::default();
    let got = c.move_renamed_secret(&n("dev", "token"), &n("staging", "api_token"), &store);
    assert!(matches!(
        refused(got),
        RenameRefusal::NotOrphaned | RenameRefusal::NothingStored
    ));
}

#[test]
fn a_failed_set_changes_nothing() {
    let (_d, c) = tree();
    let store = seeded(&c);
    store.fail_set.store(true, Ordering::SeqCst);
    let got = c.move_renamed_secret(&n("dev", "token"), &n("staging", "api_token"), &store);
    assert!(
        matches!(got, Err(EnvError::RenameNotMoved { .. })),
        "{got:?}"
    );
    assert_eq!(get(&c, &store, "dev", "token").as_deref(), Some(TOKEN));
    assert_eq!(get(&c, &store, "staging", "api_token"), None);
}

#[test]
fn a_read_back_that_differs_removes_the_target_and_keeps_the_old() {
    let (_d, c) = tree();
    let store = seeded(&c);
    store.mangle.store(true, Ordering::SeqCst);
    let got = c.move_renamed_secret(&n("dev", "token"), &n("staging", "api_token"), &store);
    assert!(
        matches!(got, Err(EnvError::RenameNotMoved { .. })),
        "{got:?}"
    );
    assert_eq!(get(&c, &store, "dev", "token").as_deref(), Some(TOKEN));
    assert_eq!(get(&c, &store, "staging", "api_token"), None);
}

#[test]
fn a_failed_delete_keeps_both_and_a_retry_finishes() {
    let (_d, c) = tree();
    let store = seeded(&c);
    store.fail_delete.store(true, Ordering::SeqCst);
    let got = c.move_renamed_secret(&n("dev", "token"), &n("staging", "api_token"), &store);
    assert!(
        matches!(got, Err(EnvError::RenameNotMoved { .. })),
        "{got:?}"
    );
    assert_eq!(
        get(&c, &store, "staging", "api_token").as_deref(),
        Some(TOKEN)
    );
    store.fail_delete.store(false, Ordering::SeqCst);
    c.move_renamed_secret(&n("dev", "token"), &n("staging", "api_token"), &store)
        .unwrap();
    assert_eq!(get(&c, &store, "dev", "token"), None);
}

#[test]
fn no_error_text_carries_the_value() {
    let (_d, c) = tree();
    let store = seeded(&c);
    store.mangle.store(true, Ordering::SeqCst);
    let got = c.move_renamed_secret(&n("dev", "token"), &n("staging", "api_token"), &store);
    let text = format!("{} {:?}", got.as_ref().unwrap_err(), got);
    assert!(!text.contains(TOKEN), "{text}");
}

#[test]
fn finishing_a_reentered_value_removes_the_old_key_only_once_the_new_one_is_set() {
    let (_d, c) = tree();
    let store = seeded(&c);
    let (from, to) = (n("dev", "token"), n("staging", "api_token"));
    assert!(!c.finish_reentered(&from, &to, &store).unwrap());
    assert_eq!(get(&c, &store, "dev", "token").as_deref(), Some(TOKEN));
    store
        .inner
        .set(&c.secret_key("staging", "api_token"), "typed")
        .unwrap();
    assert!(c.finish_reentered(&from, &to, &store).unwrap());
    assert_eq!(get(&c, &store, "dev", "token"), None);
}
