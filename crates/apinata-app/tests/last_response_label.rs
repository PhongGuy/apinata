use crate::support;

use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use apinata_app::last_response::{saved_date, saved_label, saved_tooltip, short_age};
use apinata_app::response_pane::ResponsePane;
use apinata_app::send_model::{Restored, SendModel};
use apinata_core::{Outcome, RequestResult, Response};
use chrono::{DateTime, FixedOffset, TimeZone};
use gpui_kit::{AppContext as _, TestAppContext, WindowOptions};
use support::*;

fn t(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime<FixedOffset> {
    FixedOffset::east_opt(2 * 3600)
        .unwrap()
        .with_ymd_and_hms(y, mo, d, h, mi, s)
        .unwrap()
}

#[test]
fn the_label_reads_date_time_and_age() {
    let saved = t(2026, 10, 10, 20, 41, 7);
    assert_eq!(
        saved_label(saved, t(2026, 10, 10, 20, 41, 30)),
        "10 Oct, 20:41 (just now)"
    );
    assert_eq!(
        saved_label(saved, t(2026, 10, 10, 20, 42, 7)),
        "10 Oct, 20:41 (1 minute ago)"
    );
    assert_eq!(
        saved_label(saved, t(2026, 10, 10, 22, 50, 0)),
        "10 Oct, 20:41 (2 hours ago)"
    );
    assert_eq!(
        saved_label(saved, t(2026, 10, 11, 20, 41, 7)),
        "10 Oct, 20:41 (1 day ago)"
    );
    assert_eq!(
        saved_label(saved, t(2026, 12, 20, 0, 0, 0)),
        "10 Oct, 20:41 (2 months ago)"
    );
    assert_eq!(
        saved_label(saved, t(2028, 1, 1, 0, 0, 0)),
        "10 Oct 2026, 20:41 (1 year ago)"
    );
    assert_eq!(
        saved_label(saved, t(2026, 10, 10, 20, 0, 0)),
        "10 Oct, 20:41 (just now)"
    );
    assert_eq!(
        saved_label(t(2026, 3, 5, 9, 5, 0), t(2026, 3, 5, 9, 6, 0)),
        "5 Mar, 09:05 (1 minute ago)"
    );
}

// The status row's date never gains a year as the calendar turns, so it never reflows;
// the tooltip's long label carries the year.
#[test]
fn the_status_row_date_has_no_year() {
    let saved = t(2026, 10, 10, 20, 41, 7);
    assert_eq!(saved_date(saved), "10 Oct, 20:41");
    assert_eq!(
        saved_label(saved, t(2028, 1, 1, 0, 0, 0)),
        "10 Oct 2026, 20:41 (1 year ago)"
    );
}

// The status row's age counts in the same units as the long one, with short names.
#[test]
fn the_short_age_counts_in_short_units() {
    let saved = UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    let at = |secs: u64| short_age(saved, saved + Duration::from_secs(secs));
    assert_eq!(at(59), "just now");
    assert_eq!(at(60), "1m ago");
    assert_eq!(at(3_599), "59m ago");
    assert_eq!(at(3_600), "1h ago");
    assert_eq!(at(86_399), "23h ago");
    assert_eq!(at(86_400 * 29), "29d ago");
    assert_eq!(at(86_400 * 30), "1mo ago");
    assert_eq!(at(86_400 * 364), "12mo ago");
    assert_eq!(at(86_400 * 365 * 5), "5y ago");
    assert_eq!(short_age(saved + Duration::from_secs(9), saved), "just now");
}

#[test]
fn the_tooltip_is_the_exact_time() {
    assert_eq!(
        saved_tooltip(t(2026, 10, 10, 20, 41, 7)),
        "2026-10-10 20:41:07 +02:00"
    );
}

fn shown(
    cx: &mut TestAppContext,
    body_dropped: bool,
) -> (
    gpui_kit::AnyWindowHandle,
    gpui_kit::Entity<ResponsePane>,
    gpui_kit::Entity<SendModel>,
) {
    let col = two_requests();
    let (_state, ws, _root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    let response = Response::new(200).with_text_body(if body_dropped { "" } else { "kept" });
    let result = RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(response),
        Vec::new(),
    );
    let restored = Restored {
        saved_at: UNIX_EPOCH + Duration::from_secs(1_760_000_000),
        body_dropped,
        body_masked: false,
        masked_headers: Vec::new(),
        header_digests: None,
    };
    model.update(cx, |m, cx| m.restore(Arc::new(result), restored, cx));
    support::init(cx);
    let m = model.clone();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(m, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    (win, view, model)
}

#[gpui_kit::test]
fn a_restored_response_carries_the_label_until_cleared(cx: &mut TestAppContext) {
    let (win, view, model) = shown(cx, false);
    assert!(present(cx, win, "response-saved"));
    let saved = view
        .read_with(cx, |p, cx| p.summary(cx))
        .unwrap()
        .saved
        .unwrap();
    assert!(saved.starts_with("From "), "{saved}");
    assert!(saved.ends_with(" ago)"), "{saved}");
    assert!(!present(cx, win, "response-body-dropped"));
    let fresh = RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(Response::new(200)),
        Vec::new(),
    );
    model.update(cx, |m, cx| m.show(Arc::new(fresh), cx));
    assert!(!present(cx, win, "response-saved"));
    assert_eq!(
        view.read_with(cx, |p, cx| p.summary(cx)).unwrap().saved,
        None
    );
}

#[gpui_kit::test]
fn a_dropped_body_says_so(cx: &mut TestAppContext) {
    let (win, _view, _model) = shown(cx, true);
    assert!(present(cx, win, "response-body-dropped"));
    assert!(present(cx, win, "response-saved"));
}

#[gpui_kit::test]
fn clearing_forgets_a_shown_restored_response(cx: &mut TestAppContext) {
    let (win, _view, model) = shown(cx, false);
    assert!(present(cx, win, "response-saved"));
    cx.update(apinata_app::send_model::forget_all_restored);
    cx.run_until_parked();
    assert!(!present(cx, win, "response-saved"));
    assert!(model.read_with(cx, |m, _| m.restored().is_none()));
    assert!(model.read_with(cx, |m, _| matches!(
        m.state(),
        apinata_app::send_model::SendState::Idle
    )));
}

#[gpui_kit::test]
fn clearing_leaves_a_fresh_response_alone(cx: &mut TestAppContext) {
    let (win, _view, model) = shown(cx, false);
    let fresh = RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(Response::new(200)),
        Vec::new(),
    );
    model.update(cx, |m, cx| m.show(Arc::new(fresh), cx));
    cx.update(apinata_app::send_model::forget_all_restored);
    cx.run_until_parked();
    assert!(model.read_with(cx, |m, _| matches!(
        m.state(),
        apinata_app::send_model::SendState::Done(_)
    )));
    assert!(!present(cx, win, "response-saved"));
}

// A remembered response has no hops, so its Timeline says why and how to get one back;
// a freshly sent response shows its hops and no note.
#[gpui_kit::test]
fn a_restored_timeline_says_it_is_not_saved(cx: &mut TestAppContext) {
    use apinata_app::response_pane::ResponseTab;
    use gpui_kit::test::TestWindowExt as _;
    let (win, view, model) = shown(cx, false);
    view.update(cx, |p, cx| p.set_tab(ResponseTab::Timeline, cx));
    let note = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find("response-timeline-not-saved")
                .and_then(|e| e.label().map(str::to_owned))
        })
        .unwrap();
    assert_eq!(
        note.as_deref(),
        Some("The timeline is not saved. Send the request again to see it.")
    );
    let fresh = RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(Response::new(200)),
        Vec::new(),
    );
    model.update(cx, |m, cx| m.show(Arc::new(fresh), cx));
    assert!(!present(cx, win, "response-timeline-not-saved"));
}
