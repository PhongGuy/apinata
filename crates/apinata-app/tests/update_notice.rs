//! The update notice, the manual check and the preferences controls.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::updates::{CheckForUpdates, CheckResult, Checker, Offer, Report, Updates};
use gpui_kit::component::WindowExt;
use gpui_kit::{AppContext as _, Entity, Task, TestAppContext};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use support::*;

const PAGE: &str = "https://example.com/download";

fn updates(cx: &mut TestAppContext) -> Entity<Updates> {
    cx.update(|cx| cx.global::<AppState>().updates.clone())
}

fn use_fake(cx: &mut TestAppContext, result: Result<CheckResult, String>) -> Arc<AtomicUsize> {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let checker: Checker = Arc::new(move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
        Task::ready(result.clone())
    });
    updates(cx).update(cx, |u, cx| u.set_checker(Some(checker), cx));
    calls
}

fn available(notes: Option<&str>) -> Result<CheckResult, String> {
    Ok(CheckResult::Available(Offer {
        version: "9.0.0".into(),
        notes: notes.map(Into::into),
        download_page: PAGE.into(),
    }))
}

// Scrolls the control into view first: the Network rows push later sections below the fold.
fn click(cx: &mut TestAppContext, s: &Started, id: &'static str) {
    prefs_click(cx, s, id);
    cx.run_until_parked();
}

fn notification_count(cx: &mut TestAppContext, s: &Started) -> usize {
    cx.update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

// The collection directory is returned so it outlives the window.
fn offered(cx: &mut TestAppContext, notes: Option<&str>) -> (tempfile::TempDir, Started) {
    let col = two_requests();
    let s = started(cx, &col);
    use_fake(cx, available(notes));
    assert!(!present(cx, s.win, "update-notice"));
    updates(cx).update(cx, |u, cx| u.check_now(cx));
    cx.run_until_parked();
    (col, s)
}

#[gpui_kit::test]
fn an_available_update_shows_a_non_modal_notice(cx: &mut TestAppContext) {
    let (_col, s) = offered(cx, Some("Fixed things."));
    assert!(present(cx, s.win, "update-notice"));
    assert!(
        !cx.update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
            .unwrap()
    );
    assert!(!present(cx, s.win, "update-notes-text"));
    click(cx, &s, "update-notes");
    assert!(present(cx, s.win, "update-notes-text"));
    click(cx, &s, "update-download");
    assert_eq!(cx.opened_url().as_deref(), Some(PAGE));
}

#[gpui_kit::test]
fn notes_are_offered_only_when_present(cx: &mut TestAppContext) {
    let (_col, s) = offered(cx, None);
    assert!(present(cx, s.win, "update-notice"));
    assert!(!present(cx, s.win, "update-notes"));
}

#[gpui_kit::test]
fn later_hides_the_notice_without_skipping(cx: &mut TestAppContext) {
    let (_col, s) = offered(cx, Some("n"));
    click(cx, &s, "update-later");
    assert!(!present(cx, s.win, "update-notice"));
    let skipped =
        updates(cx).read_with(cx, |u, cx| u.state().read(cx).get().skipped_version.clone());
    assert_eq!(skipped, None);
}

#[gpui_kit::test]
fn skip_hides_the_notice_and_records_the_version(cx: &mut TestAppContext) {
    let (_col, s) = offered(cx, Some("n"));
    click(cx, &s, "update-skip");
    assert!(!present(cx, s.win, "update-notice"));
    let skipped =
        updates(cx).read_with(cx, |u, cx| u.state().read(cx).get().skipped_version.clone());
    assert_eq!(skipped.as_deref(), Some("9.0.0"));
}

#[gpui_kit::test]
fn the_action_checks_and_reports_in_a_notification(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let calls = use_fake(cx, Ok(CheckResult::UpToDate));
    let before = notification_count(cx, &s);
    cx.dispatch_action(s.win, CheckForUpdates);
    cx.run_until_parked();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(notification_count(cx, &s), before + 1);
    assert_eq!(
        updates(cx).read_with(cx, |u, _| u.last_report().cloned()),
        Some(Report::UpToDate)
    );
}

#[gpui_kit::test]
fn an_unconfigured_build_says_so(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    updates(cx).update(cx, |u, cx| u.set_checker(None, cx));
    let before = notification_count(cx, &s);
    cx.dispatch_action(s.win, CheckForUpdates);
    cx.run_until_parked();
    assert_eq!(notification_count(cx, &s), before + 1);
    assert_eq!(
        updates(cx).read_with(cx, |u, _| u.last_report().cloned()),
        Some(Report::Disabled)
    );
    cx.update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap();
    frame(cx, s.win);
    assert!(present(cx, s.win, "updates-disabled"));
}

#[gpui_kit::test]
fn preferences_toggle_the_setting_and_check_now(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let calls = use_fake(cx, Ok(CheckResult::UpToDate));
    cx.update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap();
    frame(cx, s.win);
    assert!(!present(cx, s.win, "updates-disabled"));
    click(cx, &s, "check-updates");
    let on = cx.update(|cx| {
        cx.global::<AppState>()
            .settings
            .read(cx)
            .get()
            .check_for_updates
    });
    assert!(!on);
    click(cx, &s, "check-updates-now");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[gpui_kit::test]
fn check_now_is_disabled_when_checks_are_not_configured(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    updates(cx).update(cx, |u, cx| u.set_checker(None, cx));
    cx.run_until_parked();
    cx.update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap();
    frame(cx, s.win);
    assert!(present(cx, s.win, "updates-disabled"));
    let report = updates(cx).read_with(cx, |u, _| u.last_report().cloned());
    let before = notification_count(cx, &s);
    click(cx, &s, "check-updates-now");
    assert_eq!(notification_count(cx, &s), before);
    assert_eq!(
        updates(cx).read_with(cx, |u, _| u.last_report().cloned()),
        report
    );
}

#[gpui_kit::test]
fn check_now_reports_inside_preferences(cx: &mut TestAppContext) {
    use apinata_app::updates::report_text;
    let col = two_requests();
    let s = started(cx, &col);
    use_fake(cx, Ok(CheckResult::UpToDate));
    let prefs = cx
        .update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap()
        .expect("Preferences");
    frame(cx, s.win);
    click(cx, &s, "check-updates-now");
    let status = prefs
        .read_with(cx, |p, _| p.status())
        .map(|t| t.to_string());
    assert_eq!(status, Some(report_text(&Report::UpToDate)));
    assert!(report_text(&Report::UpToDate).ends_with("is up to date."));
    assert!(present(cx, s.win, "prefs-status"));
}
