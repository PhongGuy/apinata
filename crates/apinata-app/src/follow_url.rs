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

/// Shown when the URL holds a `{{variable}}`, which would send its value to wherever it points.
pub const PLACEHOLDER: &str = "A URL holding a {{variable}} does not open as a new request";

/// Shown when a relative URL would keep the query of the request URL it resolves against.
pub const BASE_QUERY: &str =
    "This URL would reuse the request's own query, so it does not open as a new request";

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

/// `raw` resolved against the request URL `base`; the toast text saying why it is refused
/// unless the result is http or https.
pub fn follow_target(raw: &str, base: &str) -> Result<String, &'static str> {
    let raw = raw.trim();
    // A placeholder would resolve at send to this collection's value, wherever the URL points.
    if raw.contains("{{") {
        return Err(PLACEHOLDER);
    }
    let (mut url, absolute) = match Url::parse(raw) {
        Ok(url) => (url, true),
        Err(url::ParseError::RelativeUrlWithoutBase) => {
            let joined = Url::parse(base).ok().and_then(|b| b.join(raw).ok());
            (joined.ok_or(NO_URL)?, false)
        }
        Err(_) => return Err(NO_URL),
    };
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return Err(NO_URL);
    }
    // The request URL's query, which may hold a key, carries over only when the target names
    // no query of its own, so that case is refused rather than followed.
    let own_query = raw.split('#').next().is_some_and(|r| r.contains('?'));
    if !absolute && url.query().is_some() && !own_query {
        return Err(BASE_QUERY);
    }
    // Userinfo is never carried into a new request; untouched, the text stays as written.
    let userinfo = !url.username().is_empty() || url.password().is_some();
    let _ = url.set_username("");
    let _ = url.set_password(None);
    Ok(if absolute && !userinfo {
        raw.to_owned()
    } else {
        url.into()
    })
}

/// Whether `a` and `b` share scheme, host and port.
pub fn same_origin(a: &str, b: &str) -> bool {
    match (Url::parse(a), Url::parse(b)) {
        (Ok(a), Ok(b)) => a.origin() == b.origin() && a.origin().is_tuple(),
        _ => false,
    }
}
