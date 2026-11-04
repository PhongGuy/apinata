//! Coverage: a `Dropped` note for every key the importer neither carries over nor deliberately
//! leaves out, so nothing in a description disappears unremarked.
//!
//! Each object kind lists the keys it reads or ignores on purpose; schema objects are not
//! walked, and keys other modules already note are listed here so nothing is noted twice.

use super::Doc;
use super::body::{MediaKind, media_kind};
use super::bounds::Budget;
use super::operations::{Operation, is_operation_key};
use super::params::is_imported;
use super::refs::Resolved;
use super::security::{definitions, definitions_key};
use super::version::Version;
use crate::import::{DocumentError, ImportNote, NoteKind};
use serde_json::Value;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

/// Consumed, then exempt, keys of each object kind; a key in neither is noted.
const ROOT_3: &[&str] = &[
    "openapi",
    "info",
    "servers",
    "paths",
    "components",
    "security",
    "tags",
    "webhooks",
    "jsonSchemaDialect",
    "$self",
];
const ROOT_2: &[&str] = &[
    "swagger",
    "info",
    "host",
    "basePath",
    "schemes",
    "consumes",
    "paths",
    "definitions",
    "parameters",
    "securityDefinitions",
    "security",
    "tags",
    "produces",
    "responses",
];
const INFO: &[&str] = &[
    "title",
    "description",
    "version",
    "summary",
    "termsOfService",
    "contact",
    "license",
];
const TAG: &[&str] = &["name", "description"];
const SERVER: &[&str] = &["url", "description", "name", "variables"];
const VARIABLE: &[&str] = &["default", "enum", "description"];
const PATH_ITEM: &[&str] = &["parameters", "$ref", "summary", "description"];
const OPERATION: &[&str] = &[
    "tags",
    "summary",
    "operationId",
    "description",
    "parameters",
    "security",
    "callbacks",
    "responses",
    "deprecated",
];
const OPERATION_3: &[&str] = &["requestBody", "servers"];
const OPERATION_2: &[&str] = &["consumes", "produces", "schemes"];
const PARAMETER_3: &[&str] = &[
    "name",
    "in",
    "description",
    "required",
    "schema",
    "example",
    "examples",
    "$ref",
    "style",
    "explode",
    "deprecated",
    "allowReserved",
    "allowEmptyValue",
    "content",
];
const PARAMETER_2: &[&str] = &[
    "name",
    "in",
    "description",
    "required",
    "schema",
    "type",
    "format",
    "items",
    "collectionFormat",
    "default",
    "maximum",
    "exclusiveMaximum",
    "minimum",
    "exclusiveMinimum",
    "maxLength",
    "minLength",
    "pattern",
    "maxItems",
    "minItems",
    "uniqueItems",
    "enum",
    "multipleOf",
    "allowEmptyValue",
    "$ref",
];
const REQUEST_BODY: &[&str] = &["content", "$ref", "description", "required"];
const MEDIA: &[&str] = &["schema", "example", "examples", "encoding"];
const SCHEME: &[&str] = &[
    "type",
    "scheme",
    "name",
    "in",
    "flows",
    "flow",
    "openIdConnectUrl",
    "authorizationUrl",
    "tokenUrl",
    "scopes",
    "$ref",
    "description",
    "bearerFormat",
];

/// Unknown keys of each shared `$ref` target already read, so another use replays them.
#[derive(Debug, Default)]
pub(crate) struct Walked {
    /// Per object kind, each target's canonical pointer and its unknown keys, charged once.
    unknown: [HashMap<String, Vec<String>>; 5],
    /// Per `responses` object inside the document, by address, the first code with links.
    links: HashMap<usize, Option<String>>,
    /// Object keys read so far, for tests that pin the walk's cost.
    #[cfg(test)]
    read: u64,
}

/// The object kinds reached through a `$ref`; a target's unknown keys depend on its kind.
#[derive(Debug, Clone, Copy)]
enum Part {
    PathItem,
    Parameter,
    RequestBody,
    Media,
    Scheme,
}

/// The tail of every coverage note's detail, after the quoted key.
const NOT_IMPORTED: &str = "` is not imported";

/// One Dropped note per key neither consumed nor exempt (Decision 7), in document order.
pub(crate) fn operation_notes(
    doc: &mut Doc<'_>,
    op: &Operation<'_>,
    notes: &mut Vec<ImportNote>,
) -> Result<(), DocumentError> {
    let object = op.op();
    let source = op.source.as_str();
    let v3 = doc.version != Version::Swagger2;
    let extra = if v3 { OPERATION_3 } else { OPERATION_2 };
    keys(doc, notes, object, source, |k| {
        OPERATION.contains(&k) || extra.contains(&k)
    })?;
    if object.get("callbacks").is_some() {
        let detail = "callbacks are not imported";
        fixed(doc, notes, &[source, ".callbacks"], detail)?;
    }
    // Responses are not walked; only a response carrying links is worth a note.
    if let Some(responses) = object.get("responses") {
        let in_document = matches!(*op.item, Cow::Borrowed(_));
        if let Some(code) = linked_code(doc, responses, in_document)? {
            let parts = [source, ".responses.", code.as_str(), ".links"];
            fixed(doc, notes, &parts, "response links are not imported")?;
        }
    }
    parameters(doc, notes, object, source)?;
    // Only the first media type of a body that is imported at all; body notes the rest.
    if let Some(raw) = object.get("requestBody").filter(|_| v3) {
        let body_source = format!("{source}.requestBody");
        let body = doc.resolve_target(raw, &body_source)?;
        let known = |k: &str| REQUEST_BODY.contains(&k);
        target_keys(doc, notes, &body, Part::RequestBody, &body_source, known)?;
        let first = body
            .value
            .get("content")
            .and_then(Value::as_object)
            .and_then(|c| c.iter().next());
        if let Some((media_type, media)) = first
            && media_kind(media_type) != MediaKind::Other
        {
            // The media type lives inside the body, so it is shared exactly when the body is.
            let media_source = format!("{body_source}.content.{media_type}");
            let media = Resolved {
                value: Cow::Borrowed(media),
                followed: body.followed.clone(),
            };
            let known = |k: &str| MEDIA.contains(&k);
            target_keys(doc, notes, &media, Part::Media, &media_source, known)?;
        }
    }
    notes.extend(doc.take_ref_notes());
    Ok(())
}

/// The same for everything outside operations; each note is charged as made.
pub(crate) fn document_notes(
    doc: &mut Doc<'_>,
    unwritten: &mut Vec<ImportNote>,
) -> Result<(), DocumentError> {
    let root = doc.root;
    let version = doc.version;
    let v3 = version != Version::Swagger2;
    let known = if v3 { ROOT_3 } else { ROOT_2 };
    keys(doc, unwritten, root, "", |k| known.contains(&k))?;
    if let Some(info) = root.get("info") {
        keys(doc, unwritten, info, "info", |k| INFO.contains(&k))?;
    }
    for (i, tag) in list(root, "tags").enumerate() {
        keys(doc, unwritten, tag, &format!("tags[{i}]"), |k| {
            TAG.contains(&k)
        })?;
    }
    for (i, server) in list(root, "servers").enumerate().filter(|_| v3) {
        let source = format!("servers[{i}]");
        keys(doc, unwritten, server, &source, |k| SERVER.contains(&k))?;
        let variables = server.get("variables").and_then(Value::as_object);
        for (name, variable) in variables.into_iter().flatten() {
            let var_source = format!("{source}.variables.{name}");
            keys(doc, unwritten, variable, &var_source, |k| {
                VARIABLE.contains(&k)
            })?;
        }
    }
    // Schemes named by any requirement are walked once each, in definition order; the set
    // borrows the definitions' own keys, so it holds no copies.
    let defined = definitions(root, version).and_then(Value::as_object);
    let mut used = HashSet::new();
    let mut mark = |security: Option<&Value>| {
        let names = security
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_object)
            .flat_map(|r| r.keys());
        for name in names {
            if let Some((key, _)) = defined.and_then(|d| d.get_key_value(name)) {
                used.insert(key.as_str());
            }
        }
    };
    mark(root.get("security"));
    let paths = root.get("paths").and_then(Value::as_object);
    for (path, raw) in paths.into_iter().flatten() {
        let source = format!("paths.{path}");
        let item = doc.resolve_target(raw, &source)?;
        let known = |k: &str| {
            PATH_ITEM.contains(&k) || (v3 && k == "servers") || is_operation_key(k, version)
        };
        target_keys(doc, unwritten, &item, Part::PathItem, &source, known)?;
        parameters(doc, unwritten, &item.value, &source)?;
        let operations = item.value.as_object().into_iter().flatten();
        for (_, op) in operations.filter(|(k, _)| is_operation_key(k, version)) {
            mark(op.get("security"));
        }
    }
    for (name, raw) in defined.into_iter().flatten() {
        if !used.contains(name.as_str()) {
            continue;
        }
        let source = format!("{}.{name}", definitions_key(version));
        let scheme = doc.resolve_target(raw, &source)?;
        let known = |k: &str| SCHEME.contains(&k);
        target_keys(doc, unwritten, &scheme, Part::Scheme, &source, known)?;
    }
    unwritten.extend(doc.take_ref_notes());
    Ok(())
}

/// The first response code in `responses` carrying links. A `responses` borrowed from the
/// document is scanned once and its answer kept (charged) by address, since path items shared
/// through `$ref` would otherwise rescan it per path; an owned copy was charged when merged.
fn linked_code(
    doc: &mut Doc<'_>,
    responses: &Value,
    in_document: bool,
) -> Result<Option<String>, DocumentError> {
    let at = std::ptr::from_ref(responses) as usize;
    if in_document && let Some(code) = doc.walked.links.get(&at) {
        doc.budget
            .charge_text(code.as_deref().unwrap_or_default())?;
        return Ok(code.clone());
    }
    let mut code = None;
    for (key, response) in responses.as_object().into_iter().flatten() {
        #[cfg(test)]
        {
            doc.walked.read += 1;
        }
        if response.get("links").is_some() {
            code = Some(key.as_str());
            break;
        }
    }
    doc.budget.charge_text(code.unwrap_or_default())?;
    let code = code.map(str::to_owned);
    if in_document {
        doc.budget
            .charge_text(code.as_deref().unwrap_or_default())?;
        doc.walked.links.insert(at, code.clone());
    }
    Ok(code)
}

/// The `parameters` of a path item or operation at `source`, each resolved where `params`
/// resolves it; a parameter `params` drops is already noted, so its keys are not.
fn parameters(
    doc: &mut Doc<'_>,
    notes: &mut Vec<ImportNote>,
    owner: &Value,
    source: &str,
) -> Result<(), DocumentError> {
    let known = if doc.version == Version::Swagger2 {
        PARAMETER_2
    } else {
        PARAMETER_3
    };
    for (i, raw) in list(owner, "parameters").enumerate() {
        let param_source = format!("{source}.parameters[{i}]");
        let param = doc.resolve_target(raw, &param_source)?;
        if is_imported(&param.value, doc.version) {
            let known = |k: &str| known.contains(&k);
            target_keys(doc, notes, &param, Part::Parameter, &param_source, known)?;
        }
    }
    Ok(())
}

/// The entries of `value[key]` when it is an array, else none.
fn list<'v>(value: &'v Value, key: &str) -> impl Iterator<Item = &'v Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

/// `keys` for a value reached through `$ref`: the first use of a target reads its keys and
/// keeps the unknown ones (charged), later uses replay them, so a shared target costs its size
/// once rather than once per use.
fn target_keys(
    doc: &mut Doc<'_>,
    notes: &mut Vec<ImportNote>,
    resolved: &Resolved<'_>,
    part: Part,
    source: &str,
    known: impl Fn(&str) -> bool,
) -> Result<(), DocumentError> {
    let Some(pointer) = resolved.followed.last() else {
        return keys(doc, notes, &resolved.value, source, known);
    };
    if !doc.walked.unknown[part as usize].contains_key(pointer.as_str()) {
        let mut unknown = Vec::new();
        for key in object_keys(&resolved.value) {
            #[cfg(test)]
            {
                doc.walked.read += 1;
            }
            if !exempt(key) && !known(key) {
                doc.budget.charge_text(key)?;
                unknown.push(key.clone());
            }
        }
        doc.budget.charge_text(pointer)?;
        doc.walked.unknown[part as usize].insert(pointer.clone(), unknown);
    }
    let unknown = &doc.walked.unknown[part as usize][pointer.as_str()];
    for key in unknown {
        note_key(&mut doc.budget, notes, source, key)?;
    }
    Ok(())
}

/// Note each key of `object` that is neither `known` nor exempt.
fn keys(
    doc: &mut Doc<'_>,
    notes: &mut Vec<ImportNote>,
    object: &Value,
    source: &str,
    known: impl Fn(&str) -> bool,
) -> Result<(), DocumentError> {
    for key in object_keys(object) {
        #[cfg(test)]
        {
            doc.walked.read += 1;
        }
        if !exempt(key) && !known(key) {
            note_key(&mut doc.budget, notes, source, key)?;
        }
    }
    Ok(())
}

/// The keys of `value` when it is an object, else none.
fn object_keys(value: &Value) -> impl Iterator<Item = &String> {
    value.as_object().into_iter().flat_map(|o| o.keys())
}

/// Keys exempt in every object: extensions, YAML merge keys (noted on load) and `externalDocs`.
fn exempt(key: &str) -> bool {
    key.starts_with("x-") || key == "<<" || key == "externalDocs"
}

/// One coverage note for `key` under `source`, charged before its strings are made.
fn note_key(
    budget: &mut Budget,
    notes: &mut Vec<ImportNote>,
    source: &str,
    key: &str,
) -> Result<(), DocumentError> {
    let source_len = if source.is_empty() {
        0
    } else {
        source.len() + 1
    } + key.len();
    let detail_len = 1 + key.len() + NOT_IMPORTED.len();
    budget.charge(1)?;
    budget.charge_bytes((source_len + detail_len) as u64)?;
    let note_source = if source.is_empty() {
        key.to_owned()
    } else {
        format!("{source}.{key}")
    };
    let detail = format!("`{key}{NOT_IMPORTED}");
    notes.push(ImportNote::new(note_source, NoteKind::Dropped, detail));
    Ok(())
}

/// Charge, then make and keep, one `Dropped` note whose source is `parts` joined.
fn fixed(
    doc: &mut Doc<'_>,
    notes: &mut Vec<ImportNote>,
    parts: &[&str],
    detail: &str,
) -> Result<(), DocumentError> {
    let source_len: usize = parts.iter().map(|p| p.len()).sum();
    doc.budget.charge(1)?;
    doc.budget
        .charge_bytes((source_len + detail.len()) as u64)?;
    notes.push(ImportNote::new(parts.concat(), NoteKind::Dropped, detail));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::bounds::Bounds;
    use super::super::operations::collect;
    use super::*;
    use serde_json::{Map, json};

    #[test]
    fn a_shared_target_is_read_once_and_noted_for_every_operation() {
        // Every operation refs one body whose many extension keys are free to note; reading
        // them again per operation would grow as operations times keys, uncharged.
        let mut body = Map::new();
        for i in 0..1000 {
            body.insert(format!("x-{i}"), json!(1));
        }
        body.insert("zzProbe".into(), json!(1));
        body.insert("content".into(), json!({"application/json": {"x-m": 1}}));
        let ops: Map<String, Value> = (0..50)
            .map(|i| {
                let op = json!({"requestBody": {"$ref": "#/components/requestBodies/b"}});
                (format!("/p{i}"), json!({"post": op}))
            })
            .collect();
        let root = json!({"openapi": "3.0.3", "paths": ops,
            "components": {"requestBodies": {"b": body}}});
        let mut doc = Doc::new(&root, Version::V30, Bounds::DEFAULT);
        let operations = collect(&mut doc, &mut Vec::new()).expect("collects");
        for op in &operations {
            let mut notes = Vec::new();
            operation_notes(&mut doc, op, &mut notes).expect("notes");
            let want = format!("{}.requestBody.zzProbe", op.source);
            let sources: Vec<&str> = notes.iter().map(|n| n.source.as_str()).collect();
            assert_eq!(sources, [want.as_str()]);
        }
        // Each operation reads its own key once; the body and its media type are read once.
        assert!(doc.walked.read < 1100, "{} keys read", doc.walked.read);
    }

    #[test]
    fn shared_responses_are_scanned_for_links_once() {
        // Every path refs one path item whose operation has many responses; scanning them
        // again per path would grow as paths times responses, uncharged.
        let mut responses = Map::new();
        for i in 0..1000 {
            responses.insert(format!("{}", 1000 + i), json!({"description": "d"}));
        }
        responses.insert("200".into(), json!({"links": {}}));
        let mut paths = Map::new();
        paths.insert("/p".into(), json!({"get": {"responses": responses}}));
        for i in 0..50 {
            paths.insert(format!("/q{i}"), json!({"$ref": "#/paths/~1p"}));
        }
        let root = json!({"openapi": "3.0.3", "paths": paths});
        let mut doc = Doc::new(&root, Version::V30, Bounds::DEFAULT);
        let operations = collect(&mut doc, &mut Vec::new()).expect("collects");
        assert_eq!(operations.len(), 51);
        for op in &operations {
            let mut notes = Vec::new();
            operation_notes(&mut doc, op, &mut notes).expect("notes");
            let want = format!("{}.responses.200.links", op.source);
            let sources: Vec<&str> = notes.iter().map(|n| n.source.as_str()).collect();
            assert_eq!(sources, [want.as_str()]);
        }
        // Each operation reads its own key once; the shared responses are scanned once.
        assert!(doc.walked.read < 1200, "{} keys read", doc.walked.read);
    }
}
