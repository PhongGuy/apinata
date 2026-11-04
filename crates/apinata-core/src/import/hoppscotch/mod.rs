//! Importing Hoppscotch collection and environment exports.

mod auth;
mod body;
mod env;
mod model;
mod template;
mod tree;

use super::budget::{ENTRIES, NOTE_BYTES};
use super::load::Loaded;
use super::{CollectionOptions, DocumentError, ImportBundle, ImportedEnvironment};
use serde_json::Value;

/// The newest collection and request versions whose shape is known.
const COLLECTION_VERSION: u64 = 12;
const REQUEST_VERSION: u64 = 17;

/// Parse a Hoppscotch collection export, one collection or an array of them.
pub(crate) fn parse_hoppscotch(
    loaded: Loaded,
    _options: &CollectionOptions,
) -> Result<Vec<ImportBundle>, DocumentError> {
    parse_with(loaded, NOTE_BYTES, ENTRIES)
}

/// `parse_hoppscotch` with at most `note_bytes` of note text and `entries` entries; past
/// either the import is `TooLarge`.
fn parse_with(
    loaded: Loaded,
    note_bytes: u64,
    entries: u64,
) -> Result<Vec<ImportBundle>, DocumentError> {
    let roots = match loaded.value {
        Value::Array(list) if list.is_empty() => {
            return Err(invalid("the export is an empty list"));
        }
        Value::Array(list) => list,
        object @ Value::Object(_) => vec![object],
        _ => return Err(invalid("not a Hoppscotch collection export")),
    };
    // Only a root must hold both lists; a nested folder missing either reads it as empty.
    for root in &roots {
        if !["folders", "requests"]
            .iter()
            .all(|key| root.get(key).is_some_and(Value::is_array))
        {
            return Err(invalid(
                "not a Hoppscotch collection: `folders` and `requests` must be lists",
            ));
        }
    }
    let mut budget = tree::Budget::new(note_bytes, entries);
    roots
        .into_iter()
        .map(|root| {
            Ok(ImportBundle {
                collection: tree::root(root, &mut budget)?,
                environments: Vec::new(),
            })
        })
        .collect()
}

/// Parse a Hoppscotch environment export, one environment or an array of them.
pub(crate) fn parse_hoppscotch_environments(
    loaded: Loaded,
) -> Result<Vec<ImportedEnvironment>, DocumentError> {
    env::environments(loaded.value, &mut tree::Budget::new(NOTE_BYTES, ENTRIES))
}

fn invalid(message: &str) -> DocumentError {
    DocumentError::Invalid {
        message: message.to_owned(),
    }
}

/// The note for a collection `v` read with the newest known shape, or `None` when the version
/// is missing (version 1) or known.
fn collection_version_note(v: Option<&Value>) -> Option<String> {
    match v {
        None => None,
        Some(v)
            if v.as_u64()
                .is_some_and(|n| (1..=COLLECTION_VERSION).contains(&n)) =>
        {
            None
        }
        Some(v) => Some(format!(
            "version {v} is not a known collection version; read as version {COLLECTION_VERSION}"
        )),
    }
}

/// How a request entry is read, from its `v` and, for a legacy entry, its members.
enum RequestKind {
    /// A version 0 request: `url` and `path`, string auth and body.
    Legacy,
    /// A version 1 or later request; `newer` when its version is past the newest known.
    Rest { newer: bool },
    /// A GraphQL request, which is not imported.
    GraphQl,
    /// A `v` that names no request version.
    Unknown,
}

impl RequestKind {
    /// A REST `v` is a digit string; GraphQL's is a number, and a legacy GraphQL entry has a
    /// `query` and a `url` but no `endpoint`.
    fn of(request: &Value) -> RequestKind {
        match request.get("v") {
            None => {
                let has = |key| request.get(key).is_some();
                if has("query") && has("url") && !has("endpoint") {
                    RequestKind::GraphQl
                } else {
                    RequestKind::Legacy
                }
            }
            Some(Value::Number(_)) => RequestKind::GraphQl,
            Some(Value::String(v)) if !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()) => {
                // A digit string too long for `u64` is still a newer version.
                match v.parse::<u64>() {
                    Ok(0) => RequestKind::Legacy,
                    Ok(n) if n <= REQUEST_VERSION => RequestKind::Rest { newer: false },
                    _ => RequestKind::Rest { newer: true },
                }
            }
            Some(_) => RequestKind::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::load::load;
    use serde_json::json;

    fn parse(root: &Value, note_bytes: u64, entries: u64) -> Result<usize, DocumentError> {
        let bundles = parse_with(load(root.to_string().as_bytes())?, note_bytes, entries)?;
        Ok(bundles.iter().map(|b| b.collection.unwritten.len()).sum())
    }

    fn too_large(limit: u64, unit: &'static str) -> DocumentError {
        DocumentError::TooLarge { limit, unit }
    }

    #[test]
    fn note_sources_past_the_byte_budget_are_too_large() {
        // Every unwritten entry below deep long-named folders repeats all their names.
        let mut node = json!({"name": "leaf", "folders": [], "requests": vec![json!(1); 100]});
        for _ in 0..8 {
            node = json!({"name": "n".repeat(10_000), "folders": [node], "requests": []});
        }
        let root = json!({"name": "c", "folders": [node], "requests": []});
        let small = parse(&root, 1_000_000, ENTRIES).unwrap_err();
        assert_eq!(small, too_large(1_000_000, "imported bytes"));
        assert_eq!(parse(&root, NOTE_BYTES, ENTRIES), Ok(100));
    }

    #[test]
    fn folders_requests_rows_and_notes_are_counted() {
        let rows: Vec<Value> = (0..50)
            .map(|i| json!({"key": format!("k{i}"), "value": "v"}))
            .collect();
        let mut requests: Vec<Value> = (0..10)
            .map(|i| json!({"v": "17", "name": format!("r{i}"), "params": rows, "headers": rows}))
            .collect();
        requests.push(json!(1));
        let root = json!({"name": "c", "requests": [], "folders": [
            {"name": "f", "folders": [], "requests": requests}]});
        // 1 folder, 10 requests, 1000 rows and 1 note.
        assert_eq!(
            parse(&root, NOTE_BYTES, 1011).unwrap_err(),
            too_large(1011, "imported entries")
        );
        assert_eq!(parse(&root, NOTE_BYTES, 1012), Ok(1));
    }
}
