//! Sealed files: a serde value encrypted with AES-256-GCM, its key kept in a [`SecretStore`]
//! under a name of the caller's choosing. Errors are plain messages naming the caller's store.

use crate::secrets::{SecretKey, SecretStore};
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use rand::{TryRngCore, rngs::OsRng};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::Write as _;
use std::path::Path;

const FILE_VERSION: u32 = 1;

/// Where a sealed store keeps its data.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Persistence {
    /// In an encrypted file, with the key in the keychain.
    Encrypted,
    /// In process memory only, lost at exit.
    MemoryOnly {
        /// Why the file is not used.
        reason: String,
    },
    /// The store has not been loaded yet.
    NotLoaded,
}

/// The file's shape: the nonce and the ciphertext of the JSON value.
#[derive(Serialize, Deserialize)]
struct FileBody {
    version: u32,
    nonce: String,
    data: String,
}

/// The 32-byte key stored under `SecretKey::new("", "", name)`, created when missing. A
/// concurrent creator may win the race, so the stored value is re-read and adopted. A malformed
/// stored key is never replaced; `label` names the store in error messages.
pub(crate) fn file_key(
    keys: &dyn SecretStore,
    name: &str,
    label: &str,
) -> Result<[u8; 32], String> {
    let name = SecretKey::new("", "", name);
    let unavailable = |e| format!("the keychain is unavailable: {e}");
    let stored = match keys.get(&name).map_err(unavailable)? {
        Some(stored) => stored,
        None => {
            let mut fresh = [0u8; 32];
            OsRng
                .try_fill_bytes(&mut fresh)
                .map_err(|e| format!("no random key could be made: {e}"))?;
            keys.set(&name, &STANDARD.encode(fresh))
                .map_err(unavailable)?;
            keys.get(&name)
                .map_err(unavailable)?
                .ok_or_else(|| format!("the keychain did not keep the {label} key"))?
        }
    };
    STANDARD
        .decode(stored.trim())
        .ok()
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .ok_or_else(|| format!("the stored {label} key is not valid"))
}

/// The decrypted value, or `None` when the file is missing, malformed or sealed with another key.
pub(crate) fn read<T: DeserializeOwned>(path: &Path, key: &[u8; 32]) -> Option<T> {
    let body: FileBody = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    if body.version != FILE_VERSION {
        return None;
    }
    let nonce = STANDARD.decode(body.nonce).ok()?;
    let data = STANDARD.decode(body.data).ok()?;
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    let plain = cipher
        .decrypt(&Nonce::try_from(&nonce[..]).ok()?, data.as_ref())
        .ok()?;
    serde_json::from_slice(&plain).ok()
}

/// Seals `value` with a fresh nonce and replaces the file atomically; tempfile makes it 0600.
pub(crate) fn write<T: Serialize + ?Sized>(
    path: &Path,
    key: &[u8; 32],
    value: &T,
    label: &str,
) -> Result<(), String> {
    let failed = |e: &dyn fmt::Display| format!("the {label} could not be written: {e}");
    let plain = serde_json::to_vec(value).map_err(|e| failed(&e))?;
    let mut nonce = [0u8; 12];
    OsRng.try_fill_bytes(&mut nonce).map_err(|e| failed(&e))?;
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| failed(&e))?;
    let nonce_value = Nonce::try_from(&nonce[..]).map_err(|e| failed(&e))?;
    let data = cipher
        .encrypt(&nonce_value, plain.as_ref())
        .map_err(|e| failed(&e))?;
    let body = FileBody {
        version: FILE_VERSION,
        nonce: STANDARD.encode(nonce),
        data: STANDARD.encode(data),
    };
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(|e| failed(&e))?;
    let mut file = tempfile::NamedTempFile::new_in(dir).map_err(|e| failed(&e))?;
    file.write_all(&serde_json::to_vec(&body).map_err(|e| failed(&e))?)
        .map_err(|e| failed(&e))?;
    file.persist(path).map_err(|e| failed(&e.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::{MemoryStore, SecretError, SecretKey, SecretStore};
    use serde_json::{Value, json};
    use tempfile::TempDir;

    struct Locked;
    impl SecretStore for Locked {
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
            Ok(())
        }
    }

    #[test]
    fn a_sealed_value_reads_back_and_the_file_holds_no_plaintext() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("sealed.json");
        let key = file_key(&MemoryStore::new(), "test-key", "test store").unwrap();
        write(&path, &key, &json!(["plain-SECRET"]), "test store").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("plain-SECRET"), "{text}");
        let body: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(body["version"], 1);
        assert!(
            body["nonce"].is_string() && body["data"].is_string(),
            "{body}"
        );
        assert_eq!(read::<Value>(&path, &key), Some(json!(["plain-SECRET"])));
    }

    #[test]
    fn each_name_has_its_own_key_and_a_stored_key_is_reused() {
        let keys = MemoryStore::new();
        let a = file_key(&keys, "a-key", "a").unwrap();
        assert_eq!(file_key(&keys, "a-key", "a").unwrap(), a);
        assert_ne!(file_key(&keys, "b-key", "b").unwrap(), a);
        assert!(
            keys.get(&SecretKey::new("", "", "a-key"))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn a_missing_file_another_key_or_garbage_reads_as_none() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("sealed.json");
        let key = file_key(&MemoryStore::new(), "k", "t").unwrap();
        let other = file_key(&MemoryStore::new(), "k", "t").unwrap();
        assert_eq!(read::<Value>(&path, &key), None);
        write(&path, &key, &json!(1), "t").unwrap();
        assert_eq!(read::<Value>(&path, &other), None);
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(read::<Value>(&path, &key), None);
    }

    #[test]
    fn a_malformed_stored_key_is_reported_with_the_label_and_kept() {
        let keys = MemoryStore::new();
        let name = SecretKey::new("", "", "k");
        keys.set(&name, "short").unwrap();
        assert_eq!(
            file_key(&keys, "k", "cookie jar").unwrap_err(),
            "the stored cookie jar key is not valid"
        );
        assert_eq!(keys.get(&name).unwrap().as_deref(), Some("short"));
    }

    #[test]
    fn an_unreachable_store_is_reported() {
        let err = file_key(&Locked, "k", "t").unwrap_err();
        assert!(err.starts_with("the keychain is unavailable: "), "{err}");
    }

    #[test]
    fn a_failed_write_names_the_store() {
        let dir = TempDir::new().unwrap();
        let blocker = dir.path().join("file");
        std::fs::write(&blocker, "x").unwrap();
        let err = write(
            &blocker.join("sealed.json"),
            &[1u8; 32],
            &json!(1),
            "cookie jar",
        )
        .unwrap_err();
        assert!(
            err.starts_with("the cookie jar could not be written: "),
            "{err}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_owner_only_and_each_write_has_a_fresh_nonce() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("sealed.json");
        let key = [3u8; 32];
        write(&path, &key, &json!(1), "t").unwrap();
        let nonce = |p: &Path| -> Value {
            serde_json::from_slice::<Value>(&std::fs::read(p).unwrap()).unwrap()["nonce"].clone()
        };
        let first = nonce(&path);
        write(&path, &key, &json!(1), "t").unwrap();
        assert_ne!(nonce(&path), first);
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
