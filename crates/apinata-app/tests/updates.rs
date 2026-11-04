//! The update model: rate limit, toggle, skip, reports, and its place in the app.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::persist::Paths;
use apinata_app::persisted::Persisted;
use apinata_app::settings::Settings;
use apinata_app::updates::{
    CheckResult, Checker, Offer, Report, UpdateState, Updates, UpdatesEvent,
};
use gpui_kit::{AppContext as _, Entity, Task, TestAppContext};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use support::*;

const DAY: u64 = 86_400;

fn offer(version: &str) -> Offer {
    Offer {
        version: version.into(),
        notes: Some("Notes.".into()),
        download_page: "https://example.com/download".into(),
    }
}

fn fake(result: Result<CheckResult, String>) -> (Checker, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let checker: Checker = Arc::new(move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
        Task::ready(result.clone())
    });
    (checker, calls)
}

fn at(secs: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(secs)
}

// No window opens here, so gpui-kit is not initialised.
fn model(
    cx: &mut TestAppContext,
    state: UpdateState,
    checker: Option<Checker>,
) -> (tempfile::TempDir, Entity<Updates>) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("update-state.json");
    let persisted = cx.new(|_| Persisted::new(state, file));
    let m = cx.new(|_| Updates::new(persisted, checker));
    (dir, m)
}

fn state(cx: &mut TestAppContext, m: &Entity<Updates>) -> UpdateState {
    m.read_with(cx, |u, cx| u.state().read(cx).get().clone())
}

fn events(cx: &mut TestAppContext, m: &Entity<Updates>) -> Rc<RefCell<Vec<UpdatesEvent>>> {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    cx.update(|cx| {
        cx.subscribe(m, move |_, e: &UpdatesEvent, _| {
            sink.borrow_mut().push(e.clone())
        })
        .detach()
    });
    seen
}

#[test]
fn settings_default_to_checking_and_old_files_load() {
    assert!(Settings::default().check_for_updates);
    let old: Settings = serde_json::from_str(r#"{"theme":"dark"}"#).unwrap();
    assert!(old.check_for_updates);
}

#[test]
fn paths_hold_the_update_state() {
    let dir = std::path::Path::new("/state");
    assert_eq!(
        Paths::in_dir(dir).update_state,
        dir.join("update-state.json")
    );
    let platform = Paths::platform().expect("a home directory");
    assert_eq!(
        Some(platform.update_state),
        dirs::data_local_dir().map(|d| d.join("apinata/update-state.json"))
    );
}

#[gpui_kit::test]
fn a_due_check_runs_records_the_time_and_offers(cx: &mut TestAppContext) {
    let (check, calls) = fake(Ok(CheckResult::Available(offer("9.0.0"))));
    let (_d, m) = model(cx, UpdateState::default(), Some(check));
    m.update(cx, |u, cx| u.check_if_due(true, at(10 * DAY), cx));
    cx.run_until_parked();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(state(cx, &m).last_check, Some(10 * DAY));
    assert_eq!(
        m.read_with(cx, |u, _| u.offer().cloned()),
        Some(offer("9.0.0"))
    );
    assert_eq!(
        m.read_with(cx, |u, _| u.last_report().cloned()),
        None,
        "automatic checks report nothing"
    );
}

#[gpui_kit::test]
fn a_check_within_a_day_is_not_repeated(cx: &mut TestAppContext) {
    let (check, calls) = fake(Ok(CheckResult::UpToDate));
    let last = UpdateState {
        last_check: Some(10 * DAY),
        ..Default::default()
    };
    let (_d, m) = model(cx, last, Some(check));
    m.update(cx, |u, cx| u.check_if_due(true, at(11 * DAY - 1), cx));
    cx.run_until_parked();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    m.update(cx, |u, cx| u.check_if_due(true, at(11 * DAY + 1), cx));
    cx.run_until_parked();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[gpui_kit::test]
fn the_setting_off_stops_automatic_checks(cx: &mut TestAppContext) {
    let (check, calls) = fake(Ok(CheckResult::UpToDate));
    let (_d, m) = model(cx, UpdateState::default(), Some(check));
    m.update(cx, |u, cx| u.check_if_due(false, at(10 * DAY), cx));
    cx.run_until_parked();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(state(cx, &m).last_check, None);
}

#[gpui_kit::test]
fn a_skipped_version_is_not_offered_automatically(cx: &mut TestAppContext) {
    let (check, _) = fake(Ok(CheckResult::Available(offer("9.0.0"))));
    let skipped = UpdateState {
        skipped_version: Some("9.0.0".into()),
        ..Default::default()
    };
    let (_d, m) = model(cx, skipped.clone(), Some(check));
    m.update(cx, |u, cx| u.check_if_due(true, at(10 * DAY), cx));
    cx.run_until_parked();
    assert_eq!(m.read_with(cx, |u, _| u.offer().cloned()), None);

    let (newer, _) = fake(Ok(CheckResult::Available(offer("9.1.0"))));
    let (_d, m) = model(cx, skipped, Some(newer));
    m.update(cx, |u, cx| u.check_if_due(true, at(10 * DAY), cx));
    cx.run_until_parked();
    assert_eq!(
        m.read_with(cx, |u, _| u.offer().map(|o| o.version.clone())),
        Some("9.1.0".into())
    );
}

#[gpui_kit::test]
fn automatic_errors_are_silent(cx: &mut TestAppContext) {
    let (check, _) = fake(Err("could not reach the update server".into()));
    let (_d, m) = model(cx, UpdateState::default(), Some(check));
    let seen = events(cx, &m);
    m.update(cx, |u, cx| u.check_if_due(true, at(10 * DAY), cx));
    cx.run_until_parked();
    assert!(seen.borrow().is_empty());
    assert_eq!(
        m.read_with(cx, |u, _| (u.offer().cloned(), u.last_report().cloned())),
        (None, None)
    );
}

#[gpui_kit::test]
fn a_manual_check_ignores_the_skip_and_reports(cx: &mut TestAppContext) {
    let (check, _) = fake(Ok(CheckResult::Available(offer("9.0.0"))));
    let skipped = UpdateState {
        skipped_version: Some("9.0.0".into()),
        ..Default::default()
    };
    let (_d, m) = model(cx, skipped, Some(check));
    let seen = events(cx, &m);
    m.update(cx, |u, cx| u.check_now(cx));
    cx.run_until_parked();
    assert_eq!(
        m.read_with(cx, |u, _| u.offer().cloned()),
        Some(offer("9.0.0"))
    );
    assert_eq!(
        *seen.borrow(),
        [UpdatesEvent::Reported(Report::Available("9.0.0".into()))]
    );
    assert!(state(cx, &m).last_check.is_some());
}

#[gpui_kit::test]
fn manual_results_are_reported(cx: &mut TestAppContext) {
    for (result, report) in [
        (Ok(CheckResult::UpToDate), Report::UpToDate),
        (
            Err("the update server answered HTTP 500".to_string()),
            Report::Failed("the update server answered HTTP 500".into()),
        ),
    ] {
        let (check, _) = fake(result);
        let (_d, m) = model(cx, UpdateState::default(), Some(check));
        m.update(cx, |u, cx| u.check_now(cx));
        cx.run_until_parked();
        assert_eq!(
            m.read_with(cx, |u, _| u.last_report().cloned()),
            Some(report)
        );
    }
}

#[gpui_kit::test]
fn without_a_checker_updates_are_disabled(cx: &mut TestAppContext) {
    let (_d, m) = model(cx, UpdateState::default(), None);
    assert!(!m.read_with(cx, |u, _| u.enabled()));
    m.update(cx, |u, cx| u.check_if_due(true, at(10 * DAY), cx));
    m.update(cx, |u, cx| u.check_now(cx));
    cx.run_until_parked();
    assert_eq!(
        m.read_with(cx, |u, _| u.last_report().cloned()),
        Some(Report::Disabled)
    );
    assert_eq!(state(cx, &m).last_check, None);
}

#[gpui_kit::test]
fn later_hides_and_skip_records(cx: &mut TestAppContext) {
    let (check, _) = fake(Ok(CheckResult::Available(offer("9.0.0"))));
    let (_d, m) = model(cx, UpdateState::default(), Some(check));
    m.update(cx, |u, cx| u.check_now(cx));
    cx.run_until_parked();
    m.update(cx, |u, cx| u.later(cx));
    assert_eq!(m.read_with(cx, |u, _| u.offer().cloned()), None);
    assert_eq!(state(cx, &m).skipped_version, None);
    m.update(cx, |u, cx| u.check_now(cx));
    cx.run_until_parked();
    m.update(cx, |u, cx| u.skip(cx));
    assert_eq!(m.read_with(cx, |u, _| u.offer().cloned()), None);
    assert_eq!(state(cx, &m).skipped_version.as_deref(), Some("9.0.0"));
}

#[gpui_kit::test]
fn start_holds_the_updates_over_their_file(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let updates = cx.update(|cx| cx.global::<AppState>().updates.clone());
    let path = updates.read_with(cx, |u, cx| u.state().read(cx).path().to_path_buf());
    assert_eq!(path, s.files.path().join("update-state.json"));
    assert!(
        !updates.read_with(cx, |u, _| u.enabled()),
        "start installs no checker; main.rs does"
    );
    assert_eq!(
        updates.read_with(cx, |u, cx| u.state().read(cx).get().last_check),
        None,
        "start never checks"
    );
}
