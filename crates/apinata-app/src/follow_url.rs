//! Open URL as new request: a link a response gave, as a GET draft in the request's folder.

use gpui_kit::actions;
use url::Url;

actions!(
    apinata,
    [
        /// Opens the http or https URL at the response body's caret as a new request.
        OpenUrlAsRequest
    ]
);

/// Emitted by the response body when Open URL as new request is chosen: the URL as found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenUrl(pub String);

/// The menu entry's label, in the body editor's menu and on the Location header row.
pub const OPEN_URL: &str = "Open URL as new request";

/// Shown when the command has no http or https URL to open.
pub const NO_URL: &str = "Only an http or https URL opens as a new request";

/// The http or https URL around byte `offset` of `text`, if there is one.
pub fn url_at(text: &str, offset: usize) -> Option<&str> {
    if offset > text.len() || !text.is_char_boundary(offset) {
        return None;
    }
    // The run of text around the caret ends at a space, a quote or an angle bracket; the URL
    // starts at its scheme inside that run and leaves off trailing sentence punctuation.
    let stop = |c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '`');
    let start = (text[..offset].char_indices().rev())
        .find(|&(_, c)| stop(c))
        .map_or(0, |(i, c)| i + c.len_utf8());
    let end = text[offset..].find(stop).map_or(text.len(), |i| offset + i);
    let run = &text[start..end];
    let lower = run.to_ascii_lowercase();
    let at = ["https://", "http://"]
        .iter()
        .filter_map(|scheme| lower.find(scheme))
        .min()?;
    let url = run[at..].trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '}']);
    let within = (start + at..=start + at + url.len()).contains(&offset);
    let parsed = Url::parse(url).ok()?;
    (within && parsed.host().is_some()).then_some(url)
}

/// `raw` resolved against the request URL `base`; `None` unless the result is http or https.
pub fn follow_target(raw: &str, base: &str) -> Option<String> {
    let raw = raw.trim();
    let (url, absolute) = match Url::parse(raw) {
        Ok(url) => (url, true),
        Err(url::ParseError::RelativeUrlWithoutBase) => {
            (Url::parse(base).ok()?.join(raw).ok()?, false)
        }
        Err(_) => return None,
    };
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return None;
    }
    // An absolute target keeps its text as written; a relative one takes the joined form.
    Some(if absolute { raw.to_owned() } else { url.into() })
}

/// Whether `a` and `b` share scheme, host and port.
pub fn same_origin(a: &str, b: &str) -> bool {
    match (Url::parse(a), Url::parse(b)) {
        (Ok(a), Ok(b)) => a.origin() == b.origin() && a.origin().is_tuple(),
        _ => false,
    }
}
