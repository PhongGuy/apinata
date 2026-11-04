//! The body bar names the detected format, copies text bodies and saves any body.

use crate::support;

use apinata_app::body_view::BodyView;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, WindowOptions};
use support::*;
use wiremock::ResponseTemplate;

fn show_body(
    cx: &mut TestAppContext,
    response: ResponseTemplate,
) -> (AnyWindowHandle, Entity<BodyView>) {
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
    (win, view)
}

#[gpui_kit::test]
fn a_json_body_is_labelled_and_copied_as_shown(cx: &mut TestAppContext) {
    let (win, view) = show_body(
        cx,
        ResponseTemplate::new(200).set_body_raw("{\"a\":1}", "application/json"),
    );
    assert_eq!(view.read_with(cx, |v, _| v.format_label()), Some("JSON"));
    assert!(present(cx, win, "body-format"));
    cx.update_window(win, |_, w, cx| w.click("body-copy", cx))
        .unwrap();
    cx.run_until_parked();
    let copied = cx.read_from_clipboard().and_then(|c| c.text());
    assert_eq!(copied.as_deref(), Some("{\n  \"a\": 1\n}"));
    assert!(present(cx, win, "body-save"));
}

#[gpui_kit::test]
fn a_binary_body_offers_copy_and_save(cx: &mut TestAppContext) {
    let bytes = vec![0u8, 159, 146, 150];
    let (win, view) = show_body(
        cx,
        ResponseTemplate::new(200).set_body_raw(bytes, "application/octet-stream"),
    );
    assert_eq!(view.read_with(cx, |v, _| v.format_label()), Some("Binary"));
    assert!(present(cx, win, "body-copy"));
    assert!(present(cx, win, "body-save"));
}

#[gpui_kit::test]
fn an_empty_body_has_no_format_or_actions(cx: &mut TestAppContext) {
    let (win, view) = show_body(cx, ResponseTemplate::new(204));
    assert_eq!(view.read_with(cx, |v, _| v.format_label()), None);
    assert!(!present(cx, win, "body-copy"));
    assert!(!present(cx, win, "body-save"));
}
