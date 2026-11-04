use crate::support;

use apinata_app::body_text::{ImageInfo, ImageKind};
use apinata_app::body_view::{BodyMode, BodyView, Shown};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, WindowOptions};
use support::*;
use wiremock::ResponseTemplate;

fn host(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<BodyView>) {
    support::init(cx);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| BodyView::new(w, cx))
            })
        })
        .unwrap();
    (win, view)
}

fn show(cx: &mut TestAppContext, win: AnyWindowHandle, view: &Entity<BodyView>, s: &Sent) {
    let r = s.result.clone();
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(r), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

fn text(cx: &mut TestAppContext, view: &Entity<BodyView>) -> (String, String) {
    let ed = view.read_with(cx, |v, _| v.editor().clone());
    ed.read_with(cx, |e, _| {
        (e.value().to_string(), e.language_name().to_string())
    })
}

#[gpui_kit::test]
fn json_is_pretty_by_default_and_raw_on_request(cx: &mut TestAppContext) {
    let s = send_once(
        cx,
        ResponseTemplate::new(200).set_body_raw("{\"a\":[1,2]}", "application/json"),
        "",
    );
    let (win, view) = host(cx);
    show(cx, win, &view, &s);
    assert_eq!(
        view.read_with(cx, |v, _| (v.shown(), v.mode())),
        (Shown::Editor, BodyMode::Pretty)
    );
    assert_eq!(
        text(cx, &view),
        ("{\n  \"a\": [\n    1,\n    2\n  ]\n}".into(), "json".into())
    );
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click("body-raw", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(text(cx, &view), ("{\"a\":[1,2]}".into(), "text".into()));
}

#[gpui_kit::test]
fn the_last_mode_wins(cx: &mut TestAppContext) {
    let s = send_once(
        cx,
        ResponseTemplate::new(200).set_body_raw("{\"a\":1}", "application/json"),
        "",
    );
    let (win, view) = host(cx);
    let r = s.result.clone();
    // Both prepares are queued before either runs; only the newer may land.
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| {
            v.show(Some(r), w, cx);
            v.set_mode(BodyMode::Raw, w, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(text(cx, &view).0, "{\"a\":1}");
}

#[gpui_kit::test]
fn a_long_body_is_shown_as_rows(cx: &mut TestAppContext) {
    let body = vec!["line"; 30_000].join("\n");
    let s = send_once(cx, ResponseTemplate::new(200).set_body_string(body), "");
    let (win, view) = host(cx);
    show(cx, win, &view, &s);
    assert_eq!(view.read_with(cx, |v, _| v.shown()), Shown::Lines(30_000));
    assert!(present(cx, win, "body-lines"));
    assert!(!present(cx, win, "body-editor"));
}

#[gpui_kit::test]
fn a_binary_body_offers_save(cx: &mut TestAppContext) {
    let bytes = vec![0u8, 159, 146, 150];
    let s = send_once(
        cx,
        ResponseTemplate::new(200).set_body_raw(bytes.clone(), "application/octet-stream"),
        "",
    );
    let (win, view) = host(cx);
    show(cx, win, &view, &s);
    assert_eq!(view.read_with(cx, |v, _| v.shown()), Shown::Binary(4));
    assert!(present(cx, win, "body-binary"));
    let out = tempfile::tempdir().unwrap();
    let target = out.path().join("body.bin");
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click("body-save", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.did_prompt_for_new_path());
    let chosen = target.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.run_until_parked();
    assert_eq!(std::fs::read(&target).unwrap(), bytes);
}

#[gpui_kit::test]
fn an_empty_body_says_so(cx: &mut TestAppContext) {
    let s = send_once(cx, ResponseTemplate::new(204), "");
    let (win, view) = host(cx);
    show(cx, win, &view, &s);
    assert_eq!(view.read_with(cx, |v, _| v.shown()), Shown::Empty);
    assert!(present(cx, win, "body-empty"));
}

#[gpui_kit::test]
fn a_new_body_forgets_the_previous_format_until_prepared(cx: &mut TestAppContext) {
    let text_body = send_once(
        cx,
        ResponseTemplate::new(200).set_body_raw("{\"a\":1}", "application/json"),
        "",
    );
    let binary = send_once(
        cx,
        ResponseTemplate::new(200).set_body_raw(vec![0u8, 159, 146], "application/octet-stream"),
        "",
    );
    let (win, view) = host(cx);
    show(cx, win, &view, &text_body);
    assert_eq!(view.read_with(cx, |v, _| v.format_label()), Some("JSON"));
    // Before the binary body's prepare lands, nothing may still claim the old text kind.
    let r = binary.result.clone();
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(r), w, cx))
    })
    .unwrap();
    assert_eq!(view.read_with(cx, |v, _| v.format_label()), None);
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, _| v.format_label()), Some("Binary"));
}

#[gpui_kit::test]
fn a_text_body_sits_in_a_framed_scrolling_list(cx: &mut TestAppContext) {
    let body = vec!["line"; 30_000].join("\n");
    let s = send_once(cx, ResponseTemplate::new(200).set_body_string(body), "");
    let (win, view) = host(cx);
    cx.simulate_window_resize(win, gpui_kit::size(gpui_kit::px(800.), gpui_kit::px(500.)));
    show(cx, win, &view, &s);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let frame = w.find("body-lines-frame").bounds();
        assert!(frame.size.height > gpui_kit::px(400.), "{frame:?}");
        assert!(frame.bottom() <= gpui_kit::px(500.5), "{frame:?}");
        assert!(w.try_find(("line-number", 0usize)).is_some());
        w.scroll(
            "body-lines-frame",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-2000.))),
            cx,
        );
        assert!(
            w.try_find(("line-number", 0usize)).is_none(),
            "the list did not scroll"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn copy_and_save_confirm_with_a_toast(cx: &mut TestAppContext) {
    use gpui_kit::component::WindowExt as _;
    let s = send_once(
        cx,
        ResponseTemplate::new(200).set_body_raw("{\"a\":1}", "application/json"),
        "",
    );
    let (win, view) = host(cx);
    show(cx, win, &view, &s);
    let count = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, cx| w.notifications(cx).len())
            .unwrap()
    };
    let before = count(cx);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click("body-copy", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(count(cx), before + 1);
    let out = tempfile::tempdir().unwrap();
    let target = out.path().join("body.json");
    cx.update_window(win, |_, w, cx| w.click("body-save", cx))
        .unwrap();
    cx.run_until_parked();
    let chosen = target.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.run_until_parked();
    assert!(target.exists());
    assert_eq!(count(cx), before + 2);
}

#[gpui_kit::test]
fn a_result_without_text_closes_the_find_bar(cx: &mut TestAppContext) {
    let s = send_once(
        cx,
        ResponseTemplate::new(200).set_body_raw("{\"a\":1}", "application/json"),
        "",
    );
    let (win, view) = host(cx);
    show(cx, win, &view, &s);
    cx.update_window(win, |_, w, cx| view.update(cx, |v, cx| v.open_find(w, cx)))
        .unwrap();
    assert!(view.read_with(cx, |v, _| v.find_open()));
    cx.update_window(win, |_, w, cx| view.update(cx, |v, cx| v.show(None, w, cx)))
        .unwrap();
    cx.run_until_parked();
    assert!(!view.read_with(cx, |v, _| v.find_open()));
}

#[gpui_kit::test]
fn the_filter_placeholder_fits_array_and_object_roots(cx: &mut TestAppContext) {
    let (_win, view) = host(cx);
    let input = view.read_with(cx, |v, _| v.filter_input().clone());
    assert_eq!(
        input.read_with(cx, |i, _| i.presentation().placeholder().to_string()),
        "$..id"
    );
}

#[gpui_kit::test]
fn xml_is_pretty_and_highlighted_as_markup(cx: &mut TestAppContext) {
    let s = send_once(
        cx,
        ResponseTemplate::new(200).set_body_raw("<r><a>1</a></r>", "application/xml"),
        "",
    );
    let (win, view) = host(cx);
    show(cx, win, &view, &s);
    assert_eq!(view.read_with(cx, |v, _| v.format_label()), Some("XML"));
    assert_eq!(
        text(cx, &view),
        ("<r>\n  <a>1</a>\n</r>".into(), "html".into())
    );
    assert!(view.read_with(cx, |v, _| v.can_copy()));
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::RgbaImage::new(w, h)
        .write_to(&mut out, image::ImageFormat::Png)
        .unwrap();
    out.into_inner()
}

#[gpui_kit::test]
fn an_image_body_shows_a_preview_and_its_caption(cx: &mut TestAppContext) {
    let bytes = png(3, 2);
    let s = send_once(
        cx,
        ResponseTemplate::new(200).set_body_raw(bytes.clone(), "image/png"),
        "",
    );
    let (win, view) = host(cx);
    show(cx, win, &view, &s);
    let info = ImageInfo {
        kind: ImageKind::Png,
        size: bytes.len() as u64,
        dimensions: Some((3, 2)),
    };
    assert_eq!(
        view.read_with(cx, |v, _| (
            v.shown(),
            v.format_label(),
            v.can_copy(),
            v.can_save()
        )),
        (Shown::Image(info), Some("PNG"), false, true)
    );
    assert!(present(cx, win, "body-image"));
    assert!(present(cx, win, "body-image-caption"));
    assert!(!present(cx, win, "body-pretty"));
    assert!(!present(cx, win, "body-editor"));
}

#[gpui_kit::test]
fn an_svg_body_is_an_image(cx: &mut TestAppContext) {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="12"><rect width="10" height="12"/></svg>"#;
    let s = send_once(
        cx,
        ResponseTemplate::new(200).set_body_raw(svg, "image/svg+xml"),
        "",
    );
    let (win, view) = host(cx);
    show(cx, win, &view, &s);
    assert_eq!(
        view.read_with(cx, |v, _| v.shown()),
        Shown::Image(ImageInfo {
            kind: ImageKind::Svg,
            size: svg.len() as u64,
            dimensions: Some((10, 12)),
        })
    );
    assert!(present(cx, win, "body-image"));
}

#[gpui_kit::test]
fn a_saved_image_is_its_raw_bytes(cx: &mut TestAppContext) {
    let bytes = png(2, 2);
    let s = send_once(
        cx,
        ResponseTemplate::new(200).set_body_raw(bytes.clone(), "image/png"),
        "",
    );
    let (win, view) = host(cx);
    show(cx, win, &view, &s);
    let out = tempfile::tempdir().unwrap();
    let target = out.path().join("image.png");
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click("body-save", cx)
    })
    .unwrap();
    cx.run_until_parked();
    let chosen = target.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.run_until_parked();
    assert_eq!(std::fs::read(&target).unwrap(), bytes);
}

#[gpui_kit::test]
fn an_image_then_json_shows_the_editor(cx: &mut TestAppContext) {
    let image = send_once(
        cx,
        ResponseTemplate::new(200).set_body_raw(png(2, 2), "image/png"),
        "",
    );
    let json = send_once(
        cx,
        ResponseTemplate::new(200).set_body_raw("{\"a\":1}", "application/json"),
        "",
    );
    let (win, view) = host(cx);
    show(cx, win, &view, &image);
    assert!(present(cx, win, "body-image"));
    show(cx, win, &view, &json);
    assert_eq!(view.read_with(cx, |v, _| v.shown()), Shown::Editor);
    assert!(present(cx, win, "body-editor"));
    assert!(!present(cx, win, "body-image"));
    assert!(!present(cx, win, "body-image-caption"));
}

#[gpui_kit::test]
fn find_opens_no_bar_on_an_image(cx: &mut TestAppContext) {
    let s = send_once(
        cx,
        ResponseTemplate::new(200).set_body_raw(png(2, 2), "image/png"),
        "",
    );
    let (win, view) = host(cx);
    show(cx, win, &view, &s);
    cx.update_window(win, |_, w, cx| view.update(cx, |v, cx| v.open_find(w, cx)))
        .unwrap();
    assert!(!view.read_with(cx, |v, _| v.find_open()));
    assert!(!present(cx, win, "body-find"));
}
