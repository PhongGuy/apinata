//! Network preferences reach the next send without a restart.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::executor::{self, proxy_password_key};
use apinata_app::settings::{ProxyMode, ProxyPrefs, Settings};
use apinata_core::{MemoryStore, SecretStore};
use gpui_kit::TestAppContext;
use std::sync::Arc;
use support::*;

fn requests_to(cx: &mut TestAppContext, server: &wiremock::MockServer) -> Vec<wiremock::Request> {
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    handle
        .block_on(server.received_requests())
        .unwrap_or_default()
}

// Sends one GET to `target` through whatever shared executor is installed now.
fn send_now(cx: &mut TestAppContext, target: &wiremock::MockServer) {
    let exec = cx.update(|cx| executor::shared(cx)).unwrap();
    send_direct(cx, &exec, &target.uri());
}

fn edit_settings(cx: &mut TestAppContext, f: impl FnOnce(&mut Settings)) {
    cx.update(|cx| {
        let settings = cx.global::<AppState>().settings.clone();
        settings.update(cx, |p, cx| p.update(cx, f));
    });
    cx.run_until_parked();
}

// A memory store whose reads answer only after `delay`, so a password rebuild is still in
// flight when the edit that started it returns.
struct SlowStore {
    inner: MemoryStore,
    delay: std::time::Duration,
}

impl SecretStore for SlowStore {
    fn get(
        &self,
        key: &apinata_core::SecretKey,
    ) -> Result<Option<String>, apinata_core::SecretError> {
        std::thread::sleep(self.delay);
        self.inner.get(key)
    }
    fn set(
        &self,
        key: &apinata_core::SecretKey,
        value: &str,
    ) -> Result<(), apinata_core::SecretError> {
        self.inner.set(key, value)
    }
    fn delete(&self, key: &apinata_core::SecretKey) -> Result<(), apinata_core::SecretError> {
        self.inner.delete(key)
    }
}

fn manual_proxy(proxy: &wiremock::MockServer, username: &str) -> ProxyPrefs {
    let addr = proxy.address();
    ProxyPrefs {
        mode: ProxyMode::Manual,
        protocol: "http".into(),
        host: addr.ip().to_string(),
        port: addr.port().to_string(),
        username: username.into(),
        bypass: String::new(),
        pac_source: String::new(),
    }
}

fn ready(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    cx.update(apinata_app::tokio_bridge::init);
}

#[gpui_kit::test]
fn a_proxy_chosen_in_preferences_carries_the_next_send(cx: &mut TestAppContext) {
    ready(cx);
    let col = two_requests();
    let _s = started(cx, &col);
    let target = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let proxy = mock_server(cx, wiremock::ResponseTemplate::new(200));
    send_now(cx, &target);
    assert_eq!(requests_to(cx, &proxy).len(), 0);
    edit_settings(cx, |s| s.proxy = manual_proxy(&proxy, ""));
    send_now(cx, &target);
    assert_eq!(requests_to(cx, &proxy).len(), 1);
    assert_eq!(requests_to(cx, &target).len(), 1);
}

#[gpui_kit::test]
fn a_proxy_user_chosen_in_preferences_sends_the_stored_password(cx: &mut TestAppContext) {
    ready(cx);
    let col = two_requests();
    let inner = MemoryStore::new();
    inner.set(&proxy_password_key(), "p1").unwrap();
    let delay = std::time::Duration::from_millis(200);
    let _s = started_with_store(cx, &col, Arc::new(SlowStore { inner, delay }));
    let target = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let proxy = mock_server(cx, wiremock::ResponseTemplate::new(200));
    edit_settings(cx, |s| s.proxy = manual_proxy(&proxy, "u"));
    network_installed(cx);
    send_now(cx, &target);
    let got = requests_to(cx, &proxy);
    assert_eq!(got.len(), 1);
    // Basic base64 of "u:p1".
    let auth = got[0]
        .headers
        .get("proxy-authorization")
        .map(|v| v.to_str().unwrap().to_owned());
    assert_eq!(auth.as_deref(), Some("Basic dTpwMQ=="));
}

#[gpui_kit::test]
fn turning_the_proxy_off_again_sends_direct(cx: &mut TestAppContext) {
    ready(cx);
    let col = two_requests();
    let _s = started(cx, &col);
    let target = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let proxy = mock_server(cx, wiremock::ResponseTemplate::new(200));
    edit_settings(cx, |s| s.proxy = manual_proxy(&proxy, ""));
    edit_settings(cx, |s| s.proxy.mode = ProxyMode::Off);
    send_now(cx, &target);
    assert_eq!(requests_to(cx, &proxy).len(), 0);
    assert_eq!(requests_to(cx, &target).len(), 1);
}

// The response size limit is part of the rebuilt executor too: a smaller cap cuts the next body.
#[gpui_kit::test]
fn a_smaller_body_limit_applies_to_the_next_send(cx: &mut TestAppContext) {
    ready(cx);
    let col = two_requests();
    let _s = started(cx, &col);
    let body = "x".repeat(4096);
    let target = mock_server(
        cx,
        wiremock::ResponseTemplate::new(200).set_body_string(body),
    );
    edit_settings(cx, |s| s.max_body_bytes = Some(16));
    let exec = cx.update(|cx| executor::shared(cx)).unwrap();
    let col = collection_for(&target.uri(), "");
    let c = apinata_core::Collection::open(col.path()).unwrap();
    let ctx = apinata_core::VarContext::new(&c, apinata_core::ContextOptions::new()).unwrap();
    let req = apinata_core::resolve(c.request(&ip("a.yml")).unwrap(), &ctx).unwrap();
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let resp = handle.block_on(exec.send(&req)).unwrap();
    assert!(resp.body().len() <= 16, "the 16-byte limit did not apply");
}

// A store whose proxy password reads each wait for their own answer, in call order, so a test
// decides which read lands first. An unanswered read gives up after five seconds.
struct OrderedStore {
    gates: Vec<std::sync::Mutex<std::sync::mpsc::Receiver<String>>>,
    calls: std::sync::atomic::AtomicUsize,
    answered: std::sync::atomic::AtomicUsize,
}

impl OrderedStore {
    fn new(reads: usize) -> (Arc<OrderedStore>, Vec<std::sync::mpsc::Sender<String>>) {
        let (senders, gates) = (0..reads)
            .map(|_| {
                let (tx, rx) = std::sync::mpsc::channel();
                (tx, std::sync::Mutex::new(rx))
            })
            .unzip();
        let store = OrderedStore {
            gates,
            calls: Default::default(),
            answered: Default::default(),
        };
        (Arc::new(store), senders)
    }

    fn calls(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn answered(&self) -> usize {
        self.answered.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl SecretStore for OrderedStore {
    fn get(
        &self,
        key: &apinata_core::SecretKey,
    ) -> Result<Option<String>, apinata_core::SecretError> {
        if *key != proxy_password_key() {
            return Ok(None);
        }
        let ix = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let reply = self.gates[ix]
            .lock()
            .unwrap()
            .recv_timeout(std::time::Duration::from_secs(5))
            .ok();
        self.answered
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(reply)
    }
    fn set(&self, _: &apinata_core::SecretKey, _: &str) -> Result<(), apinata_core::SecretError> {
        Ok(())
    }
    fn delete(&self, _: &apinata_core::SecretKey) -> Result<(), apinata_core::SecretError> {
        Ok(())
    }
}

// Polls the app until `done` holds, for at most three seconds.
fn wait_until(cx: &mut TestAppContext, done: impl Fn(&mut TestAppContext) -> bool) -> bool {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while std::time::Instant::now() < end {
        cx.run_until_parked();
        if done(cx) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    false
}

// The Proxy-Authorization header `proxy` saw on the one send made through it.
fn proxy_auth(cx: &mut TestAppContext, proxy: &wiremock::MockServer) -> Option<String> {
    let got = requests_to(cx, proxy);
    assert_eq!(got.len(), 1, "one send through the proxy");
    got[0]
        .headers
        .get("proxy-authorization")
        .map(|v| v.to_str().unwrap().to_owned())
}

// Two rebuilds wait on the keychain; the newer read lands first. The older read landing
// afterwards must not install its password over the newer one.
#[gpui_kit::test]
fn an_older_password_read_landing_last_is_dropped(cx: &mut TestAppContext) {
    ready(cx);
    let col = two_requests();
    let (store, answers) = OrderedStore::new(2);
    let _s = started_with_store(cx, &col, store.clone());
    let target = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let proxy = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let before = cx.update(|cx| executor::shared(cx)).unwrap();
    edit_settings(cx, |s| s.proxy = manual_proxy(&proxy, "u"));
    assert!(wait_until(cx, |_| store.calls() == 1));
    edit_settings(cx, |s| s.proxy.bypass = "example.invalid".into());
    assert!(wait_until(cx, |_| store.calls() == 2));
    answers[1].send("new".into()).unwrap();
    assert!(wait_until(cx, |cx| {
        !Arc::ptr_eq(&cx.update(|cx| executor::shared(cx)).unwrap(), &before)
    }));
    answers[0].send("old".into()).unwrap();
    assert!(wait_until(cx, |_| store.answered() == 2));
    for _ in 0..5 {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    send_now(cx, &target);
    // Basic base64 of "u:new".
    assert_eq!(proxy_auth(cx, &proxy).as_deref(), Some("Basic dTpuZXc="));
}

// The startup password read is slow; meanwhile a new password is saved and rebuilds the
// executor. The startup read landing last must not bring the old password back.
#[gpui_kit::test]
fn a_slow_startup_password_read_does_not_undo_a_newer_one(cx: &mut TestAppContext) {
    ready(cx);
    let col = two_requests();
    let (store, answers) = OrderedStore::new(2);
    let target = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let proxy = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let mut settings = test_settings();
    settings.proxy = manual_proxy(&proxy, "u");
    let _s = started_on(
        cx,
        &col,
        |paths| apinata_app::persist::save_json(&paths.settings, &settings).unwrap(),
        store.clone(),
    );
    assert!(wait_until(cx, |_| store.calls() == 1));
    let before = cx.update(|cx| executor::shared(cx)).unwrap();
    // What saving a password in Preferences does once the store has it.
    cx.update(|cx| {
        let now = cx.global::<AppState>().settings.read(cx).get().clone();
        executor::rebuild(now, store.clone(), cx);
    });
    assert!(wait_until(cx, |_| store.calls() == 2));
    answers[1].send("new".into()).unwrap();
    assert!(wait_until(cx, |cx| {
        !Arc::ptr_eq(&cx.update(|cx| executor::shared(cx)).unwrap(), &before)
    }));
    answers[0].send("old".into()).unwrap();
    assert!(wait_until(cx, |_| store.answered() == 2));
    for _ in 0..5 {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    send_now(cx, &target);
    // Basic base64 of "u:new".
    assert_eq!(proxy_auth(cx, &proxy).as_deref(), Some("Basic dTpuZXc="));
}

// While a rebuild waits on the keychain, a settings write that moves no network preference
// starts no second read.
#[gpui_kit::test]
fn an_unrelated_write_during_a_password_read_reads_once(cx: &mut TestAppContext) {
    ready(cx);
    let col = two_requests();
    let (store, answers) = OrderedStore::new(2);
    let _s = started_with_store(cx, &col, store.clone());
    let proxy = mock_server(cx, wiremock::ResponseTemplate::new(200));
    edit_settings(cx, |s| s.proxy = manual_proxy(&proxy, "u"));
    assert!(wait_until(cx, |_| store.calls() == 1));
    edit_settings(cx, |s| s.wrap_editors = !s.wrap_editors);
    assert!(
        !wait_until(cx, |_| store.calls() > 1),
        "a second read started"
    );
    answers[0].send("pw".into()).unwrap();
    assert!(wait_until(cx, |_| store.answered() == 1));
}

// A store whose first proxy password read panics and whose later reads answer from `inner`.
struct PanicOnceStore {
    inner: MemoryStore,
    calls: std::sync::atomic::AtomicUsize,
}

impl SecretStore for PanicOnceStore {
    fn get(
        &self,
        key: &apinata_core::SecretKey,
    ) -> Result<Option<String>, apinata_core::SecretError> {
        if *key == proxy_password_key()
            && self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0
        {
            panic!("the secret service fell over");
        }
        self.inner.get(key)
    }
    fn set(
        &self,
        key: &apinata_core::SecretKey,
        value: &str,
    ) -> Result<(), apinata_core::SecretError> {
        self.inner.set(key, value)
    }
    fn delete(&self, key: &apinata_core::SecretKey) -> Result<(), apinata_core::SecretError> {
        self.inner.delete(key)
    }
}

// A password read that panics must not leave its target marked as on the way: the next write
// still applies those settings, with the password the second read finds.
#[gpui_kit::test]
fn a_panicking_password_read_leaves_the_next_write_applied(cx: &mut TestAppContext) {
    ready(cx);
    let col = two_requests();
    let inner = MemoryStore::new();
    inner.set(&proxy_password_key(), "p1").unwrap();
    let store = Arc::new(PanicOnceStore {
        inner,
        calls: Default::default(),
    });
    let _s = started_with_store(cx, &col, store.clone());
    let target = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let proxy = mock_server(cx, wiremock::ResponseTemplate::new(200));
    edit_settings(cx, |s| s.proxy = manual_proxy(&proxy, "u"));
    assert!(wait_until(cx, |_| {
        store.calls.load(std::sync::atomic::Ordering::SeqCst) == 1
    }));
    for _ in 0..5 {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    edit_settings(cx, |s| s.wrap_editors = !s.wrap_editors);
    network_installed(cx);
    send_now(cx, &target);
    // Basic base64 of "u:p1".
    assert_eq!(proxy_auth(cx, &proxy).as_deref(), Some("Basic dTpwMQ=="));
}
