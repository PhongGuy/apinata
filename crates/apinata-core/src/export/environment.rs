//! One environment, flattened through its `extends` chain, as a Postman environment file.

use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::{ExportNote, PostmanExport};
use crate::Collection;
use crate::environment::{EnvValue, resolve_environment};
use crate::js;

/// Export environment `name` as Postman environment JSON, or `None` when it does not exist.
/// Secret variables export with an empty value; a broken `extends` chain becomes a note.
pub fn postman_environment(collection: &Collection, name: &str) -> Option<PostmanExport> {
    let resolved = resolve_environment(collection.environments(), name)?;
    // Secrets are typed but never valued: the file may hold one, and it must not leave.
    let values: Vec<Value> = resolved
        .vars
        .iter()
        .map(|(key, value)| {
            let (text, kind) = match value {
                EnvValue::Plain(v) => (js::to_text(v), "default"),
                EnvValue::Secret => (String::new(), "secret"),
            };
            json!({"key": key, "value": text, "type": kind, "enabled": true})
        })
        .collect();
    let mut root = Map::new();
    root.insert(
        "id".into(),
        Uuid::new_v5(&Uuid::NAMESPACE_OID, name.as_bytes())
            .to_string()
            .into(),
    );
    root.insert("name".into(), name.into());
    root.insert("values".into(), values.into());
    root.insert("_postman_variable_scope".into(), "environment".into());
    let mut json =
        serde_json::to_string_pretty(&Value::Object(root)).expect("a JSON value serializes");
    json.push('\n');
    let source = format!("environments / {name}");
    let notes = resolved
        .warnings
        .into_iter()
        .map(|w| ExportNote::new(source.clone(), w))
        .collect();
    Some(PostmanExport { json, notes })
}
