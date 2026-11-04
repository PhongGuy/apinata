//! Copy takes exactly the text the body shows: pretty or raw, filtered, long or short.

use crate::support;

use std::sync::Arc;

use apinata_app::body_text::reindent_json;
use apinata_app::body_view::{BodyMode, BodyView, Shown};
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, WindowOptions};
use support::*;

const JSON: &str = r#"{"data":[{"id":7},{"id":8}]}"#;
const PRETTY: &str =
    "{\n  \"data\": [\n    {\n      \"id\": 7\n    },\n    {\n      \"id\": 8\n    }\n  ]\n}";

fn shown(cx: &mut TestAppContext, body: &str, ct: &str) -> (AnyWindowHandle, Entity<BodyView>) {
    support::init(cx);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| BodyView::new(w, cx))
            })
        })
        .unwrap();
    let response = Response::new(200)
        .with_header("content-type", ct)
        .with_text_body(body);
    let r = Arc::new(RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(response),
        Vec::new(),
    ));
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(r), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    (win, view)
}

fn copied(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    view: &Entity<BodyView>,
) -> Option<String> {
    cx.update_window(win, |_, w, cx| view.update(cx, |v, cx| v.copy(w, cx)))
        .unwrap();
    cx.run_until_parked();
    cx.read_from_clipboard().and_then(|c| c.text())
}

fn set_mode(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    view: &Entity<BodyView>,
    mode: BodyMode,
) {
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_mode(mode, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

fn long_json() -> String {
    let items: Vec<String> = (0..30_000).map(|i| format!("{{\"id\":{i}}}")).collect();
    format!("[{}]", items.join(","))
}

#[gpui_kit::test]
fn pretty_and_raw_copy_as_shown(cx: &mut TestAppContext) {
    let (win, view) = shown(cx, JSON, "application/json");
    assert_eq!(copied(cx, win, &view).as_deref(), Some(PRETTY));
    assert_eq!(
        view.read_with(cx, |v, _| v.copy_message()),
        "Copied the response body"
    );
    set_mode(cx, win, &view, BodyMode::Raw);
    assert_eq!(copied(cx, win, &view).as_deref(), Some(JSON));
}

#[gpui_kit::test]
fn a_filtered_body_copies_the_matches(cx: &mut TestAppContext) {
    let (win, view) = shown(cx, JSON, "application/json");
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_filter("$.data[0].id", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(copied(cx, win, &view).as_deref(), Some("[\n  7\n]"));
    assert_eq!(
        view.read_with(cx, |v, _| v.copy_message()),
        "Copied the filtered body"
    );
}

#[gpui_kit::test]
fn a_long_body_copies_unchunked(cx: &mut TestAppContext) {
    let long = long_json();
    let (win, view) = shown(cx, &long, "application/json");
    assert!(matches!(
        view.read_with(cx, |v, _| v.shown()),
        Shown::Lines(_)
    ));
    assert_eq!(copied(cx, win, &view), reindent_json(&long));
    set_mode(cx, win, &view, BodyMode::Raw);
    assert!(matches!(
        view.read_with(cx, |v, _| v.shown()),
        Shown::Lines(_)
    ));
    assert_eq!(copied(cx, win, &view), Some(long));
}

#[gpui_kit::test]
fn nothing_to_copy_when_the_filter_matches_nothing(cx: &mut TestAppContext) {
    let (win, view) = shown(cx, JSON, "application/json");
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_filter("$.nothing", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!view.read_with(cx, |v, _| v.can_copy()));
}
