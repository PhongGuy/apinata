//! `Dropped` notes for Postman input the importer has no place for: saved example responses,
//! folder variables, any member a model struct leaves in its `rest` map, and the members of
//! list entries and body modes the parser never reads. Also reads the `event` scripts of the
//! collection, folders and requests, with a `Converted` note per script carried.

use super::model::{Body, Collection, Item, METADATA, Request, Url};
use super::script::{
    directive_prologue_end, needs_cryptojs_require, translate, unavailable_globals,
};
use super::{ImportNote, ImportedScripts, NoteKind, js, js_string, truthy};
use serde::Deserialize;
use serde_json::{Map, Value};

/// Notes for the collection's own and its `info`'s unmapped members; returns its scripts.
pub(super) fn collection(collection: &Collection, notes: &mut Vec<ImportNote>) -> ImportedScripts {
    let mut scripts = ImportedScripts::default();
    unmapped(&collection.rest, "collection / ", notes, Some(&mut scripts));
    unmapped(&collection.info.rest, "collection / info.", notes, None);
    auth(collection.auth.as_ref(), "collection / auth.", notes);
    scripts
}

/// Notes for a folder whose breadcrumb is `crumb`; returns its scripts.
pub(super) fn folder(item: &Item, crumb: &str, notes: &mut Vec<ImportNote>) -> ImportedScripts {
    let mut scripts = ImportedScripts::default();
    unmapped(
        &item.rest,
        &format!("{crumb} / "),
        notes,
        Some(&mut scripts),
    );
    auth(item.auth.as_ref(), &format!("{crumb} / auth."), notes);
    scripts
}

/// Notes for a written request item, its `request` and the request's URL, body and auth;
/// returns the item's scripts.
pub(super) fn request(
    item: &Item,
    request: &Request,
    body: &Body,
    crumb: &str,
    notes: &mut Vec<ImportNote>,
) -> ImportedScripts {
    let mut scripts = ImportedScripts::default();
    unmapped(
        &item.rest,
        &format!("{crumb} / "),
        notes,
        Some(&mut scripts),
    );
    // Only the request's own auth is read; one beside `request` would otherwise vanish.
    if truthy(item.auth.as_ref()) {
        notes.push(ImportNote::new(
            format!("{crumb} / auth"),
            NoteKind::Dropped,
            "the item authentication is not kept; only the request's is",
        ));
    }
    unmapped(&request.rest, &format!("{crumb} / request."), notes, None);
    // Header text, whole or per entry, is split into entries, so it loses nothing.
    if !matches!(request.header, Some(Value::String(_))) {
        let header = |entry: &Map<String, Value>, key: &str| {
            ENTRY.contains(&key) || (key == "name" && entry.get("name") == entry.get("key"))
        };
        list(
            request.header.as_ref(),
            true,
            &header,
            &format!("{crumb} / request.header"),
            notes,
        );
    }
    if let Some(value @ Value::Object(_)) = &request.url
        && let Ok(url) = Url::deserialize(value)
    {
        unmapped(&url.rest, &format!("{crumb} / request.url."), notes, None);
        let at = format!("{crumb} / request.url.");
        list(
            url.query.as_ref(),
            false,
            &known(&ENTRY),
            &format!("{at}query"),
            notes,
        );
        list(
            url.variable.as_ref(),
            false,
            &known(&PATH_VARIABLE),
            &format!("{at}variable"),
            notes,
        );
    }
    unmapped(&body.rest, &format!("{crumb} / request.body."), notes, None);
    body_modes(body, &format!("{crumb} / request.body."), notes);
    auth(
        request.auth.as_ref(),
        &format!("{crumb} / request.auth."),
        notes,
    );
    scripts
}

/// The members read from header, query and urlencoded entries; `type` is metadata there.
const ENTRY: [&str; 5] = ["key", "value", "disabled", "description", "type"];
/// Path variables have no enabled flag, so `disabled` is not read there.
const PATH_VARIABLE: [&str; 4] = ["key", "value", "description", "type"];
const FORMDATA: [&str; 7] = [
    "key",
    "value",
    "disabled",
    "description",
    "type",
    "src",
    "contentType",
];

/// The body members each mode reads; the parser keeps only the one `mode` names.
const BODY_MODES: [&str; 5] = ["raw", "urlencoded", "formdata", "file", "graphql"];

/// A member filter that accepts the names in `names`.
fn known<'a>(names: &'a [&'a str]) -> impl Fn(&Map<String, Value>, &str) -> bool + 'a {
    move |_, key| names.contains(&key)
}

/// Notes for body data the parser does not read: everything when the mode is missing or not
/// one it knows, else the data under the other modes and unread members of the active list.
fn body_modes(body: &Body, at: &str, notes: &mut Vec<ImportNote>) {
    let members = [
        &body.raw,
        &body.urlencoded,
        &body.formdata,
        &body.file,
        &body.graphql,
    ];
    let mode = body.mode.as_ref().and_then(Value::as_str);
    let Some(mode) = mode.filter(|m| BODY_MODES.contains(m)) else {
        if members.iter().any(|m| m.as_ref().is_some_and(has_data)) {
            notes.push(ImportNote::new(
                format!("{at}mode"),
                NoteKind::Dropped,
                "the body mode is missing or unknown, so the body is left out",
            ));
        }
        return;
    };
    for (name, member) in BODY_MODES.iter().zip(members) {
        if *name != mode && member.as_ref().is_some_and(has_data) {
            notes.push(ImportNote::new(
                format!("{at}{name}"),
                NoteKind::Dropped,
                format!("only the `{mode}` body is kept"),
            ));
        }
    }
    match mode {
        "urlencoded" => list(
            body.urlencoded.as_ref(),
            false,
            &known(&ENTRY),
            &format!("{at}urlencoded"),
            notes,
        ),
        "formdata" => list(
            body.formdata.as_ref(),
            false,
            &known(&FORMDATA),
            &format!("{at}formdata"),
            notes,
        ),
        _ => {}
    }
}

/// Notes for a list member at `at`: the whole member when it is not a list but carries data,
/// else each non-object entry (text too unless `text_ok`) and each entry member that `read`
/// rejects and is not metadata. A `null` entry fails the item elsewhere, so it is passed over.
fn list(
    value: Option<&Value>,
    text_ok: bool,
    read: &dyn Fn(&Map<String, Value>, &str) -> bool,
    at: &str,
    notes: &mut Vec<ImportNote>,
) {
    let entries = match value {
        Some(Value::Array(entries)) => entries,
        Some(other) if has_data(other) => {
            notes.push(ImportNote::new(
                at,
                NoteKind::Dropped,
                "this member is not a list",
            ));
            return;
        }
        _ => return,
    };
    for (index, entry) in entries.iter().enumerate() {
        let source = format!("{at}[{index}]");
        match entry {
            Value::Object(entry) => {
                let unread = |key: &str| !read(entry, key);
                members(entry, &unread, &format!("{source}."), notes);
            }
            Value::Null => {}
            Value::String(_) if text_ok => {}
            _ => notes.push(ImportNote::new(
                source,
                NoteKind::Dropped,
                "the entry is not an object",
            )),
        }
    }
}

/// One `Dropped` note at `<at><key>` per member of `entry` that `unread` accepts and that is
/// not metadata.
pub(super) fn members(
    entry: &Map<String, Value>,
    unread: &dyn Fn(&str) -> bool,
    at: &str,
    notes: &mut Vec<ImportNote>,
) {
    for key in entry.keys() {
        if unread(key) && !METADATA.contains(&key.as_str()) {
            notes.push(ImportNote::new(
                format!("{at}{key}"),
                NoteKind::Dropped,
                "this member has no counterpart",
            ));
        }
    }
}

/// Whether a value holds anything beyond `null`, `false`, empty text and empty containers.
fn has_data(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(_) => true,
        Value::String(s) => !s.is_empty(),
        Value::Array(items) => items.iter().any(has_data),
        Value::Object(map) => map.values().any(has_data),
    }
}

/// One note per member of `rest` that is not metadata, at `<at><key>`. With `scripts`, the
/// `event` member is read into them; without, it is an ordinary unmapped member. An empty
/// `response` list is no loss.
fn unmapped(
    rest: &Map<String, Value>,
    at: &str,
    notes: &mut Vec<ImportNote>,
    mut scripts: Option<&mut ImportedScripts>,
) {
    for (key, value) in rest {
        if METADATA.contains(&key.as_str()) {
            continue;
        }
        let source = format!("{at}{key}");
        match (key.as_str(), value) {
            ("event", Value::Array(events)) if scripts.is_some() => {
                if let Some(scripts) = scripts.as_deref_mut() {
                    *scripts = events_to_scripts(events, &source, notes);
                }
            }
            ("response", Value::Array(examples)) if examples.is_empty() => {}
            ("response", _) => notes.push(ImportNote::new(
                source,
                NoteKind::Dropped,
                "saved example responses are not imported",
            )),
            ("variable", _) => notes.push(ImportNote::new(
                source,
                NoteKind::Dropped,
                "variables are only kept on the collection",
            )),
            _ => notes.push(ImportNote::new(
                source,
                NoteKind::Dropped,
                "this member has no counterpart",
            )),
        }
    }
}

/// The scripts of an `event` list, as the reference reads them (`postman-to-bruno.js`
/// `importScriptsFromEvents`, usebruno/bruno @1301ea17): `prerequest` is the pre-request
/// script, `test` the post-response one, and the last non-blank event of a kind wins. Each
/// carried script gets a `Converted` note, plus one per `pm` API that was left as written;
/// a script that is superseded or of another kind gets a `Dropped` one.
fn events_to_scripts(events: &[Value], at: &str, notes: &mut Vec<ImportNote>) -> ImportedScripts {
    let kind = |event: &Value| match event.get("listen").and_then(Value::as_str) {
        Some("prerequest") => Some(0),
        Some("test") => Some(1),
        _ => None,
    };
    let mut winner = [None, None];
    for (index, event) in events.iter().enumerate() {
        if let (Some(kind), Some(_)) = (kind(event), exec_text(event)) {
            winner[kind] = Some(index);
        }
    }
    let mut out = ImportedScripts::default();
    for (index, event) in events.iter().enumerate() {
        let Some(code) = exec_text(event) else {
            continue;
        };
        let source = format!("{at}[{index}]");
        let Some(k) = kind(event) else {
            notes.push(ImportNote::new(
                source,
                NoteKind::Dropped,
                "only `prerequest` and `test` scripts are kept",
            ));
            continue;
        };
        let label = ["pre-request", "post-response"][k];
        if winner[k] != Some(index) {
            notes.push(ImportNote::new(
                source,
                NoteKind::Dropped,
                format!("a later {label} script replaces this one"),
            ));
            continue;
        }
        let translated = translate(&code);
        let detail = match translated.replaced {
            0 => format!("{label} script carried as written; it asks before it first runs"),
            n => format!(
                "{label} script carried; {n} pm call{} translated to the app's script API; it asks before it first runs",
                if n == 1 { "" } else { "s" }
            ),
        };
        notes.push(ImportNote::new(source.clone(), NoteKind::Converted, detail));
        for name in &translated.unsupported {
            notes.push(ImportNote::new(
                source.clone(),
                NoteKind::Converted,
                format!("uses {name}, which has no counterpart here; the line was kept as written"),
            ));
        }
        let mut code = translated.code;
        if needs_cryptojs_require(&code) {
            // After a directive prologue, so `'use strict'` stays the first statement.
            match directive_prologue_end(&code) {
                0 => code.insert_str(0, "const CryptoJS = require('crypto-js');\n"),
                at => code.insert_str(at, "\nconst CryptoJS = require('crypto-js');"),
            }
            notes.push(ImportNote::new(
                source.clone(),
                NoteKind::Converted,
                "uses CryptoJS; added const CryptoJS = require('crypto-js') at the top",
            ));
        }
        for name in unavailable_globals(&code) {
            notes.push(ImportNote::new(
                source.clone(),
                NoteKind::Converted,
                format!("uses {name}, which is not available here; the script was kept as written"),
            ));
        }
        match k {
            0 => out.pre_request = code,
            _ => out.post_response = code,
        }
    }
    out
}

/// An event's `script.exec`, a string or lines joined by a newline, when it has non-blank text.
fn exec_text(event: &Value) -> Option<String> {
    let text = match event.get("script").and_then(|s| s.get("exec"))? {
        Value::String(code) => code.clone(),
        Value::Array(lines) => lines.iter().map(js_string).collect::<Vec<_>>().join("\n"),
        _ => return None,
    };
    (!js::js_trim(&text).is_empty()).then_some(text)
}

/// Notes for an auth object's members other than `type` and the parameters it names.
fn auth(value: Option<&Value>, at: &str, notes: &mut Vec<ImportNote>) {
    let Some(Value::Object(auth)) = value else {
        return;
    };
    let kind = auth.get("type").and_then(Value::as_str);
    for key in auth.keys() {
        if key == "type" || Some(key.as_str()) == kind || METADATA.contains(&key.as_str()) {
            continue;
        }
        notes.push(ImportNote::new(
            format!("{at}{key}"),
            NoteKind::Dropped,
            "this authentication member has no counterpart",
        ));
    }
}
