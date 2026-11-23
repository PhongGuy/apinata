//! The redirect count beside the status and the note when the redirect limit ended a chain.

use crate::support;

use std::sync::Arc;
use std::time::SystemTime;

use apinata_app::response_pane::{ResponsePane, ResponseTab};
use apinata_app::send_model::{Restored, SendModel};
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, WindowOptions};
use support::*;
use wiremock::ResponseTemplate;

fn pane(
    cx: &mut TestAppContext,
    model: &Entity<SendModel>,
) -> (AnyWindowHandle, Entity<ResponsePane>) {
    support::init(cx);
    let model = model.clone();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    (win, view)
}

fn label(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> Option<String> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_loop_stopped_by_the_limit_says_so_in_the_body_area(cx: &mut TestAppContext) {
    let s = send_once(
        cx,
        ResponseTemplate::new(302).insert_header("location", "/ok"),
        "settings:\n  maxRedirects: 5\n",
    );
    let (win, view) = pane(cx, &s.model);
    assert_eq!(
        label(cx, win, "response-redirect-stop").as_deref(),
        Some("Stopped after 5 redirects (the redirect limit). The last Location was /ok.")
    );
    assert_eq!(
        label(cx, win, "response-redirects").as_deref(),
        Some("5 redirects")
    );
    cx.update_window(win, |_, w, cx| w.click("response-redirects", cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(view.read_with(cx, |p, _| p.tab()), ResponseTab::Timeline);
}

#[gpui_kit::test]
fn a_response_with_no_redirects_shows_neither(cx: &mut TestAppContext) {
    let s = send_once(cx, ResponseTemplate::new(200), "");
    let (win, _view) = pane(cx, &s.model);
    assert!(present(cx, win, "response-status"));
    assert!(!present(cx, win, "response-redirects"));
    assert!(!present(cx, win, "response-redirect-stop"));
}

#[gpui_kit::test]
fn a_kept_stop_shows_the_masked_location_and_no_count_without_a_timeline(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, _root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    let response = Response::new(302)
        .with_status_text("Found")
        .with_header("location", "http://u:pw-S3cret@127.0.0.1:9/loop")
        .with_redirects(5, true);
    let result = RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(response),
        Vec::new(),
    );
    let restored = Restored {
        saved_at: SystemTime::now(),
        body_dropped: false,
        body_masked: false,
        masked_headers: Vec::new(),
        header_digests: None,
    };
    model.update(cx, |m, cx| m.restore(Arc::new(result), restored, cx));
    let (win, _view) = pane(cx, &model);
    let note = label(cx, win, "response-redirect-stop").expect("the note is shown");
    assert!(
        note.starts_with("Stopped after 5 redirects (the redirect limit)."),
        "{note}"
    );
    assert!(note.contains("127.0.0.1:9/loop"), "{note}");
    assert!(!note.contains("pw-S3cret"), "{note}");
    assert!(!present(cx, win, "response-redirects"));
}

#[gpui_kit::test]
fn the_count_is_reached_and_opened_from_the_keyboard(cx: &mut TestAppContext) {
    let s = send_once(
        cx,
        ResponseTemplate::new(302).insert_header("location", "/ok"),
        "settings:\n  maxRedirects: 2\n",
    );
    let (win, view) = pane(cx, &s.model);
    // Keyboard travel starts from the body, as it does once a response arrives.
    cx.update_window(win, |_, w, cx| view.update(cx, |p, cx| p.focus_body(w, cx)))
        .unwrap();
    enter_on(cx, win, "response-redirects");
    frame(cx, win);
    assert_eq!(view.read_with(cx, |p, _| p.tab()), ResponseTab::Timeline);
}

#[gpui_kit::test]
fn a_kept_stop_whose_body_was_dropped_still_shows_the_note(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, _root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    let response = Response::new(302)
        .with_status_text("Found")
        .with_header("location", "/loop")
        .with_redirects(5, true);
    let result = RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(response),
        Vec::new(),
    );
    let restored = Restored {
        saved_at: SystemTime::now(),
        body_dropped: true,
        body_masked: false,
        masked_headers: Vec::new(),
        header_digests: None,
    };
    model.update(cx, |m, cx| m.restore(Arc::new(result), restored, cx));
    let (win, _view) = pane(cx, &model);
    assert!(present(cx, win, "response-body-dropped"));
    assert_eq!(
        label(cx, win, "response-redirect-stop").as_deref(),
        Some("Stopped after 5 redirects (the redirect limit). The last Location was /loop.")
    );
}
