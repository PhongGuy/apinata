//! A process-wide cache of fetched PAC scripts.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::OnceCell;

/// The time source, injectable so tests can move it by hand.
pub(crate) type Clock = Arc<dyn Fn() -> Instant + Send + Sync>;

/// How long a fetched script is reused, counted from when its fetch started.
pub(crate) const TTL: Duration = Duration::from_secs(300);

/// The fetch outcome shared by every caller that asked while it was in flight.
type Shared = Arc<OnceCell<Result<Arc<str>, String>>>;

struct Entry {
    started: Instant,
    cell: Shared,
}

/// Fetched scripts by normalised source. An entry lives `TTL` from when its fetch started;
/// concurrent callers share one fetch; a failed fetch leaves no entry.
pub(crate) struct PacCache {
    clock: Clock,
    entries: Mutex<HashMap<String, Entry>>,
}

impl PacCache {
    /// An empty cache reading time from `clock`.
    pub(crate) fn new(clock: Clock) -> PacCache {
        PacCache {
            clock,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// The script for `key`, running `fetch` only when no live entry exists.
    pub(crate) async fn get<F, Fut>(&self, key: &str, fetch: F) -> Result<Arc<str>, String>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<String, String>>,
    {
        // Pick the live entry or start a fresh one under the lock, then await outside it.
        // Waiters share the cell; if its initialiser is dropped, the next waiter fetches.
        let cell = {
            let now = (self.clock)();
            let mut entries = self.lock();
            match entries.get(key) {
                Some(entry) if now.saturating_duration_since(entry.started) < TTL => {
                    Arc::clone(&entry.cell)
                }
                _ => {
                    let cell: Shared = Arc::default();
                    let entry = Entry {
                        started: now,
                        cell: Arc::clone(&cell),
                    };
                    entries.insert(key.to_owned(), entry);
                    cell
                }
            }
        };
        let result = cell
            .get_or_init(|| async { fetch().await.map(Arc::from) })
            .await
            .clone();
        // A failure is not cached, but a newer entry started since must survive.
        if result.is_err() {
            let mut entries = self.lock();
            if entries
                .get(key)
                .is_some_and(|e| Arc::ptr_eq(&e.cell, &cell))
            {
                entries.remove(key);
            }
        }
        result
    }

    /// Drops `key`, so its next `get` fetches again (a script that failed to compile).
    pub(crate) fn forget(&self, key: &str) {
        self.lock().remove(key);
    }

    /// Drops every entry.
    pub(crate) fn clear(&self) {
        self.lock().clear();
    }

    // A panic while holding the lock leaves the map itself consistent, so poison is ignored.
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Entry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The cache every executor in the process shares.
pub(crate) static PAC_CACHE: LazyLock<PacCache> =
    LazyLock::new(|| PacCache::new(Arc::new(Instant::now)));

/// Forgets every fetched PAC script, so the next request fetches again.
pub fn clear_pac_cache() {
    PAC_CACHE.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

    /// A clock the test moves by hand.
    fn manual_clock() -> (Arc<Mutex<Instant>>, Clock) {
        let now = Arc::new(Mutex::new(Instant::now()));
        let read = Arc::clone(&now);
        (now, Arc::new(move || *read.lock().expect("clock")))
    }

    /// A fetch that counts its calls, takes 50 ms and returns `answer`.
    fn counted(
        calls: &Arc<AtomicUsize>,
        answer: Result<&'static str, &'static str>,
    ) -> impl FnOnce() -> std::pin::Pin<Box<dyn Future<Output = Result<String, String>> + Send>>
    {
        let calls = Arc::clone(calls);
        move || {
            Box::pin(async move {
                calls.fetch_add(1, SeqCst);
                tokio::time::sleep(Duration::from_millis(50)).await;
                answer.map(str::to_owned).map_err(str::to_owned)
            })
        }
    }

    #[tokio::test]
    async fn an_entry_lives_five_minutes_from_its_fetch() {
        assert_eq!(TTL, Duration::from_secs(300));
        let (now, clock) = manual_clock();
        let cache = PacCache::new(clock);
        let calls = Arc::new(AtomicUsize::new(0));
        assert_eq!(
            cache.get("k", counted(&calls, Ok("a"))).await.as_deref(),
            Ok("a")
        );
        *now.lock().expect("clock") += Duration::from_secs(299);
        assert_eq!(
            cache.get("k", counted(&calls, Ok("b"))).await.as_deref(),
            Ok("a")
        );
        assert_eq!(calls.load(SeqCst), 1);
        *now.lock().expect("clock") += Duration::from_secs(1);
        assert_eq!(
            cache.get("k", counted(&calls, Ok("b"))).await.as_deref(),
            Ok("b")
        );
        assert_eq!(calls.load(SeqCst), 2);
    }

    #[tokio::test]
    async fn concurrent_first_requests_share_one_fetch() {
        let cache = PacCache::new(Arc::new(Instant::now));
        let calls = Arc::new(AtomicUsize::new(0));
        let (a, b) = tokio::join!(
            cache.get("k", counted(&calls, Ok("a"))),
            cache.get("k", counted(&calls, Ok("b")))
        );
        assert_eq!((a.as_deref(), b.as_deref()), (Ok("a"), Ok("a")));
        assert_eq!(calls.load(SeqCst), 1);
    }

    #[tokio::test]
    async fn a_cancelled_fetch_lets_the_next_caller_fetch() {
        let cache = PacCache::new(Arc::new(Instant::now));
        let calls = Arc::new(AtomicUsize::new(0));
        let stalled = Arc::clone(&calls);
        let first = cache.get("k", move || async move {
            stalled.fetch_add(1, SeqCst);
            std::future::pending::<Result<String, String>>().await
        });
        // Start the stalled fetch, then drop it as a cancelled send would.
        assert!(
            tokio::time::timeout(Duration::from_millis(20), first)
                .await
                .is_err()
        );
        assert_eq!(
            cache.get("k", counted(&calls, Ok("a"))).await.as_deref(),
            Ok("a")
        );
        assert_eq!(calls.load(SeqCst), 2);
    }

    #[tokio::test]
    async fn failures_are_not_cached() {
        let cache = PacCache::new(Arc::new(Instant::now));
        let calls = Arc::new(AtomicUsize::new(0));
        let failed = cache.get("k", counted(&calls, Err("down"))).await;
        assert_eq!(failed.as_deref(), Err(&"down".to_owned()));
        assert_eq!(
            cache.get("k", counted(&calls, Ok("a"))).await.as_deref(),
            Ok("a")
        );
        assert_eq!(calls.load(SeqCst), 2);
    }

    #[tokio::test]
    async fn keys_are_separate_and_forget_and_clear_drop_entries() {
        let cache = PacCache::new(Arc::new(Instant::now));
        let calls = Arc::new(AtomicUsize::new(0));
        cache.get("k", counted(&calls, Ok("a"))).await.expect("k");
        cache
            .get("other", counted(&calls, Ok("o")))
            .await
            .expect("other");
        assert_eq!(calls.load(SeqCst), 2);
        cache.forget("k");
        assert_eq!(
            cache.get("k", counted(&calls, Ok("b"))).await.as_deref(),
            Ok("b")
        );
        assert_eq!(
            cache
                .get("other", counted(&calls, Ok("x")))
                .await
                .as_deref(),
            Ok("o")
        );
        assert_eq!(calls.load(SeqCst), 3);
        cache.clear();
        assert_eq!(
            cache
                .get("other", counted(&calls, Ok("x")))
                .await
                .as_deref(),
            Ok("x")
        );
        assert_eq!(calls.load(SeqCst), 4);
    }
}
