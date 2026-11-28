//! Raster image bodies are decoded by the app under limits: the header is checked against
//! the caps before any pixel is decoded, and a decoder panic becomes a refusal.

use apinata_app::body_text::ImageKind;
use apinata_app::image_guard::{self, ImageLimits, PREVIEW_LIMITS, Refusal};

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

// An animated WebP of `frames` 1x1 frames, each a copy of one lossless 1x1 still.
fn animated_webp(frames: usize) -> Vec<u8> {
    let mut still = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut still)
        .encode(&[255, 0, 0, 255], 1, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    // The still is RIFF, size, WEBP, then one VP8L chunk; that chunk is each frame's payload.
    let vp8l = &still[12..];
    let chunk = |tag: &[u8], body: &[u8]| {
        let mut c = tag.to_vec();
        c.extend_from_slice(&(body.len() as u32).to_le_bytes());
        c.extend_from_slice(body);
        if body.len() % 2 == 1 {
            c.push(0);
        }
        c
    };
    let mut body = b"WEBP".to_vec();
    // VP8X: animation and alpha flags, then a 1x1 canvas as width-1 and height-1.
    body.extend(chunk(b"VP8X", &[0x12, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
    body.extend(chunk(b"ANIM", &[0, 0, 0, 0, 0, 0]));
    for _ in 0..frames {
        // Offset 0,0, size 1x1 (stored minus one), 100 ms, no blending.
        let mut frame = vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 100, 0, 0, 0x02];
        frame.extend_from_slice(vp8l);
        body.extend(chunk(b"ANMF", &frame));
    }
    let mut riff = b"RIFF".to_vec();
    riff.extend_from_slice(&(body.len() as u32).to_le_bytes());
    riff.extend(body);
    riff
}

#[test]
fn an_animated_webp_with_more_frames_than_allowed_is_refused() {
    let limits = ImageLimits {
        max_frames: 3,
        ..PREVIEW_LIMITS
    };
    let three = image_guard::decode_preview(ImageKind::Webp, &animated_webp(3), &limits);
    assert_eq!(three.map(|i| i.frame_count()).ok(), Some(3));
    assert!(matches!(
        image_guard::decode_preview(ImageKind::Webp, &animated_webp(4), &limits),
        Err(Refusal::TooManyFrames)
    ));
}

#[test]
fn a_decoder_panic_is_contained_as_a_refusal() {
    let decoded = image_guard::contained(|| panic!("a decoder bug"));
    assert_eq!(decoded.err(), Some(Refusal::Crashed));
    assert!(Refusal::Crashed.note(&PREVIEW_LIMITS).is_some());
}
