//! One environment, flattened through its `extends` chain, as a Postman environment file, and
//! every environment of a collection or of the global set as uniquely named files.

use std::collections::HashSet;

use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::{ExportNote, PostmanExport, file_stem};
use crate::environment::{EnvValue, resolve_environment};
use crate::js;
use crate::{Collection, Environment, GlobalEnvironments};

/// One environment exported for a directory: its file name there and its contents.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct EnvironmentFile {
    /// `<stem>.postman_environment.json`, the stem numbered ` (n)` when another file took it.
    pub file_name: String,
    /// The environment's Postman JSON and notes, as [`postman_environment`] gives them.
    pub export: PostmanExport,
}

/// Export environment `name` as Postman environment JSON, or `None` when it does not exist.
/// Secret variables export with an empty value; a broken `extends` chain becomes a note.
pub fn postman_environment(collection: &Collection, name: &str) -> Option<PostmanExport> {
    export_one(collection.environments(), name, "environments")
}

/// Every environment of `collection`, in name order, as files that can share one directory.
pub fn postman_environment_files(collection: &Collection) -> Vec<EnvironmentFile> {
    export_all(collection.environments(), "environments")
}

/// [`postman_environment`] for global environment `name`.
pub fn postman_global_environment(
    globals: &GlobalEnvironments,
    name: &str,
) -> Option<PostmanExport> {
    export_one(globals.environments(), name, "global environments")
}

/// [`postman_environment_files`] for every global environment.
pub fn postman_global_environment_files(globals: &GlobalEnvironments) -> Vec<EnvironmentFile> {
    export_all(globals.environments(), "global environments")
}

// Environments come sorted by name, so a colliding stem is numbered in a stable order.
fn export_all(envs: &[Environment], scope: &str) -> Vec<EnvironmentFile> {
    let mut taken = HashSet::new();
    envs.iter()
        .filter_map(|env| {
            let export = export_one(envs, env.name(), scope)?;
            let stem = unique_stem(&mut taken, file_stem(env.name()));
            Some(EnvironmentFile {
                file_name: format!("{stem}.postman_environment.json"),
                export,
            })
        })
        .collect()
}

/// `stem` if it is free, else the first free `stem (n)` from 2 up; the result is marked taken.
/// Names compare lower-cased, since Windows and macOS file systems ignore case.
fn unique_stem(taken: &mut HashSet<String>, stem: String) -> String {
    let mut candidate = stem.clone();
    let mut n = 2;
    while taken.contains(&candidate.to_lowercase()) {
        candidate = format!("{stem} ({n})");
        n += 1;
    }
    taken.insert(candidate.to_lowercase());
    candidate
}

// `scope` heads each note's source, so a global's notes say where they came from.
fn export_one(envs: &[Environment], name: &str, scope: &str) -> Option<PostmanExport> {
    let resolved = resolve_environment(envs, name)?;
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
    // A `serde_json::Value` has only string keys, so serializing it cannot fail.
    #[allow(clippy::expect_used)]
    let mut json =
        serde_json::to_string_pretty(&Value::Object(root)).expect("a JSON value serializes");
    json.push('\n');
    let source = format!("{scope} / {name}");
    let notes = resolved
        .warnings
        .into_iter()
        .map(|w| ExportNote::new(source.clone(), w))
        .collect();
    Some(PostmanExport { json, notes })
}
