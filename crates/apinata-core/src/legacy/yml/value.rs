//! JavaScript and lodash value semantics the reference mapping relies on. `None` stands for
//! JavaScript `undefined` throughout, and a JSON `null` for `null`.

use crate::js::{js_trim, number_to_string, parse_number, stringify, stringify_pretty};
use serde_json::Value;
use yaml_serde::Mapping;

/// Integers up to 2^53 are exact in an f64, so they print the same as JSON integers.
const SAFE_INTEGER: f64 = 9_007_199_254_740_992.0;

/// lodash `_.get(value, path)` for a dotted path; `None` where lodash gives `undefined`.
pub(super) fn get<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(value, |at, key| match at {
        Value::Object(map) => map.get(key),
        Value::Array(items) => key.parse::<usize>().ok().and_then(|i| items.get(i)),
        _ => None,
    })
}

/// JavaScript truthiness.
pub(super) fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(_) | Value::Object(_)) => true,
    }
}

/// `isNonEmptyString`: a string whose JavaScript `trim()` is not empty.
pub(super) fn is_non_empty_string(value: Option<&Value>) -> bool {
    matches!(value, Some(Value::String(s)) if !js_trim(s).is_empty())
}

/// JavaScript `value || fallback`.
pub(super) fn js_or(value: Option<&Value>, fallback: Value) -> Value {
    match value {
        Some(v) if truthy(Some(v)) => v.clone(),
        _ => fallback,
    }
}

/// JavaScript `Number(value)`; arrays and objects go through `String()` first, as JS does.
pub(super) fn to_number(value: Option<&Value>) -> f64 {
    match value {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => parse_number(s).unwrap_or(f64::NAN),
        Some(other) => parse_number(&ensure_string(Some(other))).unwrap_or(f64::NAN),
    }
}

/// A finite JavaScript number as JSON: integral values become integers so they print as JS does.
pub(super) fn number(n: f64) -> Value {
    debug_assert!(n.is_finite(), "JSON cannot hold {n}");
    if n.fract() == 0.0 && n.abs() <= SAFE_INTEGER {
        Value::from(n as i64)
    } else {
        Value::from(n)
    }
}

/// `ensureString`: `''` for `null` and `undefined`, else JavaScript `String(value)`.
pub(super) fn ensure_string(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(v) => js_string(v),
    }
}

/// JavaScript `String(value)`; an array joins its items, rendering `null` items as empty.
fn js_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.as_f64().map_or_else(|| n.to_string(), number_to_string),
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .map(|item| ensure_string(Some(item)))
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_owned(),
    }
}

/// bruno-common `valueToString`: objects and arrays as compact JSON, scalars as `String()`.
pub(super) fn value_to_string(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(v @ (Value::Array(_) | Value::Object(_))) => stringify(v),
        Some(v) => js_string(v),
    }
}

/// `serializeVariableValue`: objects and arrays as two-space JSON, everything else as text.
pub(super) fn serialize_variable_value(value: Option<&Value>) -> String {
    match value {
        Some(v @ (Value::Array(_) | Value::Object(_))) => stringify_pretty(v),
        other => value_to_string(other),
    }
}

/// Converts a JSON object document to a YAML mapping, keeping its key order.
pub(super) fn to_mapping(doc: Value) -> Mapping {
    match yaml_serde::to_value(doc) {
        Ok(yaml_serde::Value::Mapping(mapping)) => mapping,
        other => panic!("a mapped document is a JSON object, got {other:?}"),
    }
}
