//! Legacy `.bru` parsers that return the same JSON as the reference parsers, plus the
//! semantic helpers the grammars share: pair-list mapping and lodash-style merging.

mod auth;
mod collection;
mod environment;
mod example;
mod json;
mod lex;
mod request;

pub(crate) use collection::parse_collection;
pub(crate) use environment::parse_environment;
#[cfg(test)]
pub(crate) use json::parse as parse_js_json;
pub(crate) use request::parse_request;

use crate::js::{js_trim, number_to_string, parse_number};
use lex::{Annotation, Pair, PairValue};
use serde_json::{Map, Value};

/// A rejected `.bru` file: the 1-based line of the rightmost parse failure and what was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BruError {
    pub(crate) line: usize,
    pub(crate) message: String,
}

#[cfg(test)]
mod tests;

/// `bruno.json` as `JSON.parse` reads it; invalid JSON is a `BruError` on the failing line.
pub(crate) fn parse_bruno_json(text: &str) -> Result<Value, BruError> {
    json::parse(text).map_err(|e| BruError {
        line: 1 + text[..e.offset].matches('\n').count(),
        message: e.message,
    })
}

fn pair_value(value: &PairValue) -> Value {
    match value {
        PairValue::Text(text) => Value::String(text.clone()),
        PairValue::List(items) => items.iter().cloned().map(Value::String).collect(),
    }
}

/// JavaScript truthiness for a dictionary value, which is a string or a list.
fn is_truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::String(text)) => !text.is_empty(),
        Some(_) => true,
    }
}

/// A JavaScript number as JSON: integral values as integers, and a non-finite one as its
/// JavaScript text (`"Infinity"`, `"-Infinity"`, `"NaN"`), which JSON cannot hold.
fn js_number(n: f64) -> Value {
    if !n.is_finite() {
        return Value::String(number_to_string(n));
    }
    // Every integral f64 below 2^64 in magnitude converts exactly, so the casts are lossless.
    if n.fract() == 0.0 && n.abs() < 18_446_744_073_709_551_616.0 {
        if n >= 0.0 {
            return Value::from(n as u64);
        }
        if n >= -9_223_372_036_854_775_808.0 {
            return Value::from(n as i64);
        }
    }
    serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number)
}

const PROTO_KEY: &str = "__proto__";

/// `mapPairListToKeyValPair`: every pair as `{key: value}`, folded together with `_.merge`.
fn pairs_to_object(pairs: &[Pair]) -> Map<String, Value> {
    let mut out = Map::new();
    for pair in pairs {
        // JS assigns `__proto__` on the pair object instead of defining a key: a string is
        // ignored, and a list becomes the prototype whose indexes `_.merge` then copies.
        if pair.key == PROTO_KEY {
            if let PairValue::List(items) = &pair.value {
                for (index, item) in items.iter().enumerate() {
                    merge_key(
                        &mut out,
                        &index.to_string(),
                        &Value::String(item.clone()),
                        false,
                    );
                }
            }
            continue;
        }
        merge_key(&mut out, &pair.key, &pair_value(&pair.value), false);
    }
    out
}

/// `mapPairListToKeyValPairs` with `parseEnabled`: a leading `~` on the name disables the row.
fn pairs_to_rows(pairs: &[Pair]) -> Vec<Map<String, Value>> {
    rows_of(pairs, true)
}

/// `pairs_to_rows` for pairs whose key is a plain name rather than an object key, so a
/// `__proto__` name stays a name.
fn named_rows(pairs: &[Pair]) -> Vec<Map<String, Value>> {
    rows_of(pairs, false)
}

fn rows_of(pairs: &[Pair], object_keys: bool) -> Vec<Map<String, Value>> {
    pairs
        .iter()
        .map(|pair| {
            let (name, enabled) = match pair.key.strip_prefix('~') {
                Some(rest) => (rest, false),
                None => (pair.key.as_str(), true),
            };
            let mut row = Map::new();
            // A `__proto__` pair has no own key, so its name and value are undefined.
            if !(object_keys && pair.key == PROTO_KEY) {
                row.insert("name".into(), Value::String(name.to_owned()));
                row.insert("value".into(), pair_value(&pair.value));
            }
            row.insert("enabled".into(), Value::Bool(enabled));
            if !pair.annotations.is_empty() {
                row.insert("annotations".into(), annotations_json(&pair.annotations));
                if let Some(description) = description(&pair.annotations) {
                    row.insert("description".into(), Value::String(description));
                }
            }
            row
        })
        .collect()
}

fn annotations_json(annotations: &[Annotation]) -> Value {
    annotations
        .iter()
        .map(|a| {
            let mut out = Map::new();
            out.insert("name".into(), Value::String(a.name.clone()));
            if let Some(value) = &a.value {
                out.insert("value".into(), Value::String(value.clone()));
            }
            Value::Object(out)
        })
        .collect()
}

/// `applyDescriptionFromAnnotations`: the first `@description`, with a multiline value's
/// escaped delimiters restored.
fn description(annotations: &[Annotation]) -> Option<String> {
    let found = annotations.iter().find(|a| a.name == "description")?;
    let value = found.value.clone().unwrap_or_default();
    Some(if value.contains('\n') {
        value.replace(r"\'\'\'", "'''").replace(r"\\'", r"\'")
    } else {
        value
    })
}

/// `extractTypedAnnotations`: the last type annotation other than `string` sets `dataType`
/// and converts the row's value with `parseValueByDataType`.
fn apply_data_type(row: &mut Map<String, Value>, annotations: &[Annotation]) {
    let Some(found) = annotations
        .iter()
        .rev()
        .find(|a| ["string", "number", "boolean", "object"].contains(&a.name.as_str()))
    else {
        return;
    };
    if found.name == "string" {
        return;
    }
    row.insert("dataType".into(), Value::String(found.name.clone()));
    // A `__proto__` row has no value to convert.
    if let Some(value) = row.get("value").cloned() {
        row.insert("value".into(), value_by_data_type(value, &found.name));
    }
}

/// `parseValueByDataType` over a dictionary value (a string or a list), with numbers as
/// `JSON.stringify` prints them.
fn value_by_data_type(value: Value, data_type: &str) -> Value {
    match (data_type, &value) {
        // A list is a JavaScript object: `@object` keeps it and `@number` reads its text.
        ("number", Value::Array(items)) => {
            let text = items
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(",");
            parse_number(&text).map_or(value, js_number)
        }
        ("number", Value::String(text)) if !js_trim(text).is_empty() => {
            parse_number(text).map_or(value, js_number)
        }
        ("boolean", Value::String(text)) if text == "true" => Value::Bool(true),
        ("boolean", Value::String(text)) if text == "false" => Value::Bool(false),
        ("object", Value::String(text)) => match json::parse(js_trim(text)) {
            Ok(parsed @ (Value::Object(_) | Value::Array(_))) => parsed,
            _ => value,
        },
        _ => value,
    }
}

/// Folds `src` into `target` the way lodash `_.merge` does, or `_.mergeWith` with an
/// array-concatenating customizer when `concat` is set.
fn merge_object(target: &mut Map<String, Value>, src: &Map<String, Value>, concat: bool) {
    for (key, value) in src {
        merge_key(target, key, value, concat);
    }
}

fn merge_key(target: &mut Map<String, Value>, key: &str, src: &Value, concat: bool) {
    match target.get_mut(key) {
        Some(slot) => {
            let old = slot.take();
            *slot = merged(Some(old), src, concat);
        }
        None => {
            target.insert(key.to_owned(), merged(None, src, concat));
        }
    }
}

/// One lodash merge step: arrays merge by index (or concatenate), objects merge by key,
/// and anything else replaces the old value.
fn merged(old: Option<Value>, src: &Value, concat: bool) -> Value {
    match src {
        Value::Array(items) => {
            let mut base = match old {
                Some(Value::Array(base)) if concat => {
                    let mut base = base;
                    base.extend(items.iter().cloned());
                    return Value::Array(base);
                }
                Some(Value::Array(base)) => base,
                _ => Vec::new(),
            };
            for (index, item) in items.iter().enumerate() {
                match base.get_mut(index) {
                    Some(slot) => {
                        let previous = slot.take();
                        *slot = merged(Some(previous), item, concat);
                    }
                    None => base.push(merged(None, item, concat)),
                }
            }
            Value::Array(base)
        }
        Value::Object(fields) => {
            let mut base = match old {
                Some(Value::Object(base)) => base,
                _ => Map::new(),
            };
            merge_object(&mut base, fields, concat);
            Value::Object(base)
        }
        other => other.clone(),
    }
}
