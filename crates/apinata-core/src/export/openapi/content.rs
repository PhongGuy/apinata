//! Operation parameters and request bodies, with a JSON Schema inferred from JSON bodies.

use super::schemas::{Schemas, infer_schema};
use crate::edit::{BodySpec, KeyValue, MultipartValue, ParamKind};
use crate::export::{ExportNote, literal_credential, literal_credential_query};
use crate::request::RequestFile;
use serde_json::{Map, Value, json};
use std::collections::HashSet;

/// Headers OpenAPI ignores as parameters, since other fields describe them.
const IGNORED_HEADERS: [&str; 3] = ["authorization", "content-type", "accept"];

/// The operation's `parameters`, in order path, query, header; empty means none. Path
/// parameters take their names from `key`, the path the operation is filed under, and their
/// examples from the request's rows named by the template at the same position in `own`.
/// Header rows are the `inherited` ones, then the request's own; a header parameter whose
/// example is a literal credential gets a note at `<source> / headers`.
pub(super) fn parameters(
    req: &RequestFile,
    inherited: &[KeyValue],
    key: &str,
    own: &str,
    source: &str,
    notes: &mut Vec<ExportNote>,
) -> Vec<Value> {
    let rows = req.shown_params();
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for (name, own_name) in template_names(key).zip(template_names(own)) {
        if seen.insert((name.to_owned(), "path")) {
            let example = rows
                .iter()
                .find(|p| p.kind == ParamKind::Path && p.name == own_name)
                .map_or("", |p| p.value.as_str());
            out.push(parameter(name, "path", None, true, example));
        }
    }
    for p in rows.iter().filter(|p| p.kind == ParamKind::Query) {
        if seen.insert((p.name.clone(), "query")) {
            if literal_credential_query(&p.name, &p.value) {
                notes.push(ExportNote::new(
                    format!("{source} / params"),
                    format!(
                        "query parameter {} holds a literal credential, written into the document as a parameter example",
                        p.name
                    ),
                ));
            }
            let description = p.description.as_deref();
            out.push(parameter(
                &p.name,
                "query",
                description,
                p.enabled,
                &p.value,
            ));
        }
    }
    // Header names are case-insensitive, so the first spelling stands for them all. Inherited
    // rows come first: an own row that would override one is enabled and already took it out.
    for h in inherited.iter().cloned().chain(req.headers()) {
        let lower = h.name.to_ascii_lowercase();
        if !IGNORED_HEADERS.contains(&lower.as_str()) && seen.insert((lower, "header")) {
            if literal_credential(&h) {
                notes.push(ExportNote::new(
                    format!("{source} / headers"),
                    format!(
                        "header {} holds a literal credential, written into the document as a parameter example",
                        h.name
                    ),
                ));
            }
            let description = h.description.as_deref();
            out.push(parameter(
                &h.name,
                "header",
                description,
                h.enabled,
                &h.value,
            ));
        }
    }
    out
}

/// One parameter object; a blank description and an empty example are left out.
fn parameter(
    name: &str,
    at: &str,
    description: Option<&str>,
    required: bool,
    example: &str,
) -> Value {
    let mut p = Map::new();
    p.insert("name".into(), name.into());
    p.insert("in".into(), at.into());
    if let Some(d) = description.filter(|d| !d.trim().is_empty()) {
        p.insert("description".into(), d.into());
    }
    p.insert("required".into(), required.into());
    p.insert("schema".into(), json!({"type": "string"}));
    if !example.is_empty() {
        p.insert("example".into(), example.into());
    }
    p.into()
}

/// The `{name}` pieces of a templated path, in order.
fn template_names(path: &str) -> impl Iterator<Item = &str> {
    path.split('{')
        .skip(1)
        .filter_map(|piece| piece.split_once('}').map(|(name, _)| name))
}

/// The operation's `requestBody`, or `None` for no body; `notes` gets the body notes. A JSON
/// body's inferred schema is hoisted into `schemas` under the request's name.
pub(super) fn request_body(
    req: &RequestFile,
    source: &str,
    schemas: &mut Schemas,
    notes: &mut Vec<ExportNote>,
) -> Option<Value> {
    let body = req.body();
    let default = match &body {
        BodySpec::None => return None,
        BodySpec::Graphql { .. } => {
            let detail = "the GraphQL query and variables are not described; the operation has no request body";
            notes.push(ExportNote::new(format!("{source} / body"), detail));
            return None;
        }
        BodySpec::Unsupported { kind } => {
            let detail = format!("the {kind} body cannot be exported");
            notes.push(ExportNote::new(format!("{source} / body"), detail));
            return None;
        }
        BodySpec::Json(_) => "application/json",
        BodySpec::Xml(_) => "application/xml",
        BodySpec::Text(_) => "text/plain",
        BodySpec::Sparql(_) => "application/sparql-query",
        BodySpec::FormUrlEncoded(_) => "application/x-www-form-urlencoded",
        BodySpec::Multipart(_) => "multipart/form-data",
        BodySpec::File(_) => "application/octet-stream",
    };
    let media_type = content_type(req).unwrap_or_else(|| default.to_owned());
    let media = match &body {
        BodySpec::Json(text) => match serde_json::from_str::<Value>(text) {
            Ok(value) => {
                let schema = schemas.hoist(req.name(), infer_schema(&value));
                json!({"schema": schema, "example": value})
            }
            Err(_) => {
                let detail = "the JSON body does not parse; it was exported as text";
                notes.push(ExportNote::new(format!("{source} / body"), detail));
                text_media(text)
            }
        },
        BodySpec::Xml(text) | BodySpec::Text(text) | BodySpec::Sparql(text) => text_media(text),
        BodySpec::FormUrlEncoded(fields) => form_media(
            fields
                .iter()
                .filter(|f| f.enabled)
                .map(|f| (f.name.as_str(), Some(f.value.as_str()))),
        ),
        BodySpec::Multipart(fields) => {
            form_media(fields.iter().filter(|f| f.enabled).map(|f| match &f.value {
                MultipartValue::Text(text) => (f.name.as_str(), Some(text.as_str())),
                MultipartValue::Files(_) => (f.name.as_str(), None),
            }))
        }
        BodySpec::None | BodySpec::Graphql { .. } | BodySpec::Unsupported { .. } => return None,
        // A file body's schema names the media type it is sent as.
        BodySpec::File(_) => json!({"schema": {"type": "string", "contentMediaType": media_type}}),
    };
    let mut content = Map::new();
    content.insert(media_type, media);
    Some(json!({"content": content}))
}

/// The first enabled `Content-Type` header without its parameters, when non-blank.
fn content_type(req: &RequestFile) -> Option<String> {
    let header = req
        .headers()
        .into_iter()
        .find(|h| h.enabled && h.name.eq_ignore_ascii_case("content-type"))?;
    let bare = header.value.split(';').next().unwrap_or("").trim();
    (!bare.is_empty()).then(|| bare.to_owned())
}

fn text_media(text: &str) -> Value {
    json!({"schema": {"type": "string"}, "example": text})
}

/// An object schema over form fields, the first of a repeated name winning; a field without
/// a text value is a file part, which gets a binary schema and no example entry. An empty
/// example is left out.
fn form_media<'a>(fields: impl Iterator<Item = (&'a str, Option<&'a str>)>) -> Value {
    let mut properties = Map::new();
    let mut example = Map::new();
    for (name, value) in fields {
        if properties.contains_key(name) {
            continue;
        }
        match value {
            Some(text) => {
                properties.insert(name.into(), json!({"type": "string"}));
                example.insert(name.into(), text.into());
            }
            None => {
                let file =
                    json!({"type": "string", "contentMediaType": "application/octet-stream"});
                properties.insert(name.into(), file);
            }
        }
    }
    let mut media = json!({"schema": {"type": "object", "properties": properties}});
    if !example.is_empty() {
        media["example"] = example.into();
    }
    media
}
