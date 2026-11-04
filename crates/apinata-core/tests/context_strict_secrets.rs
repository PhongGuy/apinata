//! `ContextOptions::strict_secrets`: a secret store error fails the context instead of
//! becoming a warning.

use crate::common;

use apinata_core::{
    Collection, ContextError, ContextOptions, MemoryStore, SecretError, SecretKey, SecretStore,
    VarContext,
};
use common::write_tree;
use std::sync::Arc;
use tempfile::TempDir;

/// A store whose reads fail.
struct Locked;

impl SecretStore for Locked {
    fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
        Err(SecretError::Unavailable {
            message: "keyring locked".into(),
        })
    }
    fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
        unreachable!("never written")
    }
    fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
        unreachable!("never written")
    }
}

/// `dev` declares secrets `alpha` and `beta`.
fn collection() -> (TempDir, Collection) {
    let dir = write_tree(&[
        (
            "opencollection.yml",
            "opencollection: \"1.0.0\"\ninfo:\n  name: t\n",
        ),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: alpha\n    secret: true\n  - name: beta\n    secret: true\n",
        ),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    (dir, c)
}

#[test]
fn a_store_error_fails_the_context_naming_the_first_secret() {
    let (_d, c) = collection();
    let options = ContextOptions::new()
        .env("dev")
        .secrets(Arc::new(Locked))
        .strict_secrets();
    let err = VarContext::new(&c, options).unwrap_err();
    assert!(
        matches!(
            &err,
            ContextError::SecretUnavailable { environment, name, message }
                if environment == "dev" && name == "alpha"
                    && message == "secret store unavailable: keyring locked"
        ),
        "{err:?}"
    );
    assert_eq!(
        err.to_string(),
        "secret `alpha` of environment `dev` could not be read: secret store unavailable: keyring locked"
    );
}

#[test]
fn without_strict_secrets_the_error_stays_a_warning() {
    let (_d, c) = collection();
    let options = ContextOptions::new().env("dev").secrets(Arc::new(Locked));
    let ctx = VarContext::new(&c, options).expect("context");
    assert_eq!(ctx.warnings().len(), 2);
}

#[test]
fn a_missing_value_or_an_overridden_secret_is_not_an_error() {
    let (_d, c) = collection();
    let empty = ContextOptions::new()
        .env("dev")
        .secrets(Arc::new(MemoryStore::new()))
        .strict_secrets();
    assert!(VarContext::new(&c, empty).is_ok());
    // Overrides replace both secrets, so the failing store is never asked.
    let overridden = ContextOptions::new()
        .env("dev")
        .sensitive_env_var("alpha", "a")
        .env_var("beta", "b")
        .secrets(Arc::new(Locked))
        .strict_secrets();
    assert!(VarContext::new(&c, overridden).is_ok());
}
