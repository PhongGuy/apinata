//! Decodes untrusted image bodies for the preview under fixed caps. A raster's header is
//! checked before any pixel is decoded, an animation is counted frame by frame, and an SVG
//! is drawn with every external or embedded reference refused, so no local file is opened.

use std::io::Cursor;
use std::sync::{Arc, LazyLock};

use gpui_kit::RenderImage;
use image::ImageDecoder as _;
use resvg::{tiny_skia, usvg};

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

// An SVG is drawn no larger than this on either side, scaled down to fit.
const SVG_MAX_SIDE: f32 = 4096.0;

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
            Refusal::Undecodable => None,
        }
    }
}

/// Checks a raster's declared size against the caps without decoding pixels. An SVG always
/// passes, since it is drawn scaled down to a fixed size.
pub fn check(kind: ImageKind, bytes: &[u8], limits: &ImageLimits) -> Result<(), Refusal> {
    let Some(format) = raster_format(kind) else {
        return Ok(());
    };
    let decoder = image::ImageReader::with_format(Cursor::new(bytes), format)
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

/// Decodes an image body into the BGRA frames gpui paints, refusing anything over `limits`.
pub fn decode_preview(
    kind: ImageKind,
    bytes: &[u8],
    limits: &ImageLimits,
) -> Result<Arc<RenderImage>, Refusal> {
    let frames = match raster_format(kind) {
        None => vec![draw_svg(bytes)?],
        Some(format) => {
            check(kind, bytes, limits)?;
            decode_raster(format, bytes, limits)?
        }
    };
    Ok(Arc::new(RenderImage::new(frames)))
}

fn raster_format(kind: ImageKind) -> Option<image::ImageFormat> {
    Some(match kind {
        ImageKind::Png => image::ImageFormat::Png,
        ImageKind::Jpeg => image::ImageFormat::Jpeg,
        ImageKind::Gif => image::ImageFormat::Gif,
        ImageKind::Webp => image::ImageFormat::WebP,
        ImageKind::Bmp => image::ImageFormat::Bmp,
        ImageKind::Svg => return None,
    })
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

// System fonts are loaded once, on the first SVG, for its text.
static FONTS: LazyLock<Arc<usvg::fontdb::Database>> = LazyLock::new(|| {
    let mut db = usvg::fontdb::Database::new();
    db.load_system_fonts();
    Arc::new(db)
});

// Draws an SVG with no base directory and an `<image>` resolver that refuses every href, a
// local path, a URL or an embedded data: image alike, then scales it to fit the size cap.
fn draw_svg(bytes: &[u8]) -> Result<image::Frame, Refusal> {
    let options = usvg::Options {
        resources_dir: None,
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        fontdb: FONTS.clone(),
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_data(bytes, &options).map_err(|_| Refusal::Undecodable)?;
    let size = tree.size();
    let scale = (SVG_MAX_SIDE / size.width())
        .min(SVG_MAX_SIDE / size.height())
        .min(1.0);
    let width = (size.width() * scale).ceil().max(1.0) as u32;
    let height = (size.height() * scale).ceil().max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or(Refusal::Undecodable)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    // tiny-skia keeps premultiplied RGBA; gpui wants straight alpha.
    let pixels: Vec<u8> = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    let buffer = image::RgbaImage::from_raw(width, height, pixels).ok_or(Refusal::Undecodable)?;
    Ok(bgra(image::Frame::new(buffer)))
}
