//! Converting a request file between HTTP and GraphQL, keeping every section both kinds share,
//! and recognising a body that is really a GraphQL query.

use std::fmt;

use yaml_serde::{Mapping, Value};

use crate::collection::Collection;
use crate::edit::{BodySpec, KeyValue};
use crate::error::SaveError;
use crate::item_path::ItemPath;
use crate::request::{RequestFile, RequestKind};
use crate::{js, order};

/// The members a GraphQL-over-HTTP JSON payload may carry; anything else means a plain JSON API.
const PAYLOAD_KEYS: [&str; 4] = ["query", "variables", "operationName", "extensions"];

/// A finished conversion: what it left out, and enough of the file before and after to undo it.
#[derive(Clone)]
#[non_exhaustive]
pub struct KindConversion {
    to: RequestKind,
    dropped: Vec<String>,
    notes: Vec<String>,
    before: Mapping,
    after: Mapping,
}

impl KindConversion {
    /// The kind the request was converted to.
    pub fn kind(&self) -> RequestKind {
        self.to
    }

    /// One sentence per part of the request the new kind cannot hold and so lost.
    pub fn dropped(&self) -> &[String] {
        &self.dropped
    }

    /// One sentence per change worth knowing about that loses nothing.
    pub fn notes(&self) -> &[String] {
        &self.notes
    }
}

/// What a conversion would do, worked out without touching the request.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ConversionPreview {
    dropped: Vec<String>,
    notes: Vec<String>,
}

impl ConversionPreview {
    /// What the conversion would lose, as [`KindConversion::dropped`] will say it.
    pub fn dropped(&self) -> &[String] {
        &self.dropped
    }

    /// What else is worth knowing, as [`KindConversion::notes`] will say it.
    pub fn notes(&self) -> &[String] {
        &self.notes
    }
}

/// Shows the kind and what was dropped, never the file, since bodies carry credentials.
impl fmt::Debug for KindConversion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KindConversion")
            .field("to", &self.to)
            .field("dropped", &self.dropped)
            .field("notes", &self.notes)
            .finish_non_exhaustive()
    }
}

/// Why a conversion, or its undo, was refused or failed; the messages are the ones users see.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConvertError {
    /// No request at the path.
    #[error("no request at {item}")]
    NotFound {
        /// The path asked for.
        item: ItemPath,
    },
    /// The request already is of the kind asked for.
    #[error("{item} is already a {} request", kind_name(*kind))]
    SameKind {
        /// The request.
        item: ItemPath,
        /// Its kind.
        kind: RequestKind,
    },
    /// The request holds unsaved edits, which a conversion would write unasked.
    #[error("Save or discard your unsaved edits in this request first")]
    Unsaved {
        /// The request.
        item: ItemPath,
    },
    /// The request changed after the conversion, so undoing it would lose that change.
    #[error("{item} changed after it was converted, so the conversion cannot be undone")]
    Changed {
        /// The request.
        item: ItemPath,
    },
    /// Writing the file failed; the request is left as it was before the call.
    #[error(transparent)]
    Save(#[from] SaveError),
}

fn kind_name(kind: RequestKind) -> &'static str {
    match kind {
        RequestKind::Graphql => "GraphQL",
        RequestKind::Http => "HTTP",
    }
}

impl Collection {
    /// What [`Collection::convert_request`] would drop and note, refusing as it would; the
    /// request and its file stay as they are.
    pub fn conversion_preview(
        &self,
        item: &ItemPath,
        to: RequestKind,
    ) -> Result<ConversionPreview, ConvertError> {
        let r = self
            .request(item)
            .ok_or_else(|| ConvertError::NotFound { item: item.clone() })?;
        refuse(item, r, to)?;
        let (dropped, notes) = rewrite(&mut r.clone(), to);
        Ok(ConversionPreview { dropped, notes })
    }

    /// Rewrite the saved request at `item` as a request of kind `to`, in place. Everything both
    /// kinds hold is kept as written; the body is translated where it can be and otherwise
    /// dropped, and [`KindConversion::dropped`] says so. Unsaved edits refuse the conversion.
    pub fn convert_request(
        &mut self,
        item: &ItemPath,
        to: RequestKind,
    ) -> Result<KindConversion, ConvertError> {
        let r = self
            .request_mut(item)
            .ok_or_else(|| ConvertError::NotFound { item: item.clone() })?;
        refuse(item, r, to)?;
        let before = r.document().mapping().clone();
        let (dropped, notes) = rewrite(r, to);
        if let Err(e) = r.save() {
            r.document_mut().set_mapping(before);
            return Err(e.into());
        }
        let after = r.document().mapping().clone();
        Ok(KindConversion {
            to,
            dropped,
            notes,
            before,
            after,
        })
    }

    /// Put back the file `done` converted, provided the request is still exactly as the
    /// conversion left it; any later change refuses with [`ConvertError::Changed`].
    pub fn undo_conversion(
        &mut self,
        item: &ItemPath,
        done: &KindConversion,
    ) -> Result<(), ConvertError> {
        let r = self
            .request_mut(item)
            .ok_or_else(|| ConvertError::NotFound { item: item.clone() })?;
        if r.is_dirty() || !r.document().holds(&done.after) {
            let item = item.clone();
            return Err(ConvertError::Changed { item });
        }
        r.document_mut().set_mapping(done.before.clone());
        if let Err(e) = r.save() {
            r.document_mut().set_mapping(done.after.clone());
            return Err(e.into());
        }
        Ok(())
    }
}

// A conversion to the kind the request already is, or over unsaved edits, is refused.
fn refuse(item: &ItemPath, r: &RequestFile, to: RequestKind) -> Result<(), ConvertError> {
    let item = item.clone();
    if r.kind() == to {
        return Err(ConvertError::SameKind { item, kind: to });
    }
    if r.is_dirty() {
        return Err(ConvertError::Unsaved { item });
    }
    Ok(())
}

// Moves the protocol root under its new key, retypes `info` and translates the body, in
// memory; returns what could not come along, then what else changed worth saying.
fn rewrite(r: &mut RequestFile, to: RequestKind) -> (Vec<String>, Vec<String>) {
    let body = r.body();
    let content_type = content_type(&r.headers()).map(str::to_owned);
    let from_key = r.root_key();
    let to_key = match to {
        RequestKind::Graphql => "graphql",
        RequestKind::Http => "http",
    };
    let doc = r.document_mut();
    let mut root = doc
        .get(&[from_key])
        .cloned()
        .unwrap_or(Value::Mapping(Mapping::new()));
    if let Value::Mapping(map) = &mut root {
        map.shift_remove("body");
        // A raw query turns into a JSON body, so its content type would mislabel it.
        if to == RequestKind::Graphql && is_graphql_type(content_type.as_deref()) {
            drop_content_type(map);
        }
    }
    doc.remove(&[from_key]);
    doc.set(&[to_key], root, &[order::TOP]);
    doc.set(
        &["info", "type"],
        Value::from(to_key),
        &[order::TOP, order::INFO],
    );

    let (mut dropped, mut notes) = (Vec::new(), Vec::new());
    match to {
        RequestKind::Graphql => {
            match graphql_payload(&body, content_type.as_deref()) {
                Some((query, variables)) => r.set_body(BodySpec::Graphql { query, variables }),
                None if body != BodySpec::None => dropped.push(
                    "The body was dropped: it is not a GraphQL query and variables.".to_owned(),
                ),
                None => {}
            }
            let doc = r.document_mut();
            let examples = doc
                .get(&["examples"])
                .and_then(Value::as_sequence)
                .map(Vec::len);
            if doc.remove(&["examples"]) {
                dropped.push(match examples {
                    Some(1) => {
                        "1 saved example was dropped: GraphQL requests have none.".to_owned()
                    }
                    Some(n) => {
                        format!("{n} saved examples were dropped: GraphQL requests have none.")
                    }
                    None => {
                        "The saved examples were dropped: GraphQL requests have none.".to_owned()
                    }
                });
            }
        }
        RequestKind::Http => {
            if let BodySpec::Graphql { query, variables } = body
                && !(query.is_empty() && variables.is_empty())
            {
                r.set_body(BodySpec::Json(payload_json(&query, &variables)));
            }
            if r.method().eq_ignore_ascii_case("GET") {
                notes.push(
                    "It keeps the GET method; most servers expect a JSON body with POST."
                        .to_owned(),
                );
            }
        }
    }
    (dropped, notes)
}

/// The JSON body a GraphQL request sends: the query, and the variables re-indented when they
/// parse (else kept as written, so a `{{var}}` reference survives); no variables send `{}`.
fn payload_json(query: &str, variables: &str) -> String {
    let raw = variables.trim();
    let variables = match serde_json::from_str::<serde_json::Value>(raw) {
        _ if raw.is_empty() => "{}".to_owned(),
        Ok(value) => js::stringify_pretty(&value),
        Err(_) => raw.to_owned(),
    };
    let query = serde_json::Value::String(query.to_owned()).to_string();
    format!(
        "{{\n  \"query\": {query},\n  \"variables\": {}\n}}",
        variables.replace('\n', "\n  ")
    )
}

/// The value of the last enabled Content-Type header.
pub(crate) fn content_type(headers: &[KeyValue]) -> Option<&str> {
    headers
        .iter()
        .rev()
        .find(|h| h.enabled && h.name.eq_ignore_ascii_case("content-type"))
        .map(|h| h.value.as_str())
}

// Removes every Content-Type entry of the root's `headers`, and the key once it is empty.
fn drop_content_type(root: &mut Mapping) {
    let Some(Value::Sequence(headers)) = root.get_mut("headers") else {
        return;
    };
    headers.retain(|h| {
        !h.get("name")
            .and_then(Value::as_str)
            .is_some_and(|n| n.eq_ignore_ascii_case("content-type"))
    });
    if headers.is_empty() {
        root.shift_remove("headers");
    }
}

/// Whether `content_type` names a raw GraphQL query body.
pub(crate) fn is_graphql_type(content_type: Option<&str>) -> bool {
    content_type.is_some_and(|ct| ct.to_ascii_lowercase().contains("application/graphql"))
}

/// The query and variables text of a body that is a GraphQL request: a raw query sent as
/// `application/graphql`, or a JSON object holding a string `query` and only the other
/// members of a GraphQL payload. Object variables come back indented as `JSON.stringify` does.
pub(crate) fn graphql_payload(
    body: &BodySpec,
    content_type: Option<&str>,
) -> Option<(String, String)> {
    let text = match body {
        BodySpec::Json(text) | BodySpec::Text(text) => text,
        _ => return None,
    };
    if is_graphql_type(content_type) {
        return Some((text.clone(), String::new()));
    }
    let serde_json::Value::Object(payload) = serde_json::from_str(text).ok()? else {
        return None;
    };
    if payload.keys().any(|k| !PAYLOAD_KEYS.contains(&k.as_str())) {
        return None;
    }
    let serde_json::Value::String(query) = payload.get("query")? else {
        return None;
    };
    if !reads_as_graphql(query) {
        return None;
    }
    let variables = match payload.get("variables") {
        None | Some(serde_json::Value::Null) => String::new(),
        Some(serde_json::Value::String(v)) => v.clone(),
        Some(other) => js::stringify_pretty(other),
    };
    Some((query.clone(), variables))
}

/// Whether `query` opens like a GraphQL document, past blank and `#` comment lines: a selection
/// set, or an operation or fragment keyword standing alone. A search term such as `shoes` does not.
fn reads_as_graphql(query: &str) -> bool {
    let start = query
        .lines()
        .map(str::trim_start)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .unwrap_or("");
    if start.starts_with('{') {
        return true;
    }
    ["query", "mutation", "subscription", "fragment"]
        .iter()
        .any(|keyword| {
            start.strip_prefix(keyword).is_some_and(|rest| {
                !rest.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
            })
        })
}
