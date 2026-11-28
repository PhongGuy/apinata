//! An HTML body laid out by the kit's renderer, which has no script engine and drops `<style>`
//! and `<script>`. The page is untrusted, so its links open nothing and only images it carries
//! inline as `data:` URLs are shown; no other URL ever reaches a loader.

use std::sync::Arc;

use gpui_kit::component::text::TextView;
use gpui_kit::{ElementId, Image, ImageFormat, ImageSource, SharedString, SharedUri};

/// Where a previewed image comes from: a `data:` image decoded in place, and for any other
/// URL (remote, `file:`, relative) an empty image that fails to decode, so nothing is fetched.
pub fn image_source(uri: &SharedUri) -> ImageSource {
    let (format, bytes) = data_image(uri).unwrap_or((ImageFormat::Png, Vec::new()));
    ImageSource::Image(Arc::new(Image::from_bytes(format, bytes)))
}

// The format and bytes of a `data:<mime>[;base64],<data>` image, or `None` for any other URL.
fn data_image(uri: &str) -> Option<(ImageFormat, Vec<u8>)> {
    let rest = uri
        .get(..5)?
        .eq_ignore_ascii_case("data:")
        .then(|| &uri[5..])?;
    let (header, data) = rest.split_once(',')?;
    let mime = header.split(';').next()?.trim().to_ascii_lowercase();
    let format = ImageFormat::from_mime_type(&mime)?;
    let base64 = (header.split(';').skip(1)).any(|p| p.trim().eq_ignore_ascii_case("base64"));
    let bytes = if base64 {
        use base64::Engine as _;
        let packed: String = data.chars().filter(|c| !c.is_ascii_whitespace()).collect();
        base64::engine::general_purpose::STANDARD
            .decode(packed)
            .ok()?
    } else {
        percent_encoding::percent_decode_str(data).collect()
    };
    Some((format, bytes))
}

/// The preview of `html`, scrolling in its own list. Links are inert, since a page's own
/// links could hand any scheme to the system opener.
pub fn view(id: impl Into<ElementId>, html: SharedString) -> TextView {
    TextView::html(id, html)
        .scrollable(true)
        .image_source(image_source)
        .on_link_click(|_, _, _, _| {})
}
