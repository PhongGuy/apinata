//! Startup must not wait on the secret service for the proxy password: the executor is
//! installed at once, and the password arrives later through a reinstall.

use crate::support;

use apinata_app::app::{AppState, start};
use apinata_app::executor::{self, proxy_password_key};
use apinata_app::persist::{Paths, save_json};
use apinata_app::settings::{ProxyMode, ProxyPrefs, Settings};
use apinata_core::{MemoryStore, SecretError, SecretKey, SecretStore};
use gpui_kit::component::WindowExt;
use gpui_kit::{AppContext as _, TestAppContext};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

enum Reply {
    Value(Option<String>),
    Locked,
}

// A store whose proxy password read blocks until the test answers it. A watchdog answers
// after five seconds, so a startup that waits on the store fails the timing assertion
// instead of hanging the suite.
struct GatedStore {
    inner: MemoryStore,
    gate: Mutex<Receiver<Reply>>,
    answered: AtomicBool,
}

impl GatedStore {
    fn new() -> (Arc<GatedStore>, Sender<Reply>) {
        let (tx, rx) = channel();
        let watchdog = tx.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(5));
            let _ = watchdog.send(Reply::Value(None));
        });
        let store = GatedStore {
            inner: MemoryStore::new(),
            gate: Mutex::new(rx),
            answered: AtomicBool::new(false),
        };
        (Arc::new(store), tx)
    }
}

impl SecretStore for GatedStore {
    fn get(&self, key: &SecretKey) -> Result<Option<String>, SecretError> {
        if *key != proxy_password_key() {
            return self.inner.get(key);
        }
        let reply = self.gate.lock().unwrap().recv();
        self.answered.store(true, Ordering::SeqCst);
        match reply {
            Ok(Reply::Value(v)) => Ok(v),
            _ => Err(SecretError::Unavailable {
                message: "locked".into(),
            }),
        }
    }
    fn set(&self, key: &SecretKey, value: &str) -> Result<(), SecretError> {
        self.inner.set(key, value)
    }
    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        self.inner.delete(key)
    }
}

fn manual(port: &str) -> Settings {
    Settings {
        proxy: ProxyPrefs {
            mode: ProxyMode::Manual,
            protocol: "http".into(),
            host: "127.0.0.1".into(),
            port: port.into(),
            username: "u".into(),
            bypass: "localhost".into(),
            pac_source: String::new(),
        },
        ..Settings::default()
    }
}

struct Started {
    win: gpui_kit::AnyWindowHandle,
    took: Duration,
    _files: tempfile::TempDir,
}

fn start_gated(cx: &mut TestAppContext, store: Arc<GatedStore>) -> Started {
    support::init(cx);
    cx.executor().allow_parking();
    cx.update(apinata_app::tokio_bridge::init);
    let files = tempfile::tempdir().unwrap();
    let paths = Paths::in_dir(files.path());
    save_json(&paths.settings, &manual("3128")).unwrap();
    let began = Instant::now();
    let (win, _) = cx
        .update(|cx| start(&paths, store, Vec::new(), cx))
        .unwrap();
    Started {
        win,
        took: began.elapsed(),
        _files: files,
    }
}

fn shared(cx: &mut TestAppContext) -> Option<Arc<apinata_core::Executor>> {
    cx.update(|cx| executor::shared(cx))
}

// Parks until `done` holds, polling the background tasks, for at most three seconds.
fn wait_for(cx: &mut TestAppContext, done: impl Fn(&mut TestAppContext) -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(3);
    while Instant::now() < end {
        cx.run_until_parked();
        if done(cx) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn notices(cx: &mut TestAppContext, s: &Started) -> usize {
    cx.update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

#[gpui_kit::test]
fn start_installs_the_executor_without_waiting_for_the_store(cx: &mut TestAppContext) {
    let (store, _release) = GatedStore::new();
    let s = start_gated(cx, store);
    assert!(s.took < Duration::from_secs(2), "start took {:?}", s.took);
    assert!(shared(cx).is_some());
}

#[gpui_kit::test]
fn the_executor_is_reinstalled_once_the_password_arrives(cx: &mut TestAppContext) {
    let (store, release) = GatedStore::new();
    let s = start_gated(cx, store);
    let first = shared(cx).expect("an executor at once");
    release.send(Reply::Value(Some("pw".into()))).unwrap();
    assert!(wait_for(cx, |cx| {
        shared(cx).is_some_and(|e| !Arc::ptr_eq(&e, &first))
    }));
    assert_eq!(notices(cx, &s), 0);
}

#[gpui_kit::test]
fn an_unreadable_password_warns_and_keeps_the_executor(cx: &mut TestAppContext) {
    let (store, release) = GatedStore::new();
    let s = start_gated(cx, store);
    release.send(Reply::Locked).unwrap();
    assert!(wait_for(cx, |cx| notices(cx, &s) == 1));
    assert!(shared(cx).is_some());
}

#[gpui_kit::test]
fn a_proxy_changed_meanwhile_skips_the_reinstall(cx: &mut TestAppContext) {
    let (store, release) = GatedStore::new();
    let s = start_gated(cx, store.clone());
    let first = shared(cx).expect("an executor at once");
    cx.update(|cx| {
        let settings = cx.global::<AppState>().settings.clone();
        settings.update(cx, |p, cx| p.update(cx, |v| v.proxy.port = "3129".into()));
    });
    release.send(Reply::Value(Some("pw".into()))).unwrap();
    // Once the read has landed and been handled, the executor must still be the first one.
    assert!(wait_for(cx, |_| store.answered.load(Ordering::SeqCst)));
    wait_for(cx, |_| false);
    assert!(Arc::ptr_eq(&shared(cx).unwrap(), &first));
    assert_eq!(notices(cx, &s), 0);
}
