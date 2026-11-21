//! The platform keychain, opened lazily, and a wrapper that bounds each call of a secret store.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::Duration;

use crate::keychain::KeychainStore;
use crate::secrets::{SecretError, SecretKey, SecretStore};

/// How long one keychain call may take before it is reported unavailable.
pub const KEYCHAIN_TIMEOUT: Duration = Duration::from_secs(10);

/// The platform keychain, opened on its first call (calls already run on the blocking pool).
#[derive(Default)]
#[non_exhaustive]
pub struct LazyKeychain {
    store: OnceLock<Result<KeychainStore, SecretError>>,
}

impl LazyKeychain {
    /// The opened keychain, or why it could not be opened; the outcome of the first open sticks.
    fn store(&self) -> Result<&KeychainStore, SecretError> {
        self.store
            .get_or_init(KeychainStore::new)
            .as_ref()
            .map_err(Clone::clone)
    }
}

impl SecretStore for LazyKeychain {
    fn get(&self, key: &SecretKey) -> Result<Option<String>, SecretError> {
        self.store()?.get(key)
    }

    fn set(&self, key: &SecretKey, value: &str) -> Result<(), SecretError> {
        self.store()?.set(key, value)
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        self.store()?.delete(key)
    }

    fn list(&self, collection: &Path) -> Result<Vec<SecretKey>, SecretError> {
        self.store()?.list(collection)
    }

    fn list_roots(&self) -> Result<Vec<PathBuf>, SecretError> {
        self.store()?.list_roots()
    }
}

/// Runs each call of `inner` on its own thread; past `limit` it answers
/// `SecretError::Unavailable { message: "keychain unavailable: no answer within {limit} s" }`.
/// While a timed-out call is still running, new calls are refused at once ("keychain
/// unavailable: an earlier call has not returned") without spawning. Two concurrent callers can
/// both pass that check, so the bound on hung threads is the number of simultaneous callers;
/// writes are serialised by the collection lock, so for them it is one. A panicking call is
/// "keychain unavailable: the keychain call failed" and never counts as stuck.
#[non_exhaustive]
pub struct TimeoutStore {
    inner: Arc<dyn SecretStore>,
    limit: Duration,
    /// Calls that timed out and have not returned yet.
    stuck: Arc<AtomicUsize>,
}

/// Dropped when a call's thread ends, by return or by panic: if the caller already gave up on
/// it (whichever side flips `late` second), the call is no longer stuck.
struct Settle {
    late: Arc<AtomicBool>,
    stuck: Arc<AtomicUsize>,
}

impl Drop for Settle {
    fn drop(&mut self) {
        if self.late.swap(true, Ordering::AcqRel) {
            self.stuck.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

fn unavailable(why: &str) -> SecretError {
    SecretError::Unavailable {
        message: format!("keychain unavailable: {why}"),
    }
}

impl TimeoutStore {
    /// Bound every call of `inner` to `limit`.
    pub fn new(inner: Arc<dyn SecretStore>, limit: Duration) -> TimeoutStore {
        TimeoutStore {
            inner,
            limit,
            stuck: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Run `call` on a fresh thread and wait at most `limit` for its answer.
    fn bounded<T: Send + 'static>(
        &self,
        call: impl FnOnce(&dyn SecretStore) -> Result<T, SecretError> + Send + 'static,
    ) -> Result<T, SecretError> {
        if self.stuck.load(Ordering::Acquire) > 0 {
            return Err(unavailable("an earlier call has not returned"));
        }
        let late = Arc::new(AtomicBool::new(false));
        let settle = Settle {
            late: Arc::clone(&late),
            stuck: Arc::clone(&self.stuck),
        };
        let inner = Arc::clone(&self.inner);
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _settle = settle;
            let _ = tx.send(call(inner.as_ref()));
        });
        match rx.recv_timeout(self.limit) {
            Ok(result) => return result,
            // The call panicked; its thread is gone, so nothing is stuck.
            Err(RecvTimeoutError::Disconnected) => {
                return Err(unavailable("the keychain call failed"));
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
        // Counted before `late` flips, so the thread's decrement can never come first.
        self.stuck.fetch_add(1, Ordering::AcqRel);
        if late.swap(true, Ordering::AcqRel) {
            // The thread ended between the timeout and here; any answer is in the channel.
            self.stuck.fetch_sub(1, Ordering::AcqRel);
            return rx
                .try_recv()
                .unwrap_or_else(|_| Err(unavailable("the keychain call failed")));
        }
        Err(unavailable(&format!(
            "no answer within {} s",
            self.limit.as_secs()
        )))
    }
}

impl SecretStore for TimeoutStore {
    fn get(&self, key: &SecretKey) -> Result<Option<String>, SecretError> {
        let key = key.clone();
        self.bounded(move |store| store.get(&key))
    }

    fn set(&self, key: &SecretKey, value: &str) -> Result<(), SecretError> {
        let (key, value) = (key.clone(), value.to_owned());
        self.bounded(move |store| store.set(&key, &value))
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        let key = key.clone();
        self.bounded(move |store| store.delete(&key))
    }

    fn list(&self, collection: &Path) -> Result<Vec<SecretKey>, SecretError> {
        let collection = collection.to_path_buf();
        self.bounded(move |store| store.list(&collection))
    }

    fn list_roots(&self) -> Result<Vec<PathBuf>, SecretError> {
        self.bounded(|store| store.list_roots())
    }
}

/// Why the platform keychain is not used for this run.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct KeychainUnavailable {
    /// Plain words for the user, such as "it did not answer in time".
    pub reason: String,
    /// What the platform reported, for logs only; empty after a timeout.
    pub detail: String,
}

/// Open the platform keychain and check that it answers, by reading an entry that cannot exist
/// within `limit`; the probe never writes. The store comes back with every call bounded by
/// `limit`.
pub fn open_keychain(limit: Duration) -> Result<Arc<dyn SecretStore>, KeychainUnavailable> {
    let opened = KeychainStore::new().map(|k| Arc::new(k) as Arc<dyn SecretStore>);
    checked(opened, limit)
}

fn checked(
    opened: Result<Arc<dyn SecretStore>, SecretError>,
    limit: Duration,
) -> Result<Arc<dyn SecretStore>, KeychainUnavailable> {
    let unavailable = |reason: &str, detail: String| KeychainUnavailable {
        reason: reason.to_owned(),
        detail,
    };
    let store = opened.map_err(|e| unavailable("it could not be reached", e.to_string()))?;
    // Opening the store does not talk to the platform service, so a read is the first sign of a
    // service that is registered but never answers; a hung read is left on its own thread.
    let (tx, rx) = mpsc::channel();
    let probed = Arc::clone(&store);
    thread::spawn(move || {
        let _ = tx.send(probed.get(&SecretKey::new("", "", "keychain-probe")));
    });
    match rx.recv_timeout(limit) {
        Ok(Ok(_)) => Ok(Arc::new(TimeoutStore::new(store, limit))),
        Ok(Err(e)) => Err(unavailable("it did not answer", e.to_string())),
        Err(RecvTimeoutError::Timeout) => {
            Err(unavailable("it did not answer in time", String::new()))
        }
        Err(RecvTimeoutError::Disconnected) => Err(unavailable(
            "it did not answer",
            "the keychain call failed".to_owned(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemoryStore;
    use std::time::Instant;

    fn key() -> SecretKey {
        SecretKey::new("/srv/shop", "prod", "token")
    }

    #[test]
    fn a_store_that_answers_in_time_is_passed_through() {
        let store = TimeoutStore::new(Arc::new(MemoryStore::new()), Duration::from_secs(5));
        store.set(&key(), "v").expect("set");
        assert_eq!(store.get(&key()).expect("get").as_deref(), Some("v"));
        store.delete(&key()).expect("delete");
        assert_eq!(store.get(&key()).expect("get"), None);
    }

    #[test]
    fn list_roots_is_passed_through() {
        let inner = MemoryStore::new();
        inner.set(&key(), "v").expect("set");
        let store = TimeoutStore::new(Arc::new(inner), Duration::from_secs(5));
        assert_eq!(
            store.list_roots().expect("roots"),
            [std::path::PathBuf::from("/srv/shop")]
        );
    }

    /// A store whose calls never return, counting how many were started.
    #[derive(Default)]
    struct Hung {
        started: std::sync::atomic::AtomicUsize,
    }

    impl Hung {
        fn hang<T>(&self) -> T {
            self.started.fetch_add(1, Ordering::SeqCst);
            loop {
                thread::park();
            }
        }
    }

    impl SecretStore for Hung {
        fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
            self.hang()
        }
        fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
            self.hang()
        }
        fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
            self.hang()
        }
    }

    #[test]
    fn a_hung_store_costs_one_thread_and_later_calls_are_refused_at_once() {
        let hung = Arc::new(Hung::default());
        let limit = Duration::from_millis(500);
        let store = TimeoutStore::new(Arc::<Hung>::clone(&hung), limit);
        let first = store.get(&key()).expect_err("the first call times out");
        assert!(
            first
                .to_string()
                .contains("keychain unavailable: no answer within"),
            "{first}"
        );
        // A refusal that waited would take at least `limit`, so each call is held to it.
        for _ in 0..20 {
            let started = Instant::now();
            let e = store.set(&key(), "v").expect_err("refused");
            assert!(e.to_string().contains("keychain unavailable"), "{e}");
            assert!(started.elapsed() < limit, "{:?}", started.elapsed());
        }
        assert_eq!(
            hung.started.load(Ordering::SeqCst),
            1,
            "one thread, never more"
        );
    }

    /// A store whose calls panic, counting them.
    #[derive(Default)]
    struct Panicky {
        calls: std::sync::atomic::AtomicUsize,
    }

    impl Panicky {
        fn boom<T>(&self) -> T {
            self.calls.fetch_add(1, Ordering::SeqCst);
            panic!("keychain backend panicked")
        }
    }

    impl SecretStore for Panicky {
        fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
            self.boom()
        }
        fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
            self.boom()
        }
        fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
            self.boom()
        }
    }

    #[test]
    fn a_panicking_call_is_unavailable_and_the_next_call_is_tried_again() {
        let panicky = Arc::new(Panicky::default());
        let store = TimeoutStore::new(Arc::<Panicky>::clone(&panicky), Duration::from_secs(5));
        for _ in 0..2 {
            let e = store.get(&key()).expect_err("fails");
            assert!(e.to_string().contains("the keychain call failed"), "{e}");
        }
        assert_eq!(
            panicky.calls.load(Ordering::SeqCst),
            2,
            "the second call was tried"
        );
    }

    /// Panics once, after `delay`.
    struct LatePanic {
        delay: Duration,
    }

    impl SecretStore for LatePanic {
        fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
            thread::sleep(self.delay);
            panic!("late keychain panic")
        }
        fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
            Ok(())
        }
        fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
            Ok(())
        }
    }

    #[test]
    fn a_panic_after_the_timeout_still_unblocks_later_calls() {
        let store = TimeoutStore::new(
            Arc::new(LatePanic {
                delay: Duration::from_millis(200),
            }),
            Duration::from_millis(50),
        );
        store.get(&key()).expect_err("times out");
        store
            .set(&key(), "v")
            .expect_err("refused while the first runs");
        // Retry until a deadline: under load even the instant `set` can miss the 50 ms limit.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            thread::sleep(Duration::from_millis(100));
            match store.set(&key(), "v") {
                Ok(()) => break,
                Err(_) if Instant::now() < deadline => {}
                Err(e) => panic!("tried again once the panicked call ended: {e:?}"),
            }
        }
    }

    #[test]
    fn the_lazy_keychain_forwards_list() {
        // The stored failure stands in for a real keychain, which tests must never open.
        let closed = LazyKeychain {
            store: OnceLock::from(Err(SecretError::Failed {
                message: "closed".into(),
            })),
        };
        let got = closed.list(std::path::Path::new("/a"));
        assert_eq!(
            got,
            Err(SecretError::Failed {
                message: "closed".into()
            })
        );
    }

    #[test]
    fn the_timeout_store_forwards_list() {
        let inner = Arc::new(MemoryStore::new());
        inner.set(&SecretKey::new("/a", "dev", "x"), "1").unwrap();
        let bounded = TimeoutStore::new(inner, Duration::from_secs(5));
        let got = bounded.list(std::path::Path::new("/a")).unwrap();
        assert_eq!(got, [SecretKey::new("/a", "dev", "x")]);
    }

    /// A store that records every call and answers each read with `answer`.
    struct Probed {
        answer: Result<Option<String>, SecretError>,
        calls: std::sync::Mutex<Vec<&'static str>>,
    }

    impl Probed {
        fn new(answer: Result<Option<String>, SecretError>) -> Arc<Probed> {
            Arc::new(Probed {
                answer,
                calls: Default::default(),
            })
        }
    }

    impl SecretStore for Probed {
        fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
            self.calls.lock().unwrap().push("get");
            self.answer.clone()
        }
        fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
            self.calls.lock().unwrap().push("set");
            Ok(())
        }
        fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
            self.calls.lock().unwrap().push("delete");
            Ok(())
        }
    }

    fn opened(store: Arc<Probed>) -> Result<Arc<dyn SecretStore>, SecretError> {
        Ok(store)
    }

    #[test]
    fn a_keychain_that_has_no_probe_entry_is_kept_and_only_read() {
        let store = Probed::new(Ok(None));
        let kept = checked(opened(Arc::clone(&store)), Duration::from_secs(5));
        assert!(kept.is_ok());
        assert_eq!(*store.calls.lock().unwrap(), ["get"]);
    }

    #[test]
    fn a_keychain_whose_read_fails_falls_back_in_plain_words() {
        let raw = "Couldn't access platform storage: SS error: result not returned from SS API";
        let store = Probed::new(Err(SecretError::Unavailable {
            message: raw.into(),
        }));
        let why = checked(opened(Arc::clone(&store)), Duration::from_secs(5))
            .err()
            .expect("falls back");
        assert_eq!(why.reason, "it did not answer");
        assert!(why.detail.contains(raw), "{}", why.detail);
        assert_eq!(*store.calls.lock().unwrap(), ["get"]);
    }

    #[test]
    fn a_keychain_that_does_not_answer_in_time_falls_back() {
        let started = Instant::now();
        let why = checked(Ok(Arc::new(Hung::default())), Duration::from_millis(200))
            .err()
            .expect("falls back");
        assert_eq!(why.reason, "it did not answer in time");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_keychain_that_cannot_be_opened_falls_back_in_plain_words() {
        let why = checked(
            Err(SecretError::Unavailable {
                message: "SS error".into(),
            }),
            Duration::from_secs(5),
        )
        .err()
        .expect("falls back");
        assert_eq!(why.reason, "it could not be reached");
        assert!(why.detail.contains("SS error"), "{}", why.detail);
    }

    #[test]
    fn a_panicking_probe_falls_back() {
        let why = checked(Ok(Arc::new(Panicky::default())), Duration::from_secs(5))
            .err()
            .expect("falls back");
        assert_eq!(why.reason, "it did not answer");
    }
}
