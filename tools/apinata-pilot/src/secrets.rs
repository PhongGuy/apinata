//! A secret store kept as a file in the pilot launch's data directory, so a relaunch with
//! `reused` keeps saved secrets and the cookie jar's key while a fresh launch starts empty.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use anyhow::Context;
use apinata_core::{SecretError, SecretKey, SecretStore};
use serde::{Deserialize, Serialize};

/// The store's file name inside the launch data directory.
pub const FILE_NAME: &str = "pilot-secrets.json";

/// One stored value; `SecretKey` is not serde, so the file spells its parts out.
#[derive(Serialize, Deserialize)]
struct Entry {
    collection: PathBuf,
    environment: String,
    variable: String,
    value: String,
}

/// Values are stored plainly, only inside the pilot's temporary launch data directory.
pub struct FileSecrets {
    path: PathBuf,
    entries: Mutex<HashMap<SecretKey, String>>,
}

impl FileSecrets {
    /// Opens the store kept in `data_dir`; `reused` keeps a previous launch's file, otherwise it is removed.
    pub fn open(data_dir: &Path, reused: bool) -> anyhow::Result<FileSecrets> {
        let path = data_dir.join(FILE_NAME);
        let mut entries = HashMap::new();
        if reused {
            // An absent file is an empty store; a corrupt one fails the launch rather than
            // silently dropping the secrets it held.
            match std::fs::read(&path) {
                Ok(bytes) => {
                    // serde's message can quote a stored value, so report only where it broke.
                    let list: Vec<Entry> = serde_json::from_slice(&bytes).map_err(|e| {
                        let (line, column) = (e.line(), e.column());
                        anyhow::anyhow!(
                            "{} is malformed at line {line}, column {column}",
                            path.display()
                        )
                    })?;
                    for e in list {
                        let key = SecretKey::new(e.collection, e.environment, e.variable);
                        entries.insert(key, e.value);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
            }
        } else {
            // A crash between write and rename can leave the temp file; it goes too.
            for stale in [&path, &temp_path(&path)] {
                match std::fs::remove_file(stale) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => {
                        return Err(e).with_context(|| format!("removing {}", stale.display()));
                    }
                }
            }
        }
        Ok(FileSecrets {
            path,
            entries: Mutex::new(entries),
        })
    }

    fn entries(&self) -> MutexGuard<'_, HashMap<SecretKey, String>> {
        // A panic while holding the lock cannot leave the map half-written, so recover it.
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Rewrites the whole file through a sibling temp file and a rename, so a reader never
    /// sees a partial write. Errors name the path, never a value.
    fn write(&self, entries: &HashMap<SecretKey, String>) -> Result<(), SecretError> {
        let list: Vec<Entry> = entries
            .iter()
            .map(|(k, v)| Entry {
                collection: k.collection.clone(),
                environment: k.environment.clone(),
                variable: k.variable.clone(),
                value: v.clone(),
            })
            .collect();
        let failed = |what: &str, e: &dyn fmt::Display| SecretError::Failed {
            message: format!("{what} {}: {e}", self.path.display()),
        };
        let bytes = serde_json::to_vec(&list).map_err(|e| failed("encoding", &e))?;
        let tmp = temp_path(&self.path);
        std::fs::write(&tmp, bytes).map_err(|e| failed("writing", &e))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| failed("replacing", &e))
    }
}

/// The sibling a write goes through before it is renamed over the store file.
fn temp_path(path: &Path) -> PathBuf {
    path.with_extension("json.tmp")
}

impl SecretStore for FileSecrets {
    fn get(&self, key: &SecretKey) -> Result<Option<String>, SecretError> {
        Ok(self.entries().get(key).cloned())
    }

    fn set(&self, key: &SecretKey, value: &str) -> Result<(), SecretError> {
        let mut entries = self.entries();
        entries.insert(key.clone(), value.to_owned());
        self.write(&entries)
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        let mut entries = self.entries();
        if entries.remove(key).is_some() {
            self.write(&entries)?;
        }
        Ok(())
    }

    fn list(&self, collection: &Path) -> Result<Vec<SecretKey>, SecretError> {
        let entries = self.entries();
        let keys = entries.keys().filter(|k| k.collection == collection);
        Ok(keys.cloned().collect())
    }
}

/// Shows how many values the store holds, never the values.
impl fmt::Debug for FileSecrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileSecrets")
            .field("entries", &self.entries().len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_names_the_keys_of_one_collection() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSecrets::open(dir.path(), false).unwrap();
        store.set(&SecretKey::new("/a", "dev", "x"), "1").unwrap();
        store.set(&SecretKey::new("/b", "dev", "y"), "2").unwrap();
        let got = store.list(Path::new("/a")).unwrap();
        assert_eq!(got, [SecretKey::new("/a", "dev", "x")]);
    }
}
