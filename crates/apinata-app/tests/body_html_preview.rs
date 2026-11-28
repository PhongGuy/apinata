//! HTML Preview: an HTML body laid out by the kit's renderer from the format menu, with links
//! that open nothing and images that load only from data: URLs.

use crate::support;

use std::sync::Arc;

use apinata_app::body_view::BodyView;
use apinata_app::html_preview;
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::{
    AnyWindowHandle, AppContext, Entity, ImageFormat, ImageSource, SharedUri, TestAppContext,
    WindowOptions, point, px,
};
use support::*;

fn typed(content_type: &str, body: &str) -> Arc<RequestResult> {
    let r = Response::new(200)
        .with_header("content-type", content_type)
        .with_body(body.as_bytes().to_vec(), "");
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
