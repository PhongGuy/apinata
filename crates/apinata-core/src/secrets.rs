//! Where secret environment values live: a [`SecretStore`] keyed by collection, environment and
//! variable. [`MemoryStore`] serves tests and headless use; `KeychainStore` (cargo feature
//! `keychain`) uses the platform keychain.

use crate::collection::Collection;
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Names one secret value: the collection's canonical root, the environment's file stem and
/// the variable name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct SecretKey {
    /// The collection root, canonicalised.
    pub collection: PathBuf,
    /// The environment's file stem.
    pub environment: String,
    /// The variable name.
    pub variable: String,
}

impl SecretKey {
    /// A key from its three parts, taken as given.
    pub fn new(
        collection: impl Into<PathBuf>,
        environment: impl Into<String>,
        variable: impl Into<String>,
    ) -> SecretKey {
        SecretKey {
            collection: collection.into(),
            environment: environment.into(),
            variable: variable.into(),
        }
    }
}

/// Why a secret store call failed. Never carries a secret value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SecretError {
    /// No store can be reached, or it is locked.
    #[error("secret store unavailable: {message}")]
    Unavailable {
        /// What the platform reported.
        message: String,
    },
    /// The store was reached but the call failed.
    #[error("secret store error: {message}")]
    Failed {
        /// What the platform reported.
        message: String,
    },
}

/// A place to keep secret values. Calls may block, so shells call them off the UI thread.
pub trait SecretStore: Send + Sync {
    /// The value for `key`, or `None` when there is none.
    fn get(&self, key: &SecretKey) -> Result<Option<String>, SecretError>;
    /// Store `value` for `key`, replacing any earlier value.
    fn set(&self, key: &SecretKey, value: &str) -> Result<(), SecretError>;
    /// Remove the value for `key`; removing a missing value succeeds.
    fn delete(&self, key: &SecretKey) -> Result<(), SecretError>;
    /// The keys stored under `collection`, in every environment, in no fixed order. Never reads
    /// a value. The default is `SecretError::Failed`: a store that cannot list has no orphans
    /// to offer, which is not the same as having none.
    fn list(&self, collection: &Path) -> Result<Vec<SecretKey>, SecretError> {
        let _ = collection;
        Err(SecretError::Failed {
            message: "this secret store cannot list its entries".to_owned(),
        })
    }
}

/// A secret store in process memory, empty at start and lost at exit.
#[derive(Default)]
#[non_exhaustive]
pub struct MemoryStore {
    entries: Mutex<HashMap<SecretKey, String>>,
}

impl MemoryStore {
    /// An empty store.
    pub fn new() -> MemoryStore {
        MemoryStore::default()
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, HashMap<SecretKey, String>> {
        // A panic while holding the lock cannot leave the map half-written, so recover it.
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, key: &SecretKey) -> Result<Option<String>, SecretError> {
        Ok(self.entries().get(key).cloned())
    }

    fn set(&self, key: &SecretKey, value: &str) -> Result<(), SecretError> {
        self.entries().insert(key.clone(), value.to_owned());
        Ok(())
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        self.entries().remove(key);
        Ok(())
    }

    fn list(&self, collection: &Path) -> Result<Vec<SecretKey>, SecretError> {
        let entries = self.entries();
        let keys = entries.keys().filter(|k| k.collection == collection);
        Ok(keys.cloned().collect())
    }
}

/// Shows how many values the store holds, never the values.
impl fmt::Debug for MemoryStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryStore")
            .field("entries", &self.entries().len())
            .finish()
    }
}

/// Something an environment operation could not finish after its file change succeeded.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EnvWarning {
    /// A secret value could not be moved, stored, read or removed.
    Secret {
        /// The environment the value belongs to.
        environment: String,
        /// The variable.
        variable: String,
        /// What the store reported.
        error: SecretError,
    },
    /// An environment that extends a renamed one could not be saved with the new name.
    Extends {
        /// The environment whose `extends` was not updated on disk.
        environment: String,
        /// Why the save failed.
        message: String,
    },
}

impl fmt::Display for EnvWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EnvWarning::Secret {
                environment,
                variable,
                error,
            } => write!(
                f,
                "secret `{variable}` of environment `{environment}`: {error}"
            ),
            EnvWarning::Extends {
                environment,
                message,
            } => write!(
                f,
                "environment `{environment}` keeps its old `extends`: {message}"
            ),
        }
    }
}

impl Collection {
    /// The key of `variable` in `environment`: the root canonicalised, else as given, so a
    /// moved collection no longer finds its secrets.
    pub fn secret_key(&self, environment: &str, variable: &str) -> SecretKey {
        let root = std::fs::canonicalize(self.root()).unwrap_or_else(|_| self.root().to_path_buf());
        SecretKey::new(root, environment, variable)
    }
}
