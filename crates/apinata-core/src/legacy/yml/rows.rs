//! Row mappers: ports of the reference's `toOpenCollection*` converters for headers, params,
//! variables, actions, scripts, assertions, bodies, omitted headers and the app block. Each
//! returns `None` where the reference returns `undefined`, and builds keys in its insertion order.

use super::value::{ensure_string, is_non_empty_string, js_or, serialize_variable_value, truthy};
use crate::js::js_trim;
use serde_json::{Map, Value, json};

pub(super) const HTTP_SCRIPT_KEYS: &[&str] = &["req", "res"];
pub(super) const GRPC_SCRIPT_KEYS: &[&str] = &[
    "beforeCallStart",
    "beforeMessageSend",
    "afterMessageReceive",
    "afterCallEnd",
];

/// Script keys with their OpenCollection types, in the order the reference pushes them.
const SCRIPT_TYPES: [(&str, &str); 6] = [
    ("req", "before-request"),
    ("res", "after-response"),
    ("beforeCallStart", "grpc:before-call-start"),
    ("beforeMessageSend", "grpc:before-message-send"),
    ("afterMessageReceive", "grpc:after-message-receive"),
    ("afterCallEnd", "grpc:after-call-end"),
];

const OPERATORS: [&str; 28] = [
    "eq",
    "neq",
    "gt",
    "gte",
    "lt",
    "lte",
    "in",
    "notIn",
    "contains",
    "notContains",
    "length",
    "matches",
    "notMatches",
    "startsWith",
    "endsWith",
    "between",
    "isEmpty",
    "isNotEmpty",
    "isNull",
    "isUndefined",
    "isDefined",
    "isTruthy",
    "isFalsy",
    "isJson",
    "isNumber",
    "isString",
    "isBoolean",
    "isArray",
];

/// The unary operators are the tail of `OPERATORS`, from `isEmpty` on.
const UNARY_FROM: usize = 16;

/// A field of a row; a row that is not an object has no fields, as property access on a string.
fn field<'a>(row: &'a Value, key: &str) -> Option<&'a Value> {
    row.as_object().and_then(|map| map.get(key))
}

/// JavaScript `x || ''`.
fn or_empty(value: Option<&Value>) -> Value {
    js_or(value, Value::String(String::new()))
}

/// The rows of a non-empty array; the reference's `!x?.length` guard, with a non-array as absent.
fn non_empty(rows: Option<&Value>) -> Option<&Vec<Value>> {
    rows.and_then(Value::as_array)
        .filter(|rows| !rows.is_empty())
}

/// Copies a row's description when its trimmed text is non-empty.
fn describe(out: &mut Map<String, Value>, row: &Value) {
    if is_non_empty_string(field(row, "description")) {
        out.insert(
            "description".into(),
            field(row, "description").cloned().unwrap_or_default(),
        );
    }
}

/// Marks a row disabled only when it says `enabled: false`.
fn disable(out: &mut Map<String, Value>, row: &Value) {
    if field(row, "enabled") == Some(&Value::Bool(false)) {
        out.insert("disabled".into(), Value::Bool(true));
    }
}

/// A `{name, value}` row with its optional description and disabled flag.
fn key_value(row: &Value) -> Value {
    let mut out = Map::new();
    out.insert("name".into(), or_empty(field(row, "name")));
    out.insert("value".into(), or_empty(field(row, "value")));
    describe(&mut out, row);
    disable(&mut out, row);
    Value::Object(out)
}

/// `toOpenCollectionHttpHeaders`.
pub(super) fn http_headers(headers: Option<&Value>) -> Option<Value> {
    non_empty(headers).map(|rows| rows.iter().map(key_value).collect())
}

/// `toOpenCollectionResponseHeaders`: name and value only.
pub(super) fn response_headers(headers: Option<&Value>) -> Option<Value> {
    non_empty(headers).map(|rows| {
        rows.iter()
            .map(|row| json!({"name": or_empty(field(row, "name")), "value": or_empty(field(row, "value"))}))
            .collect()
    })
}

/// `toOpenCollectionParams`: a header row with its `type` after the value.
pub(super) fn params(params: Option<&Value>) -> Option<Value> {
    non_empty(params).map(|rows| {
        rows.iter()
            .map(|row| {
                let mut out = Map::new();
                out.insert("name".into(), or_empty(field(row, "name")));
                out.insert("value".into(), or_empty(field(row, "value")));
                if let Some(kind) = field(row, "type") {
                    out.insert("type".into(), kind.clone());
                }
                describe(&mut out, row);
                disable(&mut out, row);
                Value::Object(out)
            })
            .collect()
    })
}

/// `toOpenCollectionVariables`: the request-phase variables of a `{req, res}` object or a list;
/// a non-string data type wraps the value as `{type, data}`.
pub(super) fn variables(vars: Option<&Value>) -> Option<Value> {
    let req = match vars {
        Some(Value::Object(map)) if map.contains_key("req") => map.get("req"),
        other => other,
    };
    non_empty(req).map(|rows| {
        rows.iter()
            .map(|row| {
                let text = serialize_variable_value(field(row, "value"));
                let value = match field(row, "dataType") {
                    Some(kind) if truthy(Some(kind)) && kind != "string" => {
                        json!({"type": kind, "data": text})
                    }
                    _ => Value::String(text),
                };
                let mut out = Map::new();
                out.insert("name".into(), or_empty(field(row, "name")));
                out.insert("value".into(), value);
                describe(&mut out, row);
                disable(&mut out, row);
                Value::Object(out)
            })
            .collect()
    })
}

/// `toOpenCollectionActions`: each response variable becomes an after-response `set-variable`.
pub(super) fn actions(res_vars: Option<&Value>) -> Option<Value> {
    non_empty(res_vars).map(|rows| {
        rows.iter()
            .map(|row| {
                let scope = if truthy(field(row, "local")) {
                    "request"
                } else {
                    "runtime"
                };
                let mut out = Map::new();
                out.insert("type".into(), "set-variable".into());
                out.insert("phase".into(), "after-response".into());
                out.insert(
                    "selector".into(),
                    json!({"expression": ensure_string(field(row, "value")), "method": "jsonq"}),
                );
                out.insert(
                    "variable".into(),
                    json!({"name": or_empty(field(row, "name")), "scope": scope}),
                );
                describe(&mut out, row);
                disable(&mut out, row);
                Value::Object(out)
            })
            .collect()
    })
}

/// The JS-trimmed text of a string with non-blank content.
fn trimmed_code(code: Option<&Value>) -> Option<&str> {
    code.and_then(Value::as_str)
        .map(js_trim)
        .filter(|code| !code.is_empty())
}

/// `toOpenCollectionScripts`: the allowed script phases in reference order, then the tests.
pub(super) fn scripts(request: Option<&Value>, keys: &[&str]) -> Option<Value> {
    let script = request.and_then(|r| field(r, "script"));
    let mut out: Vec<Value> = SCRIPT_TYPES
        .iter()
        .filter(|(key, _)| keys.contains(key))
        .filter_map(|(key, kind)| {
            trimmed_code(script.and_then(|s| field(s, key)))
                .map(|code| json!({"type": kind, "code": code}))
        })
        .collect();
    if let Some(code) = trimmed_code(request.and_then(|r| field(r, "tests"))) {
        out.push(json!({"type": "tests", "code": code}));
    }
    (!out.is_empty()).then_some(Value::Array(out))
}

/// `parseAssertionOperator`: a leading known operator splits off, a unary one drops the value,
/// and anything else is an `eq` against the whole untrimmed text.
fn assertion_operator(text: &Value) -> (&'static str, Option<Value>) {
    let Some(s) = text.as_str().filter(|s| !s.is_empty()) else {
        return ("eq", Some(text.clone()));
    };
    let mut words = js_trim(s).split(' ');
    let first = words.next().unwrap_or_default();
    match OPERATORS.iter().position(|op| *op == first) {
        Some(i) if i >= UNARY_FROM => (OPERATORS[i], None),
        Some(i) => (
            OPERATORS[i],
            Some(words.collect::<Vec<_>>().join(" ").into()),
        ),
        None => ("eq", Some(text.clone())),
    }
}

/// `toOpenCollectionAssertions`.
pub(super) fn assertions(assertions: Option<&Value>) -> Option<Value> {
    non_empty(assertions).map(|rows| {
        rows.iter()
            .map(|row| {
                let (operator, value) = assertion_operator(&or_empty(field(row, "value")));
                let mut out = Map::new();
                out.insert("expression".into(), or_empty(field(row, "name")));
                out.insert("operator".into(), operator.into());
                if let Some(value) = value {
                    out.insert("value".into(), value);
                }
                describe(&mut out, row);
                disable(&mut out, row);
                Value::Object(out)
            })
            .collect()
    })
}

/// A list body: its `data` key appears only when the mapped list is non-empty.
fn list_body(kind: &str, rows: Option<&Value>, map: impl Fn(&Value) -> Value) -> Value {
    let data: Vec<Value> = rows
        .and_then(Value::as_array)
        .map(|rows| rows.iter().map(map).collect())
        .unwrap_or_default();
    let mut out = Map::new();
    out.insert("type".into(), kind.into());
    if !data.is_empty() {
        out.insert("data".into(), Value::Array(data));
    }
    Value::Object(out)
}

/// A multipart entry: a file entry's missing value is an empty list, a text entry's is `''`.
fn multipart_entry(row: &Value) -> Value {
    let kind = field(row, "type");
    let empty = if kind.and_then(Value::as_str) == Some("file") {
        json!([])
    } else {
        json!("")
    };
    let mut out = Map::new();
    out.insert("name".into(), or_empty(field(row, "name")));
    if let Some(kind) = kind {
        out.insert("type".into(), kind.clone());
    }
    out.insert("value".into(), js_or(field(row, "value"), empty));
    if is_non_empty_string(field(row, "contentType")) {
        out.insert(
            "contentType".into(),
            field(row, "contentType").cloned().unwrap_or_default(),
        );
    }
    describe(&mut out, row);
    disable(&mut out, row);
    Value::Object(out)
}

/// A file body variant; `selected` defaults to false only when it is null or absent.
fn file_entry(row: &Value) -> Value {
    let selected = field(row, "selected")
        .filter(|v| !v.is_null())
        .cloned()
        .unwrap_or(Value::Bool(false));
    let mut out = Map::new();
    out.insert("filePath".into(), or_empty(field(row, "filePath")));
    out.insert("contentType".into(), or_empty(field(row, "contentType")));
    out.insert("selected".into(), selected);
    describe(&mut out, row);
    Value::Object(out)
}

/// `toOpenCollectionBody` for every mode but GraphQL, which its own writer handles.
pub(super) fn body(body: Option<&Value>) -> Option<Value> {
    let body = body.filter(|b| truthy(Some(b)))?;
    let mode = field(body, "mode").and_then(Value::as_str)?;
    let raw =
        |kind: &str, key: &str| Some(json!({"type": kind, "data": or_empty(field(body, key))}));
    match mode {
        "json" | "text" | "xml" | "sparql" => raw(mode, mode),
        "formUrlEncoded" => Some(list_body("form-urlencoded", field(body, mode), key_value)),
        "multipartForm" => Some(list_body(
            "multipart-form",
            field(body, mode),
            multipart_entry,
        )),
        "file" => Some(list_body("file", field(body, mode), file_entry)),
        _ => None,
    }
}

/// `normalizeOmitHeaders`: trimmed string names; anything else is dropped.
pub(super) fn omit_headers(names: Option<&Value>) -> Option<Value> {
    let names: Vec<Value> = non_empty(names)?
        .iter()
        .filter_map(|name| name.as_str().map(js_trim).filter(|n| !n.is_empty()))
        .map(Value::from)
        .collect();
    (!names.is_empty()).then_some(Value::Array(names))
}

/// `toOpenCollectionApp`: kept when enabled, even before any code is written.
pub(super) fn app(app: Option<&Value>) -> Option<Value> {
    let app = app.filter(|a| truthy(Some(a)))?;
    let enabled = field(app, "enabled") == Some(&Value::Bool(true));
    let code = field(app, "code").filter(|c| truthy(Some(c)));
    if code.is_none() && !enabled {
        return None;
    }
    let mut out = Map::new();
    if enabled {
        out.insert("enabled".into(), Value::Bool(true));
    }
    if let Some(code) = code {
        out.insert("code".into(), code.clone());
    }
    Some(Value::Object(out))
}
