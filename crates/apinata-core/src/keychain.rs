//! [`KeychainStore`]: secrets in the platform keychain through `keyring-core`, one entry per
//! variable under service `apinata`, the account being [`SecretKey`]'s three parts.

use crate::secrets::{SecretError, SecretKey, SecretStore};
use keyring_core::{CredentialStore, Entry, Error};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const SERVICE: &str = "apinata";

/// The platform keychain: Secret Service on Linux, the login keychain on macOS and the
/// Credential Manager on Windows.
#[non_exhaustive]
pub struct KeychainStore {
    store: Arc<CredentialStore>,
}

impl KeychainStore {
    /// Open the platform keychain. Fails with [`SecretError::Unavailable`] on a platform without
    /// one or when it cannot be reached.
    pub fn new() -> Result<KeychainStore, SecretError> {
        let store = platform_store().map_err(map_error)?;
        Ok(KeychainStore { store })
    }

    /// A store over an injected credential store, so tests need no real keyring.
    #[cfg(test)]
    fn with_store(store: Arc<CredentialStore>) -> KeychainStore {
        KeychainStore { store }
    }

    fn entry(&self, key: &SecretKey) -> Result<Entry, SecretError> {
        self.entry_named(&account(key))
    }

    fn entry_named(&self, account: &str) -> Result<Entry, SecretError> {
        self.store.build(SERVICE, account, None).map_err(map_error)
    }

    /// Move a secret found under the legacy account to the current one. The new entry is written
    /// before the old one goes, so a failure never loses the secret; both steps are best-effort.
    fn migrate(&self, key: &SecretKey, legacy: &Entry, value: &str) {
        let written = self
            .entry(key)
            .and_then(|e| e.set_password(value).map_err(map_error));
        if written.is_ok() {
            let _ = legacy.delete_credential();
        }
    }
}

#[cfg(target_os = "linux")]
fn platform_store() -> keyring_core::Result<Arc<CredentialStore>> {
    Ok(zbus_secret_service_keyring_store::Store::new()?)
}

#[cfg(target_os = "macos")]
fn platform_store() -> keyring_core::Result<Arc<CredentialStore>> {
    Ok(apple_native_keyring_store::keychain::Store::new()?)
}

#[cfg(windows)]
fn platform_store() -> keyring_core::Result<Arc<CredentialStore>> {
    Ok(windows_native_keyring_store::Store::new()?)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn platform_store() -> keyring_core::Result<Arc<CredentialStore>> {
    Err(Error::NoDefaultStore)
}

impl SecretStore for KeychainStore {
    fn get(&self, key: &SecretKey) -> Result<Option<String>, SecretError> {
        match self.entry(key)?.get_password() {
            Ok(value) => return Ok(Some(value)),
            Err(Error::NoEntry) => {}
            Err(e) => return Err(map_error(e)),
        }
        // Secrets saved before the verbatim prefix was dropped sit under the old account name.
        let Some(legacy) = legacy_account(key) else {
            return Ok(None);
        };
        let legacy = self.entry_named(&legacy)?;
        match legacy.get_password() {
            Ok(value) => {
                self.migrate(key, &legacy, &value);
                Ok(Some(value))
            }
            Err(Error::NoEntry) => Ok(None),
            Err(e) => Err(map_error(e)),
        }
    }

    fn set(&self, key: &SecretKey, value: &str) -> Result<(), SecretError> {
        self.entry(key)?.set_password(value).map_err(map_error)
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        // The legacy entry goes too, or the fallback in `get` would bring the secret back.
        if let Some(legacy) = legacy_account(key) {
            match self.entry_named(&legacy)?.delete_credential() {
                Ok(()) | Err(Error::NoEntry) => {}
                Err(e) => return Err(map_error(e)),
            }
        }
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(Error::NoEntry) => Ok(()),
            Err(e) => Err(map_error(e)),
        }
    }

    fn list(&self, collection: &Path) -> Result<Vec<SecretKey>, SecretError> {
        let lossy = collection.to_string_lossy();
        let paths = [account_path(&strip_verbatim(&lossy)), account_path(&lossy)];
        let spec = search_spec();
        let spec = spec.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let mut seen = HashSet::new();
        let mut keys = Vec::new();
        for entry in self.store.search(&spec).map_err(map_error)? {
            // No specifiers is a foreign item, or a locked one the search failed to unlock; a
            // skip can hide an orphan but never offers a live secret.
            let Some((service, account)) = entry.get_specifiers() else {
                continue;
            };
            if service != SERVICE {
                continue;
            }
            let Ok([path, environment, variable]) = serde_json::from_str::<[String; 3]>(&account)
            else {
                continue;
            };
            if paths.contains(&path) && seen.insert((environment.clone(), variable.clone())) {
                keys.push(SecretKey::new(collection, environment, variable));
            }
        }
        Ok(keys)
    }

    fn list_roots(&self) -> Result<Vec<PathBuf>, SecretError> {
        let spec = search_spec();
        let spec = spec.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let mut roots = HashSet::new();
        for entry in self.store.search(&spec).map_err(map_error)? {
            // The same skips as `list`; an empty path is a per-install key, not a collection.
            let Some((service, account)) = entry.get_specifiers() else {
                continue;
            };
            if service != SERVICE {
                continue;
            }
            let Ok([path, _, _]) = serde_json::from_str::<[String; 3]>(&account) else {
                continue;
            };
            if !path.is_empty() {
                roots.insert(PathBuf::from(unprefixed(&path)));
            }
        }
        Ok(roots.into_iter().collect())
    }
}

/// An account's path without the verbatim prefix a legacy account kept, so both name one root.
fn unprefixed(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("//?/UNC/") {
        return format!("//{rest}");
    }
    match path.strip_prefix("//?/") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest.to_owned(),
        _ => path.to_owned(),
    }
}

/// What the platform's `search` accepts: Windows takes only a regex on the target name.
fn search_spec() -> HashMap<&'static str, String> {
    #[cfg(windows)]
    return HashMap::from([("pattern", format!(r"\.{SERVICE}$"))]);
    #[cfg(not(windows))]
    HashMap::from([("service", SERVICE.to_owned())])
}

/// The keychain account for `key`: a JSON array of its three parts, the path with forward
/// slashes as Bruno keys its secrets. A Windows verbatim prefix is dropped first, so a
/// canonical path and the same path as typed give one key.
fn account(key: &SecretKey) -> String {
    let lossy = key.collection.to_string_lossy();
    account_for(&strip_verbatim(&lossy), key)
}

/// The account name saved before the verbatim prefix was dropped, or `None` when the path has
/// no prefix and the two names are the same.
fn legacy_account(key: &SecretKey) -> Option<String> {
    let lossy = key.collection.to_string_lossy();
    if strip_verbatim(&lossy) == lossy {
        return None;
    }
    Some(account_for(&lossy, key))
}

fn account_for(path: &str, key: &SecretKey) -> String {
    serde_json::json!([account_path(path), key.environment, key.variable]).to_string()
}

fn account_path(path: &str) -> String {
    path.replace('\\', "/")
}

/// `path` without a leading `\\?\` (drive) or `\\?\UNC\` (share) prefix, which Windows adds to
/// canonical paths.
fn strip_verbatim(path: &str) -> std::borrow::Cow<'_, str> {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}").into();
    }
    match path.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest.into(),
        _ => path.into(),
    }
}

/// Some `keyring_core` errors carry the stored bytes, so those get a fixed message instead of
/// their own text.
fn map_error(error: Error) -> SecretError {
    let message = match &error {
        Error::BadEncoding(_) => "the stored value is not UTF-8".to_owned(),
        Error::BadDataFormat(..) => "the stored value is malformed".to_owned(),
        Error::Ambiguous(items) => format!("{} entries match", items.len()),
        other => other.to_string(),
    };
    match error {
        Error::NoStorageAccess(_) | Error::PlatformFailure(_) | Error::NoDefaultStore => {
            SecretError::Unavailable { message }
        }
        _ => SecretError::Failed { message },
    }
}

impl fmt::Debug for KeychainStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("KeychainStore { .. }")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_account_is_a_json_array_with_forward_slashes() {
        let key = SecretKey::new(r"C:\work\api", "dev", "token");
        assert_eq!(account(&key), r#"["C:/work/api","dev","token"]"#);
        let key = SecretKey::new("/home/u/c", "a \"b\"", "t");
        assert_eq!(account(&key), r#"["/home/u/c","a \"b\"","t"]"#);
    }

    #[test]
    fn the_verbatim_prefix_does_not_change_the_account() {
        let plain = account(&SecretKey::new(r"C:\work\api", "dev", "t"));
        let verbatim = account(&SecretKey::new(r"\\?\C:\work\api", "dev", "t"));
        assert_eq!(plain, verbatim);
        let unc = account(&SecretKey::new(r"\\?\UNC\srv\share\c", "dev", "t"));
        assert_eq!(unc, account(&SecretKey::new(r"\\srv\share\c", "dev", "t")));
        // Only a leading prefix goes; a device path that is not a drive stays as given.
        let device = account(&SecretKey::new(r"\\?\Volume{1}\c", "dev", "t"));
        assert!(device.contains("//?/Volume{1}/c"), "{device}");
    }

    fn store() -> (KeychainStore, Arc<CredentialStore>) {
        let inner = keyring_core::mock::Store::new().unwrap();
        (KeychainStore::with_store(inner.clone()), inner)
    }

    fn put(store: &Arc<CredentialStore>, account: &str, value: &str) {
        let entry = store.build(SERVICE, account, None).unwrap();
        entry.set_password(value).unwrap();
    }

    fn read(store: &Arc<CredentialStore>, account: &str) -> Option<String> {
        store
            .build(SERVICE, account, None)
            .unwrap()
            .get_password()
            .ok()
    }

    fn verbatim_key() -> SecretKey {
        SecretKey::new(r"\\?\C:\work\api", "dev", "t")
    }

    #[test]
    fn the_legacy_account_keeps_the_prefix_and_only_exists_when_it_differs() {
        assert_eq!(
            legacy_account(&verbatim_key()).as_deref(),
            Some(r#"["//?/C:/work/api","dev","t"]"#)
        );
        assert_eq!(
            legacy_account(&SecretKey::new(r"C:\work\api", "dev", "t")),
            None
        );
        assert_eq!(
            legacy_account(&SecretKey::new("/home/u/c", "dev", "t")),
            None
        );
    }

    #[test]
    fn a_hit_on_the_legacy_account_migrates_to_the_new_one() {
        let (keychain, raw) = store();
        let key = verbatim_key();
        let old = legacy_account(&key).unwrap();
        put(&raw, &old, "s3cret");
        assert_eq!(keychain.get(&key).unwrap().as_deref(), Some("s3cret"));
        assert_eq!(read(&raw, &account(&key)).as_deref(), Some("s3cret"));
        assert_eq!(read(&raw, &old), None);
    }

    #[test]
    fn the_new_account_wins_over_the_legacy_one() {
        let (keychain, raw) = store();
        let key = verbatim_key();
        let old = legacy_account(&key).unwrap();
        put(&raw, &old, "old");
        put(&raw, &account(&key), "new");
        assert_eq!(keychain.get(&key).unwrap().as_deref(), Some("new"));
        assert_eq!(read(&raw, &old).as_deref(), Some("old"));
    }

    #[test]
    fn a_miss_on_both_accounts_is_none() {
        let (keychain, _raw) = store();
        assert_eq!(keychain.get(&verbatim_key()).unwrap(), None);
    }

    #[test]
    fn delete_clears_both_accounts() {
        let (keychain, raw) = store();
        let key = verbatim_key();
        let old = legacy_account(&key).unwrap();
        put(&raw, &old, "old");
        put(&raw, &account(&key), "new");
        keychain.delete(&key).unwrap();
        assert_eq!(read(&raw, &old), None);
        assert_eq!(keychain.get(&key).unwrap(), None);
    }

    fn listed(keychain: &KeychainStore, root: &str) -> Vec<(String, String)> {
        let mut got: Vec<_> = keychain
            .list(std::path::Path::new(root))
            .unwrap()
            .into_iter()
            .map(|k| {
                assert_eq!(k.collection, std::path::Path::new(root));
                (k.environment, k.variable)
            })
            .collect();
        got.sort();
        got
    }

    #[test]
    fn list_finds_the_collection_in_every_environment() {
        let (keychain, raw) = store();
        put(&raw, r#"["/home/u/c","dev","a"]"#, "1");
        put(&raw, r#"["/home/u/c","prod","b"]"#, "2");
        put(&raw, r#"["/home/u/other","dev","c"]"#, "3");
        let got = listed(&keychain, "/home/u/c");
        assert_eq!(
            got,
            [("dev".into(), "a".into()), ("prod".into(), "b".into())]
        );
    }

    #[test]
    fn list_skips_other_services_and_accounts_it_did_not_write() {
        let (keychain, raw) = store();
        // The mock's search matches service as a substring, so this one reaches the check.
        let foreign = raw
            .build("not-apinata", r#"["/home/u/c","dev","x"]"#, None)
            .unwrap();
        foreign.set_password("no").unwrap();
        put(&raw, "not json", "no");
        put(&raw, r#"["/home/u/c","dev"]"#, "no");
        put(&raw, r#"["/home/u/c","dev",3]"#, "no");
        assert!(listed(&keychain, "/home/u/c").is_empty());
    }

    #[test]
    fn list_never_lists_the_empty_path_keys() {
        let (keychain, raw) = store();
        put(&raw, r#"["","","oauth2-token-key"]"#, "k");
        put(&raw, r#"["","app","proxy-password"]"#, "k");
        assert!(listed(&keychain, "/home/u/c").is_empty());
    }

    #[test]
    fn list_reports_a_legacy_and_a_current_account_once() {
        let (keychain, raw) = store();
        let key = verbatim_key();
        put(&raw, &legacy_account(&key).unwrap(), "old");
        put(&raw, &account(&key), "new");
        assert_eq!(
            listed(&keychain, r"\\?\C:\work\api"),
            [("dev".to_owned(), "t".to_owned())]
        );
        let only_old = SecretKey::new(r"\\?\C:\work\api", "stage", "u");
        put(&raw, &legacy_account(&only_old).unwrap(), "old");
        assert_eq!(listed(&keychain, r"\\?\C:\work\api").len(), 2);
    }

    fn roots(keychain: &KeychainStore) -> Vec<std::path::PathBuf> {
        let mut got = keychain.list_roots().unwrap();
        got.sort();
        got
    }

    #[test]
    fn list_roots_names_each_root_once_without_foreign_or_per_install_entries() {
        let (keychain, raw) = store();
        put(&raw, r#"["/home/u/c","dev","a"]"#, "1");
        put(&raw, r#"["/home/u/c","prod","b"]"#, "2");
        put(&raw, r#"["/home/u/old","dev","c"]"#, "3");
        put(&raw, r#"["","","oauth2-token-key"]"#, "k");
        put(&raw, r#"["","app","proxy-password"]"#, "k");
        put(&raw, "not json", "no");
        put(&raw, r#"["/home/u/x","dev"]"#, "no");
        let foreign = raw
            .build("not-apinata", r#"["/home/u/y","dev","x"]"#, None)
            .unwrap();
        foreign.set_password("no").unwrap();
        assert_eq!(
            roots(&keychain),
            [
                std::path::PathBuf::from("/home/u/c"),
                std::path::PathBuf::from("/home/u/old")
            ]
        );
    }

    #[test]
    fn list_roots_folds_a_legacy_account_into_its_root() {
        let (keychain, raw) = store();
        let key = verbatim_key();
        put(&raw, &legacy_account(&key).unwrap(), "old");
        put(&raw, &account(&key), "new");
        assert_eq!(roots(&keychain), [std::path::PathBuf::from("C:/work/api")]);
    }

    #[test]
    fn list_roots_reads_no_value() {
        let (keychain, raw) = store();
        put(&raw, r#"["/home/u/c","dev","a"]"#, "1");
        let entry = raw
            .build(SERVICE, r#"["/home/u/c","dev","a"]"#, None)
            .unwrap();
        let cred = entry
            .as_any()
            .downcast_ref::<keyring_core::mock::Cred>()
            .unwrap();
        cred.set_error(Error::Invalid("armed".into(), "armed".into()));
        assert_eq!(roots(&keychain).len(), 1);
        assert!(entry.get_password().is_err());
    }

    #[test]
    fn list_reads_no_value() {
        let (keychain, raw) = store();
        put(&raw, r#"["/home/u/c","dev","a"]"#, "1");
        let entry = raw
            .build(SERVICE, r#"["/home/u/c","dev","a"]"#, None)
            .unwrap();
        // A mock error is spent by the first read, so one still armed proves list made none.
        let cred = entry
            .as_any()
            .downcast_ref::<keyring_core::mock::Cred>()
            .unwrap();
        cred.set_error(Error::Invalid("armed".into(), "armed".into()));
        assert_eq!(listed(&keychain, "/home/u/c").len(), 1);
        assert!(entry.get_password().is_err());
    }
}
