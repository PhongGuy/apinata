//! A large response body says why it is plain text, and in a narrow window choosing the last
//! tab in view keeps it in view without pushing the strip's first tab out past a blank band.

use crate::support;

use apinata_app::body_text::EDITOR_MAX_BYTES;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, ElementId, Pixels, TestAppContext, VisualTestContext,
    px, size,
};
use support::*;
use wiremock::ResponseTemplate;
use wiremock::matchers::path;

// The app at `width`, the request open and sent once to a local server answering `body`.
fn sent(
    cx: &mut TestAppContext,
    width: f32,
    body: String,
    mime: &str,
) -> (Started, wiremock::MockServer) {
    http_ready(cx);
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(wiremock::MockServer::builder().start());
    handle.block_on(
        wiremock::Mock::given(path("/ok"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, mime))
            .mount(&server),
    );
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    std::mem::forget(col);
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(width), px(800.)));
    settle(cx, s.win);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("send", cx))
        .unwrap();
    wait_real(cx, |cx| present(cx, s.win, "response-size"));
    settle(cx, s.win);
    (s, server)
}

fn settle(cx: &mut TestAppContext, win: AnyWindowHandle) {
    for _ in 0..4 {
        frame(cx, win);
    }
}

fn bounds(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    id: impl Into<ElementId>,
) -> Bounds<Pixels> {
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).bounds()
    })
    .unwrap()
}

// Lets the kit's click glide and the strip's own reveal run to their end.
fn settle_motion(cx: &mut TestAppContext, win: AnyWindowHandle) {
    for _ in 0..40 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        frame(cx, win);
    }
}

// A JSON array of photo-like records, about `bytes` long.
fn photos(bytes: usize) -> String {
    let mut s = String::from("[");
    let mut i = 0;
    while s.len() < bytes {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            "{{\"albumId\":{},\"id\":{i},\"title\":\"photo {i}\"}}",
            i / 50
        ));
        i += 1;
    }
    s.push(']');
    s
}

// The app `width` px wide with the sidebar shown, after the large JSON response arrived.
fn narrow(cx: &mut TestAppContext, width: f32) -> (Started, wiremock::MockServer) {
    let (s, server) = sent(cx, width, photos(1_000_000), "application/json");
    if !s.shell.read_with(cx, |sh, _| sh.sidebar_visible()) {
        press_in_shell(cx, &s, &key("b"));
    }
    settle(cx, s.win);
    (s, server)
}

// Clicks the rightmost of `item`'s tabs whose centre shows in `viewport`, the click that
// makes the kit glide on to peek at the next tab; returns its index.
fn click_rightmost(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    viewport: &'static str,
    item: &'static str,
) -> usize {
    let target = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let v = w.find(viewport).bounds();
            (0..16usize)
                .rfind(|ix| {
                    w.try_find((item, *ix)).is_some_and(|e| {
                        let c = e.bounds().center().x;
                        c > v.left() && c < v.right()
                    })
                })
                .unwrap()
        })
        .unwrap();
    assert!(target > 0, "the strip overflows at 640 px");
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click((item, target), cx)
    })
    .unwrap();
    settle_motion(cx, win);
    target
}

fn assert_inside(b: Bounds<Pixels>, strip: Bounds<Pixels>, what: &str) {
    assert!(
        b.left() >= strip.left() - px(0.5) && b.right() <= strip.right() + px(0.5),
        "{what} {b:?} outside {strip:?}"
    );
}

// Choosing the last section in view keeps the first section where it started, at the strip's
// left padding, with no blank band before it. At 660 px the click's peek cuts the first section
// and the chosen one still fits once it comes back whole.
#[gpui_kit::test]
fn at_660_the_section_strip_has_no_band_at_its_left(cx: &mut TestAppContext) {
    let (s, _server) = narrow(cx, 660.);
    let start = bounds(cx, s.win, ("section", 0usize));
    let chosen = click_rightmost(cx, s.win, "section-strip-viewport", "section");
    let strip = bounds(cx, s.win, "section-strip-viewport");
    assert_inside(
        bounds(cx, s.win, ("section", chosen)),
        strip,
        "the chosen section",
    );
    let first = bounds(cx, s.win, ("section", 0usize));
    assert!(
        (first.left() - start.left()).abs() < px(0.5),
        "the first section moved from {start:?} to {first:?}"
    );
    assert!(!present(cx, s.win, "section-strip-fade-left"));
}

// Choosing the last response tab in view glides the strip on to peek at the next tab, which
// pushes Body out at the left; with room to show it whole beside the chosen tab, it comes back.
// At 652 px the test font leaves that room, where at 640 px the chosen tab would meet the fade.
#[gpui_kit::test]
fn a_narrow_response_strip_keeps_the_active_tab_and_body_in_view(cx: &mut TestAppContext) {
    let (s, _server) = narrow(cx, 640.);
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(652.), px(800.)));
    settle_motion(cx, s.win);
    let chosen = click_rightmost(cx, s.win, "response-tab-strip-viewport", "response-tab");
    let strip = bounds(cx, s.win, "response-tab-strip-viewport");
    let tab = |cx: &mut TestAppContext, ix: usize| bounds(cx, s.win, ("response-tab", ix));
    assert_inside(tab(cx, chosen), strip, "the active tab");
    assert_inside(tab(cx, 0), strip, "Body");
}

// At 640 px the active tab stays in view, and Body, chosen again from the overflow menu, too.
#[gpui_kit::test]
fn at_640_the_active_response_tab_stays_in_view(cx: &mut TestAppContext) {
    let (s, _server) = narrow(cx, 640.);
    let chosen = click_rightmost(cx, s.win, "response-tab-strip-viewport", "response-tab");
    let strip = bounds(cx, s.win, "response-tab-strip-viewport");
    assert_inside(
        bounds(cx, s.win, ("response-tab", chosen)),
        strip,
        "the active tab",
    );
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.click("response-tab-overflow", cx)
    })
    .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.within("popup-menu").click(0usize, cx))
        .unwrap();
    settle_motion(cx, s.win);
    let strip = bounds(cx, s.win, "response-tab-strip-viewport");
    assert_inside(bounds(cx, s.win, ("response-tab", 0usize)), strip, "Body");
}

// Plain text keeps its size when shown pretty, so the editor's byte limit is the whole test.
#[gpui_kit::test]
fn a_body_over_the_editor_limit_says_it_is_plain_text(cx: &mut TestAppContext) {
    let (s, _server) = sent(cx, 1280., "a".repeat(EDITOR_MAX_BYTES + 1), "text/plain");
    let note = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find("body-banner")
                .and_then(|e| e.label().map(str::to_owned))
        })
        .unwrap();
    assert_eq!(
        note.as_deref(),
        Some("Large body: shown as plain text, folding and editing are off")
    );
}

#[gpui_kit::test]
fn a_body_at_the_editor_limit_has_no_note(cx: &mut TestAppContext) {
    let (s, _server) = sent(cx, 1280., "a".repeat(EDITOR_MAX_BYTES), "text/plain");
    assert!(
        present(cx, s.win, "body-editor"),
        "the body is in the editor"
    );
    assert!(!present(cx, s.win, "body-banner"));
}
