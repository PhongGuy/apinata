//! A legacy environment file to `environments/X.yml`: the reference's `stringifyEnvironment`
//! over `parseBruEnvironment`.

use super::value::{get, js_or, serialize_variable_value, to_mapping, truthy};
use crate::js::js_trim;
use crate::naming::validate_name;
use serde_json::{Map, Value, json};
use yaml_serde::Mapping;

/// Maps an environment parse to its YAML document. The parse carries no name, so none is written.
pub(crate) fn map_environment(parsed: &Value) -> Mapping {
    let mut doc = Map::new();
    if let Some(extends) = extends_from(get(parsed, "extends")) {
        doc.insert("extends".into(), extends);
    }
    if let Some(color) = get(parsed, "color").filter(|c| truthy(Some(c))) {
        doc.insert("color".into(), color.clone());
    }
    if let Some(Value::Array(rows)) = get(parsed, "variables")
        && !rows.is_empty()
    {
        doc.insert("variables".into(), rows.iter().map(variable).collect());
    }
    if let Some(secrets) = get(parsed, "externalSecrets").filter(|s| truthy(Some(s))) {
        doc.insert("externalSecrets".into(), external_secrets(secrets));
    }
    to_mapping(Value::Object(doc))
}

/// `validatedEnvironmentExtendsFrom`: one trimmed valid name, or a non-empty list of them in
/// which every entry is valid; anything else drops the key.
fn extends_from(reference: Option<&Value>) -> Option<Value> {
    let name = |value: &Value| match value {
        Value::String(s) => Some(js_trim(s))
            .filter(|n| validate_name(n))
            .map(Value::from),
        _ => None,
    };
    match reference? {
        Value::Array(names) if !names.is_empty() => names
            .iter()
            .map(name)
            .collect::<Option<Vec<_>>>()
            .map(Value::Array),
        Value::Array(_) => None,
        other => name(other),
    }
}

/// One environment variable. A secret row carries no value; a typed row wraps its text in
/// `{type, data}`; `description` is kept whenever the parse has one.
fn variable(row: &Value) -> Value {
    let name = js_or(get(row, "name"), json!(""));
    let data_type = get(row, "dataType")
        .filter(|t| truthy(Some(t)) && *t != "string")
        .cloned();
    let mut out = Map::new();
    if get(row, "secret") == Some(&Value::Bool(true)) {
        out.insert("secret".into(), Value::Bool(true));
        out.insert("name".into(), name);
        if let Some(kind) = data_type {
            out.insert("type".into(), kind);
        }
    } else {
        let text = serialize_variable_value(get(row, "value"));
        out.insert("name".into(), name);
        let value = match data_type {
            Some(kind) => json!({"type": kind, "data": text}),
            None => Value::String(text),
        };
        out.insert("value".into(), value);
    }
    if get(row, "enabled") == Some(&Value::Bool(false)) {
        out.insert("disabled".into(), Value::Bool(true));
    }
    if let Some(description) = get(row, "description") {
        out.insert("description".into(), description.clone());
    }
    Value::Object(out)
}

/// `externalSecrets` rebuilt as `{type, variables}` with each entry shallow-copied in order.
fn external_secrets(secrets: &Value) -> Value {
    let mut out = Map::new();
    if let Some(kind) = get(secrets, "type") {
        out.insert("type".into(), kind.clone());
    }
    let variables = match get(secrets, "variables") {
        Some(Value::Array(items)) => items.iter().map(spread).collect(),
        _ => Vec::new(),
    };
    out.insert("variables".into(), Value::Array(variables));
    Value::Object(out)
}

/// JavaScript `{ ...value }` for the object entries the reader produces; anything else is `{}`.
fn spread(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(map.clone()),
        _ => Value::Object(Map::new()),
    }
}
