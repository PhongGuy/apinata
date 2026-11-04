//! Where the approvals live, how the app starts them, and watching the inbox.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::approvals::{Approvals, Decision};
use apinata_app::persist::Paths;
use apinata_app::watch::set::{WatchFactory, notify_factory};
use apinata_core::{ApprovalInbox, ScriptTrust, TrustStore};
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use std::path::Path;
use std::sync::Arc;
use support::*;
use tempfile::TempDir;

// A directory the app accepts as a collection root.
fn collection() -> TempDir {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("opencollection.yml"),
        "opencollection: 1.0.0\n",
    )
    .unwrap();
    dir
}

fn model(cx: &mut TestAppContext, state: &TempDir) -> Entity<Approvals> {
    cx.executor().allow_parking();
    let (file, dir) = (
        state.path().join("trusted-hosts.json"),
        state.path().join("approvals"),
    );
    let m = cx.new(|cx| Approvals::new(file, dir, cx));
    cx.run_until_parked();
    m
}

#[test]
fn paths_hold_the_trust_file_and_inbox() {
    let dir = Path::new("/state");
    let p = Paths::in_dir(dir);
    assert_eq!(p.trusted_hosts, dir.join("trusted-hosts.json"));
    assert_eq!(p.approvals, dir.join("approvals"));
    let platform = Paths::platform().expect("a home directory");
    assert_eq!(Some(platform.trusted_hosts), TrustStore::default_path());
    assert_eq!(Some(platform.approvals), ApprovalInbox::default_dir());
    assert_eq!(p.script_trust, dir.join("script-trust.json"));
    assert_eq!(Some(platform.script_trust), ScriptTrust::default_path());
}

#[gpui_kit::test]
fn start_builds_the_approvals_over_its_paths(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let a = cx.update(|cx| cx.global::<AppState>().approvals.clone());
    let (file, dir) = a.read_with(cx, |a, _| {
        (a.trust_file().to_path_buf(), a.inbox_dir().to_path_buf())
    });
    assert_eq!(file, s.files.path().join("trusted-hosts.json"));
    assert_eq!(dir, s.files.path().join("approvals"));
}

#[gpui_kit::test]
fn the_watcher_queues_a_request_filed_later(cx: &mut TestAppContext) {
    let (state, coll) = (TempDir::new().unwrap(), collection());
    let m = model(cx, &state);
    m.update(cx, |a, cx| a.set_watch_factory(Some(notify_factory()), cx));
    let inbox = ApprovalInbox::new(state.path().join("approvals"));
    assert!(
        inbox.dir().is_dir(),
        "the inbox is created so it can be watched"
    );
    let store = TrustStore::empty(state.path().join("trusted-hosts.json"));
    inbox
        .request(&store, coll.path(), "api.example.com", "a.yml")
        .unwrap();
    wait_for(cx, "the watcher to queue the request", |cx| {
        m.read_with(cx, |a, _| a.queue().len()) == 1
    });
}

#[gpui_kit::test]
fn a_decision_saved_elsewhere_drops_the_pair(cx: &mut TestAppContext) {
    let (state, coll) = (TempDir::new().unwrap(), collection());
    let inbox = ApprovalInbox::new(state.path().join("approvals"));
    let trust_file = state.path().join("trusted-hosts.json");
    inbox
        .request(
            &TrustStore::empty(&trust_file),
            coll.path(),
            "api.example.com",
            "a.yml",
        )
        .unwrap();
    let m = model(cx, &state);
    m.update(cx, |a, cx| a.set_watch_factory(Some(notify_factory()), cx));
    assert_eq!(m.read_with(cx, |a, _| a.queue().len()), 1);
    // As `apinata trust add` would.
    let mut store = TrustStore::empty(&trust_file);
    store.trust(coll.path(), "api.example.com").unwrap();
    store.save().unwrap();
    wait_for(cx, "the trust file change to drop the pair", |cx| {
        m.read_with(cx, |a, _| a.queue().is_empty())
    });
}

#[gpui_kit::test]
fn a_pair_set_aside_is_retried_when_the_trust_file_changes(cx: &mut TestAppContext) {
    let (state, coll) = (TempDir::new().unwrap(), collection());
    let trust_file = state.path().join("trusted-hosts.json");
    let inbox = ApprovalInbox::new(state.path().join("approvals"));
    inbox
        .request(
            &TrustStore::empty(&trust_file),
            coll.path(),
            "api.example.com",
            "a.yml",
        )
        .unwrap();
    let m = model(cx, &state);
    m.update(cx, |a, cx| a.set_watch_factory(Some(notify_factory()), cx));
    std::fs::write(&trust_file, "{ broken").unwrap();
    let p = m.read_with(cx, |a, _| a.queue()[0].clone());
    assert!(
        m.update(cx, |a, cx| a.decide(&p, Decision::Trust, cx))
            .is_err()
    );
    assert!(m.read_with(cx, |a, _| a.queue().is_empty()));
    // The user repairs the file; the pair is asked again.
    TrustStore::empty(&trust_file).save().unwrap();
    wait_for(cx, "the set-aside pair to return", |cx| {
        m.read_with(cx, |a, _| a.queue().len()) == 1
    });
}

#[gpui_kit::test]
fn a_watch_that_cannot_start_is_reported(cx: &mut TestAppContext) {
    let state = TempDir::new().unwrap();
    let m = model(cx, &state);
    let failing: WatchFactory = Arc::new(|_| Err("no watches left".to_string()));
    m.update(cx, |a, cx| a.set_watch_factory(Some(failing), cx));
    let failed = m.read_with(cx, |a, _| a.watch_failed().map(str::to_owned));
    assert_eq!(failed.as_deref(), Some("no watches left"));
}
