//! Request bodies, including Hoppscotch's raw key-value text for urlencoded forms.

use super::model::{Body, BodyMember, Content, FormField};
use super::tree::Scope;
use crate::edit::{BodySpec, FileEntry, KeyValue, MultipartField};
use crate::import::{DocumentError, NoteKind};
use crate::js;
use serde_json::Value;

/// A legacy request's own body members, read when its body is not an object.
pub(super) struct Legacy {
    pub(super) content_type: String,
    pub(super) raw_params: Option<String>,
}

/// The body of a request; a legacy one prefers `rawParams` to `body`, and an empty content
/// type there means text, or no body when the text is empty too.
pub(super) fn body(
    member: Option<BodyMember>,
    legacy: Legacy,
    scope: &mut Scope,
) -> Result<BodySpec, DocumentError> {
    let text = match member {
        Some(BodyMember::Object(b)) => return object(b, scope),
        Some(BodyMember::Legacy(text)) => text,
        None => String::new(),
    };
    let (text, at) = match legacy.raw_params {
        Some(raw) => (raw, "rawParams"),
        None => (text, "body"),
    };
    if legacy.content_type.is_empty() {
        if text.is_empty() {
            return Ok(BodySpec::None);
        }
        return Ok(BodySpec::Text(scope.value(text, || at.to_owned())?));
    }
    typed(&legacy.content_type, text, at, "contentType", scope)
}

/// A version 1 or later body object.
fn object(b: Body, scope: &mut Scope) -> Result<BodySpec, DocumentError> {
    scope.rest(b.rest, || "body.".to_owned())?;
    // Hoppscotch sends no body without a content type, so none is imported; any content
    // left behind is reported so it is not lost silently.
    let Some(kind) = b.content_type else {
        let empty = match &b.content {
            Content::Form(fields) => fields.is_empty(),
            Content::Raw(raw) => {
                raw.is_null()
                    || raw.as_str() == Some("")
                    || raw.as_array().is_some_and(Vec::is_empty)
            }
        };
        if !empty {
            let detail = "the body has no content type and is not sent; not imported";
            scope.note("body.body", NoteKind::Dropped, detail)?;
        }
        return Ok(BodySpec::None);
    };
    let raw = match b.content {
        Content::Form(fields) => return multipart(fields, scope),
        Content::Raw(raw) => raw,
    };
    if kind == "application/octet-stream" || kind == "binary" {
        let detail = "the exported file is lost; choose the file to send";
        scope.note("body", NoteKind::NeedsInput, detail)?;
        let mut file = FileEntry::new("");
        file.selected = true;
        return Ok(BodySpec::File(vec![file]));
    }
    let text = match raw {
        Value::String(text) => text,
        Value::Null => String::new(),
        other => {
            let detail = "not text; imported as its JSON text";
            scope.note("body.body", NoteKind::Converted, detail)?;
            js::to_text(&other)
        }
    };
    typed(&kind, text, "body.body", "body.contentType", scope)
}

/// Text under a text content type; an unknown type keeps the text as plain text.
fn typed(
    kind: &str,
    text: String,
    at: &str,
    kind_at: &str,
    scope: &mut Scope,
) -> Result<BodySpec, DocumentError> {
    if kind == "application/x-www-form-urlencoded" {
        let (lines, keyless) = key_values(&text);
        // A keyless line that holds a value loses it, so each one is reported.
        for _ in 0..keyless {
            let detail = "a line has no key; not imported";
            scope.note(at, NoteKind::Dropped, detail)?;
        }
        let mut fields = Vec::new();
        for mut field in lines {
            scope.entry()?;
            field.value = scope.value(field.value, || at.to_owned())?;
            fields.push(field);
        }
        return Ok(BodySpec::FormUrlEncoded(fields));
    }
    let text = scope.value(text, || at.to_owned())?;
    Ok(match kind {
        "application/json"
        | "application/ld+json"
        | "application/hal+json"
        | "application/vnd.api+json" => BodySpec::Json(text),
        "application/xml" | "text/xml" => BodySpec::Xml(text),
        "text/html" | "text/plain" => BodySpec::Text(text),
        other => {
            let detail = format!("unknown content type `{other}`; imported as text");
            scope.note(kind_at, NoteKind::Converted, detail)?;
            BodySpec::Text(text)
        }
    })
}

/// Multipart fields in order; a file field gets an empty path, since exports lose files.
fn multipart(fields: Vec<FormField>, scope: &mut Scope) -> Result<BodySpec, DocumentError> {
    let mut out = Vec::new();
    for (i, f) in fields.into_iter().enumerate() {
        scope.entry()?;
        scope.rest(f.rest, || format!("body.body[{i}]."))?;
        let mut field = match f.value {
            Some(value) => {
                let value = scope.value(value, || format!("body.body[{i}].value"))?;
                MultipartField::text(f.key, value)
            }
            None => {
                let detail = "the exported file is lost; choose the file to send";
                scope.note(&format!("body.body[{i}]"), NoteKind::NeedsInput, detail)?;
                MultipartField::files(f.key, vec![String::new()])
            }
        };
        field.content_type = f.content_type.unwrap_or_default();
        field.enabled = f.active.unwrap_or(true);
        out.push(field);
    }
    Ok(BodySpec::Multipart(out))
}

/// Hoppscotch's raw key-value lines: `key: value` per line, a leading `#` disables the
/// entry, and a key or value may be a JSON string so it can hold `:` or `#`. A line with an
/// empty key is skipped; the count of those that carried a value comes second.
fn key_values(text: &str) -> (Vec<KeyValue>, usize) {
    let mut out = Vec::new();
    let mut keyless = 0;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let (enabled, line) = match line.strip_prefix('#') {
            Some(rest) => (false, rest.trim_start()),
            None => (true, line),
        };
        let (key, value) = split_key(line);
        let value = value.map(|v| unquote(v.trim())).unwrap_or_default();
        if key.is_empty() {
            keyless += usize::from(!value.is_empty());
            continue;
        }
        let mut entry = KeyValue::new(key, value);
        entry.enabled = enabled;
        out.push(entry);
    }
    (out, keyless)
}

/// The key and what follows its `:`, if anything; a quoted key that does not parse reads
/// as a plain one.
fn split_key(line: &str) -> (String, Option<&str>) {
    if line.starts_with('"') {
        let mut stream = serde_json::Deserializer::from_str(line).into_iter::<String>();
        if let Some(Ok(key)) = stream.next() {
            let after = line[stream.byte_offset()..].trim_start();
            if let Some(value) = after.strip_prefix(':') {
                return (key, Some(value));
            }
            if after.is_empty() {
                return (key, None);
            }
        }
    }
    match line.split_once(':') {
        Some((key, value)) => (key.trim_end().to_owned(), Some(value)),
        None => (line.to_owned(), None),
    }
}

fn unquote(value: &str) -> String {
    if value.starts_with('"')
        && let Ok(text) = serde_json::from_str::<String>(value)
    {
        return text;
    }
    value.to_owned()
}
