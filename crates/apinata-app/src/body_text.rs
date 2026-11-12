//! Turns a response body into what the body view shows: its kind, a
//! pretty or raw text, and either an editor text or plain rows when the
//! text is too large for the editor.

use std::time::Duration;

use gpui_kit::{Pixels, SharedString};
use quick_xml::events::{BytesText, Event};

/// Largest text, in bytes, shown in the highlighted editor.
pub const EDITOR_MAX_BYTES: usize = 262_144;
/// Most lines shown in the highlighted editor.
pub const EDITOR_MAX_LINES: usize = 20_000;
/// Widest plain row, in bytes; longer lines wrap onto further rows.
pub const ROW_WIDTH: usize = 1_000;

/// What a body holds, which decides how it is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyKind {
    /// No bytes.
    Empty,
    /// The content type names JSON.
    Json,
    /// The content type names HTML.
    Html,
    /// The content type names XML.
    Xml,
    /// Any other text.
    Text,
    /// Invalid UTF-8 with no charset named.
    Binary,
    /// The content type names a previewable image format.
    Image(ImageKind),
}

/// An image format the body view can preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    /// `image/png`.
    Png,
    /// `image/jpeg` or `image/jpg`.
    Jpeg,
    /// `image/gif`.
    Gif,
    /// `image/webp`.
    Webp,
    /// `image/svg+xml`.
    Svg,
    /// `image/bmp`.
    Bmp,
}

impl ImageKind {
    /// The format's display name: `PNG`, `JPEG`, `GIF`, `WebP`, `SVG`, `BMP`.
    pub fn label(self) -> &'static str {
        match self {
            ImageKind::Png => "PNG",
            ImageKind::Jpeg => "JPEG",
            ImageKind::Gif => "GIF",
            ImageKind::Webp => "WebP",
            ImageKind::Svg => "SVG",
            ImageKind::Bmp => "BMP",
        }
    }

    /// The kind a media type (no parameters, lowercased) names, if any.
    fn from_media_type(media: &str) -> Option<Self> {
        Some(match media {
            "image/png" => ImageKind::Png,
            "image/jpeg" | "image/jpg" => ImageKind::Jpeg,
            "image/gif" => ImageKind::Gif,
            "image/webp" => ImageKind::Webp,
            "image/svg+xml" => ImageKind::Svg,
            "image/bmp" => ImageKind::Bmp,
            _ => return None,
        })
    }
}

/// An image body's format, size in bytes and pixel dimensions when known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageInfo {
    /// The format named by the content type.
    pub kind: ImageKind,
    /// The body size in bytes.
    pub size: u64,
    /// Width and height in pixels, when the bytes say.
    pub dimensions: Option<(u32, u32)>,
}

/// Classifies a body from its `Content-Type` header and its bytes.
pub fn body_kind(headers: &[(String, String)], body: &[u8]) -> BodyKind {
    if body.is_empty() {
        return BodyKind::Empty;
    }
    let content_type = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.to_ascii_lowercase())
        .unwrap_or_default();
    // Images go by the content type alone, so bytes that happen to decode
    // as UTF-8 never turn an image into text.
    let media = content_type.split(';').next().unwrap_or_default().trim();
    if let Some(image) = ImageKind::from_media_type(media) {
        return BodyKind::Image(image);
    }
    // An error with no length is an incomplete final character, which the
    // body cap can cut, so only an invalid sequence before it means binary.
    let invalid = std::str::from_utf8(body).is_err_and(|e| e.error_len().is_some());
    if invalid && !content_type.contains("charset=") {
        BodyKind::Binary
    } else if content_type.contains("json") {
        BodyKind::Json
    } else if content_type.contains("html") {
        BodyKind::Html
    } else if content_type.contains("xml") {
        BodyKind::Xml
    } else {
        BodyKind::Text
    }
}

pub use apinata_core::json_text::{beautify_json, minify_json, reindent_json};

/// Re-indents XML by two spaces per element, dropping whitespace-only text
/// and keeping comments, CDATA, PIs and the DOCTYPE; `None` when malformed.
/// Mixed content and `xml:space="preserve"` stay inline, whitespace intact.
pub fn reindent_xml(text: &str) -> Option<String> {
    xml_layout(text).ok()
}

/// Like [`reindent_xml`], for a request body: `Err` is a short message
/// naming the line and column where the text stops being well-formed.
/// Whitespace in mixed content or under `xml:space="preserve"` is kept as is.
pub fn beautify_xml(text: &str) -> Result<String, String> {
    xml_layout(text)
}

/// Like [`beautify_xml`] but writes no layout at all: whitespace-only text
/// between elements goes, but not in mixed content or under
/// `xml:space="preserve"`; text, comments and CDATA stay as written.
pub fn minify_xml(text: &str) -> Result<String, String> {
    xml_layout_with(text, false)
}

fn xml_layout(text: &str) -> Result<String, String> {
    xml_layout_with(text, true)
}

// Per element in document order: whether its whitespace-only text is
// meaningful, i.e. it holds non-blank text or an entity reference (mixed
// content) or `xml:space="preserve"` is in force. Errors are left to the
// main pass, which reports them.
fn xml_keep_whitespace(text: &str) -> Vec<bool> {
    let mut reader = quick_xml::Reader::from_str(text);
    let mut keep = Vec::new();
    // Open elements as (index into `keep`, preserve in force).
    let mut stack: Vec<(usize, bool)> = Vec::new();
    fn open(
        e: &quick_xml::events::BytesStart,
        stack: &mut Vec<(usize, bool)>,
        keep: &mut Vec<bool>,
    ) {
        let mut preserve = stack.last().is_some_and(|s| s.1);
        for a in e.attributes().flatten() {
            if a.key.as_ref() == b"xml:space" {
                match a.value.as_ref() {
                    b"preserve" => preserve = true,
                    b"default" => preserve = false,
                    _ => {}
                }
            }
        }
        keep.push(preserve);
        stack.push((keep.len() - 1, preserve));
    }
    // `sibling_closed`: the previous event ended an element; `gap`: an inline
    // space followed it and the next element start would make it meaningful.
    let (mut sibling_closed, mut gap) = (false, false);
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) => {
                if gap {
                    keep[stack.last().map_or(0, |s| s.0)] = true;
                }
                gap = false;
                open(&e, &mut stack, &mut keep);
            }
            Event::End(_) => {
                stack.pop();
                gap = false;
                sibling_closed = true;
                continue;
            }
            Event::Empty(e) => {
                if gap {
                    keep[stack.last().map_or(0, |s| s.0)] = true;
                }
                gap = false;
                open(&e, &mut stack, &mut keep);
                stack.pop();
                sibling_closed = true;
                continue;
            }
            Event::GeneralRef(_) => {
                if let Some(&(i, _)) = stack.last() {
                    keep[i] = true;
                }
            }
            Event::Text(t) => {
                if let Some(&(i, _)) = stack.last()
                    && t.iter().any(|b| !b.is_ascii_whitespace())
                {
                    keep[i] = true;
                }
                // A blank run without a line break between two elements is
                // an inline space, not layout.
                gap = sibling_closed
                    && !t.is_empty()
                    && !t.iter().any(|b| matches!(b, b'\n' | b'\r'))
                    && t.iter().all(|b| b.is_ascii_whitespace());
                continue;
            }
            Event::Eof => break,
            _ => gap = false,
        }
        sibling_closed = false;
    }
    keep
}

fn xml_layout_with(text: &str, indent: bool) -> Result<String, String> {
    let keep_ws = xml_keep_whitespace(text);
    let mut next_element = 0usize;
    // Open elements: whether each keeps its whitespace-only text.
    let mut keep_stack: Vec<bool> = Vec::new();
    // Depth of the element whose content is written inline (mixed or
    // preserved); the writer's own indentation is bypassed inside it.
    let mut inline_from: Option<usize> = None;
    let mut reader = quick_xml::Reader::from_str(text);
    let mut writer = if indent {
        quick_xml::Writer::new_with_indent(Vec::new(), b' ', 2)
    } else {
        quick_xml::Writer::new(Vec::new())
    };
    // Text and entity references arrive as separate events; gather them into
    // one run so a run stays on one line, and drop runs that are only layout.
    let mut run = String::new();
    // Set while the last thing written ends its own line (a closing or empty
    // tag, a comment, a PI), so a CDATA there is a sibling, not inline text.
    let mut after_markup = false;
    // The reader does not flag an element still open at the end of the text.
    let mut open = 0usize;
    loop {
        let event = reader
            .read_event()
            .map_err(|e| xml_error(text, &reader, &e))?;
        match event {
            Event::Text(t) => {
                run.push_str(std::str::from_utf8(&t).map_err(|e| xml_error(text, &reader, &e))?);
                continue;
            }
            Event::GeneralRef(r) => {
                run.push('&');
                run.push_str(std::str::from_utf8(&r).map_err(|e| xml_error(text, &reader, &e))?);
                run.push(';');
                continue;
            }
            _ => {}
        }
        let keep_blank = !run.is_empty() && keep_stack.last().copied().unwrap_or(false);
        if !run.trim().is_empty() || keep_blank {
            let t = Event::Text(BytesText::from_escaped(run.as_str()));
            write_xml(&mut writer, inline_from.is_some(), t)
                .map_err(|e| xml_error(text, &reader, &e))?;
            after_markup = false;
        }
        run.clear();
        match event {
            Event::Eof if open > 0 => {
                return Err(xml_error(text, &reader, &"an element is never closed"));
            }
            Event::Eof => break,
            // The writer keeps every CDATA inline; a sibling one is written by
            // hand on its own line, leaving the writer's line break pending.
            Event::CData(c) if after_markup && indent && inline_from.is_none() => {
                writer
                    .write_indent()
                    .map_err(|e| xml_error(text, &reader, &e))?;
                let out = writer.get_mut();
                out.extend_from_slice(b"<![CDATA[");
                out.extend_from_slice(&c);
                out.extend_from_slice(b"]]>");
            }
            e => {
                let mut starts_inline = false;
                match e {
                    Event::Start(_) => {
                        open += 1;
                        let keep = keep_ws.get(next_element).copied().unwrap_or(false);
                        next_element += 1;
                        keep_stack.push(keep);
                        starts_inline = keep && inline_from.is_none();
                    }
                    Event::Empty(_) => next_element += 1,
                    Event::End(_) => {
                        open = open.saturating_sub(1);
                        keep_stack.pop();
                    }
                    _ => {}
                }
                after_markup = !matches!(e, Event::Start(_) | Event::CData(_));
                if inline_from == Some(keep_stack.len()) && matches!(e, Event::End(_)) {
                    // Leaving the inline element: an empty text keeps the
                    // writer from breaking the line before its end tag.
                    writer
                        .write_event(Event::Text(BytesText::from_escaped("")))
                        .map_err(|e| xml_error(text, &reader, &e))?;
                    inline_from = None;
                }
                write_xml(&mut writer, inline_from.is_some(), e)
                    .map_err(|e| xml_error(text, &reader, &e))?;
                if starts_inline {
                    inline_from = Some(keep_stack.len() - 1);
                }
            }
        }
    }
    String::from_utf8(writer.into_inner()).map_err(|_| "Invalid XML: not valid UTF-8".to_string())
}

// Writes one event; `inline` skips the writer's indentation, so the event
// lands exactly as written after whatever came before it.
fn write_xml(
    writer: &mut quick_xml::Writer<Vec<u8>>,
    inline: bool,
    event: Event<'_>,
) -> std::io::Result<()> {
    if !inline {
        return writer.write_event(event);
    }
    let mut plain = quick_xml::Writer::new(Vec::new());
    plain.write_event(event)?;
    writer.get_mut().extend_from_slice(&plain.into_inner());
    Ok(())
}

// "Invalid XML: <cause> at line L column C" for a failure at the reader's position.
fn xml_error(
    text: &str,
    reader: &quick_xml::Reader<&[u8]>,
    cause: &dyn std::fmt::Display,
) -> String {
    let at = usize::try_from(reader.buffer_position())
        .unwrap_or(0)
        .min(text.len());
    let before = &text.as_bytes()[..at];
    let line = before.iter().filter(|b| **b == b'\n').count() + 1;
    let column = before.iter().rev().take_while(|b| **b != b'\n').count() + 1;
    format!("Invalid XML: {cause} at line {line} column {column}")
}

/// Reads an image body's size and, from its header or SVG root, its pixel
/// dimensions; dimensions are `None` when the bytes do not say.
pub fn image_info(kind: ImageKind, body: &[u8]) -> ImageInfo {
    let raster = |format| {
        image::ImageReader::with_format(std::io::Cursor::new(body), format)
            .into_dimensions()
            .ok()
    };
    let dimensions = match kind {
        ImageKind::Png => raster(image::ImageFormat::Png),
        ImageKind::Jpeg => raster(image::ImageFormat::Jpeg),
        ImageKind::Gif => raster(image::ImageFormat::Gif),
        ImageKind::Webp => raster(image::ImageFormat::WebP),
        ImageKind::Bmp => raster(image::ImageFormat::Bmp),
        ImageKind::Svg => svg_dimensions(body),
    };
    ImageInfo {
        kind,
        size: body.len() as u64,
        dimensions,
    }
}

/// The root `svg` element's numeric `width` and `height` (pixels, optional
/// `px`), else its `viewBox` size, rounded.
fn svg_dimensions(body: &[u8]) -> Option<(u32, u32)> {
    let mut reader = quick_xml::Reader::from_reader(body);
    let mut buf = Vec::new();
    let root = loop {
        match reader.read_event_into(&mut buf).ok()? {
            Event::Start(e) | Event::Empty(e) => break e.into_owned(),
            Event::Eof => return None,
            _ => buf.clear(),
        }
    };
    if root.local_name().as_ref() != b"svg" {
        return None;
    }
    let attr = |name: &[u8]| {
        root.attributes()
            .flatten()
            .find(|a| a.key.local_name().as_ref() == name)
            .and_then(|a| String::from_utf8(a.value.into_owned()).ok())
    };
    // A usable length is finite and positive; anything else defers to the viewBox.
    let length = |v: String| -> Option<f64> {
        let v = v.trim();
        let n: f64 = v.strip_suffix("px").unwrap_or(v).trim().parse().ok()?;
        (n.is_finite() && n > 0.0).then_some(n)
    };
    let round = |w: f64, h: f64| (w.round().max(1.0) as u32, h.round().max(1.0) as u32);
    if let (Some(w), Some(h)) = (
        attr(b"width").and_then(length),
        attr(b"height").and_then(length),
    ) {
        return Some(round(w, h));
    }
    let view_box = attr(b"viewBox")?;
    let numbers: Vec<f64> = view_box
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|n| !n.is_empty())
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    match numbers[..] {
        [_, _, w, h] if w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 => Some(round(w, h)),
        _ => None,
    }
}

/// Describes an image for the view: `PNG image, 640 × 480, 2.0 KB`, or
/// without the dimensions when they are unknown.
pub fn image_caption(info: &ImageInfo) -> String {
    let label = info.kind.label();
    let size = format_size(info.size);
    match info.dimensions {
        Some((w, h)) => format!("{label} image, {w} × {h}, {size}"),
        None => format!("{label} image, {size}"),
    }
}

/// A body ready for the view.
#[derive(Debug, Clone, PartialEq)]
pub enum Prepared {
    /// Nothing to show.
    Empty,
    /// Small enough for the editor, highlighted as `language`.
    Editor {
        /// The text to show.
        text: String,
        /// The highlighter language: `json`, `html` or `text`.
        language: &'static str,
    },
    /// Too large for the editor: plain rows for a virtualized list.
    Lines(Chunked),
    /// Binary bytes, shown only as their size.
    Binary {
        /// The body size in bytes.
        size: u64,
    },
    /// A whole image, previewed from the raw bytes.
    Image(ImageInfo),
}

/// Prepares a body from its raw bytes and decoded text: a whole image is
/// previewed whatever `pretty` says, a truncated one is shown as binary,
/// and anything else goes through [`prepare`].
pub fn prepare_body(
    kind: BodyKind,
    body: &[u8],
    text: &str,
    pretty: bool,
    truncated: bool,
) -> Prepared {
    match kind {
        BodyKind::Image(image) if !truncated => Prepared::Image(image_info(image, body)),
        _ => prepare(kind, text, body.len() as u64, pretty),
    }
}

/// Re-indents HTML by two spaces per element: one tag per line, an element holding only
/// text kept on its tag's line, and runs of whitespace in text collapsed. Void elements do
/// not indent, and `script`, `style`, `pre` and `textarea` keep their content. Stray or
/// unclosed tags never fail; they only shift the indentation.
pub fn reindent_html(text: &str) -> String {
    let tokens = html_tokens(text);
    let mut lines: Vec<String> = Vec::new();
    // The elements open at this point, innermost last.
    let mut open: Vec<&str> = Vec::new();
    let closes = |at: usize, name: &str| match tokens.get(at) {
        Some(HtmlToken::Close(tag, n)) if n == name => Some(*tag),
        _ => None,
    };
    let mut i = 0;
    while i < tokens.len() {
        let pad = "  ".repeat(open.len());
        match &tokens[i] {
            HtmlToken::Text(t) => {
                let words = collapse(t);
                if !words.is_empty() {
                    lines.push(pad + &words);
                }
            }
            HtmlToken::Other(t) | HtmlToken::Raw(t) => lines.push(pad + t.trim()),
            HtmlToken::Close(tag, name) => {
                // An end tag also closes what was left open inside it; a stray one closes nothing.
                if let Some(at) = open.iter().rposition(|n| *n == name.as_str()) {
                    open.truncate(at);
                }
                lines.push("  ".repeat(open.len()) + tag);
            }
            HtmlToken::Open(tag, _, true) => lines.push(pad + tag),
            HtmlToken::Open(tag, name, false) if HTML_RAW.contains(&name.as_str()) => {
                let content = match tokens.get(i + 1) {
                    Some(HtmlToken::Raw(c)) => {
                        i += 1;
                        *c
                    }
                    _ => "",
                };
                let end = closes(i + 1, name).inspect(|_| i += 1).unwrap_or("");
                // Whitespace inside pre and textarea is shown, so their content stays as is;
                // script and style lose only their blank edges and shared indentation.
                if content.trim().is_empty() {
                    lines.push(format!("{pad}{tag}{end}"));
                } else if matches!(name.as_str(), "pre" | "textarea") {
                    lines.push(format!("{pad}{tag}{content}{end}"));
                } else {
                    lines.push(pad.clone() + tag);
                    lines.extend(dedent(content, &format!("{pad}  ")));
                    if !end.is_empty() {
                        lines.push(pad + end);
                    }
                }
            }
            HtmlToken::Open(tag, name, false) => {
                if let Some(end) = closes(i + 1, name) {
                    lines.push(format!("{pad}{tag}{end}"));
                    i += 1;
                } else if let (Some(HtmlToken::Text(t)), Some(end)) =
                    (tokens.get(i + 1), closes(i + 2, name))
                {
                    lines.push(format!("{pad}{tag}{}{end}", collapse(t)));
                    i += 2;
                } else {
                    lines.push(pad + tag);
                    open.push(name.as_str());
                }
            }
        }
        i += 1;
    }
    if lines.is_empty() {
        return text.to_owned();
    }
    lines.join("\n")
}

// Elements without content, which never take an end tag.
const HTML_VOID: [&str; 14] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];
// Elements whose content is not markup, or whose whitespace is shown.
const HTML_RAW: [&str; 4] = ["script", "style", "pre", "textarea"];

// One piece of an HTML text; tags carry their text as written and their lower-cased name.
enum HtmlToken<'a> {
    Text(&'a str),
    // A comment, DOCTYPE or processing instruction.
    Other(&'a str),
    // The content of a script, style, pre or textarea element.
    Raw(&'a str),
    // A start tag and whether it is void or self-closed.
    Open(&'a str, String, bool),
    Close(&'a str, String),
}

// Splits HTML into tags, comments and text; a `<` that starts no complete tag stays text.
fn html_tokens(text: &str) -> Vec<HtmlToken<'_>> {
    let mut out = Vec::new();
    let mut pos = 0;
    // Where the text not yet emitted starts.
    let mut text_from = 0;
    while let Some(off) = text[pos..].find('<') {
        let at = pos + off;
        let rest = &text[at..];
        let len = if rest.starts_with("<!--") {
            rest.find("-->").map(|e| e + 3)
        } else if rest[1..].starts_with(|c: char| c.is_ascii_alphabetic() || "/!?".contains(c)) {
            tag_len(rest)
        } else {
            None
        };
        let Some(len) = len else {
            pos = at + 1;
            continue;
        };
        if text_from < at {
            out.push(HtmlToken::Text(&text[text_from..at]));
        }
        let token = html_tag(&rest[..len]);
        pos = at + len;
        let raw = match &token {
            HtmlToken::Open(_, name, false) if HTML_RAW.contains(&name.as_str()) => {
                Some(name.clone())
            }
            _ => None,
        };
        out.push(token);
        // A raw element's content runs to its own end tag, whatever it holds.
        if let Some(name) = raw {
            let end = (text[pos..].match_indices("</"))
                .map(|(e, _)| pos + e)
                .find(|&e| {
                    text.as_bytes()[e + 2..]
                        .get(..name.len())
                        .is_some_and(|n| n.eq_ignore_ascii_case(name.as_bytes()))
                })
                .unwrap_or(text.len());
            out.push(HtmlToken::Raw(&text[pos..end]));
            pos = end;
        }
        text_from = pos;
    }
    if text_from < text.len() {
        out.push(HtmlToken::Text(&text[text_from..]));
    }
    out
}

// The length of the tag at the start of `s` through its `>`, skipping quoted attribute values.
fn tag_len(s: &str) -> Option<usize> {
    let mut quote = None;
    let mut prev = b'<';
    for (i, b) in s.bytes().enumerate().skip(1) {
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if matches!(b, b'"' | b'\'') && prev == b'=' => quote = Some(b),
            None if b == b'>' => return Some(i + 1),
            None => {}
        }
        if !b.is_ascii_whitespace() {
            prev = b;
        }
    }
    None
}

fn html_tag(tag: &str) -> HtmlToken<'_> {
    if tag.starts_with("<!") || tag.starts_with("<?") {
        return HtmlToken::Other(tag);
    }
    let (close, body) = match tag.strip_prefix("</") {
        Some(body) => (true, body),
        None => (false, &tag[1..]),
    };
    let name = (body.split(|c: char| c.is_ascii_whitespace() || c == '/' || c == '>'))
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if close {
        HtmlToken::Close(tag, name)
    } else {
        let void = tag.ends_with("/>") || HTML_VOID.contains(&name.as_str());
        HtmlToken::Open(tag, name, void)
    }
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// The lines of `content` without blank edges or their shared indentation, each led by `pad`.
fn dedent(content: &str, pad: &str) -> Vec<String> {
    let lines: Vec<&str> = content.lines().map(str::trim_end).collect();
    let first = lines
        .iter()
        .position(|l| !l.is_empty())
        .unwrap_or(lines.len());
    let last = lines
        .iter()
        .rposition(|l| !l.is_empty())
        .map_or(first, |l| l + 1);
    let lines = &lines[first..last];
    let indent = |l: &str| l.len() - l.trim_start_matches([' ', '\t']).len();
    let cut = (lines.iter().filter(|l| !l.is_empty()))
        .map(|l| indent(l))
        .min()
        .unwrap_or(0);
    (lines.iter())
        .map(|l| match l.get(cut..) {
            Some(rest) if !l.is_empty() => format!("{pad}{rest}"),
            _ => String::new(),
        })
        .collect()
}

/// The text [`prepare`] shows for a text kind, pretty or raw, with its highlighter
/// language; `None` for an empty, binary or image body.
pub fn display_text(kind: BodyKind, text: &str, pretty: bool) -> Option<(String, &'static str)> {
    Some(match (kind, pretty) {
        (BodyKind::Empty | BodyKind::Binary | BodyKind::Image(_), _) => return None,
        (BodyKind::Json, true) => (
            reindent_json(text).unwrap_or_else(|| text.to_owned()),
            "json",
        ),
        (BodyKind::Html, true) => (reindent_html(text), "html"),
        // No XML grammar is bundled; the HTML one colours its tags and attributes.
        (BodyKind::Xml, true) => (
            reindent_xml(text).unwrap_or_else(|| text.to_owned()),
            "html",
        ),
        _ => (text.to_owned(), "text"),
    })
}

/// Prepares a decoded body for display, pretty or raw, picking the editor
/// when the text is within the editor limits and plain rows otherwise.
pub fn prepare(kind: BodyKind, text: &str, size: u64, pretty: bool) -> Prepared {
    let Some((text, language)) = display_text(kind, text, pretty) else {
        return match kind {
            BodyKind::Empty => Prepared::Empty,
            _ => Prepared::Binary { size },
        };
    };
    let lines = text.bytes().filter(|b| *b == b'\n').count() + 1;
    if text.len() <= EDITOR_MAX_BYTES && lines <= EDITOR_MAX_LINES {
        Prepared::Editor { text, language }
    } else {
        Prepared::Lines(chunk_lines(&text, ROW_WIDTH))
    }
}

/// Splits text into rows on `'\n'`, dropping a trailing `'\r'`, and chunks
/// any line longer than `width` bytes at char boundaries.
pub fn rows(text: &str, width: usize) -> Vec<SharedString> {
    chunk_lines(text, width).rows
}

/// Calls `emit` with each chunk of `line` of at most `width` bytes, cut at char boundaries;
/// an empty line is one empty chunk.
fn for_chunks(line: &str, width: usize, mut emit: impl FnMut(&str)) {
    let mut rest = line;
    while rest.len() > width {
        // Back off to a char boundary; a width narrower than one char
        // still takes that whole char so the loop always advances.
        let mut end = width;
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        if end == 0 {
            end = rest.chars().next().map_or(rest.len(), char::len_utf8);
        }
        emit(&rest[..end]);
        rest = &rest[end..];
    }
    emit(rest);
}

/// A text cut into display rows, each tagged with its source line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Chunked {
    /// The rows, in order.
    pub rows: Vec<SharedString>,
    /// Per row, the 1-based number of its source line on the line's first row and 0 on a
    /// continuation row.
    pub line_nos: Vec<u32>,
}

/// Splits `text` into lines (dropping a trailing `'\r'`) and each line into chunks of at most
/// `width` bytes.
pub fn chunk_lines(text: &str, width: usize) -> Chunked {
    let mut out = Chunked::default();
    for (ix, line) in text.split('\n').enumerate() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let mut number = ix as u32 + 1;
        for_chunks(line, width, |chunk| {
            out.rows.push(SharedString::from(chunk.to_owned()));
            out.line_nos.push(std::mem::take(&mut number));
        });
    }
    out
}

impl Chunked {
    /// The number of rows.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether there are no rows.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The number of source lines.
    pub fn line_count(&self) -> usize {
        self.line_nos.iter().max().map_or(0, |n| *n as usize)
    }

    /// The (line number, byte offset within the line) of byte `start` of row `row`.
    pub fn source_pos(&self, row: usize, start: usize) -> (u32, usize) {
        let (mut first, mut offset) = (row, start);
        while first > 0 && self.line_nos[first] == 0 {
            first -= 1;
            offset += self.rows[first].len();
        }
        (self.line_nos[first], offset)
    }

    /// The (row, byte offset within it) holding byte `offset` of source line `line`, or `None`
    /// if the line is absent.
    pub fn locate(&self, line: u32, offset: usize) -> Option<(usize, usize)> {
        let mut row = self.line_nos.iter().position(|n| *n == line)?;
        let mut offset = offset;
        while row + 1 < self.rows.len()
            && self.line_nos[row + 1] == 0
            && offset >= self.rows[row].len()
        {
            offset -= self.rows[row].len();
            row += 1;
        }
        Some((row, offset))
    }

    /// The index of the widest row, the one a horizontally scrolling list measures.
    pub fn widest(&self) -> usize {
        (0..self.rows.len())
            .max_by_key(|ix| cells(&self.rows[*ix]))
            .unwrap_or(0)
    }

    /// The same lines cut at `width` bytes instead; rows already short enough are reused.
    pub fn rechunk(&self, width: usize) -> Chunked {
        let mut out = Chunked::default();
        let mut ix = 0;
        while ix < self.rows.len() {
            // One source line is this row and the continuation rows that follow it.
            let end = (ix + 1..self.rows.len())
                .find(|n| self.line_nos[*n] != 0)
                .unwrap_or(self.rows.len());
            let mut number = self.line_nos[ix];
            let mut push = |chunk: SharedString| {
                out.rows.push(chunk);
                out.line_nos.push(std::mem::take(&mut number));
            };
            if end == ix + 1 && self.rows[ix].len() <= width {
                push(self.rows[ix].clone());
            } else {
                let line: String = self.rows[ix..end].iter().map(|r| r.as_ref()).collect();
                for_chunks(&line, width, |chunk| {
                    push(SharedString::from(chunk.to_owned()))
                });
            }
            ix = end;
        }
        out
    }
}

/// The monospace cells `text` takes: wide characters (CJK, emoji) count two, the rest one.
pub fn cells(text: &str) -> usize {
    text.chars()
        .map(|c| match c as u32 {
            0x1100..=0x115F
            | 0x2E80..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE30..=0xFE6F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F300..=0x1F64F
            | 0x1F900..=0x1F9FF
            | 0x20000..=0x3FFFD => 2,
            _ => 1,
        })
        .sum()
}

/// The columns a wrapping body fills in `width` pixels of `advance`-wide characters, in steps
/// of eight so a drag resize re-chunks rarely, and never more than [`ROW_WIDTH`].
pub fn wrap_columns(width: Pixels, advance: Pixels) -> usize {
    if f32::from(advance) <= 0. {
        return ROW_WIDTH;
    }
    let columns = (f32::from(width) / f32::from(advance)).floor() as usize;
    (columns / 8 * 8).clamp(8, ROW_WIDTH)
}

/// Formats a byte count with 1024-based units: `512 B`, `1.5 KB`, `10.0 MB`.
pub fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    match bytes as f64 {
        n if n < KB => format!("{bytes} B"),
        n if n < KB * KB => format!("{:.1} KB", n / KB),
        n => format!("{:.1} MB", n / (KB * KB)),
    }
}

/// Formats two byte counts in the unit of the larger, so a line showing both reads in one
/// unit system: `("1.1 MB", "0.3 MB")`.
pub fn format_size_pair(a: u64, b: u64) -> (String, String) {
    const KB: f64 = 1024.0;
    let one = |bytes: u64, unit: f64, name: &str| format!("{:.1} {name}", bytes as f64 / unit);
    match a.max(b) as f64 {
        n if n < KB => (format!("{a} B"), format!("{b} B")),
        n if n < KB * KB => (one(a, KB, "KB"), one(b, KB, "KB")),
        _ => (one(a, KB * KB, "MB"), one(b, KB * KB, "MB")),
    }
}

/// The size label, and the cut-point label when `truncated`; only a cut shares the size's unit.
pub fn size_labels(size: u64, body_len: u64, truncated: bool) -> (String, Option<String>) {
    if truncated {
        let (size, cut) = format_size_pair(size, body_len);
        (size, Some(cut))
    } else {
        (format_size(size), None)
    }
}

/// Formats a duration as whole milliseconds under a second, else seconds
/// with two decimals: `250 ms`, `1.25 s`; a nonzero time under a millisecond is `<1 ms`.
pub fn format_time(d: Duration) -> String {
    // Zero is the sending timer's start; anything else under a millisecond is not "0 ms".
    if !d.is_zero() && d < Duration::from_millis(1) {
        "<1 ms".to_string()
    } else if d < Duration::from_secs(1) {
        format!("{} ms", d.as_millis())
    } else {
        format!("{:.2} s", d.as_secs_f64())
    }
}

/// Bytes a Hex or Base64 view formats; the rest is left out with a note.
pub const BINARY_VIEW_CAP: usize = 4 << 20;

/// How a binary body is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BinaryMode {
    /// Offset, hex bytes and printable ASCII, 16 bytes per row.
    #[default]
    Hex,
    /// Standard Base64 with padding.
    Base64,
}

/// One row per 16 bytes, as the reference hex view lays them out.
pub fn hex_rows(bytes: &[u8]) -> Vec<SharedString> {
    use std::fmt::Write as _;

    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    // Offset (8) + ": " + 48 hex columns + " " + up to 16 ASCII chars.
    const ROW_LEN: usize = 8 + 2 + 48 + 1 + 16;
    bytes
        .chunks(16)
        .enumerate()
        .map(|(ix, chunk)| {
            let mut row = String::with_capacity(ROW_LEN);
            let _ = write!(row, "{:08x}: ", ix * 16);
            // Each byte takes "XX " and the column is padded to 48, so the
            // trailing space of the last byte doubles as part of the padding.
            for &b in chunk {
                row.push(DIGITS[usize::from(b >> 4)] as char);
                row.push(DIGITS[usize::from(b & 0xf)] as char);
                row.push(' ');
            }
            row.extend(std::iter::repeat_n(' ', 48 - chunk.len() * 3));
            row.push(' ');
            row.extend(chunk.iter().map(|&b| match b {
                32..=126 => b as char,
                _ => '.',
            }));
            SharedString::from(row)
        })
        .collect()
}

/// Base64 row width: the MIME line length, a multiple of 4 so each row decodes on its own.
/// The rows list neither wraps nor scrolls sideways, so a row must fit a normal window.
pub const BASE64_ROW_WIDTH: usize = 76;

/// Standard Base64 with padding.
pub fn base64_text(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The rows `mode` shows for the first `BINARY_VIEW_CAP` bytes of `bytes`;
/// Base64 is split into rows of `BASE64_ROW_WIDTH` characters.
pub fn binary_rows(bytes: &[u8], mode: BinaryMode) -> Vec<SharedString> {
    let shown = &bytes[..bytes.len().min(BINARY_VIEW_CAP)];
    match mode {
        BinaryMode::Hex => hex_rows(shown),
        BinaryMode::Base64 if shown.is_empty() => Vec::new(),
        BinaryMode::Base64 => rows(&base64_text(shown), BASE64_ROW_WIDTH),
    }
}
