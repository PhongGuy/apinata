//! Port of Bruno's `encodeUrl` (`packages/bruno-common/src/utils/url/index.ts` at 1301ea17).
//!
//! Scheme and authority stay verbatim; path segments are decoded then re-encoded, so encoding is
//! idempotent there; query names and values are encoded content-blind, and `#` is data.
//! One deliberate divergence: a `%26`, `%23` or name `%3D` passes through when it stands for a
//! literal delimiter in the matching enabled param of the request's params table, so that cell
//! stays one value. Every other escape, typed, hand-edited or from a variable, is re-encoded.

use crate::url_params::{QueryPart, delimiter_escape_at};

/// Encodes `url` as Bruno does when a request's `encodeUrl` setting is on. `written` holds the
/// byte offsets in `url` of escapes the params table accounts for; only those pass through.
pub(crate) fn encode_url(url: &str, written: &[usize]) -> String {
    let (origin_and_path, query) = match url.split_once('?') {
        Some((head, query)) => (head, Some(query)),
        None => (url, None),
    };
    let origin_len = authority_span(origin_and_path).map_or(0, |span| span.end);
    let (origin, path) = origin_and_path.split_at(origin_len);
    let mut out = origin.to_owned();
    let segments: Vec<String> = path
        .split('/')
        .map(|segment| encode_uri_component(&safe_decode(segment)))
        .collect();
    out.push_str(&segments.join("/"));
    if let Some(query) = query {
        // Pairs with an empty name are dropped; a pair without `=` keeps no value.
        let mut offset = origin_and_path.len() + 1;
        let pairs: Vec<String> = query
            .split('&')
            .filter_map(|pair| {
                let start = offset;
                offset += pair.len() + 1;
                let (name, value) = match pair.split_once('=') {
                    Some((name, value)) => (name, Some(value)),
                    None => (pair, None),
                };
                if name.is_empty() {
                    return None;
                }
                let value_start = start + name.len() + 1;
                let name = encode_query_part(name, start, QueryPart::Name, written);
                Some(match value {
                    Some(value) => {
                        let value =
                            encode_query_part(value, value_start, QueryPart::Value, written);
                        format!("{name}={value}")
                    }
                    None => name,
                })
            })
            .collect();
        out.push('?');
        out.push_str(&pairs.join("&"));
    }
    out
}

/// The byte range of the authority in a leading `scheme://authority`, matching
/// `/^([a-z][a-z0-9+.-]*:\/\/[^/?#]*)?/i`; `None` when `text` has no explicit scheme.
/// The range start is just past `://` and its end is where the path, query or fragment begins.
pub(crate) fn authority_span(text: &str) -> Option<std::ops::Range<usize>> {
    let sep = text.find("://")?;
    let mut scheme = text[..sep].chars();
    let valid = scheme.next().is_some_and(|c| c.is_ascii_alphabetic())
        && scheme.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'));
    if !valid {
        return None;
    }
    let start = sep + 3;
    let end = text[start..]
        .find(['/', '?', '#'])
        .map_or(text.len(), |end| start + end);
    Some(start..end)
}

/// `encodeURIComponent` over one query part that starts at byte `start` of the URL, keeping the
/// delimiter escapes of `part` whose offsets are in `written`.
fn encode_query_part(text: &str, start: usize, part: QueryPart, written: &[usize]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        let at = start + text.len() - rest.len();
        let kept = written
            .contains(&at)
            .then(|| delimiter_escape_at(rest, part));
        let len = match kept.flatten() {
            Some(escape) => {
                out.push_str(escape);
                escape.len()
            }
            None => {
                out.push_str(&encode_uri_component(&rest[..c.len_utf8()]));
                c.len_utf8()
            }
        };
        rest = &rest[len..];
    }
    out
}

/// JavaScript's `encodeURIComponent`: everything but `A-Z a-z 0-9 - _ . ! ~ * ' ( )` is
/// percent-encoded as UTF-8 with uppercase hex.
fn encode_uri_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for &byte in text.as_bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Bruno's `safeDecodeURIComponent`: `decodeURIComponent`, or where that throws, each
/// well-formed `%XX` decoded on its own when it is ASCII and left as is otherwise.
fn safe_decode(text: &str) -> String {
    if let Some(decoded) = decode_uri_component(text) {
        return decoded;
    }
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        match escape_at(bytes, i) {
            Some(byte) if byte.is_ascii() => {
                out.push(char::from(byte));
                i += 3;
            }
            Some(_) => {
                out.push_str(&text[i..i + 3]);
                i += 3;
            }
            None => {
                // Copy one whole character so multibyte text never splits.
                let len = text[i..].chars().next().map_or(1, char::len_utf8);
                out.push_str(&text[i..i + len]);
                i += len;
            }
        }
    }
    out
}

/// `decodeURIComponent`, or `None` where it would throw: a malformed escape or invalid UTF-8.
fn decode_uri_component(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            out.push(escape_at(bytes, i)?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The byte a well-formed `%XX` at `i` stands for.
fn escape_at(bytes: &[u8], i: usize) -> Option<u8> {
    if bytes.get(i) != Some(&b'%') {
        return None;
    }
    let hex = |b: u8| char::from(b).to_digit(16);
    let high = hex(*bytes.get(i + 1)?)?;
    let low = hex(*bytes.get(i + 2)?)?;
    u8::try_from(high * 16 + low).ok()
}

#[cfg(test)]
mod tests {
    use super::encode_url;

    /// The byte offsets of every `%` in `url`, as if the editor wrote each escape.
    fn all_written(url: &str) -> Vec<usize> {
        url.match_indices('%').map(|(i, _)| i).collect()
    }

    #[test]
    fn query_keeps_only_the_delimiter_escapes_the_editor_writes() {
        let url = "http://x.test/p?k%3D%23=1 %26 2%23%3D";
        assert_eq!(
            encode_url(url, &all_written(url)),
            "http://x.test/p?k%3D%23=1%20%26%202%23%253D"
        );
        // Every other escape stays content-blind, as the reference double-encodes it.
        let url = "http://x.test/p?q=%2f%20%2";
        assert_eq!(
            encode_url(url, &all_written(url)),
            "http://x.test/p?q=%252f%2520%252"
        );
    }

    #[test]
    fn delimiter_escapes_not_written_by_the_editor_are_encoded() {
        assert_eq!(
            encode_url("http://x.test/p?q=1%262&k%3D=v", &[]),
            "http://x.test/p?q=1%25262&k%253D=v"
        );
    }
}
