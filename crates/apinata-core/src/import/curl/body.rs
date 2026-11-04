//! The request body a curl command sends: `-F` parts, `-G` query data, a file body, or data
//! whose mode follows the Content-Type header.

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

use super::flags::Known;
use super::{Collected, DataPiece};
use crate::edit::{BodySpec, FileEntry, KeyValue, MultipartField};
use crate::import::{ImportNote, ImportedRequest, NoteKind};

/// What `curl_easy_escape` leaves alone: letters, digits and `-._~`.
const CURL_ESCAPE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

const FILE_DATA: &str =
    "the data is read from a file, which an import cannot do; add it before sending";

/// Apply `-G`, the Content-Type decision and `--json`'s header to `request`, pushing notes with offsets.
pub(super) fn apply(
    collected: Collected,
    request: &mut ImportedRequest,
    notes: &mut Vec<(usize, ImportNote)>,
) {
    let mut note = |offset: usize, written: &str, kind: NoteKind, detail: &str| {
        notes.push((
            offset,
            ImportNote::new(format!("curl flag {written}"), kind, detail),
        ));
    };
    // `-F` sends multipart, and curl refuses to mix it with data, so the data is noted.
    if !collected.form.is_empty() {
        let mut parts = Vec::new();
        for (written, word) in &collected.form {
            let mut part_note = |kind, detail: &str| note(word.offset, written, kind, detail);
            parts.extend(form_part(&word.text, &mut part_note));
        }
        for piece in &collected.data {
            let detail = "data cannot be sent alongside form fields, so it was left out";
            note(piece.offset, &piece.written, NoteKind::Dropped, detail);
        }
        request.body = BodySpec::Multipart(parts);
        return;
    }
    if collected.data.is_empty() {
        return;
    }
    if collected.data.iter().any(|p| p.flag == Known::Json) && content_type(request).is_none() {
        request
            .headers
            .push(KeyValue::new("Content-Type", "application/json"));
    }
    let content_type = content_type(request).map(str::to_owned);

    // A lone file reference keeps the file as the body; elsewhere a file cannot be inlined.
    if let [piece] = collected.data.as_slice()
        && !collected.get
        && let Some(path) = file_reference(piece)
    {
        let mut entry = FileEntry::new(path);
        entry.content_type = content_type.unwrap_or_default();
        entry.selected = true;
        request.body = BodySpec::File(vec![entry]);
        return;
    }
    let mut joined = String::new();
    let mut previous: Option<Known> = None;
    for piece in &collected.data {
        let Some(text) = inline_text(piece) else {
            note(
                piece.offset,
                &piece.written,
                NoteKind::NeedsInput,
                FILE_DATA,
            );
            continue;
        };
        // `--json` pieces concatenate; every other neighbour joins with `&`.
        let glued = piece.flag == Known::Json && previous == Some(Known::Json);
        if previous.is_some() && !glued {
            joined.push('&');
        }
        joined.push_str(&text);
        previous = Some(piece.flag);
    }
    if previous.is_none() {
        return;
    }
    if collected.get {
        let url = &mut request.url;
        if !url.contains('?') {
            url.push('?');
        } else if !url.ends_with(['?', '&']) {
            url.push('&');
        }
        url.push_str(&joined);
        return;
    }

    let mode = content_type.as_deref().map(str::to_lowercase);
    request.body = match mode.as_deref() {
        Some(ct) if ct.contains("application/x-ndjson") || ct.contains("application/ndjson") => {
            BodySpec::Text(joined)
        }
        Some(ct) if ct.contains("application/json") || ct.contains("+json") => {
            BodySpec::Json(crate::json_text::beautify_json(&joined).unwrap_or(joined))
        }
        Some(ct) if ct.contains("xml") => BodySpec::Xml(joined),
        Some(ct) if ct.contains("application/x-www-form-urlencoded") => {
            form_pairs(&joined).unwrap_or(BodySpec::Text(joined))
        }
        Some(_) => BodySpec::Text(joined),
        // Without a Content-Type curl sends a form, so text that is not pairs gets the header.
        None => form_pairs(&joined).unwrap_or_else(|| {
            request.headers.push(KeyValue::new(
                "Content-Type",
                "application/x-www-form-urlencoded",
            ));
            let first = &collected.data[0];
            let detail = "the data is not name=value pairs, so the body is kept as raw text and the Content-Type header is set to application/x-www-form-urlencoded, as curl does; change the header if the server expects another type";
            note(first.offset, &first.written, NoteKind::Converted, detail);
            BodySpec::Text(joined)
        }),
    };
}

/// The value of the last Content-Type header.
fn content_type(request: &ImportedRequest) -> Option<&str> {
    request
        .headers
        .iter()
        .rev()
        .find(|h| h.name.eq_ignore_ascii_case("content-type"))
        .map(|h| h.value.as_str())
}

/// The path of a piece curl reads from a file; `--data-urlencode` files never count.
fn file_reference(piece: &DataPiece) -> Option<&str> {
    match piece.flag {
        Known::Data | Known::DataBinary | Known::Json => piece.text.strip_prefix('@'),
        _ => None,
    }
}

/// The text a piece contributes, encoded as curl would; `None` when curl reads it from a file.
fn inline_text(piece: &DataPiece) -> Option<String> {
    if piece.flag != Known::DataUrlencode {
        return file_reference(piece).is_none().then(|| piece.text.clone());
    }
    // curl's split: the first `=` separates name and content, and a leading `=` drops the
    // name; only without any `=` does an `@` name a file.
    let text = &piece.text;
    match text.split_once('=') {
        Some(("", content)) => Some(escape(content)),
        Some((name, content)) => Some(format!("{name}={}", escape(content))),
        None if text.contains('@') => None,
        None => Some(escape(text)),
    }
}

/// curl's `--data-urlencode` escape: `curl_easy_escape`, then each space as `+`.
fn escape(text: &str) -> String {
    utf8_percent_encode(text, CURL_ESCAPE)
        .to_string()
        .replace("%20", "+")
}

/// Decoded form fields, or `None` when some `&` piece has no `=`.
fn form_pairs(joined: &str) -> Option<BodySpec> {
    if !joined.split('&').all(|pair| pair.contains('=')) {
        return None;
    }
    let pairs = url::form_urlencoded::parse(joined.as_bytes())
        .map(|(name, value)| KeyValue::new(name, value))
        .collect();
    Some(BodySpec::FormUrlEncoded(pairs))
}

/// One `-F name=content;attr=value` as curl's `--form` reads it, or `None` when it has no name.
fn form_part(text: &str, note: &mut impl FnMut(NoteKind, &str)) -> Option<MultipartField> {
    let Some((name, rest)) = text.split_once('=') else {
        note(
            NoteKind::Dropped,
            "a form field needs name=content, so it was left out",
        );
        return None;
    };
    let (file, rest) = match rest.as_bytes().first() {
        Some(b'@') => (true, &rest[1..]),
        Some(b'<') => {
            let detail = "a field whose text curl reads from a file is imported as a file upload";
            note(NoteKind::Converted, detail);
            (true, &rest[1..])
        }
        _ => (false, rest),
    };
    let (content, mut rest) = form_word(rest);
    let mut part = if file {
        MultipartField::files(name, vec![content])
    } else {
        MultipartField::text(name, content)
    };
    // Each `;` starts an attribute; only `type` maps onto a part.
    while let Some(after) = rest.strip_prefix(';') {
        let after = after.trim_start_matches(' ');
        let name_end = after.find(['=', ';']).unwrap_or(after.len());
        let attribute = &after[..name_end];
        let (value, tail) = match after[name_end..].strip_prefix('=') {
            Some(value) => form_word(value),
            None => (String::new(), &after[name_end..]),
        };
        if attribute == "type" {
            part.content_type = value;
        } else {
            // Text without `=` is not an attribute name and may be a secret, so it is not quoted.
            let detail = if after[name_end..].starts_with('=') {
                format!("the `{attribute}` part attribute was left out")
            } else {
                "text after `;` that is not an attribute was left out".to_owned()
            };
            note(NoteKind::Dropped, &detail);
        }
        rest = tail;
    }
    Some(part)
}

/// A `--form` word and what follows it: a closed `"..."` loses its quotes and unescapes `\"`
/// and `\\`; anything else, including an unclosed quote, runs to the first `;`.
fn form_word(text: &str) -> (String, &str) {
    if let Some(quoted) = text.strip_prefix('"') {
        let mut word = String::new();
        let mut chars = quoted.char_indices();
        while let Some((at, c)) = chars.next() {
            match c {
                '"' => {
                    let tail = &quoted[at + 1..];
                    return (word, &tail[tail.find(';').unwrap_or(tail.len())..]);
                }
                '\\' => match chars.next() {
                    Some((_, e @ ('"' | '\\'))) => word.push(e),
                    Some((_, other)) => {
                        word.push('\\');
                        word.push(other);
                    }
                    None => word.push('\\'),
                },
                _ => word.push(c),
            }
        }
    }
    let end = text.find(';').unwrap_or(text.len());
    (text[..end].to_owned(), &text[end..])
}
