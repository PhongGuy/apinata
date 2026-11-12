//! Binary bodies: Hex by default, Base64 on request, a capped view with notes, and copy.

use crate::support;

use std::sync::Arc;

use apinata_app::body_text::{BINARY_VIEW_CAP, BinaryMode, base64_text, format_size, hex_rows};
use apinata_app::body_view::{BodyView, Shown};
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, SharedString, TestAppContext, WindowOptions};
use support::*;

/// `n` bytes that are not UTF-8 (the first is 0xFF), so the body is binary.
fn bytes(n: usize) -> Vec<u8> {
    (0..n).map(|i| (255 - i % 256) as u8).collect()
}

fn result(body: Vec<u8>, truncated: bool) -> Arc<RequestResult> {
    let mut r = Response::new(200)
        .with_header("content-type", "application/octet-stream")
        .with_body(body, "");
    if truncated {
        let n = r.body().len() as u64;
        r = r.with_truncated(n);
    }
    Arc::new(RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(r),
        Vec::new(),
    ))
}

fn host(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<BodyView>) {
    support::init(cx);
    cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
            cx.new(|cx| BodyView::new(w, cx))
        })
    })
    .unwrap()
}

fn show(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    view: &Entity<BodyView>,
    r: Arc<RequestResult>,
) {
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(r), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

fn click(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(id, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

fn rows(cx: &mut TestAppContext, view: &Entity<BodyView>) -> Vec<SharedString> {
    view.read_with(cx, |v, _| v.shown_rows().to_vec())
}

fn label(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> Option<String> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_binary_body_shows_hex_by_default(cx: &mut TestAppContext) {
    let body = bytes(40);
    let (win, view) = host(cx);
    show(cx, win, &view, result(body.clone(), false));
    assert_eq!(
        view.read_with(cx, |v, _| (v.shown(), v.binary_mode())),
        (Shown::Binary(40), BinaryMode::Hex)
    );
    assert_eq!(rows(cx, &view), hex_rows(&body));
    assert!(
        present(cx, win, "body-binary")
            && present(cx, win, "body-hex")
            && present(cx, win, "body-base64")
    );
    assert!(!present(cx, win, "body-pretty") && !present(cx, win, "body-raw"));
    assert!(!present(cx, win, "body-binary-cap") && !present(cx, win, "body-binary-cut"));
}

#[gpui_kit::test]
fn base64_is_kept_for_the_next_binary_body(cx: &mut TestAppContext) {
    let (win, view) = host(cx);
    show(cx, win, &view, result(bytes(40), false));
    click(cx, win, "body-base64");
    assert_eq!(
        view.read_with(cx, |v, _| v.binary_mode()),
        BinaryMode::Base64
    );
    assert_eq!(rows(cx, &view).concat(), base64_text(&bytes(40)));
    show(cx, win, &view, result(bytes(9), false));
    assert_eq!(rows(cx, &view).concat(), base64_text(&bytes(9)));
    click(cx, win, "body-hex");
    assert_eq!(rows(cx, &view), hex_rows(&bytes(9)));
}

#[gpui_kit::test]
fn a_body_over_the_cap_shows_only_the_cap(cx: &mut TestAppContext) {
    let len = BINARY_VIEW_CAP + 32;
    let (win, view) = host(cx);
    show(cx, win, &view, result(bytes(len), false));
    assert_eq!(rows(cx, &view).len(), BINARY_VIEW_CAP / 16);
    assert_eq!(
        label(cx, win, "body-binary-cap"),
        Some(format!(
            "Showing the first {} of {}.",
            format_size(BINARY_VIEW_CAP as u64),
            format_size(len as u64)
        ))
    );
}

#[gpui_kit::test]
fn a_truncated_body_says_it_was_cut(cx: &mut TestAppContext) {
    let (win, view) = host(cx);
    show(cx, win, &view, result(bytes(64), true));
    assert_eq!(
        label(cx, win, "body-binary-cut"),
        Some("The body was cut at the size limit.".to_owned())
    );
}

#[gpui_kit::test]
fn copy_takes_what_the_mode_shows(cx: &mut TestAppContext) {
    let body = bytes(40);
    let (win, view) = host(cx);
    show(cx, win, &view, result(body.clone(), false));
    assert!(view.read_with(cx, |v, _| v.can_copy()));
    click(cx, win, "body-copy");
    let hex: Vec<String> = hex_rows(&body).iter().map(|r| r.to_string()).collect();
    assert_eq!(
        cx.read_from_clipboard().and_then(|c| c.text()),
        Some(hex.join("\n"))
    );
    click(cx, win, "body-base64");
    click(cx, win, "body-copy");
    assert_eq!(
        cx.read_from_clipboard().and_then(|c| c.text()),
        Some(base64_text(&body))
    );
}

#[gpui_kit::test]
fn a_text_body_offers_no_binary_modes(cx: &mut TestAppContext) {
    let (win, view) = host(cx);
    let r = Response::new(200)
        .with_header("content-type", "text/plain")
        .with_text_body("hi");
    let r = Arc::new(RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(r),
        Vec::new(),
    ));
    show(cx, win, &view, r);
    assert!(present(cx, win, "body-pretty"));
    assert!(!present(cx, win, "body-hex") && !present(cx, win, "body-base64"));
}

#[gpui_kit::test]
fn copy_keeps_base64_unbroken_across_rows(cx: &mut TestAppContext) {
    let body = bytes(300);
    let (win, view) = host(cx);
    show(cx, win, &view, result(body.clone(), false));
    click(cx, win, "body-base64");
    let shown = rows(cx, &view);
    assert!(shown.len() > 1 && shown.iter().all(|r| r.len() <= 76));
    click(cx, win, "body-copy");
    assert_eq!(
        cx.read_from_clipboard().and_then(|c| c.text()),
        Some(base64_text(&body))
    );
}

#[test]
fn code_and_body_text_turn_ligatures_off() {
    // `<=>`, `->` and an HTML comment's `-->` must stay separate glyphs.
    let features = apinata_app::body_view::code_font_features();
    let has = |tag: &str| features.0.iter().any(|(t, v)| t == tag && *v == 0);
    assert!(has("calt") && has("liga"), "{:?}", features.0);
}
