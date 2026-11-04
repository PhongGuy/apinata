//! The notice's download-and-restart flow through the normal close prompt.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::update_install;
use apinata_app::updates::{CheckResult, Checker, Offer, Prepared, Stager, Updates};
use apinata_core::update::Relaunch;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, Task, TestAppContext, VisualTestContext};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use support::*;

const IMAGE: &str = "/opt/apps/Apinata.AppImage";

fn updates(cx: &mut TestAppContext) -> Entity<Updates> {
    cx.update(|cx| cx.global::<AppState>().updates.clone())
}

fn windows(cx: &mut TestAppContext) -> usize {
    cx.update(|cx| cx.windows().len())
}

fn offered(cx: &mut TestAppContext) -> (tempfile::TempDir, Started) {
    let col = two_requests();
    let s = started(cx, &col);
    let offer = Offer {
        version: "9.0.0".into(),
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
    (col, s)
}

fn use_stager(cx: &mut TestAppContext, result: Result<(), String>) -> Arc<AtomicUsize> {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let stager: Stager = Arc::new(move |_, version| {
        counter.fetch_add(1, Ordering::SeqCst);
        let relaunch = Relaunch::Restart(PathBuf::from(IMAGE));
        Task::ready(
            result
                .clone()
                .map(|()| Prepared::relaunch_only(version, relaunch)),
        )
    });
    updates(cx).update(cx, |u, cx| u.set_stager(Some(stager), cx));
    calls
}

fn click(cx: &mut TestAppContext, s: &Started, id: &'static str) {
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.click(id, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn dirty_a(cx: &mut TestAppContext, s: &Started) {
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url("a.yml", "http://x/9"), cx)
    });
    cx.run_until_parked();
}

fn has_prepared(cx: &mut TestAppContext) -> bool {
    updates(cx).read_with(cx, |u, _| u.prepared().is_some())
}

#[gpui_kit::test]
fn the_install_button_needs_a_stager(cx: &mut TestAppContext) {
    let (_col, s) = offered(cx);
    assert!(present(cx, s.win, "update-notice"));
    assert!(!present(cx, s.win, "update-install"));
    use_stager(cx, Ok(()));
    assert!(present(cx, s.win, "update-install"));
}

#[gpui_kit::test]
fn a_clean_window_downloads_closes_and_restarts_into_the_update(cx: &mut TestAppContext) {
    let (_col, s) = offered(cx);
    use_stager(cx, Ok(()));
    click(cx, &s, "update-install");
    assert!(!confirm_open(cx));
    assert_eq!(windows(cx), 0);
    assert!(has_prepared(cx));
    let mut restart = cx.expect_restart();
    cx.update(update_install::quit);
    let got = restart.try_recv().unwrap();
    assert_eq!(got, Some((Some(PathBuf::from(IMAGE)), vec![])));
    assert!(!has_prepared(cx));
}

fn armed(cx: &mut TestAppContext) -> bool {
    updates(cx).read_with(cx, |u, _| u.armed())
}

fn close_window(cx: &mut TestAppContext, s: &Started) -> bool {
    VisualTestContext::from_window(s.win, cx).simulate_close()
}

// The title bar's close button, which runs the close flow without the platform close.
fn title_bar_close(cx: &mut TestAppContext, s: &Started) {
    let shell = s.shell.clone();
    cx.update_window(s.win, |_, w, cx| {
        shell.update(cx, |v, cx| v.request_window_close(w, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn cancelling_the_quit_keeps_the_prepared_release(cx: &mut TestAppContext) {
    let (_col, s) = offered(cx);
    let calls = use_stager(cx, Ok(()));
    dirty_a(cx, &s);
    click(cx, &s, "update-install");
    assert!(confirm_open(cx));
    answer_confirm(cx, "confirm-cancel");
    cx.run_until_parked();
    assert_eq!(windows(cx), 1);
    assert!(has_prepared(cx));
    assert!(!armed(cx), "a cancelled quit disarms the install");
    click(cx, &s, "update-install");
    assert!(confirm_open(cx), "the close flow runs again");
    assert_eq!(calls.load(Ordering::SeqCst), 1, "no second download");
}

#[gpui_kit::test]
fn an_ordinary_close_after_a_cancel_does_not_install(cx: &mut TestAppContext) {
    let (_col, s) = offered(cx);
    use_stager(cx, Ok(()));
    dirty_a(cx, &s);
    click(cx, &s, "update-install");
    answer_confirm(cx, "confirm-cancel");
    cx.run_until_parked();
    assert!(!close_window(cx, &s));
    answer_confirm(cx, "confirm-discard");
    cx.run_until_parked();
    assert_eq!(windows(cx), 0);
    let mut restart = cx.expect_restart();
    cx.update(update_install::quit);
    assert_eq!(restart.try_recv().unwrap(), None);
    assert!(has_prepared(cx), "kept for the next restart");
}

#[gpui_kit::test]
fn a_title_bar_close_after_a_cancel_does_not_install(cx: &mut TestAppContext) {
    let (_col, s) = offered(cx);
    use_stager(cx, Ok(()));
    dirty_a(cx, &s);
    click(cx, &s, "update-install");
    answer_confirm(cx, "confirm-cancel");
    cx.run_until_parked();
    title_bar_close(cx, &s);
    assert!(confirm_open(cx));
    answer_confirm(cx, "confirm-discard");
    cx.run_until_parked();
    assert_eq!(windows(cx), 0);
    let mut restart = cx.expect_restart();
    cx.update(update_install::quit);
    assert_eq!(restart.try_recv().unwrap(), None);
    assert!(has_prepared(cx), "kept for the next restart");
}

#[gpui_kit::test]
fn discarding_at_the_prompt_restarts_into_the_update(cx: &mut TestAppContext) {
    let (_col, s) = offered(cx);
    use_stager(cx, Ok(()));
    dirty_a(cx, &s);
    click(cx, &s, "update-install");
    answer_confirm(cx, "confirm-discard");
    cx.run_until_parked();
    assert_eq!(windows(cx), 0);
    assert!(armed(cx));
    let mut restart = cx.expect_restart();
    cx.update(update_install::quit);
    let got = restart.try_recv().unwrap();
    assert_eq!(got, Some((Some(PathBuf::from(IMAGE)), vec![])));
}

#[gpui_kit::test]
fn a_failed_download_keeps_the_window_and_says_why(cx: &mut TestAppContext) {
    let (_col, s) = offered(cx);
    use_stager(cx, Err("the download does not match its checksum".into()));
    click(cx, &s, "update-install");
    assert!(!confirm_open(cx));
    assert_eq!(windows(cx), 1);
    assert!(present(cx, s.win, "update-install-error"));
    assert!(
        present(cx, s.win, "update-install"),
        "the button stays for a retry"
    );
}

#[gpui_kit::test]
fn quitting_with_nothing_prepared_does_not_restart(cx: &mut TestAppContext) {
    let (_col, _s) = offered(cx);
    let mut restart = cx.expect_restart();
    cx.update(update_install::quit);
    assert_eq!(restart.try_recv().unwrap(), None);
}

#[gpui_kit::test]
fn a_failed_save_all_at_the_prompt_disarms_the_install(cx: &mut TestAppContext) {
    let (_col, s) = offered(cx);
    use_stager(cx, Ok(()));
    dirty_a(cx, &s);
    // Changed on disk under the draft and not yet reloaded, so the save conflicts.
    std::fs::write(s.root.join("a.yml"), request("a", "http://x/disk")).unwrap();
    click(cx, &s, "update-install");
    answer_confirm(cx, "confirm-save");
    cx.run_until_parked();
    assert_eq!(windows(cx), 1);
    assert!(has_prepared(cx));
    assert!(
        !armed(cx),
        "a save that keeps the window open disarms the install"
    );
}
