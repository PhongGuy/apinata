//! HTML Preview: an HTML body laid out by the kit's renderer from the format menu, with links
//! that open nothing and images that load only from data: URLs.

use crate::support;

use std::sync::Arc;

use apinata_app::body_view::BodyView;
use apinata_app::html_preview;
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Entity, ImageFormat, ImageSource, SharedUri, TestAppContext,
    WindowOptions, point, px,
};
use support::*;

fn typed(content_type: &str, body: &str) -> Arc<RequestResult> {
    let r = Response::new(200)
        .with_header("content-type", content_type)
        .with_body(body.as_bytes().to_vec(), body);
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
    cx.run_until_parked();
    frame(cx, win);
    (win, view)
}

const PAGE: &str = "<html><head><style>p{color:red}</style><script>alert(1)</script></head>\
    <body><h1>Title</h1><p>Hello</p><ul><li>one</li></ul></body></html>";

#[gpui_kit::test]
fn preview_is_offered_for_html_only(cx: &mut TestAppContext) {
    let (win, _view) = host(cx, typed("text/html", PAGE));
    let entries = format_entries(cx, win);
    assert!(entries.iter().any(|e| e == "Preview"), "{entries:?}");

    let (win, _view) = host(cx, typed("application/json", r#"{"a":1}"#));
    let entries = format_entries(cx, win);
    assert!(!entries.iter().any(|e| e == "Preview"), "{entries:?}");
}

#[gpui_kit::test]
fn preview_replaces_the_editor_until_raw_is_chosen(cx: &mut TestAppContext) {
    let (win, view) = host(cx, typed("text/html", PAGE));
    assert!(present(cx, win, "body-editor"));
    choose_format(cx, win, "Preview");
    assert!(cx.update(|cx| view.read(cx).previewing()));
    assert!(present(cx, win, "body-html-preview"));
    assert!(!present(cx, win, "body-editor"));
    let html = cx.update(|cx| view.read(cx).preview_html()).unwrap();
    assert!(html.contains("<h1>Title</h1>"), "{html}");
    assert!(
        !html.contains("alert") && !html.contains("color:red"),
        "{html}"
    );

    choose_format(cx, win, "Raw");
    assert!(!cx.update(|cx| view.read(cx).previewing()));
    assert!(present(cx, win, "body-editor"));
    assert!(!present(cx, win, "body-html-preview"));
}

#[gpui_kit::test]
fn preview_stays_on_for_the_next_html_body_only(cx: &mut TestAppContext) {
    let (win, view) = host(cx, typed("text/html", PAGE));
    choose_format(cx, win, "Preview");
    let json = typed("application/json", r#"{"a":1}"#);
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(json), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!cx.update(|cx| view.read(cx).previewing()));
    assert!(present(cx, win, "body-editor"));

    let html = typed("text/html", PAGE);
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(html), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.update(|cx| view.read(cx).previewing()));
}

#[test]
fn a_remote_or_file_image_is_never_handed_to_the_loader() {
    for uri in [
        "http://127.0.0.1:9/p.png",
        "https://example.invalid/p.png",
        "file:///etc/passwd",
        "p.png",
    ] {
        let source = html_preview::image_source(&SharedUri::from(uri.to_string()));
        assert!(
            !matches!(source, ImageSource::Resource(_)),
            "{uri} would be fetched"
        );
    }
}

#[test]
fn a_data_image_is_decoded_inline() {
    // The eight-byte PNG signature, base64.
    let uri = SharedUri::from("data:image/png;base64,iVBORw0KGgo=".to_string());
    match html_preview::image_source(&uri) {
        ImageSource::Image(image) => {
            assert_eq!(image.format(), ImageFormat::Png);
            assert_eq!(image.bytes(), b"\x89PNG\r\n\x1a\n");
        }
        _ => panic!("a data: image should be decoded inline"),
    }
}

// A bare window laying out `page` with `view`, under the text selection layer the app's
// root mounts, so link clicks are handled as in the app.
struct Page {
    html: gpui_kit::SharedString,
    kit_default: bool,
}

impl gpui_kit::Render for Page {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        use gpui_kit::{ParentElement as _, Styled as _};
        let page = if self.kit_default {
            gpui_kit::component::text::TextView::html("page", self.html.clone())
        } else {
            html_preview::view("page", self.html.clone())
        };
        gpui_kit::div()
            .size_full()
            .child(gpui_kit::base::TextSelectionLayer)
            .child(page)
    }
}

fn click_first_line(cx: &mut TestAppContext, kit_default: bool) -> Option<String> {
    support::init(cx);
    let html = format!(
        r#"<p><a href="file:///etc/passwd">{}</a></p>"#,
        "x".repeat(80)
    );
    let (_view, vcx) = cx.add_window_view(|_, _| Page {
        html: html.into(),
        kit_default,
    });
    vcx.run_until_parked();
    vcx.simulate_click(point(px(16.), px(8.)), gpui_kit::Modifiers::none());
    vcx.run_until_parked();
    vcx.opened_url()
}

#[gpui_kit::test]
fn the_kit_alone_would_open_a_page_link(cx: &mut TestAppContext) {
    // The control for the test below: the same click with the kit's default handler opens it.
    assert_eq!(
        click_first_line(cx, true).as_deref(),
        Some("file:///etc/passwd")
    );
}

#[gpui_kit::test]
fn a_page_link_in_the_preview_opens_nothing(cx: &mut TestAppContext) {
    assert_eq!(click_first_line(cx, false), None);
}

#[test]
fn nesting_past_the_bound_is_not_previewed() {
    // The scan is iterative, so this depth cannot overflow the test's own stack.
    assert_eq!(html_preview::prepare(&"<div>".repeat(50_000)), None);
    // A close tag for an element that is not open closes nothing.
    assert_eq!(html_preview::prepare(&"<div></x>".repeat(50_000)), None);
    assert_eq!(html_preview::prepare(&"<div/>".repeat(50_000)), None);
    let bound = html_preview::MAX_DEPTH;
    assert_eq!(html_preview::prepare(&"<span>".repeat(bound + 1)), None);
    let ok = "<div>".repeat(bound);
    assert_eq!(html_preview::prepare(&ok).as_deref(), Some(ok.as_str()));
}

#[test]
fn unclosed_and_void_elements_do_not_count_as_nesting() {
    // Elements whose end tag is optional, and void ones, close themselves.
    let page = format!(
        "<ul>{}</ul><p>{}<!-- <div><div> --><img src=x><input/>",
        "<li>item".repeat(1000),
        "<p>text<br>".repeat(1000)
    );
    assert!(html_preview::prepare(&page).is_some());
}

#[test]
fn script_style_and_title_text_is_removed_wherever_it_sits() {
    let page = "<title>T</title><h1>a<script>alert('<b>')</script>b<STYLE type=x>p{}</style>c</h1>\
        <span><script src=x></script></span><p>d</p><script>never closed";
    assert_eq!(
        html_preview::prepare(page).as_deref(),
        Some("<h1>abc</h1><span></span><p>d</p>")
    );
}

#[test]
fn an_svg_data_image_is_not_decoded() {
    for uri in [
        "data:image/svg+xml;base64,PHN2Zy8+",
        "data:image/svg+xml,%3Csvg%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%2F%3E",
        "data:image/tiff;base64,SUkqAA==",
    ] {
        match html_preview::image_source(&SharedUri::from(uri.to_string())) {
            ImageSource::Image(image) => {
                assert!(image.bytes().is_empty(), "{uri} was decoded")
            }
            _ => panic!("{uri} should become an empty image"),
        }
    }
}

#[gpui_kit::test]
fn a_deeply_nested_page_stays_text_with_a_note(cx: &mut TestAppContext) {
    let deep = "<div>".repeat(50_000);
    let (win, view) = host(cx, typed("text/html", &deep));
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| {
            v.set_mode(apinata_app::body_view::BodyMode::Raw, w, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert!(present(cx, win, "body-editor"), "the case needs the editor");
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_preview(true, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, win);
    assert!(present(cx, win, "body-editor"));
    assert!(!present(cx, win, "body-html-preview"));
    let note = cx
        .update_window(win, |_, w, _| {
            w.find("body-banner").label().map(str::to_owned)
        })
        .unwrap();
    assert_eq!(note.as_deref(), Some(apinata_app::body_view::TOO_DEEP_NOTE));
}

#[gpui_kit::test]
fn the_find_bar_closes_when_a_body_lands_in_the_preview(cx: &mut TestAppContext) {
    let (win, view) = host(cx, typed("text/html", PAGE));
    choose_format(cx, win, "Preview");
    let json = typed("application/json", r#"{"a":1}"#);
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| {
            v.show(Some(json), w, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| view.update(cx, |v, cx| v.open_find(w, cx)))
        .unwrap();
    assert!(cx.update(|cx| view.read(cx).find_open()));
    let html = typed("text/html", PAGE);
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(html), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.update(|cx| view.read(cx).previewing()));
    assert!(!cx.update(|cx| view.read(cx).find_open()));
}
