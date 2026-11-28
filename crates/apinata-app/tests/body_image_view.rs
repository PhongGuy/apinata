//! Image bodies: Fit by default, 100% on request with scrolling both ways, and a
//! checkerboard only behind images that can be transparent.

use crate::support;

use std::sync::Arc;

use apinata_app::body_view::{BodyView, ImageScale};
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Entity, Pixels, ScrollDelta, TestAppContext,
    WindowOptions, point, px,
};
use support::*;

fn encode(image: image::DynamicImage, format: image::ImageFormat) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    image.write_to(&mut out, format).unwrap();
    out.into_inner()
}

fn rgba_png(w: u32, h: u32) -> Vec<u8> {
    encode(
        image::DynamicImage::ImageRgba8(image::RgbaImage::new(w, h)),
        image::ImageFormat::Png,
    )
}

fn opaque(w: u32, h: u32, format: image::ImageFormat) -> Vec<u8> {
    encode(
        image::DynamicImage::ImageRgb8(image::RgbImage::new(w, h)),
        format,
    )
}

fn result(body: Vec<u8>, content_type: &str) -> Arc<RequestResult> {
    let r = Response::new(200)
        .with_header("content-type", content_type)
        .with_body(body, "");
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

fn bounds(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> Bounds<Pixels> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).bounds()
    })
    .unwrap()
}

fn scale(cx: &mut TestAppContext, view: &Entity<BodyView>) -> ImageScale {
    view.read_with(cx, |v, _| v.image_scale())
}

#[gpui_kit::test]
fn an_image_fits_by_default_and_shows_its_own_pixels_at_100_percent(cx: &mut TestAppContext) {
    let (win, view) = host(cx);
    show(cx, win, &view, result(rgba_png(4000, 3000), "image/png"));
    assert_eq!(scale(cx, &view), ImageScale::Fit);
    assert!(present(cx, win, "body-image-fit") && present(cx, win, "body-image-actual"));
    let area = bounds(cx, win, "body-image");
    let fitted = bounds(cx, win, "body-image-frame");
    assert!(fitted.size.width <= area.size.width && fitted.size.height <= area.size.height);

    click(cx, win, "body-image-actual");
    assert_eq!(scale(cx, &view), ImageScale::Actual);
    let full = bounds(cx, win, "body-image-frame");
    assert_eq!(full.origin, area.origin);
    assert!(full.size.width > area.size.width && full.size.height > area.size.height);
    // The full-size image scrolls in both directions inside the same area, one axis a notch.
    for delta in [point(px(0.), px(-60.)), point(px(-50.), px(0.))] {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.scroll("body-image", ScrollDelta::Pixels(delta), cx)
        })
        .unwrap();
        cx.run_until_parked();
    }
    let moved = bounds(cx, win, "body-image-frame");
    assert!(moved.origin.x < full.origin.x && moved.origin.y < full.origin.y);

    // The choice holds for the next image, and Fit brings the image back inside the area.
    show(cx, win, &view, result(rgba_png(3000, 2000), "image/png"));
    assert_eq!(scale(cx, &view), ImageScale::Actual);
    click(cx, win, "body-image-fit");
    assert_eq!(scale(cx, &view), ImageScale::Fit);
    let fitted = bounds(cx, win, "body-image-frame");
    assert!(fitted.size.width <= area.size.width);
}

#[gpui_kit::test]
fn only_images_offer_the_scale_toggle(cx: &mut TestAppContext) {
    let (win, view) = host(cx);
    show(cx, win, &view, result(b"{}".to_vec(), "application/json"));
    assert!(!present(cx, win, "body-image-fit"));
    show(
        cx,
        win,
        &view,
        result(vec![0xff, 0, 1], "application/octet-stream"),
    );
    assert!(!present(cx, win, "body-image-fit"));
    assert!(present(cx, win, "body-hex"));
}

#[gpui_kit::test]
fn the_checkerboard_is_drawn_only_behind_images_that_can_be_transparent(cx: &mut TestAppContext) {
    let (win, view) = host(cx);
    show(cx, win, &view, result(rgba_png(8, 8), "image/png"));
    assert!(present(cx, win, "body-image-checker"));
    show(
        cx,
        win,
        &view,
        result(opaque(8, 8, image::ImageFormat::Png), "image/png"),
    );
    assert!(present(cx, win, "body-image") && !present(cx, win, "body-image-checker"));
    show(
        cx,
        win,
        &view,
        result(opaque(8, 8, image::ImageFormat::Jpeg), "image/jpeg"),
    );
    assert!(present(cx, win, "body-image") && !present(cx, win, "body-image-checker"));
}

#[gpui_kit::test]
fn an_image_over_the_preview_limit_shows_a_note_instead_of_the_picture(cx: &mut TestAppContext) {
    let (win, view) = host(cx);
    // One side past the cap; the bytes stay tiny, so nothing large is decoded either way.
    let wide = rgba_png(apinata_app::image_guard::PREVIEW_LIMITS.max_side + 1, 1);
    show(cx, win, &view, result(wide, "image/png"));
    assert!(present(cx, win, "body-image-refused"));
    assert!(!present(cx, win, "body-image-frame"));
    let note = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find("body-image-refused").label().map(str::to_owned)
        })
        .unwrap();
    assert_eq!(
        note.as_deref(),
        Some("Image not shown: 16385 × 1 is over the 16384-pixel side limit.")
    );

    // A body within the caps is drawn again.
    show(cx, win, &view, result(rgba_png(8, 8), "image/png"));
    assert!(present(cx, win, "body-image-frame"));
    assert!(!present(cx, win, "body-image-refused"));
}
