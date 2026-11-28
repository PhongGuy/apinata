//! An HTML body laid out by the kit's renderer, which has no script engine; script, style and
//! title elements are removed first, since the kit shows their text in some places. The page
//! is untrusted, so its links open nothing, only raster images carried inline as `data:` URLs
//! are shown, and no other URL ever reaches a loader.

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
    // Raster formats only: an SVG can name local files for its decoder to read.
    let format = ImageFormat::from_mime_type(&mime).filter(|f| {
        matches!(
            f,
            ImageFormat::Png
                | ImageFormat::Jpeg
                | ImageFormat::Gif
                | ImageFormat::Webp
                | ImageFormat::Bmp
        )
    })?;
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

/// Deepest tag nesting the preview lays out. The kit parses with recursion several frames deep
/// per level on a background thread; 256 levels stay far inside a 2 MiB stack.
pub const MAX_DEPTH: usize = 256;

// Void elements, and elements whose end tag the parser implies when a sibling starts; neither
// adds a level, so pages that never close their `<p>` or `<li>` still preview.
const FLAT: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr", "p", "li", "dt", "dd", "tr", "td", "th", "option", "optgroup", "thead", "tbody",
    "tfoot", "colgroup", "rt", "rp",
];

// Elements whose text is never page content, removed with everything inside them.
const DROPPED: &[&str] = &["script", "style", "title"];

/// The HTML the preview lays out: `html` without its script, style and title elements, or
/// `None` when its tags nest deeper than [`MAX_DEPTH`]. One pass over the text, no recursion:
/// a stack of open names, where a close tag pops back to its own name and is otherwise
/// ignored, as the parser does, and a self-closing `/>` still opens an HTML element.
pub fn prepare(html: &str) -> Option<String> {
    // ASCII lowercasing keeps every byte offset, so names and end tags are matched in `lower`.
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut open: Vec<&str> = Vec::new();
    let (mut at, mut kept) = (0, 0);
    while let Some(found) = lower[at..].find('<') {
        let lt = at + found;
        let rest = &lower[lt + 1..];
        if rest.starts_with("!--") {
            at = rest.find("-->").map_or(lower.len(), |end| lt + 1 + end + 3);
            continue;
        }
        let closing = rest.starts_with('/');
        let start = lt + 1 + usize::from(closing);
        let len = lower[start..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .unwrap_or(lower.len() - start);
        let name = &lower[start..start + len];
        if !name.starts_with(|c: char| c.is_ascii_alphabetic()) {
            at = lt + 1;
            continue;
        }
        let end = lower[start..]
            .find('>')
            .map_or(lower.len(), |gt| start + gt + 1);
        if !closing && DROPPED.contains(&name) {
            // Everything up to its end tag goes; one that never closes takes the rest.
            let stop = lower[end..]
                .find(&format!("</{name}"))
                .map_or(lower.len(), |close| {
                    let close = end + close;
                    lower[close..]
                        .find('>')
                        .map_or(lower.len(), |gt| close + gt + 1)
                });
            out.push_str(&html[kept..lt]);
            (kept, at) = (stop, stop);
            continue;
        }
        if FLAT.contains(&name) {
            at = end;
            continue;
        }
        if closing {
            if let Some(ix) = open.iter().rposition(|n| *n == name) {
                open.truncate(ix);
            }
        } else {
            open.push(name);
            if open.len() > MAX_DEPTH {
                return None;
            }
        }
        at = end;
    }
    out.push_str(&html[kept..]);
    Some(out)
}
