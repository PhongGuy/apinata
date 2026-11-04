//! Request bodies: the first media type of an operation's `requestBody`, filled from its example
//! or from a sample generated from its schema.

use super::operations::Operation;
use super::params::{self, text};
use super::refs::{RefKind, Resolved};
use super::version::Version;
use super::{Doc, sample, xml};
use crate::edit::{BodySpec, KeyValue, MultipartField};
use crate::import::{DocumentError, ImportNote, ImportedRequest, NoteKind};
use crate::js;
use serde_json::{Map, Value};
use std::borrow::Cow;

/// The body handler a media type selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MediaKind {
    Json,
    Xml,
    Form,
    Multipart,
    Text,
    Other,
}

/// The kind for `media_type`, ignoring case and any parameters after `;`, in the reference
/// converter's handler order.
pub(crate) fn media_kind(media_type: &str) -> MediaKind {
    let essence = media_type.split(';').next().unwrap_or_default().trim();
    let is = |name: &str| essence.eq_ignore_ascii_case(name);
    let text_prefix = essence
        .get(..5)
        .is_some_and(|p| p.eq_ignore_ascii_case("text/"));
    if suffixed(essence, "json") {
        MediaKind::Json
    } else if is("application/x-www-form-urlencoded") {
        MediaKind::Form
    } else if is("multipart/form-data") {
        MediaKind::Multipart
    } else if suffixed(essence, "xml") {
        MediaKind::Xml
    } else if text_prefix || is("application/octet-stream") || is("*/*") {
        MediaKind::Text
    } else {
        MediaKind::Other
    }
}

/// The pattern `^[\w\-.+]+\/([\w\-.+]+\+)?<suffix>$`, ignoring ASCII case.
fn suffixed(essence: &str, suffix: &str) -> bool {
    let word = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'+');
    let Some((kind, sub)) = essence.split_once('/') else {
        return false;
    };
    if kind.is_empty() || !kind.bytes().all(word) || !sub.bytes().all(word) {
        return false;
    }
    // Every byte is ASCII by now, so slicing at any offset is safe.
    let n = sub.len();
    sub.eq_ignore_ascii_case(suffix)
        || (n > suffix.len() + 1
            && sub.as_bytes()[n - suffix.len() - 1] == b'+'
            && sub[n - suffix.len()..].eq_ignore_ascii_case(suffix))
}

/// Set `request.body` from the operation's `requestBody`, or from a Swagger 2.0 `body` or
/// `formData` parameters, noting what is left out.
pub(crate) fn apply<'a: 'o, 'o>(
    doc: &mut Doc<'a>,
    op: &'o Operation<'_>,
    request: &mut ImportedRequest,
    notes: &mut Vec<ImportNote>,
) -> Result<(), DocumentError> {
    let spec = if doc.version == Version::Swagger2 {
        swagger2(doc, op, notes)?
    } else if let Some(raw) = op.op().get("requestBody") {
        let source = format!("{}.requestBody", op.source);
        let body = doc.resolve(raw, RefKind::Object, &source)?;
        build(doc, &body, &source, notes)?
    } else {
        None
    };
    if let Some(spec) = spec {
        request.body = spec;
    }
    notes.extend(doc.take_ref_notes());
    Ok(())
}

/// The body for a resolved `requestBody`, or `None` to leave the request without one.
fn build<'a: 'v, 'v>(
    doc: &mut Doc<'a>,
    body: &'v Value,
    source: &str,
    notes: &mut Vec<ImportNote>,
) -> Result<Option<BodySpec>, DocumentError> {
    let Some(content) = body.get("content").and_then(Value::as_object) else {
        return Ok(None);
    };
    let mut entries = content.iter();
    let Some((media_type, media)) = entries.next() else {
        return Ok(None);
    };
    let content_source = format!("{source}.content");
    for (other, _) in entries {
        let detail = format!("`{other}` was not imported; only the first media type is");
        note(doc, notes, &content_source, detail)?;
    }
    let media_source = format!("{content_source}.{media_type}");
    let found = Media {
        media_type,
        schema: media.get("schema"),
        media,
        source,
        media_source: &media_source,
    };
    handle(doc, found, notes)
}

/// The Swagger 2.0 body: the last `in: body` parameter's schema as the first `consumes` type,
/// else the `formData` parameters as a multipart or urlencoded form.
fn swagger2<'a: 'o, 'o>(
    doc: &mut Doc<'a>,
    op: &'o Operation<'_>,
    notes: &mut Vec<ImportNote>,
) -> Result<Option<BodySpec>, DocumentError> {
    // `params::apply` resolved and charged these already; resolving at the same sources again
    // finds the resolver's notes already made, so a broken ref is noted once.
    let (inherited, own) = params::resolve_lists(doc, op, false)?;
    let merged = params::merge(&inherited, &own);
    let located = |at: &'static str| {
        merged
            .iter()
            .filter(move |(p, _)| p.get("in").and_then(Value::as_str) == Some(at))
    };
    let consumes = op
        .op()
        .get("consumes")
        .or_else(|| doc.root.get("consumes"))
        .and_then(Value::as_array);
    if let Some((param, source)) = located("body").next_back() {
        let Some(schema) = param.get("schema") else {
            return Ok(None);
        };
        let first = consumes.and_then(|c| c.first()).and_then(Value::as_str);
        let found = Media {
            media_type: first.unwrap_or("application/json"),
            schema: Some(schema),
            media: &Value::Null,
            source,
            media_source: source,
        };
        return handle(doc, found, notes);
    }
    let fields: Vec<_> = located("formData").collect();
    if fields.is_empty() {
        return Ok(None);
    }
    let is_file = |p: &Value| p.get("type").and_then(Value::as_str) == Some("file");
    let listed = consumes.is_some_and(|c| c.iter().any(|t| t == "multipart/form-data"));
    let multipart = listed || fields.iter().any(|(p, _)| is_file(p));
    doc.budget.charge(1)?;
    let mut out = Vec::new();
    for (param, source) in fields {
        let Some(name) = param.get("name").and_then(Value::as_str) else {
            continue;
        };
        let file = is_file(param);
        let value = ["example", "default"]
            .iter()
            .find_map(|k| param.get(*k))
            .or_else(|| param.get("enum").and_then(Value::as_array)?.first());
        let value = match value {
            Some(v) if !file => text(doc, v)?,
            _ => String::new(),
        };
        let description = param.get("description").and_then(Value::as_str);
        out.push((field(doc, name, value, description)?, file));
        if file {
            needs_file(doc, notes, source, name)?;
        }
    }
    Ok(Some(if multipart {
        BodySpec::Multipart(out.into_iter().map(part).collect())
    } else {
        BodySpec::FormUrlEncoded(out.into_iter().map(|(kv, _)| kv).collect())
    }))
}

/// One media type's body: its type, schema and examples, with the paths notes name.
struct Media<'v, 's> {
    media_type: &'s str,
    schema: Option<&'v Value>,
    /// The media type object, or `null` for a Swagger 2.0 body parameter.
    media: &'v Value,
    source: &'s str,
    media_source: &'s str,
}

/// The body one media type gives, or `None` to leave the request without one.
fn handle<'a: 'v, 'v>(
    doc: &mut Doc<'a>,
    found: Media<'v, '_>,
    notes: &mut Vec<ImportNote>,
) -> Result<Option<BodySpec>, DocumentError> {
    let Media {
        media_type,
        schema: found_schema,
        media,
        source,
        media_source,
    } = found;
    let kind = media_kind(media_type);
    if kind == MediaKind::Other {
        let detail = format!("`{media_type}` request bodies are not imported");
        note(doc, notes, source, detail)?;
        return Ok(None);
    }
    doc.budget.charge(1)?;
    let schema_source = format!("{media_source}.schema");
    let schema = found_schema
        .map(|s| doc.resolve_followed(s, RefKind::Schema, &schema_source))
        .transpose()?;
    let own = schema.as_ref().and_then(|s| s.value.get("example"));
    let found =
        |doc: &mut Doc<'a>, notes: &mut Vec<ImportNote>, keep: &mut dyn FnMut(&Value) -> bool| {
            example(doc, own, media, media_source, source, notes, keep)
        };
    let spec = match kind {
        MediaKind::Json | MediaKind::Xml => {
            let room = doc.bounds.sample_bytes;
            let mut oversized = false;
            let mut keep = |v: &Value| {
                let fits = sample::fits(v, room);
                oversized |= !fits;
                fits
            };
            let chosen = found(doc, notes, &mut keep)?;
            let fell_back = chosen.is_none();
            let mark = notes.len();
            let spec = if kind == MediaKind::Xml {
                let followed = schema.as_ref().map(|s| s.followed.as_slice());
                let schema = schema.as_ref().map_or(&Value::Null, |s| s.value.as_ref());
                doc.enter(followed.unwrap_or_default());
                let text = xml::render(doc, schema, chosen.as_deref(), source, notes);
                doc.leave(followed.unwrap_or_default());
                BodySpec::Xml(text?)
            } else {
                let raw = found_schema;
                BodySpec::Json(json(doc, chosen, schema.as_ref(), raw, source, notes)?)
            };
            // An example passed over for its size is noted only when the schema stands in for
            // it, and once even if the generated body was cut too.
            let detail = sample::size_cut(room);
            if oversized && fell_back && !notes[mark..].iter().any(|n| n.detail == detail) {
                let note = ImportNote::new(source, NoteKind::Converted, detail);
                doc.budget.charge_note(&note)?;
                notes.push(note);
            }
            spec
        }
        MediaKind::Text => match found(doc, notes, &mut |_| true)? {
            Some(value) => BodySpec::Text(text(doc, &value)?),
            None => BodySpec::Text(String::new()),
        },
        _ => {
            let multipart = kind == MediaKind::Multipart;
            let chosen = found(doc, notes, &mut Value::is_object)?;
            let schema = schema.as_ref().map(|s| s.value.as_ref());
            let fields = form(doc, chosen, schema, &schema_source, multipart, notes)?;
            if multipart {
                BodySpec::Multipart(fields.into_iter().map(part).collect())
            } else {
                BodySpec::FormUrlEncoded(fields.into_iter().map(|(kv, _)| kv).collect())
            }
        }
    };
    Ok(Some(spec))
}

/// The pretty JSON text of the chosen example, else of a sample of the schema; empty with
/// neither.
fn json<'a: 'v, 'v>(
    doc: &mut Doc<'a>,
    chosen: Option<Cow<'v, Value>>,
    schema: Option<&Resolved<'v>>,
    raw: Option<&Value>,
    source: &str,
    notes: &mut Vec<ImportNote>,
) -> Result<String, DocumentError> {
    let value = match (chosen, schema) {
        (Some(value), _) => value,
        (None, None) => return Ok(String::new()),
        // A ref the resolver cut stands for an object, as it does inside a sample.
        (None, Some(s)) if s.followed.is_empty() && is_ref(raw) => {
            Cow::Owned(Value::Object(Map::new()))
        }
        (None, Some(s)) => {
            doc.enter(&s.followed);
            let value = sample::sample(doc, &s.value, source, notes);
            doc.leave(&s.followed);
            Cow::Owned(value?)
        }
    };
    doc.budget.charge_bytes(sample::pretty_len(&value) as u64)?;
    Ok(js::stringify_pretty(&value))
}

/// The first example, in order: the schema's `example`, the media `example`, then the first
/// `examples` entry with a `value`. One that `keep` rejects is passed over.
fn example<'a: 'v, 'v>(
    doc: &mut Doc<'a>,
    own: Option<&'v Value>,
    media: &'v Value,
    media_source: &str,
    source: &str,
    notes: &mut Vec<ImportNote>,
    keep: &mut dyn FnMut(&Value) -> bool,
) -> Result<Option<Cow<'v, Value>>, DocumentError> {
    for value in [own, media.get("example")].into_iter().flatten() {
        if keep(value) {
            return Ok(Some(Cow::Borrowed(value)));
        }
    }
    let Some(examples) = media.get("examples").and_then(Value::as_object) else {
        return Ok(None);
    };
    for (key, raw) in examples {
        let entry_source = format!("{media_source}.examples.{key}");
        let entry = doc.resolve(raw, RefKind::Object, &entry_source)?;
        if entry.get("value").is_some() {
            let value = match entry {
                Cow::Borrowed(e) => Cow::Borrowed(&e["value"]),
                Cow::Owned(mut e) => Cow::Owned(e["value"].take()),
            };
            return Ok(keep(&value).then_some(value));
        }
        if let Some(url) = entry.get("externalValue").and_then(Value::as_str) {
            let detail = format!("the external example `{url}` was not fetched");
            note(doc, notes, source, detail)?;
        }
    }
    Ok(None)
}

/// Form fields from an example object's entries, else from the schema's own properties and
/// then its `allOf` members' properties, each valued by its `example`, else `default`. With
/// `files`, a `format: binary` property is a file field, flagged true and noted.
fn form<'a: 'v, 'v>(
    doc: &mut Doc<'a>,
    example: Option<Cow<'v, Value>>,
    schema: Option<&'v Value>,
    schema_source: &str,
    files: bool,
    notes: &mut Vec<ImportNote>,
) -> Result<Vec<(KeyValue, bool)>, DocumentError> {
    let mut out = Vec::new();
    if let Some(example) = example.as_deref().and_then(Value::as_object) {
        for (name, value) in example {
            let value = text(doc, value)?;
            out.push((field(doc, name, value, None)?, false));
        }
        return Ok(out);
    }
    let Some(schema) = schema else {
        return Ok(out);
    };
    let mut fields = Fields {
        files,
        out: &mut out,
        notes,
    };
    fields.properties(doc, schema, schema_source)?;
    let members = schema.get("allOf").and_then(Value::as_array);
    for (i, raw) in members.into_iter().flatten().enumerate() {
        let member_source = format!("{schema_source}.allOf[{i}]");
        let member = doc.resolve(raw, RefKind::Schema, &member_source)?;
        fields.properties(doc, &member, &member_source)?;
    }
    Ok(out)
}

/// Where `form` collects schema properties as fields.
struct Fields<'f> {
    files: bool,
    out: &'f mut Vec<(KeyValue, bool)>,
    notes: &'f mut Vec<ImportNote>,
}

impl Fields<'_> {
    /// One field per property of `schema`, in order.
    fn properties<'a: 'v, 'v>(
        &mut self,
        doc: &mut Doc<'a>,
        schema: &'v Value,
        schema_source: &str,
    ) -> Result<(), DocumentError> {
        let props = schema.get("properties").and_then(Value::as_object);
        for (name, raw) in props.into_iter().flatten() {
            let prop_source = format!("{schema_source}.properties.{name}");
            let prop = doc.resolve(raw, RefKind::Schema, &prop_source)?;
            let is = |key: &str, want: &str| prop.get(key).and_then(Value::as_str) == Some(want);
            let file = self.files && is("type", "string") && is("format", "binary");
            let value = match prop.get("example").or_else(|| prop.get("default")) {
                Some(v) if !file => text(doc, v)?,
                _ => String::new(),
            };
            let description = prop.get("description").and_then(Value::as_str);
            self.out.push((field(doc, name, value, description)?, file));
            if file {
                needs_file(doc, self.notes, &prop_source, name)?;
            }
        }
        Ok(())
    }
}

/// A multipart field from a form field, a file field with no paths when `file` is set.
fn part((kv, file): (KeyValue, bool)) -> MultipartField {
    let mut out = if file {
        MultipartField::files(kv.name, Vec::new())
    } else {
        MultipartField::text(kv.name, kv.value)
    };
    out.enabled = kv.enabled;
    out.description = kv.description;
    out
}

/// Note that the file field `name` needs a file chosen before it is sent.
fn needs_file(
    doc: &mut Doc<'_>,
    notes: &mut Vec<ImportNote>,
    source: &str,
    name: &str,
) -> Result<(), DocumentError> {
    let detail = format!("file field `{name}` needs a file");
    let note = ImportNote::new(source, NoteKind::NeedsInput, detail);
    doc.budget.charge_note(&note)?;
    notes.push(note);
    Ok(())
}

/// An enabled field, charging its entry, name and description before they are copied; the
/// value was charged as it was made.
fn field(
    doc: &mut Doc<'_>,
    name: &str,
    value: String,
    description: Option<&str>,
) -> Result<KeyValue, DocumentError> {
    let description = description.filter(|d| !d.is_empty());
    doc.budget.charge(1)?;
    doc.budget.charge_text(name)?;
    doc.budget.charge_text(description.unwrap_or_default())?;
    let mut out = KeyValue::new(name, value);
    out.description = description.map(str::to_owned);
    Ok(out)
}

fn is_ref(schema: Option<&Value>) -> bool {
    schema
        .and_then(|s| s.get("$ref"))
        .is_some_and(Value::is_string)
}

/// Push a `Dropped` note, charged as it is made.
fn note(
    doc: &mut Doc<'_>,
    notes: &mut Vec<ImportNote>,
    source: &str,
    detail: String,
) -> Result<(), DocumentError> {
    let note = ImportNote::new(source, NoteKind::Dropped, detail);
    doc.budget.charge_note(&note)?;
    notes.push(note);
    Ok(())
}
