//! Percent-encoding and decoding of a piece of text a user selected, outside any URL parsing.

use crate::encode_url::{encode_uri_component, escape_at};
use crate::interpolate::placeholders;

/// Escapes in `text` that do not decode to UTF-8 text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the escapes do not decode to UTF-8 text")]
#[non_exhaustive]
pub struct NotUtf8;

/// Percent-encodes `text` as a query value is encoded, leaving `{{var}}` placeholders whole.
pub fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (part, placeholder) in parts(text) {
        if placeholder {
            out.push_str(part);
        } else {
            out.push_str(&encode_uri_component(part));
        }
    }
    out
}

/// Decodes every well-formed `%XX` in `text` outside `{{var}}` placeholders; a `%` that starts
/// no escape stays as it is. Fails, changing nothing, when the escapes are not UTF-8.
pub fn percent_decode(text: &str) -> Result<String, NotUtf8> {
    let mut out = String::with_capacity(text.len());
    for (part, placeholder) in parts(text) {
        if placeholder {
            out.push_str(part);
        } else {
            out.push_str(&decode_part(part)?);
        }
    }
    Ok(out)
}

// `text` cut into runs, each flagged when it is a whole placeholder the resolver would find.
fn parts(text: &str) -> Vec<(&str, bool)> {
    let mut parts = Vec::new();
    let mut at = 0;
    for (range, _) in placeholders(text) {
        if range.start > at {
            parts.push((&text[at..range.start], false));
        }
        parts.push((&text[range.clone()], true));
        at = range.end;
    }
    if at < text.len() {
        parts.push((&text[at..], false));
    }
    parts
}

// Each run decodes on its own, so an escape sequence never borrows bytes across a placeholder.
fn decode_part(text: &str) -> Result<String, NotUtf8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match escape_at(bytes, i) {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| NotUtf8)
}
