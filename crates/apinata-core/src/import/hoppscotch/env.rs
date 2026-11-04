//! Hoppscotch environment exports, every version, as importable environments.

use super::invalid;
use super::model::{EnvEntry, Environment, text};
use super::template::rewrite;
use super::tree::{Budget, Crumbs};
use crate::import::{DocumentError, ImportNote, ImportedEnvironment, NoteKind};
use crate::{EnvVariable, VarValue};
use serde::Deserialize;
use serde_json::Value;

/// The newest environment version whose shape is known.
const ENVIRONMENT_VERSION: u64 = 2;

/// The environments of one export, an environment object or an array of them; note sources
/// in an array start with the environment's `[i].`.
pub(super) fn environments(
    value: Value,
    budget: &mut Budget,
) -> Result<Vec<ImportedEnvironment>, DocumentError> {
    let (list, in_array) = match value {
        Value::Array(list) if list.is_empty() => {
            return Err(invalid("the export is an empty list"));
        }
        Value::Array(list) => (list, true),
        object @ Value::Object(_) => (vec![object], false),
        _ => return Err(invalid("not a Hoppscotch environment export")),
    };
    list.into_iter()
        .enumerate()
        .map(|(i, env)| {
            let prefix = if in_array {
                format!("[{i}].")
            } else {
                String::new()
            };
            environment(env, &prefix, budget)
        })
        .collect()
}

fn environment(
    value: Value,
    prefix: &str,
    budget: &mut Budget,
) -> Result<ImportedEnvironment, DocumentError> {
    if !value.is_object() {
        return Err(invalid(
            "not a Hoppscotch environment: an entry is not an object",
        ));
    }
    let version = version_note(value.get("v"));
    let env = Environment::deserialize(value).map_err(|e| DocumentError::Invalid {
        message: format!("not a Hoppscotch environment: {e}"),
    })?;
    budget.entry()?;
    let mut notes = Vec::new();
    if let Some(detail) = version {
        note(
            &mut notes,
            budget,
            format!("{prefix}v"),
            NoteKind::Converted,
            detail,
        )?;
    }
    for key in env.rest.keys() {
        let source = format!("{prefix}{key}");
        note(
            &mut notes,
            budget,
            source,
            NoteKind::Dropped,
            "not imported",
        )?;
    }

    let mut variables = Vec::new();
    let mut sources = Vec::new();
    for (j, raw) in env.variables.into_iter().enumerate() {
        let source = format!("{prefix}variables[{j}]");
        // One wrong-typed variable is skipped with a note rather than failing the export.
        let entry = match EnvEntry::deserialize(raw) {
            Ok(entry) => entry,
            Err(e) => {
                note(&mut notes, budget, source, NoteKind::Dropped, e.to_string())?;
                continue;
            }
        };
        let name = text(entry.key);
        if name.is_empty() {
            let detail = "the variable has no name; not imported";
            note(&mut notes, budget, source, NoteKind::Dropped, detail)?;
            continue;
        }
        budget.entry()?;
        for key in entry.rest.keys() {
            let at = format!("{source}.{key}");
            note(&mut notes, budget, at, NoteKind::Dropped, "not imported")?;
        }
        // Version 2 keeps the shared value in `initialValue`; earlier ones only `value`. A
        // secret keeps its value too: the writer moves it into the secret store.
        let [initial, current, legacy] =
            [entry.initial_value, entry.current_value, entry.value].map(text);
        if !initial.is_empty() && !current.is_empty() && initial != current {
            let at = format!("{source}.currentValue");
            let detail = "the current value differs from the initial value and is not imported";
            note(&mut notes, budget, at, NoteKind::Dropped, detail)?;
        }
        let value = [initial, current, legacy]
            .into_iter()
            .find(|v| !v.is_empty())
            .unwrap_or_default();
        let (value, dynamic) = rewrite(&value);
        if !dynamic.is_empty() {
            let detail = format!(
                "uses the generated variables {}; give each a value",
                dynamic.join(", ")
            );
            note(
                &mut notes,
                budget,
                source.clone(),
                NoteKind::NeedsInput,
                detail,
            )?;
        }
        let mut variable = EnvVariable::new(name, VarValue::Text(value));
        variable.secret = entry.secret == Some(true);
        variables.push(variable);
        sources.push(source);
    }

    Ok(ImportedEnvironment {
        name: env.name.map_or_else(|| "Global".to_owned(), |n| n.0),
        variables,
        notes,
        sources,
    })
}

/// The note for an environment `v` read with the newest known shape, or `None` when it is
/// missing (version 0) or known.
fn version_note(v: Option<&Value>) -> Option<String> {
    match v {
        None => None,
        Some(v) if v.as_u64().is_some_and(|n| n <= ENVIRONMENT_VERSION) => None,
        Some(v) => Some(format!(
            "version {v} is not a known environment version; read as version {ENVIRONMENT_VERSION}"
        )),
    }
}

/// Push a note sourced `source`, charged against the budget.
fn note(
    notes: &mut Vec<ImportNote>,
    budget: &mut Budget,
    source: String,
    kind: NoteKind,
    detail: impl Into<String>,
) -> Result<(), DocumentError> {
    notes.push(budget.note(&[], &[&source], kind, detail)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(input: Value, note_bytes: u64, entries: u64) -> Result<usize, DocumentError> {
        let envs = environments(input, &mut Budget::new(note_bytes, entries))?;
        Ok(envs.iter().map(|e| e.variables.len()).sum())
    }

    fn too_large(limit: u64, unit: &'static str) -> DocumentError {
        DocumentError::TooLarge { limit, unit }
    }

    #[test]
    fn environments_variables_and_notes_are_counted() {
        let vars: Vec<Value> = (0..10)
            .map(|i| json!({"key": format!("k{i}"), "value": "v"}))
            .collect();
        let input = json!([{"name": "e", "zz": 1, "variables": vars}]);
        // 1 environment, 10 variables and 1 note.
        assert_eq!(
            parse(input.clone(), 1_000, 11).unwrap_err(),
            too_large(11, "imported entries")
        );
        assert_eq!(parse(input, 1_000, 12), Ok(10));
    }

    #[test]
    fn note_text_past_the_byte_budget_is_too_large() {
        let mut input = json!({"variables": []});
        input["z".repeat(200).as_str()] = json!(1);
        assert_eq!(
            parse(input.clone(), 100, 100).unwrap_err(),
            too_large(100, "imported bytes")
        );
        assert_eq!(parse(input, 1_000, 100), Ok(0));
    }
}
