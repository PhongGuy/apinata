//! The Postman v2.1 collection exporter: tree walk, item order, requests, URLs and notes.

mod script;

use std::collections::HashMap;

use serde_json::{Map, Value, json};

use super::auth::auth;
use super::body::body;
use super::extras::{collection_variables, events, responses};
use super::{ExportNote, PostmanExport, PostmanOptions};
use crate::collection::{Collection, Folder};
use crate::edit::{AuthSpec, BodySpec, KeyValue, Param, ParamKind, Timeout};
use crate::js::js_trim;
use crate::request::RequestFile;
use crate::resolve::HeaderSource;
use crate::sequence::sort_by_name_then_seq;

const SCHEMA: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";
const VARIABLES: &str = "variables set here have no place in the exported file";
const POST_RESPONSE: &str = "post-response variables have no place in the exported file";
const ASSERTIONS: &str = "assertions have no place in the exported file";
const SCRIPTS: &str = "scripts are exported as written; their API calls are not translated";
const KEPT: &str = "scripts exported as written; their API calls are not translated";

/// The collection as Postman v2.1 JSON, with a note for each thing the file cannot carry.
/// Only HTTP requests are exported; other item types are noted and left out.
pub fn postman(collection: &Collection) -> PostmanExport {
    postman_with(collection, &PostmanOptions::default())
}

/// [`postman`] with the given options.
pub fn postman_with(collection: &Collection, options: &PostmanOptions) -> PostmanExport {
    let mut notes = Vec::new();
    let file = collection.file();
    let mut info = Map::new();
    info.insert("name".into(), collection.name().into());
    insert_description(&mut info, file.docs());
    info.insert("schema".into(), SCHEMA.into());
    // Postman has no collection or folder headers, so each request carries copies; count
    // how many requests each source reaches before the notes are written.
    let mut copies = HashMap::new();
    count_copies(collection, collection.tree(), &mut copies);
    if !file.headers().is_empty() {
        let detail = headers_note(copies.get("").copied().unwrap_or(0));
        notes.push(ExportNote::new("collection / headers", detail));
    }
    let spec = file.auth();
    note_unsupported_auth(&mut notes, &["collection"], &spec);

    let mut scripted = false;
    let item = folder_items(
        collection,
        &copies,
        collection.tree(),
        &[],
        &mut notes,
        &mut scripted,
    );
    let mut root = Map::new();
    root.insert("info".into(), info.into());
    root.insert("item".into(), item.into());
    root.insert(
        "variable".into(),
        collection_variables(file, &mut notes).into(),
    );
    if let Some(auth) = auth(&spec) {
        root.insert("auth".into(), auth);
    }
    insert_events(&mut root, events(|p| file.script(p)), &mut scripted);
    if scripted {
        let detail = if options.keep_scripts { KEPT } else { SCRIPTS };
        notes.push(ExportNote::new("scripts", detail));
    }
    let root = Value::Object(root);
    // A `serde_json::Value` has only string keys, so serializing it cannot fail.
    #[allow(clippy::expect_used)]
    let mut json = serde_json::to_string_pretty(&root).expect("a JSON value serializes");
    json.push('\n');
    PostmanExport { json, notes }
}

/// A folder's children as Postman items: sub-folders in display order, then requests; other
/// item types are noted. `trail` holds the display names of the folders above; `scripted`
/// turns true once any event is written.
fn folder_items(
    collection: &Collection,
    copies: &HashMap<String, usize>,
    folder: &Folder,
    trail: &[&str],
    notes: &mut Vec<ExportNote>,
    scripted: &mut bool,
) -> Vec<Value> {
    let mut items = Vec::new();
    let folders =
        sort_by_name_then_seq(folder.folders().iter().collect(), |f| f.name(), |f| f.seq());
    for sub in folders {
        let path = [trail, &[sub.name()]].concat();
        let mut out = Map::new();
        out.insert("name".into(), sub.name().into());
        let mut folder_auth = AuthSpec::Inherit;
        let mut folder_events = Vec::new();
        if let Some(file) = sub.file() {
            insert_description(&mut out, file.docs());
            if !file.headers().is_empty() {
                let key = collection
                    .item_path(sub.path())
                    .map(|p| p.as_str().to_owned());
                let n = key.and_then(|k| copies.get(&k).copied()).unwrap_or(0);
                let source = [path.as_slice(), &["headers"]].concat().join(" / ");
                notes.push(ExportNote::new(source, headers_note(n)));
            }
            note_if(
                notes,
                &path,
                "variables",
                !file.vars().is_empty(),
                VARIABLES,
            );
            let post = !file.post_response_vars().is_empty();
            note_if(notes, &path, "post-response variables", post, POST_RESPONSE);
            folder_auth = file.auth();
            note_unsupported_auth(notes, &path, &folder_auth);
            folder_events = events(|p| file.script(p));
        }
        let children = folder_items(collection, copies, sub, &path, notes, scripted);
        out.insert("item".into(), children.into());
        if let Some(auth) = auth(&folder_auth) {
            out.insert("auth".into(), auth);
        }
        insert_events(&mut out, folder_events, scripted);
        items.push(Value::Object(out));
    }
    for req in folder.requests() {
        let path = [trail, &[req.name()]].concat();
        items.push(request_item(collection, req, &path, notes, scripted));
    }
    for opaque in folder.opaque_items() {
        let source = [trail, &[opaque.name()]].concat().join(" / ");
        let detail = format!(
            "only HTTP requests are exported; this {} item was left out",
            opaque.kind()
        );
        notes.push(ExportNote::new(source, detail));
    }
    items
}

/// One request item, noting the request members Postman has no place for.
fn request_item(
    collection: &Collection,
    req: &RequestFile,
    path: &[&str],
    notes: &mut Vec<ExportNote>,
    scripted: &mut bool,
) -> Value {
    note_if(notes, path, "variables", !req.vars().is_empty(), VARIABLES);
    let post = !req.post_response_vars().is_empty();
    note_if(notes, path, "post-response variables", post, POST_RESPONSE);
    note_if(
        notes,
        path,
        "assertions",
        !req.assertions().is_empty(),
        ASSERTIONS,
    );

    let mut request = Map::new();
    request.insert("method".into(), req.method().into());
    let header: Vec<Value> = inherited_rows(collection, req)
        .iter()
        .map(header)
        .chain(req.headers().iter().map(header))
        .collect();
    request.insert("header".into(), header.into());
    let auth_spec = req.auth();
    match auth(&auth_spec) {
        Some(auth) => {
            request.insert("auth".into(), auth);
        }
        None if auth_spec != AuthSpec::Inherit => {
            request.insert("auth".into(), json!({"type": "noauth"}));
        }
        None => {}
    }
    request.insert("description".into(), req.docs().into());
    request.insert("url".into(), url(req.url(), &req.shown_params()));
    let spec = req.body();
    if let BodySpec::Unsupported { kind } = &spec {
        let detail = format!("the {kind} body cannot be exported; an empty body was written");
        note_if(notes, path, "body", true, &detail);
    }
    let body = body(&spec);
    note_unsupported_auth(notes, path, &auth_spec);

    // Postman drops a GET, HEAD or OPTIONS body unless told not to prune it.
    let mut item = Map::new();
    item.insert("name".into(), req.name().into());
    let method = req.method().to_uppercase();
    let mut behavior = Map::new();
    if body.is_some() && matches!(method.as_str(), "GET" | "HEAD" | "OPTIONS") {
        behavior.insert("disableBodyPruning".into(), true.into());
    }
    settings_behavior(&mut behavior, notes, path, req);
    if !behavior.is_empty() {
        item.insert("protocolProfileBehavior".into(), behavior.into());
    }
    if let Some(body) = body {
        request.insert("body".into(), body);
    }
    item.insert("request".into(), request.into());
    insert_events(&mut item, events(|p| req.script(p)), scripted);
    let examples = req.examples();
    if !examples.is_empty() {
        item.insert("response".into(), responses(&examples).into());
    }
    Value::Object(item)
}

/// Write the settings that differ from what the importer assumes, and note the ones Postman
/// has no slot for (timeout, tags).
fn settings_behavior(
    behavior: &mut Map<String, Value>,
    notes: &mut Vec<ExportNote>,
    path: &[&str],
    req: &RequestFile,
) {
    // Postman keys the omitted default headers by lowercased name.
    let omitted: Map<String, Value> = req
        .omitted_headers()
        .iter()
        .map(|name| (name.to_lowercase(), true.into()))
        .collect();
    if !omitted.is_empty() {
        behavior.insert("disabledSystemHeaders".into(), omitted.into());
    }
    let s = req.settings();
    if !s.encode_url {
        behavior.insert("disableUrlEncoding".into(), true.into());
    }
    if !s.follow_redirects {
        behavior.insert("followRedirects".into(), false.into());
    }
    if s.max_redirects != 5 {
        behavior.insert("maxRedirects".into(), s.max_redirects.into());
    }
    if s.forward_authorization_header {
        behavior.insert("followAuthorizationHeader".into(), true.into());
    }
    let kept = " when imported, the request will inherit the default timeout";
    let timeout = match s.timeout {
        Timeout::Inherit => None,
        Timeout::Unlimited => Some(format!("the request has no timeout;{kept}")),
        Timeout::Millis(ms) => Some(format!("the {ms} ms timeout cannot be exported;{kept}")),
    };
    if let Some(detail) = timeout {
        note_if(notes, path, "timeout", true, &detail);
    }
    let tags = req.tags();
    let detail = format!(
        "tags have no place in Postman and were left out: {}",
        tags.join(", ")
    );
    note_if(notes, path, "tags", !tags.is_empty(), &detail);
}

/// Add `event` when there are any, and record that a script was exported.
fn insert_events(out: &mut Map<String, Value>, events: Vec<Value>, scripted: &mut bool) {
    if !events.is_empty() {
        *scripted = true;
        out.insert("event".into(), events.into());
    }
}

/// The headers `req` inherits and does not override, outermost first, as enabled rows.
fn inherited_rows(collection: &Collection, req: &RequestFile) -> Vec<KeyValue> {
    let Some(path) = collection.item_path(req.path()) else {
        return Vec::new();
    };
    let own = req.headers();
    collection
        .inherited_headers(&path)
        .iter()
        .filter(|h| !h.overridden(&own))
        .map(|h| KeyValue::new(h.name(), h.value()))
        .collect()
}

/// Add, for every request under `folder`, one to the count of each source that gives it a row.
/// The key is the source's item path, empty for the collection.
fn count_copies(collection: &Collection, folder: &Folder, copies: &mut HashMap<String, usize>) {
    for sub in folder.folders() {
        count_copies(collection, sub, copies);
    }
    for req in folder.requests() {
        let Some(path) = collection.item_path(req.path()) else {
            continue;
        };
        let own = req.headers();
        let mut sources: Vec<String> = Vec::new();
        for h in collection.inherited_headers(&path) {
            if h.overridden(&own) {
                continue;
            }
            let key = match h.source() {
                HeaderSource::Folder(p) => p.as_str().to_owned(),
                _ => String::new(),
            };
            if !sources.contains(&key) {
                sources.push(key);
            }
        }
        for key in sources {
            *copies.entry(key).or_default() += 1;
        }
    }
}

/// The note for a collection or folder with headers that reached `n` requests.
fn headers_note(n: usize) -> String {
    const WHY: &str = "because Postman has no collection or folder headers";
    match n {
        0 => "headers were not copied anywhere: no request uses them (the folder is empty, or every request sets its own)".into(),
        1 => format!("headers were copied into 1 request, {WHY}; importing the file back gives that request its own copy"),
        n => format!("headers were copied into {n} requests, {WHY}; importing the file back gives each of those requests its own copy"),
    }
}

pub(super) fn header(kv: &KeyValue) -> Value {
    json!({
        "key": kv.name,
        "value": kv.value,
        "description": kv.description.clone().unwrap_or_default(),
        "disabled": !kv.enabled,
        "type": "default",
    })
}

/// A Postman URL object: `raw` verbatim, protocol before the first `://`, then host and path
/// split from what precedes the first `?` or `#`.
pub(super) fn url(raw: &str, params: &[Param]) -> Value {
    let (protocol, rest) = raw.split_once("://").unwrap_or(("", raw));
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let parts = |s: &str, sep: char| -> Vec<String> {
        if s.is_empty() {
            Vec::new()
        } else {
            s.split(sep).map(str::to_owned).collect()
        }
    };

    let mut query = Vec::new();
    let mut variable = Vec::new();
    for p in params {
        let mut entry = Map::new();
        entry.insert("key".into(), p.name.as_str().into());
        entry.insert("value".into(), p.value.as_str().into());
        if let Some(d) = p.description.as_deref().filter(|d| !d.is_empty()) {
            entry.insert("description".into(), d.into());
        }
        match p.kind {
            ParamKind::Path => variable.push(Value::Object(entry)),
            _ => {
                if !p.enabled {
                    entry.insert("disabled".into(), true.into());
                }
                query.push(Value::Object(entry));
            }
        }
    }
    json!({
        "raw": raw,
        "protocol": protocol,
        "host": parts(host, '.'),
        "path": parts(path, '/'),
        "query": query,
        "variable": variable,
    })
}

/// Add `description` when the docs are not blank.
pub(super) fn insert_description(out: &mut Map<String, Value>, docs: String) {
    if !js_trim(&docs).is_empty() {
        out.insert("description".into(), docs.into());
    }
}

fn note_if(notes: &mut Vec<ExportNote>, path: &[&str], member: &str, lost: bool, detail: &str) {
    if lost {
        notes.push(ExportNote::new(
            format!("{} / {member}", path.join(" / ")),
            detail,
        ));
    }
}

/// Note an auth mode the export could not carry, at `<path> / auth`.
fn note_unsupported_auth(notes: &mut Vec<ExportNote>, path: &[&str], spec: &AuthSpec) {
    let mode = match spec {
        AuthSpec::Unsupported { mode } => mode.as_str(),
        AuthSpec::Wsse { .. } => "wsse",
        _ => return,
    };
    let detail = format!("the {mode} authentication cannot be exported");
    note_if(notes, path, "auth", true, &detail);
}
