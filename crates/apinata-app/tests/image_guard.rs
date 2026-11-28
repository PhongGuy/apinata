//! Image bodies are decoded by the app under limits: an SVG never reaches a local file, and
//! a raster's header is checked against the caps before any pixel is decoded.

use std::sync::Arc;
use std::time::Duration;

use apinata_app::body_text::ImageKind;
use apinata_app::image_guard::{self, ImageLimits, PREVIEW_LIMITS, Refusal};
use base64::Engine as _;

fn png(image: image::RgbaImage) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut out, image::ImageFormat::Png)
        .unwrap();
    out.into_inner()
}

fn red_png() -> Vec<u8> {
    png(image::RgbaImage::from_pixel(
        4,
        4,
        image::Rgba([255, 0, 0, 255]),
    ))
}

// A 4x4 SVG drawing each href as a full-size image.
fn svg_with_images(hrefs: &[&str]) -> Vec<u8> {
    let images: String = hrefs
        .iter()
        .map(|h| format!(r#"<image href="{h}" x="0" y="0" width="4" height="4"/>"#))
        .collect();
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4">{images}</svg>"#)
        .into_bytes()
}

fn all_clear(image: &gpui_kit::RenderImage) -> bool {
    image
        .as_bytes(0)
        .unwrap()
        .as_chunks::<4>()
        .0
        .iter()
        .all(|p| p[3] == 0)
}

// A GIF whose logical screen is `w` x `h` holding one 1x1 frame: a few dozen bytes.
fn gif_declaring(w: u16, h: u16) -> Vec<u8> {
    let mut gif = b"GIF89a".to_vec();
    gif.extend_from_slice(&w.to_le_bytes());
    gif.extend_from_slice(&h.to_le_bytes());
    gif.extend_from_slice(&[0x80, 0, 0]);
    gif.extend_from_slice(&[0, 0, 0, 255, 255, 255]);
    gif.extend_from_slice(&[0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0]);
    gif.extend_from_slice(&[2, 2, 0x44, 0x01, 0, 0x3B]);
    gif
}

#[test]
fn a_gif_declaring_a_huge_canvas_is_refused_from_its_header() {
    // Only the header check runs, so this stays small even if the check were missing.
    assert_eq!(
        image_guard::check(
            ImageKind::Gif,
            &gif_declaring(65535, 65535),
            &PREVIEW_LIMITS
        ),
        Err(Refusal::TooLarge {
            width: 65535,
            height: 65535
        })
    );
}

#[test]
fn a_raster_within_the_caps_passes_the_header_check() {
    assert_eq!(
        image_guard::check(ImageKind::Gif, &gif_declaring(64, 64), &PREVIEW_LIMITS),
        Ok(())
    );
}

#[test]
fn one_side_over_the_cap_is_refused_even_with_few_pixels() {
    let wide = png(image::RgbaImage::new(PREVIEW_LIMITS.max_side + 1, 1));
    assert!(matches!(
        image_guard::check(ImageKind::Png, &wide, &PREVIEW_LIMITS),
        Err(Refusal::TooLarge { .. })
    ));
}

#[test]
fn a_gif_with_more_frames_than_allowed_is_refused() {
    let mut bytes = Vec::new();
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
        let frames = (0..4).map(|_| image::Frame::new(image::RgbaImage::new(8, 8)));
        encoder.encode_frames(frames).unwrap();
    }
    let limits = ImageLimits {
        max_frames: 3,
        ..PREVIEW_LIMITS
    };
    assert!(matches!(
        image_guard::decode_preview(ImageKind::Gif, &bytes, &limits),
        Err(Refusal::TooManyFrames)
    ));
}

#[test]
fn a_png_decodes_to_the_bgra_order_gpui_paints() {
    let image = image_guard::decode_preview(ImageKind::Png, &red_png(), &PREVIEW_LIMITS).unwrap();
    assert_eq!(&image.as_bytes(0).unwrap()[..4], &[0, 0, 255, 255]);
}

#[test]
fn an_svg_is_rasterized_at_its_own_size() {
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="6" height="3"><rect width="6" height="3" fill="red"/></svg>"#;
    let image = image_guard::decode_preview(ImageKind::Svg, svg, &PREVIEW_LIMITS).unwrap();
    let size = image.size(0);
    assert_eq!((size.width.0, size.height.0), (6, 3));
    assert_eq!(&image.as_bytes(0).unwrap()[..4], &[0, 0, 255, 255]);
}

#[test]
fn an_svg_never_draws_a_local_file_it_names() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("red.png");
    std::fs::write(&file, red_png()).unwrap();
    let svg = svg_with_images(&["/etc/hostname", file.to_str().unwrap()]);
    let image = image_guard::decode_preview(ImageKind::Svg, &svg, &PREVIEW_LIMITS).unwrap();
    assert!(all_clear(&image), "a local file named by the SVG was drawn");
}

#[test]
fn an_svg_does_not_draw_an_embedded_data_image() {
    let data = base64::engine::general_purpose::STANDARD.encode(red_png());
    let svg = svg_with_images(&[&format!("data:image/png;base64,{data}")]);
    let image = image_guard::decode_preview(ImageKind::Svg, &svg, &PREVIEW_LIMITS).unwrap();
    assert!(all_clear(&image), "an embedded raster was decoded");
}

#[cfg(unix)]
#[test]
fn an_svg_naming_a_fifo_does_not_block() {
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("pipe.png");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    assert!(made.success());
    let svg = Arc::new(svg_with_images(&[fifo.to_str().unwrap()]));
    // A reader blocked on the FIFO never answers; the wait is bounded so the test fails
    // instead of hanging.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(image_guard::decode_preview(ImageKind::Svg, &svg, &PREVIEW_LIMITS).is_ok());
    });
    assert_eq!(rx.recv_timeout(Duration::from_secs(10)), Ok(true));
}
