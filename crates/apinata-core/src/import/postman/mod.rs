//! Postman v2.0 and v2.1 collections. Every rule follows the reference converter,
//! `postman-to-bruno.js` at the pinned commit, including its JavaScript coercions.

mod details;
mod env;
mod model;
mod notes;
mod script;
pub(crate) use script::pm_chain_at;

use super::load::json_error_offset;
use super::{
    ImportNote, ImportedCollection, ImportedFolder, ImportedItem, ImportedRequest,
    ImportedRequestItem, ImportedScripts, NoteKind, PostmanError,
};
use crate::edit::{AuthSpec, BodySpec, FileEntry, KeyValue, MultipartField, Param, ParamKind};
use crate::request::RequestKind;
use crate::{js, multipart};
pub use env::parse_postman_environment;
use model::{Body, Collection, Item, Request, Url};
use regress::Regex;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

const V2_SCHEMAS: [&str; 4] = [
    "https://schema.getpostman.com/json/collection/v2.0.0/collection.json",
    "https://schema.getpostman.com/json/collection/v2.1.0/collection.json",
    "https://schema.postman.com/json/collection/v2.0.0/collection.json",
    "https://schema.postman.com/json/collection/v2.1.0/collection.json",
];

// A fixed pattern, so a failure to compile is a bug every run hits, not an input error.
#[allow(clippy::expect_used)]
static JSON_TYPE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[\w\-]+\/([\w\-]+\+)?json").expect("the JSON content type pattern compiles")
});
// A fixed pattern, so a failure to compile is a bug every run hits, not an input error.
#[allow(clippy::expect_used)]
static XML_TYPE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[\w\-]+\/([\w\-]+\+)?xml").expect("the XML content type pattern compiles")
});

/// Read a Postman v2.0 or v2.1 collection, bare or in a `{collection: ...}` envelope. Items
/// that cannot be imported are left out with a note in `unwritten`. Fails on invalid JSON, a
/// schema other than v2.0 or v2.1 (checked first), or a collection of the wrong shape.
pub fn parse_postman(json: &[u8]) -> Result<ImportedCollection, PostmanError> {
    let root: Value = serde_json::from_slice(json).map_err(|e| PostmanError::Parse {
        offset: json_error_offset(json, &e),
        message: e.to_string(),
    })?;
    let root = match root {
        Value::Object(mut map)
            if map
                .get("collection")
                .is_some_and(|inner| truthy(inner.get("info"))) =>
        {
            map.remove("collection").unwrap_or_default()
        }
        other => other,
    };
    let schema = root.get("info").and_then(|info| info.get("schema"));
    if !schema
        .and_then(Value::as_str)
        .is_some_and(|s| V2_SCHEMAS.contains(&s))
    {
        let schema = match schema {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(s)) => s.clone(),
            Some(other) => js::stringify(other),
        };
        return Err(PostmanError::UnsupportedSchema { schema });
    }
    let collection = Collection::deserialize(root).map_err(invalid)?;
    let mut unwritten = Vec::new();
    let mut notes = Vec::new();
    let vars = details::variables(collection.variable.as_ref(), &mut notes);
    let auth = details::auth(
        collection.auth.as_ref(),
        AuthSpec::None,
        "collection / auth".to_owned(),
        &mut notes,
    );
    details::folder_behavior(
        collection.protocol_profile_behavior.as_ref(),
        "collection / protocolProfileBehavior".to_owned(),
        &mut notes,
    );
    let scripts = notes::collection(&collection, &mut notes);
    let items = items(&collection.item, "", &mut unwritten)?;
    Ok(ImportedCollection {
        scripts,
        name: name_or(collection.info.name.as_ref(), "Untitled Collection"),
        docs: transform_description(collection.info.description.as_ref()),
        auth,
        headers: Vec::new(),
        vars,
        items,
        notes,
        unwritten,
        spec_source: false,
    })
}

fn invalid(error: serde_json::Error) -> PostmanError {
    PostmanError::Invalid {
        message: error.to_string(),
    }
}

/// Map the siblings in `list`, whose parent's breadcrumb is `parent` (empty at the root).
/// Folder and request names are deduplicated separately; `seq` is the input position, so a
/// skipped item leaves a gap.
fn items(
    list: &[Value],
    parent: &str,
    unwritten: &mut Vec<ImportNote>,
) -> Result<Vec<ImportedItem>, PostmanError> {
    let mut folder_names = HashSet::new();
    let mut request_names = HashSet::new();
    let mut out = Vec::new();
    for (index, value) in list.iter().enumerate() {
        let seq = index as u64 + 1;
        let crumb = |label: String| {
            if parent.is_empty() {
                label
            } else {
                format!("{parent} / {label}")
            }
        };
        // Arrays pass the reference's `typeof === 'object'` check and read as memberless.
        let item = match value {
            Value::Object(_) => Item::deserialize(value).map_err(invalid)?,
            Value::Array(_) => Item::default(),
            _ => {
                unwritten.push(ImportNote::new(
                    crumb(format!("Item {seq}")),
                    NoteKind::Dropped,
                    "the item is not an object",
                ));
                continue;
            }
        };
        let crumb = crumb(name_or(item.name.as_ref(), &format!("Item {seq}")));

        if !truthy(item.request.as_ref()) {
            let name = unique(
                &mut folder_names,
                name_or(item.name.as_ref(), "Untitled Folder"),
            );
            let children = match &item.item {
                Some(Value::Array(children)) => items(children, &crumb, unwritten)?,
                Some(Value::String(s)) if !s.is_empty() => {
                    return Err(PostmanError::Invalid {
                        message: format!("the items of folder `{crumb}` are not a list"),
                    });
                }
                _ => Vec::new(),
            };
            let mut notes = Vec::new();
            let auth = details::auth(
                item.auth.as_ref(),
                AuthSpec::Inherit,
                format!("{crumb} / auth"),
                &mut notes,
            );
            details::folder_behavior(
                item.protocol_profile_behavior.as_ref(),
                format!("{crumb} / protocolProfileBehavior"),
                &mut notes,
            );
            let scripts = notes::folder(&item, &crumb, &mut notes);
            out.push(ImportedItem::Folder(ImportedFolder {
                scripts,
                name,
                seq,
                docs: transform_description(item.description.as_ref()),
                auth,
                headers: Vec::new(),
                vars: Vec::new(),
                items: children,
                notes,
            }));
            continue;
        }

        let request = match &item.request {
            Some(value @ Value::Object(_)) => Request::deserialize(value).map_err(invalid)?,
            _ => Request::default(),
        };
        let Some(method) = request
            .method
            .as_ref()
            .and_then(Value::as_str)
            .filter(|m| !js::js_trim(m).is_empty())
        else {
            unwritten.push(ImportNote::new(
                crumb,
                NoteKind::Dropped,
                "the request has no method",
            ));
            continue;
        };
        let base_name = name_or(item.name.as_ref(), "Untitled Request");
        let body = match &request.body {
            Some(value @ Value::Object(_)) => Body::deserialize(value).map_err(invalid)?,
            _ => Body::default(),
        };
        match http_request(&request, &body, method, &crumb) {
            Ok((mut out_request, mut notes)) => {
                out_request.auth = details::auth(
                    request.auth.as_ref(),
                    AuthSpec::Inherit,
                    format!("{crumb} / request.auth"),
                    &mut notes,
                );
                out_request.settings = Some(details::settings(
                    item.protocol_profile_behavior.as_ref(),
                    &crumb,
                    &mut notes,
                ));
                out_request.omitted_headers =
                    details::omitted_headers(item.protocol_profile_behavior.as_ref());
                // The reference reads only `request.description`; one beside it is lost.
                if kept_description(item.description.as_ref()).is_some() {
                    notes.push(ImportNote::new(
                        format!("{crumb} / description"),
                        NoteKind::Dropped,
                        "the item description is not kept; only the request's is",
                    ));
                }
                out_request.scripts = notes::request(&item, &request, &body, &crumb, &mut notes);
                out.push(ImportedItem::Request(ImportedRequestItem {
                    name: unique(&mut request_names, base_name),
                    seq,
                    request: out_request,
                    notes,
                }));
            }
            Err(detail) => unwritten.push(ImportNote::new(crumb, NoteKind::Dropped, detail)),
        }
    }
    Ok(out)
}

/// `base`, or `base_1`, `base_2`, ... when taken; the result is recorded as taken.
fn unique(taken: &mut HashSet<String>, base: String) -> String {
    let mut name = base.clone();
    let mut count = 1;
    while taken.contains(&name) {
        name = format!("{base}_{count}");
        count += 1;
    }
    taken.insert(name.clone());
    name
}

/// One request. An `Err` is the detail of an input the reference fails on, which drops the
/// whole item.
fn http_request(
    source: &Request,
    body: &Body,
    method: &str,
    crumb: &str,
) -> Result<(ImportedRequest, Vec<ImportNote>), String> {
    let mut notes = Vec::new();
    let url_parts = match &source.url {
        Some(value @ Value::Object(_)) => {
            Some(Url::deserialize(value).map_err(|e| format!("the URL cannot be read: {e}"))?)
        }
        Some(Value::Array(_)) => Some(Url::default()),
        _ => None,
    };
    let url = match (&source.url, &url_parts) {
        (Some(Value::String(url)), _) => url.clone(),
        (_, Some(parts)) => match &parts.raw {
            Some(Value::String(raw)) if !raw.is_empty() => match raw.split_once('#') {
                Some((kept, fragment)) => {
                    notes.push(ImportNote::new(
                        format!("{crumb} / request.url.raw"),
                        NoteKind::Converted,
                        format!("the URL fragment `#{fragment}` was removed"),
                    ));
                    kept.to_owned()
                }
                None => raw.clone(),
            },
            _ => url_from_parts(parts),
        },
        _ => String::new(),
    };
    let mut out = ImportedRequest::new(method.to_uppercase(), url);
    out.docs = transform_description(source.description.as_ref());

    let headers = normalize_headers(source.header.as_ref())?;
    if body.mode.as_ref().and_then(Value::as_str) == Some("graphql") {
        out.kind = RequestKind::Graphql;
        let (query, variables) = super::request::graphql_parts(body.graphql.as_ref());
        out.body = BodySpec::Graphql { query, variables };
    } else {
        out.body = request_body(body, &headers)?;
    }
    out.headers = headers
        .iter()
        .filter(|h| field(h, "key").is_some() || field(h, "value").is_some())
        .map(|h| {
            let mut kv = KeyValue::new(
                ensure_string(field(h, "key")),
                ensure_string(field(h, "value")),
            );
            kv.enabled = !truthy(field(h, "disabled"));
            kv.description = kept_description(field(h, "description"));
            kv
        })
        .collect();

    if let Some(parts) = &url_parts {
        for q in entries(parts.query.as_ref())? {
            if field(q, "key").is_none() && field(q, "value").is_none() {
                continue;
            }
            let mut param = Param::new(
                ensure_string(field(q, "key")),
                ensure_string(field(q, "value")),
                ParamKind::Query,
            );
            param.enabled = !truthy(field(q, "disabled"));
            param.description = kept_description(field(q, "description"));
            out.params.push(param);
        }
        for v in entries(parts.variable.as_ref())? {
            if !truthy(field(v, "key")) {
                continue;
            }
            let mut param = Param::new(
                ensure_string(field(v, "key")),
                ensure_string(field(v, "value")),
                ParamKind::Path,
            );
            param.description = kept_description(field(v, "description"));
            out.params.push(param);
        }
    }
    Ok((out, notes))
}

/// The reference's `constructUrlFromParts`: protocol, dotted host, port, path and the query
/// entries that have a key.
fn url_from_parts(url: &Url) -> String {
    let protocol = url
        .protocol
        .as_ref()
        .map_or_else(|| "http".to_owned(), js_string);
    let host = match &url.host {
        Some(Value::Array(parts)) => parts
            .iter()
            .filter(|p| truthy(Some(p)))
            .map(js_string)
            .collect::<Vec<_>>()
            .join("."),
        other => truthy_string(other.as_ref()),
    };
    let path = match &url.path {
        Some(Value::Array(parts)) => parts.iter().map(js_string).collect::<Vec<_>>().join("/"),
        other => truthy_string(other.as_ref()),
    };
    let port = if truthy(url.port.as_ref()) {
        format!(":{}", truthy_string(url.port.as_ref()))
    } else {
        String::new()
    };
    let query = match &url.query {
        Some(Value::Array(entries)) if !entries.is_empty() => {
            let pairs: Vec<String> = entries
                .iter()
                .filter(|q| truthy(Some(q)) && truthy(field(q, "key")))
                .map(|q| {
                    format!(
                        "{}={}",
                        truthy_string(field(q, "key")),
                        truthy_string(field(q, "value"))
                    )
                })
                .collect();
            format!("?{}", pairs.join("&"))
        }
        _ => String::new(),
    };
    let path = if path.is_empty() {
        path
    } else {
        format!("/{path}")
    };
    format!("{protocol}://{host}{port}{path}{query}")
}

/// Headers as entry objects: a whole string splits into lines, and a string line or entry
/// splits at its first `:`. A `null` entry fails the item, as it does in the reference.
fn normalize_headers(headers: Option<&Value>) -> Result<Vec<Value>, String> {
    match headers {
        Some(Value::String(text)) => Ok(text
            .split('\n')
            .map(|line| line.strip_suffix('\r').unwrap_or(line))
            .filter(|line| !line.is_empty())
            .map(string_header)
            .collect()),
        Some(Value::Array(list)) => list
            .iter()
            .map(|header| match header {
                Value::String(line) => Ok(string_header(line)),
                Value::Null => Err("a header entry is null".to_owned()),
                other => Ok(other.clone()),
            })
            .collect(),
        _ => Ok(Vec::new()),
    }
}

fn string_header(line: &str) -> Value {
    let (key, value) = line.split_once(':').unwrap_or((line, ""));
    serde_json::json!({ "key": js::js_trim(key), "value": js::js_trim(value) })
}

/// The body for the reference's body modes; any other mode is no body.
fn request_body(body: &Body, headers: &[Value]) -> Result<BodySpec, String> {
    Ok(match body.mode.as_ref().and_then(Value::as_str) {
        Some("formdata") => {
            let mut fields = Vec::new();
            for part in entries(body.formdata.as_ref())? {
                if field(part, "key").is_none() && field(part, "value").is_none() {
                    continue;
                }
                let kind = field(part, "type").and_then(Value::as_str);
                let src = field(part, "src");
                let name = ensure_string(field(part, "key"));
                let mut entry = if kind == Some("file") || (kind == Some("default") && truthy(src))
                {
                    let paths = match src {
                        Some(Value::Array(paths)) => {
                            paths.iter().map(|p| ensure_string(Some(p))).collect()
                        }
                        Some(src) if truthy(Some(src)) => vec![ensure_string(Some(src))],
                        _ => Vec::new(),
                    };
                    MultipartField::files(name, paths)
                } else {
                    let value = match field(part, "value") {
                        Some(Value::Array(pieces)) => pieces.iter().map(js_string).collect(),
                        other => ensure_string(other),
                    };
                    MultipartField::text(name, value)
                };
                if truthy(field(part, "contentType")) {
                    entry.content_type = ensure_string(field(part, "contentType"));
                }
                entry.enabled = !truthy(field(part, "disabled"));
                entry.description = kept_description(field(part, "description"));
                fields.push(entry);
            }
            BodySpec::Multipart(fields)
        }
        Some("urlencoded") => {
            let mut fields = Vec::new();
            for pair in entries(body.urlencoded.as_ref())? {
                if field(pair, "key").is_none() && field(pair, "value").is_none() {
                    continue;
                }
                let mut kv = KeyValue::new(
                    ensure_string(field(pair, "key")),
                    ensure_string(field(pair, "value")),
                );
                kv.enabled = !truthy(field(pair, "disabled"));
                kv.description = kept_description(field(pair, "description"));
                fields.push(kv);
            }
            BodySpec::FormUrlEncoded(fields)
        }
        Some("raw") => {
            let raw = ensure_string(body.raw.as_ref());
            let language = body
                .options
                .as_ref()
                .and_then(|o| o.pointer("/raw/language"));
            let language = if truthy(language) {
                language.and_then(Value::as_str)
            } else {
                language_by_header(headers)?
            };
            match language {
                Some("json") => BodySpec::Json(raw),
                Some("xml") => BodySpec::Xml(raw),
                _ => BodySpec::Text(raw),
            }
        }
        Some("file") => {
            let path = ensure_string(body.file.as_ref().and_then(|f| field(f, "src")));
            let mut entry = FileEntry::new(path.as_str());
            entry.content_type = multipart::file_content_type(Path::new(&path)).to_owned();
            entry.selected = true;
            BodySpec::File(vec![entry])
        }
        _ => BodySpec::None,
    })
}

/// The raw body language implied by the first enabled `Content-Type` header, if any. A
/// non-string header key fails the item, as `toLowerCase` throws in the reference.
fn language_by_header(headers: &[Value]) -> Result<Option<&'static str>, String> {
    for header in headers {
        let key = match field(header, "key") {
            None => continue,
            Some(Value::String(key)) => key,
            Some(_) => return Err("a header name is not text".to_owned()),
        };
        if key.to_lowercase() != "content-type" || truthy(field(header, "disabled")) {
            continue;
        }
        let value = field(header, "value")
            .and_then(Value::as_str)
            .unwrap_or_default();
        return Ok(if JSON_TYPE.find(value).is_some() {
            Some("json")
        } else if XML_TYPE.find(value).is_some() {
            Some("xml")
        } else {
            None
        });
    }
    Ok(None)
}

/// The entries of a list member; anything but a list has none. A `null` entry fails the
/// item, since reading its members throws in the reference.
fn entries(list: Option<&Value>) -> Result<Vec<&Value>, String> {
    match list {
        Some(Value::Array(items)) => items
            .iter()
            .map(|entry| {
                if entry.is_null() {
                    Err("a list entry is null".to_owned())
                } else {
                    Ok(entry)
                }
            })
            .collect(),
        _ => Ok(Vec::new()),
    }
}

/// A member of an entry; `null` reads as absent, as `== null` does in the reference.
fn field<'a>(entry: &'a Value, key: &str) -> Option<&'a Value> {
    entry.get(key).filter(|v| !v.is_null())
}

/// JavaScript truthiness of an optional value.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(_) | Value::Object(_)) => true,
    }
}

/// `value || default` for a name, rendered as text.
fn name_or(value: Option<&Value>, default: &str) -> String {
    if truthy(value) {
        truthy_string(value)
    } else {
        default.to_owned()
    }
}

/// `${value || ''}`: the value as text when truthy, else empty.
fn truthy_string(value: Option<&Value>) -> String {
    match value {
        Some(v) if truthy(Some(v)) => js_string(v),
        _ => String::new(),
    }
}

/// JavaScript `String(value)` as `join` applies it, where `null` gives empty text.
fn js_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Array(items) => items.iter().map(js_string).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".to_owned(),
        Value::Number(_) | Value::Bool(_) => js::to_text(value),
    }
}

/// The reference's `ensureString`: empty for null or empty, JSON text for objects and lists.
fn ensure_string(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(v @ (Value::Array(_) | Value::Object(_))) => js::stringify(v),
        Some(v) => js::to_text(v),
    }
}

/// The reference's `transformDescription`: a string, or an object's `content`.
fn transform_description(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Object(map)) => map
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        _ => String::new(),
    }
}

/// A description kept only when it is not blank.
fn kept_description(value: Option<&Value>) -> Option<String> {
    Some(transform_description(value)).filter(|d| !js::js_trim(d).is_empty())
}
