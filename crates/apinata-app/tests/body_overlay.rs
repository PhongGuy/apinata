//! The response body has no toolbar row: its format menu and actions float in the body's
//! top-right corner, so the body starts right under the response header.

use crate::support;

use apinata_app::body_text::BodyKind;
use apinata_app::body_view::BodyView;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, ElementId, Entity, Pixels, TestAppContext,
    VisualTestContext, WindowOptions, px, size,
};
use support::*;
use wiremock::ResponseTemplate;

fn json() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw("{\"a\":1}", "application/json")
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

// A bare body view showing `response`'s body.
fn show_body(
    cx: &mut TestAppContext,
    response: ResponseTemplate,
) -> (Sent, AnyWindowHandle, Entity<BodyView>) {
    let s = send_once(cx, response, "");
    support::init(cx);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| BodyView::new(w, cx))
            })
        })
        .unwrap();
    let r = s.result.clone();
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(r), w, cx))
    })
    .unwrap();
    frame(cx, win);
    (s, win, view)
}

#[gpui_kit::test]
fn the_body_starts_directly_under_the_header_row(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, json());
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1280.), px(800.)));
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("send", cx))
        .unwrap();
    wait_real(cx, |cx| present(cx, s.win, "body-editor"));
    for _ in 0..3 {
        frame(cx, s.win);
    }
    let strip = bounds(cx, s.win, "response-tab-strip");
    let body = bounds(cx, s.win, "body-editor");
    // No toolbar row fits between the header and the body: the gap is under one control.
    assert!(
        body.top() - strip.bottom() < px(16.),
        "a row sits between {strip:?} and {body:?}"
    );
    // The actions float inside the body's top-right corner, clear of its scrollbar.
    let overlay = bounds(cx, s.win, "body-actions");
    assert!(overlay.top() >= body.top() && overlay.top() < body.top() + px(16.));
    assert!(
        overlay.right() <= body.right() - px(8.),
        "{overlay:?} in {body:?}"
    );
    assert!(overlay.size.width < body.size.width / 2.);
    for id in [
        "body-format",
        "body-filter",
        "body-edit",
        "body-copy",
        "body-save",
    ] {
        let b = bounds(cx, s.win, id);
        assert!(
            overlay.contains(&b.center()),
            "{id} {b:?} is outside the overlay {overlay:?}"
        );
    }
    drop(col);
}

#[gpui_kit::test]
fn view_as_xml_relabels_a_json_body_until_the_next_response(cx: &mut TestAppContext) {
    let (_s, win, view) = show_body(cx, json());
    assert_eq!(view.read_with(cx, |v, _| v.format_label()), Some("JSON"));
    choose_format(cx, win, "XML");
    assert_eq!(view.read_with(cx, |v, _| v.format_label()), Some("XML"));
    assert_eq!(
        view.read_with(cx, |v, _| v.kind_override()),
        Some(BodyKind::Xml)
    );
    // A new response is shown as detected again.
    let (other, _, _) = show_body(cx, json());
    let r = other.result.clone();
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(r), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, _| v.kind_override()), None);
    assert_eq!(view.read_with(cx, |v, _| v.format_label()), Some("JSON"));
}

#[gpui_kit::test]
fn the_format_menu_switches_raw_and_wrap(cx: &mut TestAppContext) {
    let (_s, win, view) = show_body(cx, json());
    choose_format(cx, win, "Raw");
    assert_eq!(
        view.read_with(cx, |v, _| v.mode()),
        apinata_app::body_view::BodyMode::Raw
    );
    let wrapped = view.read_with(cx, |v, _| v.wrap());
    choose_format(cx, win, "Wrap");
    assert_eq!(view.read_with(cx, |v, _| v.wrap()), !wrapped);
    choose_format(cx, win, "Pretty");
    assert_eq!(
        view.read_with(cx, |v, _| v.mode()),
        apinata_app::body_view::BodyMode::Pretty
    );
}

#[gpui_kit::test]
fn view_as_hex_shows_a_text_body_as_bytes(cx: &mut TestAppContext) {
    let (_s, win, view) = show_body(cx, json());
    choose_format(cx, win, "Hex");
    assert!(matches!(
        view.read_with(cx, |v, _| v.shown()),
        apinata_app::body_view::Shown::Binary(_)
    ));
}

#[gpui_kit::test]
fn an_empty_body_has_no_overlay(cx: &mut TestAppContext) {
    let (_s, win, _view) = show_body(cx, ResponseTemplate::new(204));
    assert!(!present(cx, win, "body-actions"));
    assert!(!present(cx, win, "body-format"));
}

// A body that is one JSON line of `len` characters.
fn long_line(len: usize) -> ResponseTemplate {
    let body = format!("{{\"k\":\"{}\"}}", "x ".repeat(len / 2));
    ResponseTemplate::new(200).set_body_raw(body, "application/json")
}

// Whether the overlay sits wholly above the text element `id`, so no line runs under it.
fn overlay_above(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> bool {
    frame(cx, win);
    let overlay = bounds(cx, win, "body-actions");
    let text = bounds(cx, win, id);
    overlay.bottom() <= text.top()
}

#[gpui_kit::test]
fn wrapped_raw_text_starts_below_the_overlay(cx: &mut TestAppContext) {
    let (_s, win, view) = show_body(cx, long_line(3000));
    choose_format(cx, win, "Raw");
    assert!(view.read_with(cx, |v, _| v.wrap()));
    assert!(overlay_above(cx, win, "body-editor"));
}

#[gpui_kit::test]
fn wrapped_pretty_text_starts_below_the_overlay(cx: &mut TestAppContext) {
    let (_s, win, view) = show_body(cx, long_line(3000));
    assert_eq!(
        view.read_with(cx, |v, _| v.mode()),
        apinata_app::body_view::BodyMode::Pretty
    );
    assert!(overlay_above(cx, win, "body-editor"));
}

#[gpui_kit::test]
fn wrapped_long_body_rows_start_below_the_overlay(cx: &mut TestAppContext) {
    let (_s, win, view) = show_body(cx, long_line(300_000));
    choose_format(cx, win, "Raw");
    assert!(matches!(
        view.read_with(cx, |v, _| v.shown()),
        apinata_app::body_view::Shown::Lines(_)
    ));
    assert!(overlay_above(cx, win, "body-lines-frame"));
}

#[gpui_kit::test]
fn short_wrapped_lines_keep_the_overlay_floating(cx: &mut TestAppContext) {
    let (_s, win, view) = show_body(cx, json());
    choose_format(cx, win, "Raw");
    assert!(view.read_with(cx, |v, _| v.wrap()));
    assert!(!overlay_above(cx, win, "body-editor"));
}

#[gpui_kit::test]
fn unwrapped_text_keeps_the_overlay_floating(cx: &mut TestAppContext) {
    let (_s, win, view) = show_body(cx, long_line(3000));
    choose_format(cx, win, "Raw");
    choose_format(cx, win, "Wrap");
    assert!(!view.read_with(cx, |v, _| v.wrap()));
    // An unwrapped line scrolls sideways, so its end can always be brought out from under.
    let id = match view.read_with(cx, |v, _| v.shown()) {
        apinata_app::body_view::Shown::Lines(_) => "body-lines-frame",
        _ => "body-editor",
    };
    assert!(!overlay_above(cx, win, id));
}
