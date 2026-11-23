//! One banner under the body bar says how much of the body is shown: cut, large, or both;
//! the status line marks a cut on the size itself.

use crate::support;

use std::sync::Arc;

use apinata_app::body_text::{EDITOR_MAX_BYTES, size_labels};
use apinata_app::body_view::{BodyView, LARGE_BODY_NOTE};
use apinata_app::response_pane::ResponsePane;
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Entity, Pixels, TestAppContext, WindowOptions,
};
use support::*;
use wiremock::ResponseTemplate;

// A plain-text body of `len` bytes, cut at the size limit from `full` bytes when given.
fn text(len: usize, full: Option<u64>) -> Arc<RequestResult> {
    let mut r = Response::new(200)
        .with_header("content-type", "text/plain")
        .with_text_body("a".repeat(len));
    if let Some(full) = full {
        r = r.with_truncated(full);
    }
    Arc::new(RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(r),
        Vec::new(),
    ))
}

fn host(cx: &mut TestAppContext, r: Arc<RequestResult>) -> (AnyWindowHandle, Entity<BodyView>) {
    support::init(cx);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| BodyView::new(w, cx))
            })
        })
        .unwrap();
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(r), w, cx))
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

fn bounds(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> Bounds<Pixels> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).bounds()
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_cut_large_body_says_how_much_is_shown_as_plain_text(cx: &mut TestAppContext) {
    let len = EDITOR_MAX_BYTES + 1;
    let full = 10 * 1024 * 1024;
    let (win, _view) = host(cx, text(len, Some(full)));
    let (size, cut) = size_labels(full, len as u64, true);
    assert_eq!(
        label(cx, win, "body-banner"),
        Some(format!(
            "Showing the first {} of {size} as plain text.",
            cut.unwrap()
        ))
    );
    assert!(!present(cx, win, "body-large-note"));
}

#[gpui_kit::test]
fn a_cut_small_body_says_how_much_is_shown(cx: &mut TestAppContext) {
    let (win, _view) = host(cx, text(1000, Some(5000)));
    let (size, cut) = size_labels(5000, 1000, true);
    assert_eq!(
        label(cx, win, "body-banner"),
        Some(format!("Showing the first {} of {size}.", cut.unwrap()))
    );
}

#[gpui_kit::test]
fn a_whole_large_body_keeps_the_plain_text_note(cx: &mut TestAppContext) {
    let (win, _view) = host(cx, text(EDITOR_MAX_BYTES + 1, None));
    assert_eq!(label(cx, win, "body-banner"), Some(LARGE_BODY_NOTE.into()));
}

#[gpui_kit::test]
fn a_whole_small_body_has_no_banner(cx: &mut TestAppContext) {
    let (win, _view) = host(cx, text(1000, None));
    assert!(!present(cx, win, "body-banner"));
}

// The banner sits between the bar and the find bar, so opening find does not move it.
#[gpui_kit::test]
fn the_banner_keeps_its_place_when_find_opens(cx: &mut TestAppContext) {
    let (win, view) = host(cx, text(EDITOR_MAX_BYTES + 1, Some(10 * 1024 * 1024)));
    let before = bounds(cx, win, "body-banner");
    cx.update_window(win, |_, w, cx| view.update(cx, |v, cx| v.open_find(w, cx)))
        .unwrap();
    frame(cx, win);
    assert!(view.read_with(cx, |v, _| v.find_open()));
    assert_eq!(bounds(cx, win, "body-banner"), before);
}

// The size alone carries the cut: no second label, and the accessible name says where it was cut.
#[gpui_kit::test]
fn the_status_line_names_the_cut_on_the_size(cx: &mut TestAppContext) {
    let s = send_once(cx, ResponseTemplate::new(200), "");
    support::init(cx);
    s.model
        .update(cx, |m, cx| m.show(text(1000, Some(5000)), cx));
    let model = s.model.clone();
    let (win, _pane) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    let (size, cut) = size_labels(5000, 1000, true);
    assert!(!present(cx, win, "response-truncated"));
    assert_eq!(
        label(cx, win, "response-size"),
        Some(format!("{size}, cut at {}", cut.unwrap()))
    );
}
