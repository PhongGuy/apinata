//! Postman environments, as the reference converter and its import modal read them.

use super::details::clean_name;
use super::notes::members;
use super::truthy;
use crate::edit::VarValue;
use crate::env_file::EnvVariable;
use crate::import::load::json_error_offset;
use crate::import::{ImportNote, ImportedEnvironment, NoteKind, PostmanError};
use crate::js;
use serde_json::Value;
use std::collections::HashMap;

/// One kept `values` entry before secret deduplication.
struct Row {
    index: usize,
    key: String,
    variable: EnvVariable,
    /// Whether the raw JSON value is JavaScript-truthy, which decides between secret twins.
    has_value: bool,
}

/// Read a Postman environment export. Entries whose key and value are both null are skipped;
/// of secrets sharing a name only one is kept, preferring the first with a value. Fails on
/// invalid JSON, a blank or missing name, or `values` (or an entry's key) of the wrong type.
pub fn parse_postman_environment(json: &[u8]) -> Result<ImportedEnvironment, PostmanError> {
    let root: Value = serde_json::from_slice(json).map_err(|e| PostmanError::Parse {
        offset: json_error_offset(json, &e),
        message: e.to_string(),
    })?;
    let name = coerce_env_name(root.get("name"))
        .ok_or_else(|| invalid("the environment has no valid name"))?;
    let entries = match root.get("values") {
        None => &[][..],
        Some(Value::Array(entries)) => entries.as_slice(),
        Some(_) => return Err(invalid("the environment values are not a list")),
    };

    let mut notes = Vec::new();
    if let Value::Object(root) = &root {
        members(
            root,
            &|key| !matches!(key, "name" | "values"),
            "",
            &mut notes,
        );
    }
    let mut rows = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        // Reading a member of `null` throws in the reference; any other non-object has none.
        match entry {
            Value::Null => return Err(invalid(&format!("values[{index}] is null"))),
            Value::Object(entry) => {
                let unread = |key: &str| !matches!(key, "key" | "value" | "enabled" | "type");
                members(entry, &unread, &format!("values[{index}]."), &mut notes);
            }
            _ => notes.push(ImportNote::new(
                format!("values[{index}]"),
                NoteKind::Dropped,
                "the entry is not an object",
            )),
        }
        let key = entry.get("key").filter(|k| !k.is_null());
        let raw = entry.get("value").filter(|v| !v.is_null());
        if key.is_none() && raw.is_none() {
            continue;
        }
        let key = match key {
            None => "",
            Some(Value::String(key)) => key.as_str(),
            Some(_) => return Err(invalid(&format!("values[{index}].key is not text"))),
        };
        let text = match raw {
            None => String::new(),
            Some(Value::String(s)) => s.clone(),
            Some(v @ (Value::Number(_) | Value::Bool(_))) => js::to_text(v),
            Some(other) => js::stringify_pretty(other),
        };
        let mut variable = EnvVariable::new(clean_name(key), VarValue::Text(text));
        variable.enabled = entry.get("enabled") != Some(&Value::Bool(false));
        variable.secret = entry.get("type").and_then(Value::as_str) == Some("secret");
        rows.push(Row {
            index,
            key: key.to_owned(),
            variable,
            has_value: truthy(raw),
        });
    }

    let dropped = duplicate_secrets(&rows);
    let mut env = ImportedEnvironment {
        name,
        variables: Vec::new(),
        notes,
        sources: Vec::new(),
    };
    for (row, dropped) in rows.into_iter().zip(dropped) {
        let source = format!("values[{}]", row.index);
        let name = &row.variable.name;
        if dropped {
            env.notes.push(ImportNote::new(
                source,
                NoteKind::Dropped,
                format!("the secret `{name}` is defined more than once; only one is kept"),
            ));
            continue;
        }
        if *name != row.key {
            env.notes.push(ImportNote::new(
                format!("{source}.key"),
                NoteKind::Converted,
                format!("the variable `{}` was renamed to `{name}`", row.key),
            ));
        }
        env.sources.push(source);
        env.variables.push(row.variable);
    }
    Ok(env)
}

/// `coerceEnvName`: text that is not blank, kept untrimmed, or a finite number as text.
fn coerce_env_name(name: Option<&Value>) -> Option<String> {
    match name? {
        Value::String(s) if !js::js_trim(s).is_empty() => Some(s.clone()),
        Value::Number(n) => n
            .as_f64()
            .filter(|n| n.is_finite())
            .map(js::number_to_string),
        _ => None,
    }
}

/// `dedupeImportedSecrets`: per row, whether it is a secret twin to drop. Among secrets sharing
/// a non-blank trimmed name, the first is kept unless a later one has a value and it has none.
fn duplicate_secrets(rows: &[Row]) -> Vec<bool> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for name in rows.iter().filter_map(secret_name) {
        *counts.entry(name).or_default() += 1;
    }
    let mut survivors: HashMap<&str, usize> = HashMap::new();
    for (i, row) in rows.iter().enumerate() {
        let Some(name) = secret_name(row).filter(|n| counts[n] > 1) else {
            continue;
        };
        match survivors.get(name) {
            Some(&kept) if rows[kept].has_value || !row.has_value => {}
            _ => {
                survivors.insert(name, i);
            }
        }
    }
    rows.iter()
        .enumerate()
        .map(|(i, row)| secret_name(row).is_some_and(|n| counts[n] > 1 && survivors[n] != i))
        .collect()
}

/// A secret row's trimmed name, when it is not blank.
fn secret_name(row: &Row) -> Option<&str> {
    let name = js::js_trim(&row.variable.name);
    (row.variable.secret && !name.is_empty()).then_some(name)
}

fn invalid(message: &str) -> PostmanError {
    PostmanError::Invalid {
        message: message.to_owned(),
    }
}
