//! Decodes untrusted raster images (PNG, JPEG, GIF, WebP, BMP) for the preview under fixed
//! caps. The header is checked before any pixel is decoded, an animation is counted frame by
//! frame, and a decoder panic becomes a refusal. SVG is never decoded here.

use std::io::Cursor;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use gpui_kit::RenderImage;
use image::ImageDecoder as _;

use crate::body_text::ImageKind;

/// Caps on what the preview decodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageLimits {
    /// The longest side, in pixels.
    pub max_side: u32,
    /// Width times height.
    pub max_pixels: u64,
    /// Decoded bytes across every frame.
    pub max_total_bytes: u64,
    /// Frames in an animation.
    pub max_frames: usize,
}

/// The limits the response preview uses.
pub const PREVIEW_LIMITS: ImageLimits = ImageLimits {
    max_side: 16_384,
    max_pixels: 40_000_000,
    max_total_bytes: 256 * 1024 * 1024,
    max_frames: 2_000,
};

/// Why an image was not decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The declared size is over the caps.
    TooLarge {
        /// Declared width.
        width: u32,
        /// Declared height.
        height: u32,
    },
    /// The animation has more frames or decoded bytes than the caps allow.
    TooManyFrames,
    /// The bytes do not decode.
    Undecodable,
    /// The decoder panicked on the bytes.
    Crashed,
}

impl Refusal {
    /// The note the preview shows in place of the image, or `None` when it has its own.
    pub fn note(self, limits: &ImageLimits) -> Option<String> {
        match self {
            Refusal::TooLarge { width, height }
                if width > limits.max_side || height > limits.max_side =>
            {
                Some(format!(
                    "Image not shown: {width} × {height} is over the {}-pixel side limit.",
                    limits.max_side
                ))
            }
            Refusal::TooLarge { width, height } => Some(format!(
                "Image not shown: {width} × {height} is over the {} megapixel limit.",
                limits.max_pixels / 1_000_000
            )),
            Refusal::TooManyFrames => Some(format!(
                "Image not shown: the animation is over the {} frame or {} MB limit.",
                limits.max_frames,
                limits.max_total_bytes / (1024 * 1024)
            )),
            Refusal::Crashed => Some("Image not shown: its decoder failed on these bytes.".into()),
            Refusal::Undecodable => None,
        }
    }
}

/// Checks a raster's declared size against the caps without decoding pixels.
pub fn check(kind: ImageKind, bytes: &[u8], limits: &ImageLimits) -> Result<(), Refusal> {
    let decoder = image::ImageReader::with_format(Cursor::new(bytes), raster_format(kind))
        .into_decoder()
        .map_err(|_| Refusal::Undecodable)?;
    let (width, height) = decoder.dimensions();
    let over = width > limits.max_side
        || height > limits.max_side
        || u64::from(width) * u64::from(height) > limits.max_pixels;
    if over {
        return Err(Refusal::TooLarge { width, height });
    }
    Ok(())
}

/// Decodes untrusted raster bytes, such as a response body or a `data:` image, into the
/// BGRA frames gpui paints. Anything over `limits` is refused before it is decoded, and a
/// panic in the decoder comes back as [`Refusal::Crashed`]. Run it off the main thread.
pub fn decode_preview(
    kind: ImageKind,
    bytes: &[u8],
    limits: &ImageLimits,
) -> Result<Arc<RenderImage>, Refusal> {
    contained(|| {
        check(kind, bytes, limits)?;
        let frames = decode_raster(raster_format(kind), bytes, limits)?;
        Ok(Arc::new(RenderImage::new(frames)))
    })
}

/// Runs a decode, turning a panic inside it into [`Refusal::Crashed`].
pub fn contained(
    decode: impl FnOnce() -> Result<Arc<RenderImage>, Refusal>,
) -> Result<Arc<RenderImage>, Refusal> {
    // The payload is dropped unread: it may quote the body.
    catch_unwind(AssertUnwindSafe(decode)).unwrap_or(Err(Refusal::Crashed))
}

fn raster_format(kind: ImageKind) -> image::ImageFormat {
    match kind {
        ImageKind::Png => image::ImageFormat::Png,
        ImageKind::Jpeg => image::ImageFormat::Jpeg,
        ImageKind::Gif => image::ImageFormat::Gif,
        ImageKind::Webp => image::ImageFormat::WebP,
        ImageKind::Bmp => image::ImageFormat::Bmp,
    }
}

// The decoder's own limits back up the header check, so a frame larger than the canvas it
// claims is refused before its buffer is allocated.
fn decoder_limits(limits: &ImageLimits) -> image::Limits {
    let mut l = image::Limits::default();
    l.max_image_width = Some(limits.max_side);
    l.max_image_height = Some(limits.max_side);
    l.max_alloc = Some(limits.max_total_bytes);
    l
}

fn decode_raster(
    format: image::ImageFormat,
    bytes: &[u8],
    limits: &ImageLimits,
) -> Result<Vec<image::Frame>, Refusal> {
    let undecodable = |_| Refusal::Undecodable;
    match format {
        image::ImageFormat::Gif => {
            let mut decoder =
                image::codecs::gif::GifDecoder::new(Cursor::new(bytes)).map_err(undecodable)?;
            decoder
                .set_limits(decoder_limits(limits))
                .map_err(undecodable)?;
            counted_frames(decoder, limits)
        }
        image::ImageFormat::WebP => {
            let mut decoder =
                image::codecs::webp::WebPDecoder::new(Cursor::new(bytes)).map_err(undecodable)?;
            decoder
                .set_limits(decoder_limits(limits))
                .map_err(undecodable)?;
            if decoder.has_animation() {
                let _ = decoder.set_background_color(image::Rgba([0, 0, 0, 0]));
                counted_frames(decoder, limits)
            } else {
                still(decoder)
            }
        }
        _ => {
            let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
            reader.limits(decoder_limits(limits));
            still(reader.into_decoder().map_err(undecodable)?)
        }
    }
}

fn still(mut decoder: impl image::ImageDecoder) -> Result<Vec<image::Frame>, Refusal> {
    let orientation = decoder.orientation().map_err(|_| Refusal::Undecodable)?;
    let mut image = image::DynamicImage::from_decoder(decoder).map_err(|_| Refusal::Undecodable)?;
    image.apply_orientation(orientation);
    Ok(vec![bgra(image::Frame::new(image.into_rgba8()))])
}

// Every frame is a full canvas, so the running total is checked before the next one is
// kept. A frame that fails ends the animation at the frames decoded so far.
fn counted_frames<'a>(
    decoder: impl image::AnimationDecoder<'a>,
    limits: &ImageLimits,
) -> Result<Vec<image::Frame>, Refusal> {
    let mut frames = Vec::new();
    let mut total = 0u64;
    for frame in decoder.into_frames() {
        let Ok(frame) = frame else { break };
        total += frame.buffer().as_raw().len() as u64;
        if frames.len() == limits.max_frames || total > limits.max_total_bytes {
            return Err(Refusal::TooManyFrames);
        }
        frames.push(bgra(frame));
    }
    if frames.is_empty() {
        return Err(Refusal::Undecodable);
    }
    Ok(frames)
}

// gpui paints BGRA; the decoders give RGBA.
fn bgra(mut frame: image::Frame) -> image::Frame {
    for pixel in frame.buffer_mut().as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    frame
}
