//! An operation's responses, built from the request's saved examples.

use crate::export::ExportNote;
use crate::{Example, ExampleBodyKind};
use serde_json::{Map, Value, json};

/// The operation's `responses`: saved examples grouped by status, or the default response
/// when there are none. `source` is the request's note source; binary bodies are noted.
pub(super) fn responses(examples: &[Example], source: &str, notes: &mut Vec<ExportNote>) -> Value {
    if examples.is_empty() {
        return json!({"default": {"description": "Default response"}});
    }
    // A Vec scanned in order keeps statuses first-seen; each group remembers the keys it
    // has used across all its media types, so a repeated name gets the next free suffix.
    let mut groups: Vec<Group> = Vec::new();
    for example in examples {
        let res = example.response.as_ref();
        let status = res
            .and_then(|r| r.status)
            .filter(|c| *c != 0)
            .unwrap_or(200);
        let index = match groups.iter().position(|g| g.status == status) {
            Some(index) => index,
            None => {
                let text = res
                    .and_then(|r| r.status_text.as_deref())
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or("Example");
                let mut response = Map::new();
                response.insert("description".into(), text.into());
                groups.push(Group {
                    status,
                    response,
                    content: Map::new(),
                    used: Vec::new(),
                });
                groups.len() - 1
            }
        };
        let Some(body) = res.and_then(|r| r.body.as_ref()) else {
            continue;
        };
        let group = &mut groups[index];
        let key = free_key(&example.name, &mut group.used);
        let media = res
            .and_then(|r| {
                r.headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
            })
            .filter(|(_, v)| !v.trim().is_empty())
            .map_or_else(|| media_type(&body.kind).to_owned(), |(_, v)| v.clone());
        let mut entry = Map::new();
        match body.kind {
            ExampleBodyKind::Binary => {
                let detail = format!("the binary body of example {} was left out", example.name);
                notes.push(ExportNote::new(format!("{source} / examples"), detail));
            }
            ExampleBodyKind::Json => {
                let value = serde_json::from_str(&body.data)
                    .unwrap_or_else(|_| Value::String(body.data.clone()));
                entry.insert("value".into(), value);
            }
            ExampleBodyKind::Xml | ExampleBodyKind::Html | ExampleBodyKind::Text => {
                entry.insert("value".into(), body.data.as_str().into());
            }
        }
        let examples = group
            .content
            .entry(media)
            .or_insert_with(|| json!({"examples": {}}));
        examples["examples"]
            .as_object_mut()
            .expect("examples is an object")
            .insert(key, entry.into());
    }
    let mut out = Map::new();
    for mut group in groups {
        if !group.content.is_empty() {
            group
                .response
                .insert("content".into(), group.content.into());
        }
        out.insert(group.status.to_string(), group.response.into());
    }
    out.into()
}

/// One status's response object, its media types, and the example keys taken so far.
struct Group {
    status: u16,
    response: Map<String, Value>,
    content: Map<String, Value>,
    used: Vec<String>,
}

/// `name`, or `name 2`, `name 3`, ... when taken; the chosen key is recorded in `used`.
fn free_key(name: &str, used: &mut Vec<String>) -> String {
    let mut key = name.to_owned();
    let mut n = 2;
    while used.contains(&key) {
        key = format!("{name} {n}");
        n += 1;
    }
    used.push(key.clone());
    key
}

/// The media type a body kind implies when the example has no `Content-Type` header.
fn media_type(kind: &ExampleBodyKind) -> &'static str {
    match kind {
        ExampleBodyKind::Json => "application/json",
        ExampleBodyKind::Xml => "application/xml",
        ExampleBodyKind::Html => "text/html",
        ExampleBodyKind::Text => "text/plain",
        ExampleBodyKind::Binary => "application/octet-stream",
    }
}
