//! Saved example responses: each response example of an operation becomes a request example.

use super::body::{MediaKind, media_kind};
use super::operations::Operation;
use super::params::text;
use super::refs::RefKind;
use super::version::Version;
use super::{Doc, sample, xml};
use crate::edit::{BodySpec, KeyValue, MultipartValue};
use crate::example::{Example, ExampleBody, ExampleBodyKind, ExampleResponse};
use crate::import::{DocumentError, ImportNote, ImportedRequest, NoteKind};
use crate::js;
use serde_json::Value;

/// Add an example to `request` for each `examples` entry, `example` or JSON or XML schema of
/// each response media type, in document order. A response without any of those becomes one
/// only when it has header examples, so a bare description adds nothing.
pub(crate) fn apply<'a: 'o, 'o>(
    doc: &mut Doc<'a>,
    op: &'o Operation<'_>,
    request: &mut ImportedRequest,
    notes: &mut Vec<ImportNote>,
) -> Result<(), DocumentError> {
    let Some(responses) = op.op().get("responses").and_then(Value::as_object) else {
        return Ok(());
    };
    let request_bytes = request_half_len(request);
    for (code, raw) in responses {
        // Swagger 2.0 skips `default`, as the reference converter does.
        if doc.version == Version::Swagger2 && code == "default" {
            continue;
        }
        let source = format!("{}.responses.{code}", op.source);
        let response = doc.resolve(raw, RefKind::Object, &source)?;
        let status = code.parse::<u16>().ok().filter(|s| *s > 0);
        let at = Response {
            code,
            status,
            description: response
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            headers: header_examples(doc, &response, &source)?,
            request_bytes,
        };
        let before = request.examples.len();
        if doc.version == Version::Swagger2 {
            swagger2(doc, op, &response, &at, &source, request, notes)?;
        } else {
            openapi3(doc, &response, &at, &source, request, notes)?;
        }
        if request.examples.len() == before && !at.headers.is_empty() {
            let example = at.example(doc, "", at.description, None, None)?;
            request.examples.push(example);
        }
    }
    notes.extend(doc.take_ref_notes());
    Ok(())
}

/// What every example of one response shares.
struct Response<'r> {
    code: &'r str,
    /// The numeric status; `default` and ranges such as `2XX` have none.
    status: Option<u16>,
    description: &'r str,
    /// Header examples as (name, value), without `Content-Type`.
    headers: Vec<(String, String)>,
    /// What each example's copy of the request half costs.
    request_bytes: u64,
}

impl Response<'_> {
    /// One example: `name` with line breaks as spaces, or `<code> Response` when blank; the media
    /// type as its `Content-Type` header ahead of the response's header examples.
    fn example(
        &self,
        doc: &mut Doc<'_>,
        name: &str,
        description: &str,
        media_type: Option<&str>,
        body: Option<ExampleBody>,
    ) -> Result<Example, DocumentError> {
        let name = name.replace("\r\n", " ").replace('\n', " ");
        let name = match js::js_trim(&name) {
            "" => format!("{} Response", self.code),
            trimmed => trimmed.to_owned(),
        };
        let mut headers = Vec::with_capacity(self.headers.len() + 1);
        if let Some(media_type) = media_type {
            headers.push(("Content-Type".to_owned(), media_type.to_owned()));
        }
        headers.extend(self.headers.iter().cloned());
        doc.budget.charge(1)?;
        let header_bytes: usize = headers.iter().map(|(n, v)| n.len() + v.len()).sum();
        let own = (name.len() + description.len() + header_bytes) as u64;
        doc.budget
            .charge_bytes(own.saturating_add(self.request_bytes))?;
        let status_text = self
            .status
            .and_then(|s| reqwest::StatusCode::from_u16(s).ok())
            .and_then(|s| s.canonical_reason())
            .map(str::to_owned);
        Ok(Example {
            name,
            description: (!js::js_trim(description).is_empty()).then(|| description.to_owned()),
            request: None,
            response: Some(ExampleResponse {
                status: self.status,
                status_text,
                headers,
                body,
            }),
        })
    }
}

/// OpenAPI 3: each media type's `examples` entries, else its `example`, else a sample of its
/// schema. An entry without a `value` (a cut `$ref`, an external value) is skipped.
fn openapi3(
    doc: &mut Doc<'_>,
    response: &Value,
    at: &Response<'_>,
    source: &str,
    request: &mut ImportedRequest,
    notes: &mut Vec<ImportNote>,
) -> Result<(), DocumentError> {
    let Some(content) = response.get("content").and_then(Value::as_object) else {
        return Ok(());
    };
    for (media_type, media) in content {
        let media_source = format!("{source}.content.{media_type}");
        if let Some(examples) = media.get("examples").and_then(Value::as_object) {
            for (key, raw) in examples {
                let entry_source = format!("{media_source}.examples.{key}");
                let entry = doc.resolve(raw, RefKind::Object, &entry_source)?;
                let Some(value) = entry.get("value") else {
                    if let Some(url) = entry.get("externalValue").and_then(Value::as_str) {
                        let detail = format!("the external example `{url}` was not fetched");
                        let note = ImportNote::new(entry_source, NoteKind::Dropped, detail);
                        doc.budget.charge_note(&note)?;
                        notes.push(note);
                    }
                    continue;
                };
                let summary = entry.get("summary").and_then(Value::as_str);
                let name = summary
                    .filter(|s| !js::js_trim(s).is_empty())
                    .unwrap_or(key);
                let description = entry
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let body = body_of(doc, media_type, value, &entry_source, notes)?;
                let example = at.example(doc, name, description, Some(media_type), Some(body))?;
                request.examples.push(example);
            }
        } else if let Some(value) = media.get("example") {
            let body = body_of(doc, media_type, value, &media_source, notes)?;
            let example = at.example(doc, "", at.description, Some(media_type), Some(body))?;
            request.examples.push(example);
        } else if let Some(schema) = media.get("schema")
            && let Some(body) = sampled(doc, media_type, schema, &media_source, notes)?
        {
            let example = at.example(doc, "", at.description, Some(media_type), Some(body))?;
            request.examples.push(example);
        }
    }
    Ok(())
}

/// Swagger 2.0: each `examples` entry under its MIME type, else a sample of the schema as the
/// first `produces` type.
fn swagger2(
    doc: &mut Doc<'_>,
    op: &Operation<'_>,
    response: &Value,
    at: &Response<'_>,
    source: &str,
    request: &mut ImportedRequest,
    notes: &mut Vec<ImportNote>,
) -> Result<(), DocumentError> {
    if let Some(examples) = response.get("examples").and_then(Value::as_object) {
        for (mime, value) in examples {
            let example_source = format!("{source}.examples.{mime}");
            let body = body_of(doc, mime, value, &example_source, notes)?;
            let example = at.example(doc, "", at.description, Some(mime), Some(body))?;
            request.examples.push(example);
        }
        return Ok(());
    }
    let Some(schema) = response.get("schema") else {
        return Ok(());
    };
    let root = doc.root;
    let produces = op
        .op()
        .get("produces")
        .or_else(|| root.get("produces"))
        .and_then(Value::as_array)
        .and_then(|p| p.first())
        .and_then(Value::as_str)
        .unwrap_or("application/json")
        .to_owned();
    if let Some(body) = sampled(doc, &produces, schema, source, notes)? {
        let example = at.example(doc, "", at.description, Some(&produces), Some(body))?;
        request.examples.push(example);
    }
    Ok(())
}

/// The value of each header that has an `example`, else a first `examples` entry with a value.
fn header_examples(
    doc: &mut Doc<'_>,
    response: &Value,
    source: &str,
) -> Result<Vec<(String, String)>, DocumentError> {
    let mut out = Vec::new();
    let headers = response.get("headers").and_then(Value::as_object);
    for (name, raw) in headers.into_iter().flatten() {
        // OpenAPI ignores a `Content-Type` response header; the media type carries it.
        if js::js_trim(name).is_empty() || name.eq_ignore_ascii_case("content-type") {
            continue;
        }
        let header_source = format!("{source}.headers.{name}");
        let header = doc.resolve(raw, RefKind::Object, &header_source)?;
        let mut value = header.get("example").cloned();
        if value.is_none() {
            let examples = header.get("examples").and_then(Value::as_object);
            for (key, raw) in examples.into_iter().flatten() {
                let entry_source = format!("{header_source}.examples.{key}");
                let entry = doc.resolve(raw, RefKind::Object, &entry_source)?;
                if let Some(v) = entry.get("value") {
                    value = Some(v.clone());
                    break;
                }
            }
        }
        if let Some(value) = value {
            out.push((name.clone(), text(doc, &value)?));
        }
    }
    Ok(out)
}

/// The saved body for an example `value` of `media_type`: a string as written, an object as
/// XML for an XML type and as two-space JSON otherwise, any other value as text.
fn body_of(
    doc: &mut Doc<'_>,
    media_type: &str,
    value: &Value,
    source: &str,
    notes: &mut Vec<ImportNote>,
) -> Result<ExampleBody, DocumentError> {
    let kind = body_kind(media_type);
    let data = match value {
        Value::Object(_) | Value::Array(_) if kind == ExampleBodyKind::Xml => {
            xml::render(doc, &Value::Null, Some(value), source, notes)?
        }
        Value::Object(_) | Value::Array(_) => {
            doc.budget.charge_bytes(sample::pretty_len(value) as u64)?;
            js::stringify_pretty(value)
        }
        other => text(doc, other)?,
    };
    Ok(ExampleBody::new(kind, data))
}

/// A body sampled from `schema` for a JSON or XML media type; `None` for any other type.
fn sampled(
    doc: &mut Doc<'_>,
    media_type: &str,
    schema: &Value,
    source: &str,
    notes: &mut Vec<ImportNote>,
) -> Result<Option<ExampleBody>, DocumentError> {
    let kind = media_kind(media_type);
    if !matches!(kind, MediaKind::Json | MediaKind::Xml) {
        return Ok(None);
    }
    let schema_source = format!("{source}.schema");
    let resolved = doc.resolve_followed(schema, RefKind::Schema, &schema_source)?;
    doc.enter(&resolved.followed);
    let data = if kind == MediaKind::Json {
        sample::sample(doc, &resolved.value, source, notes).map(|v| {
            let text = js::stringify_pretty(&v);
            (text, sample::pretty_len(&v))
        })
    } else {
        xml::render(doc, &resolved.value, None, source, notes).map(|t| (t, 0))
    };
    doc.leave(&resolved.followed);
    let (data, len) = data?;
    doc.budget.charge_bytes(len as u64)?;
    Ok(Some(ExampleBody::new(body_kind(media_type), data)))
}

/// The reference converter's body type for a media type: JSON, XML, HTML, else text.
fn body_kind(media_type: &str) -> ExampleBodyKind {
    let essence = media_type.split(';').next().unwrap_or_default().trim();
    match media_kind(media_type) {
        MediaKind::Json => ExampleBodyKind::Json,
        MediaKind::Xml => ExampleBodyKind::Xml,
        _ if essence.eq_ignore_ascii_case("text/html") => ExampleBodyKind::Html,
        _ => ExampleBodyKind::Text,
    }
}

/// The approximate bytes of the URL, params, headers and body each example copies from the
/// request; multipart and file entries count their text fields plus a fixed overhead.
fn request_half_len(request: &ImportedRequest) -> u64 {
    let kv =
        |k: &KeyValue| k.name.len() + k.value.len() + k.description.as_ref().map_or(0, String::len);
    let params: usize = request
        .params
        .iter()
        .map(|p| p.name.len() + p.value.len() + p.description.as_ref().map_or(0, String::len))
        .sum();
    let headers: usize = request.headers.iter().map(kv).sum();
    let body = match &request.body {
        BodySpec::Json(t) | BodySpec::Xml(t) | BodySpec::Text(t) | BodySpec::Sparql(t) => t.len(),
        BodySpec::FormUrlEncoded(fields) => fields.iter().map(kv).sum(),
        BodySpec::Multipart(fields) => fields
            .iter()
            .map(|f| {
                let value = match &f.value {
                    MultipartValue::Text(t) => t.len(),
                    MultipartValue::Files(paths) => paths.iter().map(String::len).sum(),
                };
                f.name.len() + f.content_type.len() + value + 64
            })
            .sum(),
        BodySpec::File(files) => files
            .iter()
            .map(|f| f.path.len() + f.content_type.len() + 64)
            .sum(),
        BodySpec::Graphql { query, variables } => query.len() + variables.len(),
        _ => 0,
    };
    (request.url.len() + params + headers + body) as u64
}
