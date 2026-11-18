//! Debug builds only: a secret store kept in one JSON file, so integration tests can store a
//! value in one run of the binary and see it moved by the next, without any keychain.

use std::path::{Path, PathBuf};

use apinata_core::{SecretError, SecretKey, SecretStore};
use serde::{Deserialize, Serialize};

/// One stored value, keyed as core keys it.
#[derive(Serialize, Deserialize)]
struct Entry {
    collection: PathBuf,
    environment: String,
    variable: String,
    value: String,
}

/// Every call reads the whole file and every change writes it back; a missing file is empty.
pub struct FileStore {
    path: PathBuf,
}

impl FileStore {
    pub fn new(path: PathBuf) -> FileStore {
        FileStore { path }
    }

    fn load(&self) -> Result<Vec<Entry>, SecretError> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(failed(e)),
        };
        serde_json::from_slice(&bytes).map_err(failed)
    }

    fn save(&self, entries: &[Entry]) -> Result<(), SecretError> {
        let bytes = serde_json::to_vec(entries).map_err(failed)?;
        std::fs::write(&self.path, bytes).map_err(failed)
    }
}

fn failed(e: impl std::fmt::Display) -> SecretError {
    SecretError::Failed {
        message: e.to_string(),
    }
}

fn is(entry: &Entry, key: &SecretKey) -> bool {
    entry.collection == key.collection
        && entry.environment == key.environment
        && entry.variable == key.variable
}

impl SecretStore for FileStore {
    fn get(&self, key: &SecretKey) -> Result<Option<String>, SecretError> {
        let entries = self.load()?;
        Ok(entries.into_iter().find(|e| is(e, key)).map(|e| e.value))
    }

    fn set(&self, key: &SecretKey, value: &str) -> Result<(), SecretError> {
        let mut entries = self.load()?;
        entries.retain(|e| !is(e, key));
        entries.push(Entry {
            collection: key.collection.clone(),
            environment: key.environment.clone(),
            variable: key.variable.clone(),
            value: value.to_owned(),
        });
        self.save(&entries)
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        let mut entries = self.load()?;
        entries.retain(|e| !is(e, key));
        self.save(&entries)
    }

    fn list(&self, collection: &Path) -> Result<Vec<SecretKey>, SecretError> {
        let entries = self.load()?;
        let mine = entries.into_iter().filter(|e| e.collection == collection);
        Ok(mine
            .map(|e| SecretKey::new(e.collection, e.environment, e.variable))
            .collect())
    }

    fn list_roots(&self) -> Result<Vec<PathBuf>, SecretError> {
        let mut roots: Vec<PathBuf> = self.load()?.into_iter().map(|e| e.collection).collect();
        roots.sort();
        roots.dedup();
        Ok(roots)
    }
}
