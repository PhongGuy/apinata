//! The OAuth 2.0 token cache: entries in memory, mirrored to an AES-256-GCM encrypted file whose
//! key lives in a [`SecretStore`]. Without a usable key the cache keeps tokens in memory only.

use super::OAuth2Error;
use crate::sealed::{self, Persistence};
use crate::secrets::SecretStore;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

/// A token counts as expired this long before the provider says it does.
pub(crate) const EXPIRY_MARGIN: Duration = Duration::from_secs(30);

const KEY_NAME: &str = "oauth2-token-key";
/// Names the cache in sealed-file error messages.
const LABEL: &str = "token cache";

/// OAuth 2.0 tokens keyed by collection, environment, token URL and credentials id. Clones share
/// one cache. Tokens never appear in its `Debug` output.
#[derive(Clone)]
#[non_exhaustive]
pub struct TokenCache {
    inner: Arc<Inner>,
}

struct Inner {
    /// The file and the key store, absent for an in-memory cache.
    backing: Option<(PathBuf, Arc<dyn SecretStore>)>,
    loaded: tokio::sync::OnceCell<()>,
    state: Mutex<State>,
    /// Serialises read-modify-write cycles on the file within this process.
    file: Mutex<()>,
    flights: Mutex<HashMap<CacheKey, Arc<Flight>>>,
}

/// The per-key lock that callers hold while fetching a token, plus the last failed sign-in so
/// callers that queued behind it share its error instead of signing in again.
#[derive(Default)]
pub(crate) struct Flight {
    /// The failed sign-in, numbered by `failures` when it was recorded.
    pub(crate) lock: tokio::sync::Mutex<Option<(u64, OAuth2Error)>>,
    failures: AtomicU64,
}

impl Flight {
    /// Marks a caller's arrival; a failure recorded later is one it waited on.
    pub(crate) fn arrive(&self) -> u64 {
        self.failures.load(Ordering::SeqCst)
    }

    /// Records a failed sign-in in `slot`, the guard's contents, for the callers queued behind it.
    pub(crate) fn fail(&self, slot: &mut Option<(u64, OAuth2Error)>, error: &OAuth2Error) {
        let n = self.failures.fetch_add(1, Ordering::SeqCst) + 1;
        *slot = Some((n, error.clone()));
    }
}

struct State {
    persistence: Persistence,
    key: Option<[u8; 32]>,
    entries: Vec<Entry>,
}

/// Identifies one token: whose collection, which environment, which endpoint, which credentials.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct CacheKey {
    pub(crate) collection: PathBuf,
    pub(crate) environment: Option<String>,
    pub(crate) token_url: String,
    pub(crate) credentials_id: String,
}

impl CacheKey {
    pub(crate) fn new(
        collection: PathBuf,
        environment: Option<String>,
        token_url: impl Into<String>,
        credentials_id: impl Into<String>,
    ) -> CacheKey {
        CacheKey {
            collection,
            environment,
            token_url: token_url.into(),
            credentials_id: credentials_id.into(),
        }
    }
}

/// A token endpoint response as stored, with the fingerprint of the settings that obtained it.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Entry {
    pub(crate) key: CacheKey,
    pub(crate) fingerprint: String,
    pub(crate) obtained_at_ms: u64,
    pub(crate) token: Map<String, Value>,
}

impl Entry {
    pub(crate) fn new(
        key: CacheKey,
        fingerprint: impl Into<String>,
        token: Map<String, Value>,
        obtained_at: SystemTime,
    ) -> Entry {
        let obtained_at_ms = obtained_at
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
        Entry {
            key,
            fingerprint: fingerprint.into(),
            obtained_at_ms,
            token,
        }
    }

    pub(crate) fn obtained_at(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_millis(self.obtained_at_ms)
    }

    pub(crate) fn field(&self, name: &str) -> Option<&str> {
        self.token.get(name).and_then(Value::as_str)
    }

    /// `expires_in` may be a number or a numeric string; without one the token never expires.
    pub(crate) fn expires_at(&self) -> Option<SystemTime> {
        let seconds = match self.token.get("expires_in")? {
            Value::Number(n) => n.as_f64()?,
            Value::String(s) => s.trim().parse::<f64>().ok()?,
            _ => return None,
        };
        if !seconds.is_finite() {
            return None;
        }
        let margin = EXPIRY_MARGIN.as_millis() as f64;
        let end_ms = (self.obtained_at_ms as f64 + seconds * 1000.0 - margin).max(0.0);
        SystemTime::UNIX_EPOCH.checked_add(Duration::from_millis(end_ms as u64))
    }

    /// A clock that reads earlier than the time the token was obtained also counts as expired.
    pub(crate) fn is_expired(&self, now: SystemTime) -> bool {
        now < self.obtained_at() || self.expires_at().is_some_and(|end| now >= end)
    }
}

/// Shows the key and the names of the response fields, never their values.
impl fmt::Debug for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Entry")
            .field("key", &self.key)
            .field("fields", &self.token.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl TokenCache {
    /// A cache that keeps tokens in memory only, ready to use.
    pub fn in_memory() -> TokenCache {
        TokenCache::build(
            None,
            Persistence::MemoryOnly {
                reason: "tokens are kept in memory only".to_owned(),
            },
            tokio::sync::OnceCell::new_with(Some(())),
        )
    }

    /// A cache backed by the encrypted file at `path`, its key kept in `keys`. Nothing is read
    /// until [`TokenCache::load`].
    pub fn open(path: impl Into<PathBuf>, keys: Arc<dyn SecretStore>) -> TokenCache {
        TokenCache::build(
            Some((path.into(), keys)),
            Persistence::NotLoaded,
            tokio::sync::OnceCell::new(),
        )
    }

    fn build(
        backing: Option<(PathBuf, Arc<dyn SecretStore>)>,
        persistence: Persistence,
        loaded: tokio::sync::OnceCell<()>,
    ) -> TokenCache {
        TokenCache {
            inner: Arc::new(Inner {
                backing,
                loaded,
                state: Mutex::new(State {
                    persistence,
                    key: None,
                    entries: Vec::new(),
                }),
                file: Mutex::new(()),
                flights: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Reads the key and the file, once, off the async runtime. Later calls return at once.
    pub async fn load(&self) {
        self.inner
            .loaded
            .get_or_init(|| async {
                let inner = self.inner.clone();
                if let Err(e) = tokio::task::spawn_blocking(move || inner.load_blocking()).await {
                    self.inner.state().persistence = Persistence::MemoryOnly {
                        reason: format!("the token cache could not be loaded: {e}"),
                    };
                }
            })
            .await;
    }

    /// The default file: `oauth2-tokens.json` under the user's local data directory.
    pub fn default_path() -> Option<PathBuf> {
        dirs::data_local_dir().map(|d| d.join("apinata").join("oauth2-tokens.json"))
    }

    /// Where the tokens are kept.
    pub fn persistence(&self) -> Persistence {
        self.inner.state().persistence.clone()
    }

    /// Removes every token for one credentials id in a collection and environment, across token
    /// URLs, and returns how many were removed.
    pub async fn clear(
        &self,
        collection: &Path,
        environment: Option<&str>,
        credentials_id: &str,
    ) -> usize {
        let collection = collection.to_owned();
        let environment = environment.map(str::to_owned);
        let id = credentials_id.to_owned();
        self.update(move |entries| {
            let collection = std::fs::canonicalize(&collection).unwrap_or(collection);
            let before = entries.len();
            entries.retain(|e| {
                !(e.key.collection == collection
                    && e.key.environment == environment
                    && e.key.credentials_id == id)
            });
            before - entries.len()
        })
        .await
    }

    /// Removes one credentials id's entries from the in-memory view only, matching as
    /// `entries_for` does; `clear` persists the removal.
    pub(crate) fn forget(
        &self,
        collection: &Path,
        environment: Option<&str>,
        credentials_id: &str,
    ) {
        self.inner.state().entries.retain(|e| {
            !(e.key.collection == collection
                && e.key.environment.as_deref() == environment
                && e.key.credentials_id == credentials_id)
        });
    }

    /// The entry for `key` when it was obtained with the same settings, from the loaded view.
    pub(crate) fn lookup(&self, key: &CacheKey, fingerprint: &str) -> Option<Entry> {
        self.inner
            .state()
            .entries
            .iter()
            .find(|e| &e.key == key && e.fingerprint == fingerprint)
            .cloned()
    }

    /// Adds `entry`, replacing any entry with the same key.
    pub(crate) async fn store(&self, entry: Entry) {
        self.update(move |entries| {
            entries.retain(|e| e.key != entry.key);
            entries.push(entry);
        })
        .await;
    }

    pub(crate) async fn remove(&self, key: &CacheKey) {
        let key = key.clone();
        self.update(move |entries| entries.retain(|e| e.key != key))
            .await;
    }

    /// The cached entries of one collection and environment, every credentials id.
    pub(crate) fn entries_for(&self, collection: &Path, environment: Option<&str>) -> Vec<Entry> {
        self.inner
            .state()
            .entries
            .iter()
            .filter(|e| {
                e.key.collection == collection && e.key.environment.as_deref() == environment
            })
            .cloned()
            .collect()
    }

    /// The flight that callers hold while fetching the token for `key`, one per key.
    pub(crate) fn flight(&self, key: &CacheKey) -> Arc<Flight> {
        let mut flights = self.inner.flights.lock().unwrap_or_else(|e| e.into_inner());
        flights.entry(key.clone()).or_default().clone()
    }

    /// Applies `change` to the entry list after loading, merging with the file when encrypted.
    async fn update<R: Send + 'static>(
        &self,
        change: impl FnOnce(&mut Vec<Entry>) -> R + Send + 'static,
    ) -> R {
        self.load().await;
        let inner = self.inner.clone();
        match tokio::task::spawn_blocking(move || inner.update_blocking(change)).await {
            Ok(result) => result,
            Err(e) => std::panic::resume_unwind(e.into_panic()),
        }
    }
}

impl Inner {
    fn state(&self) -> MutexGuard<'_, State> {
        // Each critical section leaves the state whole, so a poisoned lock is safe to reuse.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn load_blocking(&self) {
        let Some((path, keys)) = &self.backing else {
            return;
        };
        let (persistence, key, entries) = match sealed::file_key(keys.as_ref(), KEY_NAME, LABEL) {
            Ok(key) => {
                let entries = sealed::read::<Vec<Entry>>(path, &key).unwrap_or_default();
                (Persistence::Encrypted, Some(key), entries)
            }
            Err(reason) => (Persistence::MemoryOnly { reason }, None, Vec::new()),
        };
        let mut state = self.state();
        state.persistence = persistence;
        state.key = key;
        state.entries = entries;
    }

    /// Re-reads the file so writes from other processes survive, applies the change, and writes
    /// the merged list back. A failed write falls back to memory and keeps the change.
    fn update_blocking<R>(&self, change: impl FnOnce(&mut Vec<Entry>) -> R) -> R {
        let _file = self.file.lock().unwrap_or_else(|e| e.into_inner());
        let key = {
            let state = self.state();
            match (&state.persistence, state.key, &self.backing) {
                (Persistence::Encrypted, Some(key), Some((path, _))) => Some((key, path.clone())),
                _ => None,
            }
        };
        let Some((key, path)) = key else {
            return change(&mut self.state().entries);
        };
        let mut entries = sealed::read::<Vec<Entry>>(&path, &key).unwrap_or_default();
        let result = change(&mut entries);
        let written = sealed::write(&path, &key, &entries, LABEL);
        let mut state = self.state();
        if let Err(reason) = written {
            state.persistence = Persistence::MemoryOnly { reason };
        }
        state.entries = entries;
        result
    }
}

/// Shows where tokens are kept and how many there are, never a token.
impl fmt::Debug for TokenCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.inner.state();
        f.debug_struct("TokenCache")
            .field("persistence", &state.persistence)
            .field("entries", &state.entries.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::{MemoryStore, SecretError, SecretKey, SecretStore};
    use base64::Engine as _;
    use serde_json::json;
    use std::sync::Mutex as StdMutex;
    use tempfile::TempDir;

    const TOKEN: &str = "tok-7f3a-SECRET";

    fn key(url: &str, id: &str, env: Option<&str>) -> CacheKey {
        CacheKey::new(PathBuf::from("/c"), env.map(str::to_owned), url, id)
    }
    fn entry(k: CacheKey, token: Value, at: SystemTime) -> Entry {
        let Value::Object(map) = token else {
            panic!("an object")
        };
        Entry::new(k, "fp", map, at)
    }
    fn fresh(k: CacheKey) -> Entry {
        entry(
            k,
            json!({"access_token": TOKEN, "expires_in": 3600}),
            SystemTime::now(),
        )
    }
    async fn opened(dir: &TempDir, keys: Arc<dyn SecretStore>) -> TokenCache {
        let cache = TokenCache::open(dir.path().join("oauth2-tokens.json"), keys);
        cache.load().await;
        cache
    }
    fn file_json(dir: &TempDir) -> Value {
        serde_json::from_slice(&std::fs::read(dir.path().join("oauth2-tokens.json")).unwrap())
            .unwrap()
    }

    /// A keychain that cannot be reached.
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

    /// A keychain where another process stores its own key between our `set` and our re-read.
    struct Racing {
        winner: String,
        stored: StdMutex<Option<String>>,
    }
    impl SecretStore for Racing {
        fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
            Ok(self.stored.lock().unwrap().clone())
        }
        fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
            *self.stored.lock().unwrap() = Some(self.winner.clone());
            Ok(())
        }
        fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn a_token_survives_a_reopen_and_the_file_holds_no_plaintext() {
        let dir = TempDir::new().unwrap();
        let keys: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
        let k = key("https://id.test/token", "main", Some("dev"));
        opened(&dir, keys.clone())
            .await
            .store(fresh(k.clone()))
            .await;
        let text = String::from_utf8(std::fs::read(dir.path().join("oauth2-tokens.json")).unwrap())
            .unwrap();
        assert!(!text.contains(TOKEN) && !text.contains("id.test"), "{text}");
        let file = file_json(&dir);
        assert_eq!(file["version"], 1);
        assert!(
            file["nonce"].is_string() && file["data"].is_string(),
            "{file}"
        );
        let again = opened(&dir, keys).await;
        assert_eq!(again.persistence(), Persistence::Encrypted);
        assert_eq!(
            again
                .lookup(&k, "fp")
                .expect("stored")
                .field("access_token"),
            Some(TOKEN)
        );
    }

    #[tokio::test]
    async fn every_write_uses_a_fresh_nonce() {
        let dir = TempDir::new().unwrap();
        let cache = opened(&dir, Arc::new(MemoryStore::new())).await;
        cache.store(fresh(key("https://a.test/t", "x", None))).await;
        let first = file_json(&dir)["nonce"].clone();
        cache.store(fresh(key("https://a.test/t", "x", None))).await;
        assert_ne!(first, file_json(&dir)["nonce"]);
    }

    #[tokio::test]
    async fn a_file_that_will_not_decrypt_reads_as_empty() {
        let dir = TempDir::new().unwrap();
        let k = key("https://a.test/t", "x", None);
        opened(&dir, Arc::new(MemoryStore::new()))
            .await
            .store(fresh(k.clone()))
            .await;
        // Another key cannot read the file; that is a notice, not an error.
        let other = opened(&dir, Arc::new(MemoryStore::new())).await;
        assert_eq!(other.persistence(), Persistence::Encrypted);
        assert!(other.lookup(&k, "fp").is_none());
        std::fs::write(dir.path().join("oauth2-tokens.json"), "not json").unwrap();
        let garbage = opened(&dir, Arc::new(MemoryStore::new())).await;
        assert!(garbage.lookup(&k, "fp").is_none());
        garbage.store(fresh(k.clone())).await;
        assert!(garbage.lookup(&k, "fp").is_some());
    }

    #[tokio::test]
    async fn without_a_keychain_tokens_stay_in_memory_and_nothing_is_written() {
        let dir = TempDir::new().unwrap();
        let cache = opened(&dir, Arc::new(Locked)).await;
        assert!(
            matches!(cache.persistence(), Persistence::MemoryOnly { ref reason } if reason.contains("locked")),
            "{:?}",
            cache.persistence()
        );
        let k = key("https://a.test/t", "x", None);
        cache.store(fresh(k.clone())).await;
        assert!(cache.lookup(&k, "fp").is_some());
        assert!(!dir.path().join("oauth2-tokens.json").exists());
    }

    #[tokio::test]
    async fn a_key_another_process_stored_first_is_adopted() {
        let dir = TempDir::new().unwrap();
        let winner = base64::engine::general_purpose::STANDARD.encode([7u8; 32]);
        let racing = Arc::new(Racing {
            winner: winner.clone(),
            stored: StdMutex::new(None),
        });
        let k = key("https://a.test/t", "x", None);
        opened(&dir, racing).await.store(fresh(k.clone())).await;
        let store = MemoryStore::new();
        store
            .set(&SecretKey::new("", "", "oauth2-token-key"), &winner)
            .unwrap();
        assert!(
            opened(&dir, Arc::new(store))
                .await
                .lookup(&k, "fp")
                .is_some()
        );
    }

    #[tokio::test]
    async fn writes_merge_with_what_another_process_stored() {
        let dir = TempDir::new().unwrap();
        let keys: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
        let (a, b) = (
            opened(&dir, keys.clone()).await,
            opened(&dir, keys.clone()).await,
        );
        let (ka, kb) = (
            key("https://a.test/t", "a", None),
            key("https://a.test/t", "b", None),
        );
        a.store(fresh(ka.clone())).await;
        b.store(fresh(kb.clone())).await;
        let c = opened(&dir, keys).await;
        assert!(c.lookup(&ka, "fp").is_some() && c.lookup(&kb, "fp").is_some());
    }

    #[tokio::test]
    async fn clear_removes_one_credentials_id_across_token_urls() {
        let dir = TempDir::new().unwrap();
        let keys: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
        let cache = opened(&dir, keys.clone()).await;
        let gone = [
            key("https://a.test/t", "main", Some("dev")),
            key("https://b.test/t", "main", Some("dev")),
        ];
        let kept = [
            key("https://a.test/t", "other", Some("dev")),
            key("https://a.test/t", "main", Some("prod")),
            key("https://a.test/t", "main", None),
        ];
        for k in gone.iter().chain(&kept) {
            cache.store(fresh(k.clone())).await;
        }
        assert_eq!(cache.clear(Path::new("/c"), Some("dev"), "main").await, 2);
        let again = opened(&dir, keys).await;
        for k in &gone {
            assert!(again.lookup(k, "fp").is_none(), "{k:?}");
        }
        for k in &kept {
            assert!(again.lookup(k, "fp").is_some(), "{k:?}");
        }
    }

    #[tokio::test]
    async fn a_fingerprint_mismatch_reads_as_no_token() {
        let cache = TokenCache::in_memory();
        let k = key("https://a.test/t", "x", None);
        cache.store(fresh(k.clone())).await;
        assert!(cache.lookup(&k, "other").is_none());
        assert_eq!(cache.entries_for(Path::new("/c"), None).len(), 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        opened(&dir, Arc::new(MemoryStore::new()))
            .await
            .store(fresh(key("https://a.test/t", "x", None)))
            .await;
        let meta = std::fs::metadata(dir.path().join("oauth2-tokens.json")).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    }

    #[tokio::test]
    async fn nothing_is_read_until_load() {
        let dir = TempDir::new().unwrap();
        let cache = TokenCache::open(dir.path().join("t.json"), Arc::new(Locked));
        assert_eq!(cache.persistence(), Persistence::NotLoaded);
        cache.load().await;
        assert!(matches!(
            cache.persistence(),
            Persistence::MemoryOnly { .. }
        ));
        assert!(matches!(
            TokenCache::in_memory().persistence(),
            Persistence::MemoryOnly { .. }
        ));
    }

    #[test]
    fn expiry_takes_a_thirty_second_margin_and_reads_numeric_strings() {
        let at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let k = key("https://a.test/t", "x", None);
        let number = entry(
            k.clone(),
            json!({"access_token": "t", "expires_in": 100}),
            at,
        );
        assert!(!number.is_expired(at + Duration::from_secs(69)));
        assert!(number.is_expired(at + Duration::from_secs(70)));
        let text = entry(
            k.clone(),
            json!({"access_token": "t", "expires_in": "100"}),
            at,
        );
        assert_eq!(text.expires_at(), Some(at + Duration::from_secs(70)));
        let never = entry(k, json!({"access_token": "t"}), at);
        assert_eq!(never.expires_at(), None);
        assert!(!never.is_expired(at + Duration::from_secs(10_000_000)));
        // A clock set back before the token was obtained reads as expired.
        assert!(never.is_expired(at - Duration::from_secs(1)));
    }

    #[tokio::test]
    async fn debug_never_shows_a_token() {
        let cache = TokenCache::in_memory();
        let k = key("https://a.test/t", "x", None);
        cache.store(fresh(k.clone())).await;
        let shown = format!("{cache:?} {:?}", cache.lookup(&k, "fp").unwrap());
        assert!(!shown.contains(TOKEN), "{shown}");
    }

    #[tokio::test]
    async fn secret_values_list_cached_token_fields_but_not_public_ones() {
        use crate::{Collection, ContextOptions, VarContext};
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("opencollection.yml"),
            "opencollection: 1.0.0\ninfo:\n  name: c\n",
        )
        .unwrap();
        let c = Collection::open(dir.path()).unwrap();
        let cache = TokenCache::in_memory();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let k = CacheKey::new(root, None, "http://127.0.0.1:9/token", "main");
        let token = json!({"access_token": TOKEN, "token_type": "Bearer-public-type"});
        cache.store(entry(k, token, SystemTime::now())).await;
        let ctx = VarContext::new(&c, ContextOptions::new().oauth2_tokens(cache)).unwrap();
        let values = ctx.secret_values();
        assert!(values.contains(&TOKEN.to_owned()), "{values:?}");
        assert!(!values.iter().any(|v| v == "Bearer-public-type"));
    }
}
