//! Importing HAR 1.2 captures: one collection with a folder per host.

mod body;
mod budget;
mod credentials;
mod entry;
mod headers;
mod json_members;
mod model;
mod secret_names;

use super::budget::{ENTRIES, NOTE_BYTES};
use super::load::Loaded;
use super::{
    CollectionOptions, DocumentError, ImportBundle, ImportNote, ImportedCollection, ImportedFolder,
    ImportedItem, ImportedRequestItem, NoteKind,
};
use crate::edit::AuthSpec;
use crate::js;
use budget::{Budget, Prefixed};
use credentials::Credentials;
use serde_json::{Map, Value};

/// Parse a HAR log into one collection, named from the options.
pub(crate) fn parse_har(
    loaded: Loaded,
    options: &CollectionOptions,
) -> Result<Vec<ImportBundle>, DocumentError> {
    parse_with_limits(loaded, options, NOTE_BYTES, ENTRIES)
}

/// `parse_har` with at most `note_bytes` of note text and `entries` entries; past either the
/// import is `TooLarge`.
fn parse_with_limits(
    loaded: Loaded,
    options: &CollectionOptions,
    note_bytes: u64,
    entries: u64,
) -> Result<Vec<ImportBundle>, DocumentError> {
    let Value::Object(mut root) = loaded.value else {
        return Err(missing_entries());
    };
    let Some(Value::Object(mut log)) = root.remove("log") else {
        return Err(missing_entries());
    };
    let Some(Value::Array(list)) = log.remove("entries") else {
        return Err(missing_entries());
    };
    if let Some(version) = log.get("version").filter(|v| !v.is_null()).map(js::to_text)
        && !(version.is_empty() || version.starts_with("1."))
    {
        return Err(DocumentError::Unsupported {
            found: format!("HAR version {version}"),
        });
    }
    let mut budget = Budget::new(note_bytes, entries);
    let mut notes = Vec::new();
    unknown(&root, &[], "collection / ", &mut budget, &mut notes)?;
    let exempt = ["version", "creator", "browser", "pages", "comment"];
    unknown(&log, &exempt, "collection / log.", &mut budget, &mut notes)?;
    // Hosts keep first-appearance order, which a sorted map would lose.
    let mut hosts: Vec<(String, Vec<ImportedRequestItem>)> = Vec::new();
    let mut unwritten = Vec::new();
    let mut creds = Credentials::new();
    for (i, value) in list.into_iter().enumerate() {
        let Some((host, mut item)) = entry::map(i, value, &mut unwritten, &mut creds, &mut budget)?
        else {
            continue;
        };
        let at = match hosts.iter().position(|(h, _)| *h == host) {
            Some(at) => at,
            None => {
                budget.entry()?;
                hosts.push((host, Vec::new()));
                hosts.len() - 1
            }
        };
        let requests = &mut hosts[at].1;
        item.seq = requests.len() as u64 + 1;
        requests.push(item);
    }
    let items = hosts
        .into_iter()
        .enumerate()
        .map(|(i, (name, requests))| {
            ImportedItem::Folder(ImportedFolder {
                scripts: Default::default(),
                name,
                seq: i as u64 + 1,
                docs: String::new(),
                auth: AuthSpec::Inherit,
                headers: Vec::new(),
                vars: Vec::new(),
                items: requests.into_iter().map(ImportedItem::Request).collect(),
                notes: Vec::new(),
            })
        })
        .collect();
    let collection = ImportedCollection {
        scripts: Default::default(),
        name: options
            .name
            .clone()
            .unwrap_or_else(|| "HAR capture".to_owned()),
        docs: String::new(),
        auth: AuthSpec::None,
        headers: Vec::new(),
        vars: Vec::new(),
        items,
        notes,
        unwritten,
        spec_source: false,
    };
    Ok(vec![ImportBundle {
        collection,
        environments: creds.environment().into_iter().collect(),
    }])
}

/// A `Dropped` note `{at}{key}` for each member of `rest` that is neither exempt nor a
/// `_`-prefixed HAR custom field. The budget is charged before a source is built, so a huge
/// member name cannot outgrow it.
fn unknown(
    rest: &Map<String, Value>,
    exempt: &[&str],
    at: &str,
    budget: &mut Budget,
    notes: &mut Vec<ImportNote>,
) -> Result<(), DocumentError> {
    for key in rest.keys() {
        if key.starts_with('_') || exempt.contains(&key.as_str()) {
            continue;
        }
        notes.push(budget.note(at, key, NoteKind::Dropped, "member is not imported")?);
    }
    Ok(())
}

fn missing_entries() -> DocumentError {
    DocumentError::Invalid {
        message: "log.entries is missing".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::load::load;

    /// The number of unwritten entries, or the error.
    fn parse(input: &str, note_bytes: u64, entries: u64) -> Result<usize, DocumentError> {
        let bundles = parse_with_limits(
            load(input.as_bytes())?,
            &CollectionOptions::default(),
            note_bytes,
            entries,
        )?;
        Ok(bundles[0].collection.unwritten.len())
    }

    fn too_large(limit: u64, unit: &'static str) -> DocumentError {
        DocumentError::TooLarge { limit, unit }
    }

    #[test]
    fn folders_requests_headers_and_notes_are_counted() {
        let input = r#"{"log":{"entries":[
            {"request":{"method":"GET","url":"https://x.test/a","headers":[{"name":"A","value":"1"},{"name":"B","value":"2"},{"name":"C","value":"3"}]}},
            {"request":{"method":"GET","url":"https://x.test/b"}},
            {"response":{}}]}}"#;
        // 1 folder, 2 requests, 3 headers and 1 unwritten note.
        assert_eq!(
            parse(input, NOTE_BYTES, 6).unwrap_err(),
            too_large(6, "imported entries")
        );
        assert_eq!(parse(input, NOTE_BYTES, 7), Ok(1));
    }

    #[test]
    fn note_sources_past_the_byte_budget_are_too_large() {
        // 100 unknown request members: each Dropped note repeats the 10,000-byte name, about 1 MB in all.
        let members: Vec<String> = (0..100).map(|i| format!(r#""zz{i}":1"#)).collect();
        let input = format!(
            r#"{{"log":{{"entries":[{{"comment":"{}","request":{{"method":"GET","url":"https://x.test/",{}}}}}]}}}}"#,
            "n".repeat(10_000),
            members.join(",")
        );
        assert_eq!(
            parse(&input, 500_000, ENTRIES).unwrap_err(),
            too_large(500_000, "imported bytes")
        );
        assert_eq!(parse(&input, NOTE_BYTES, ENTRIES), Ok(0));
    }

    #[test]
    fn an_uncut_entry_charges_only_its_note() {
        // Before the fix the request and its `token` variable were charged before the drop.
        let input = r#"{"log":{"entries":[{"request":{"method":"GET","url":"https:/\t/ann:pw@x.test/a?token=v"}}]}}"#;
        assert_eq!(parse(input, NOTE_BYTES, 1), Ok(1));
    }

    #[test]
    fn unwritten_note_bytes_are_charged() {
        // Each note is `entries[i]` (10 bytes) and its 40-byte detail.
        let input = r#"{"log":{"entries":[{"response":{}},{"response":{}}]}}"#;
        assert_eq!(
            parse(input, 99, ENTRIES).unwrap_err(),
            too_large(99, "imported bytes")
        );
        assert_eq!(parse(input, 100, ENTRIES), Ok(2));
    }
}
