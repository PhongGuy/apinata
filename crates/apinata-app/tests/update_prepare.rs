//! Preparing an offered release: the stager, reuse, failure and hand-over.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::updates::{CheckResult, Checker, InstallPhase, Offer, Prepared, Stager, Updates};
use apinata_core::update::Relaunch;
use gpui_kit::{AppContext as _, Entity, Task, TestAppContext};
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use support::*;

fn updates(cx: &mut TestAppContext) -> Entity<Updates> {
    cx.update(|cx| cx.global::<AppState>().updates.clone())
}

fn relaunch() -> Relaunch {
    Relaunch::Restart(PathBuf::from("/opt/apps/Apinata.AppImage"))
}

fn offer_version(cx: &mut TestAppContext, version: &str) {
    let offer = Offer {
        version: version.into(),
        notes: None,
        download_page: "https://example.com/download".into(),
    };
    let checker: Checker =
        Arc::new(move |_| Task::ready(Ok(CheckResult::Available(offer.clone()))));
    updates(cx).update(cx, |u, cx| {
        u.set_checker(Some(checker), cx);
        u.check_now(cx);
    });
    cx.run_until_parked();
}

// The collection directory is returned so it outlives the window.
fn offered(cx: &mut TestAppContext) -> (tempfile::TempDir, Started) {
    let col = two_requests();
    let s = started(cx, &col);
    offer_version(cx, "9.0.0");
    (col, s)
}

fn use_stager(cx: &mut TestAppContext, result: Result<(), String>) -> Arc<AtomicUsize> {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let stager: Stager = Arc::new(move |_, version| {
        counter.fetch_add(1, Ordering::SeqCst);
        let r = result
            .clone()
            .map(|()| Prepared::relaunch_only(version, relaunch()));
        Task::ready(r)
    });
    updates(cx).update(cx, |u, cx| u.set_stager(Some(stager), cx));
    calls
}

fn prepare(cx: &mut TestAppContext) -> Option<bool> {
    let seen = Rc::new(Cell::new(None));
    let sink = seen.clone();
    updates(cx).update(cx, |u, cx| u.prepare(move |ok, _| sink.set(Some(ok)), cx));
    cx.run_until_parked();
    seen.get()
}

fn phase(cx: &mut TestAppContext) -> InstallPhase {
    updates(cx).read_with(cx, |u, _| u.install_phase().clone())
}

#[gpui_kit::test]
fn without_a_stager_nothing_can_be_installed(cx: &mut TestAppContext) {
    let (_col, _s) = offered(cx);
    assert!(!updates(cx).read_with(cx, |u, _| u.can_install()));
    assert_eq!(prepare(cx), Some(false));
    assert_eq!(phase(cx), InstallPhase::Idle);
}

#[gpui_kit::test]
fn a_prepared_release_is_ready_and_reported(cx: &mut TestAppContext) {
    let (_col, _s) = offered(cx);
    let calls = use_stager(cx, Ok(()));
    assert!(updates(cx).read_with(cx, |u, _| u.can_install()));
    assert_eq!(prepare(cx), Some(true));
    assert_eq!(phase(cx), InstallPhase::Ready);
    let version = updates(cx).read_with(cx, |u, _| u.prepared().map(|p| p.version().to_owned()));
    assert_eq!(version.as_deref(), Some("9.0.0"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[gpui_kit::test]
fn preparing_again_reuses_the_prepared_release(cx: &mut TestAppContext) {
    let (_col, _s) = offered(cx);
    let calls = use_stager(cx, Ok(()));
    assert_eq!(prepare(cx), Some(true));
    assert_eq!(prepare(cx), Some(true));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[gpui_kit::test]
fn a_failed_download_is_reported_and_can_be_retried(cx: &mut TestAppContext) {
    let (_col, _s) = offered(cx);
    let message = "the download does not match its checksum".to_owned();
    use_stager(cx, Err(message.clone()));
    assert_eq!(prepare(cx), Some(false));
    assert_eq!(phase(cx), InstallPhase::Failed(message));
    assert!(updates(cx).read_with(cx, |u, _| u.prepared().is_none()));
    use_stager(cx, Ok(()));
    assert_eq!(prepare(cx), Some(true));
}

#[gpui_kit::test]
fn a_newer_offer_replaces_the_prepared_release(cx: &mut TestAppContext) {
    let (_col, _s) = offered(cx);
    let calls = use_stager(cx, Ok(()));
    assert_eq!(prepare(cx), Some(true));
    offer_version(cx, "9.1.0");
    assert_eq!(prepare(cx), Some(true));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let version = updates(cx).read_with(cx, |u, _| u.prepared().map(|p| p.version().to_owned()));
    assert_eq!(version.as_deref(), Some("9.1.0"));
}

#[gpui_kit::test]
fn only_an_armed_release_is_handed_over_and_only_once(cx: &mut TestAppContext) {
    let (_col, _s) = offered(cx);
    use_stager(cx, Ok(()));
    prepare(cx);
    assert!(updates(cx).update(cx, |u, _| u.take_armed()).is_none());
    assert!(updates(cx).read_with(cx, |u, _| u.prepared().is_some()));
    updates(cx).update(cx, |u, _| u.set_armed(true));
    let taken = updates(cx).update(cx, |u, _| u.take_armed());
    assert_eq!(taken.unwrap().install(), Ok(relaunch()));
    assert!(!updates(cx).read_with(cx, |u, _| u.armed()));
    assert!(updates(cx).update(cx, |u, _| u.take_armed()).is_none());
}

// A stager whose download finishes only when the returned sender sends.
fn pending_stager(cx: &mut TestAppContext) -> async_channel::Sender<()> {
    let (tx, rx) = async_channel::bounded::<()>(1);
    let stager: Stager = Arc::new(move |cx, version| {
        let rx = rx.clone();
        cx.background_spawn(async move {
            let _ = rx.recv().await;
            Ok(Prepared::relaunch_only(version, relaunch()))
        })
    });
    updates(cx).update(cx, |u, cx| u.set_stager(Some(stager), cx));
    tx
}

#[gpui_kit::test]
fn a_download_overtaken_by_a_newer_offer_is_not_kept(cx: &mut TestAppContext) {
    let (_col, _s) = offered(cx);
    let finish = pending_stager(cx);
    let seen = Rc::new(Cell::new(None));
    let sink = seen.clone();
    updates(cx).update(cx, |u, cx| u.prepare(move |ok, _| sink.set(Some(ok)), cx));
    cx.run_until_parked();
    assert_eq!(phase(cx), InstallPhase::Downloading);
    offer_version(cx, "9.1.0");
    finish.try_send(()).unwrap();
    cx.run_until_parked();
    assert_eq!(seen.get(), Some(false));
    assert!(matches!(phase(cx), InstallPhase::Failed(_)));
    assert!(updates(cx).read_with(cx, |u, _| u.prepared().is_none()));
}

#[gpui_kit::test]
fn a_release_for_another_version_is_not_kept(cx: &mut TestAppContext) {
    let (_col, _s) = offered(cx);
    let stager: Stager =
        Arc::new(|_, _| Task::ready(Ok(Prepared::relaunch_only("8.0.0", relaunch()))));
    updates(cx).update(cx, |u, cx| u.set_stager(Some(stager), cx));
    assert_eq!(prepare(cx), Some(false));
    assert!(matches!(phase(cx), InstallPhase::Failed(_)));
    assert!(updates(cx).read_with(cx, |u, _| u.prepared().is_none()));
}

#[gpui_kit::test]
fn arming_before_anything_is_prepared_does_not_stick(cx: &mut TestAppContext) {
    let (_col, _s) = offered(cx);
    use_stager(cx, Ok(()));
    updates(cx).update(cx, |u, _| u.set_armed(true));
    prepare(cx);
    assert!(!updates(cx).read_with(cx, |u, _| u.armed()));
}

#[gpui_kit::test]
fn a_newer_offer_disarms_the_prepared_release(cx: &mut TestAppContext) {
    let (_col, _s) = offered(cx);
    use_stager(cx, Ok(()));
    prepare(cx);
    updates(cx).update(cx, |u, _| u.set_armed(true));
    offer_version(cx, "9.1.0");
    assert!(!updates(cx).read_with(cx, |u, _| u.armed()));
}
