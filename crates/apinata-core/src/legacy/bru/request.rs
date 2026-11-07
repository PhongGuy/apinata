//! The request grammar (`bruToJson`): its block table in the reference's order, and each
//! block's semantic action.

use super::auth::{AUTH_BLOCKS, auth_block, auth_block_error};
use super::example::example_json;
use super::lex::{Block, BlockBody, BlockKind, Dialect, Pair, PairValue, outdent, parse_blocks};
use super::{
    BruError, apply_data_type, is_truthy, js_number, merge_object, pair_value, pairs_to_object,
    pairs_to_rows,
};
use crate::js::{js_trim, parse_number};
use serde_json::{Map, Value, json};
use std::sync::OnceLock;

const VERBS: [&str; 9] = [
    "get", "post", "put", "delete", "patch", "options", "head", "connect", "trace",
];

/// Block keywords in the reference grammar's ordered-choice order, split around the auth
/// blocks; a keyword that is a prefix of a later one (`head` before `headers`, `body` before
/// `body:grpc`) relies on the body failing and backtracking.
const BLOCKS_BEFORE_AUTH: &[(&str, BlockKind)] = &[
    ("meta", BlockKind::Dictionary),
    ("get", BlockKind::Dictionary),
    ("post", BlockKind::Dictionary),
    ("put", BlockKind::Dictionary),
    ("delete", BlockKind::Dictionary),
    ("patch", BlockKind::Dictionary),
    ("options", BlockKind::Dictionary),
    ("head", BlockKind::Dictionary),
    ("connect", BlockKind::Dictionary),
    ("trace", BlockKind::Dictionary),
    ("http", BlockKind::Dictionary),
    ("grpc", BlockKind::Dictionary),
    ("ws", BlockKind::Dictionary),
    ("query", BlockKind::Dictionary),
    ("params:path", BlockKind::Dictionary),
    ("params:query", BlockKind::Dictionary),
    ("headers", BlockKind::Dictionary),
    ("metadata", BlockKind::Dictionary),
];

const BLOCKS_AFTER_AUTH: &[(&str, BlockKind)] = &[
    ("body:json", BlockKind::Text),
    ("body:text", BlockKind::Text),
    ("body:xml", BlockKind::Text),
    ("body:sparql", BlockKind::Text),
    ("body:graphql", BlockKind::Text),
    ("body:graphql:vars", BlockKind::Text),
    ("body:form-urlencoded", BlockKind::Dictionary),
    ("body:multipart-form", BlockKind::Dictionary),
    ("body:file", BlockKind::Dictionary),
    ("body", BlockKind::Text),
    ("body:grpc", BlockKind::Dictionary),
    ("body:ws", BlockKind::Dictionary),
    ("vars:pre-request", BlockKind::Dictionary),
    ("vars:post-response", BlockKind::Dictionary),
    ("assert", BlockKind::AssertDictionary),
    ("script:pre-request", BlockKind::Text),
    ("script:post-response", BlockKind::Text),
    ("script:grpc:before-call-start", BlockKind::Text),
    ("script:grpc:before-message-send", BlockKind::Text),
    ("script:grpc:after-message-receive", BlockKind::Text),
    ("script:grpc:after-call-end", BlockKind::Text),
    ("tests", BlockKind::Text),
    ("app", BlockKind::Dictionary),
    ("settings", BlockKind::Dictionary),
    ("docs", BlockKind::Text),
    ("example", BlockKind::Example),
];

/// Parses a request `.bru` file into the JSON the reference `bruToJson` returns.
pub(crate) fn parse_request(text: &str) -> Result<Value, BruError> {
    let table = [BLOCKS_BEFORE_AUTH, AUTH_BLOCKS, BLOCKS_AFTER_AUTH].concat();
    let blocks = parse_blocks(text, &table, Dialect::Request)?;
    // The reference's assert and `body:file` actions call string methods on every value, so a
    // list value throws after a successful parse; reject the file at that block's line.
    for block in &blocks {
        let what = match block.keyword {
            "assert" => "an assertion value",
            "body:file" => "a file body value",
            _ => "",
        };
        if !what.is_empty()
            && let Some(line) = block_with_list(block)
        {
            return Err(BruError {
                line,
                message: format!("a list is not a valid {what}"),
            });
        }
        if let Some(error) = auth_block_error(block) {
            return Err(error);
        }
    }
    let mut out = Map::new();
    for block in blocks {
        merge_object(&mut out, &block_json(&block), true);
    }
    Ok(Value::Object(out))
}

/// The block's line when one of its pairs holds a list value.
fn block_with_list(block: &Block) -> Option<usize> {
    match &block.body {
        BlockBody::Pairs(pairs) => pairs
            .iter()
            .any(|p| matches!(p.value, PairValue::List(_)))
            .then_some(block.line),
        BlockBody::Text(_) => None,
    }
}

fn block_json(block: &Block) -> Map<String, Value> {
    let value = match &block.body {
        BlockBody::Text(source) if block.keyword == "example" => {
            json!({ "examples": [example_json(source)] })
        }
        BlockBody::Pairs(pairs) => pairs_block(block.keyword, pairs),
        BlockBody::Text(source) => text_block(block.keyword, &outdent(source)),
    };
    match value {
        Value::Object(map) => map,
        _ => unreachable!("every block maps to an object"),
    }
}

fn pairs_block(keyword: &str, pairs: &[Pair]) -> Value {
    if let Some(auth) = auth_block(keyword, pairs) {
        return auth;
    }
    match keyword {
        "meta" => {
            let mut meta = pairs_to_object(pairs);
            if !is_truthy(meta.get("seq")) {
                meta.insert("seq".into(), json!(1));
            }
            if !is_truthy(meta.get("type")) {
                meta.insert("type".into(), json!("http"));
            }
            json!({ "meta": meta })
        }
        "http" => {
            let mut dict = pairs_to_object(pairs);
            let mut http = Map::new();
            if let Some(method) = dict.shift_remove("method") {
                http.insert("method".into(), method);
            }
            http.extend(dict);
            json!({ "http": http })
        }
        verb if VERBS.contains(&verb) => {
            let mut http = Map::new();
            http.insert("method".into(), json!(verb));
            http.extend(pairs_to_object(pairs));
            json!({ "http": http })
        }
        "query" | "params:query" => json!({ "params": params(pairs, "query") }),
        "params:path" => json!({ "params": params(pairs, "path") }),
        "headers" => json!({ "headers": pairs_to_rows(pairs) }),
        "metadata" => json!({ "metadata": pairs_to_rows(pairs) }),
        "grpc" => json!({ "grpc": pairs_to_object(pairs) }),
        "ws" => json!({ "ws": pairs_to_object(pairs) }),
        "vars:pre-request" => json!({ "vars": { "req": vars(pairs, true) } }),
        "vars:post-response" => json!({ "vars": { "res": vars(pairs, false) } }),
        "assert" => json!({ "assertions": pairs_to_rows(pairs) }),
        "body:form-urlencoded" => json!({ "body": { "formUrlEncoded": pairs_to_rows(pairs) } }),
        "body:multipart-form" => json!({ "body": { "multipartForm": multipart(pairs, true) } }),
        "body:file" => json!({ "body": { "file": file_rows(pairs) } }),
        "body:grpc" => json!({ "body": { "mode": "grpc", "grpc": [{
            "name": raw(pairs, "name"),
            "content": raw(pairs, "content"),
        }] } }),
        "body:ws" => json!({ "body": { "mode": "ws", "ws": [{
            "name": raw(pairs, "name"),
            "type": raw(pairs, "type"),
            "content": raw(pairs, "content"),
            "selected": raw(pairs, "selected") == json!("true"),
        }] } }),
        "app" => {
            let app = pairs_to_object(pairs);
            let code = app
                .get("code")
                .filter(|v| is_truthy(Some(v)))
                .cloned()
                .unwrap_or(Value::Null);
            json!({ "app": { "code": code, "enabled": to_bool(app.get("enabled")) } })
        }
        "settings" => json!({ "settings": settings(pairs) }),
        other => unreachable!("{other} is not a dictionary block"),
    }
}

/// `_.find(pairs, { name })?.value ?? ''` over raw names, `~` included.
fn raw(pairs: &[Pair], name: &str) -> Value {
    pairs
        .iter()
        .find(|p| p.key == name)
        .map_or_else(|| json!(""), |p| pair_value(&p.value))
}

/// `toBool`: only the exact string `true` is true.
fn to_bool(value: Option<&Value>) -> bool {
    value == Some(&json!("true"))
}

/// JavaScript `String(value)` for a dictionary value; a list joins its items with commas.
fn js_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(","),
        _ => String::new(),
    }
}

/// The settings action: typed flags and numbers with the reference's defaults; keys it does
/// not know are dropped.
fn settings(pairs: &[Pair]) -> Value {
    let dict = pairs_to_object(pairs);
    let mut out = Map::new();
    out.insert("encodeUrl".into(), json!(to_bool(dict.get("encodeUrl"))));
    let timeout = match dict.get("timeout") {
        Some(v) if *v == "inherit" => Some(json!("inherit")),
        Some(v) => parse_int(&js_string(v)).map(js_number),
        None => None,
    };
    out.insert("timeout".into(), timeout.unwrap_or(json!(0)));
    if let Some(v) = dict.get("followRedirects") {
        out.insert("followRedirects".into(), json!(to_bool(Some(v))));
    }
    // A blank is "unset" rather than zero redirects; negatives and non-numbers are dropped.
    if let Some(v) = dict.get("maxRedirects").filter(|v| is_truthy(Some(v)))
        && let Some(n) = parse_number(&js_string(v)).filter(|n| n.is_finite() && *n >= 0.0)
    {
        out.insert("maxRedirects".into(), js_number(n.trunc()));
    }
    if let Some(v) = dict.get("forwardAuthorizationHeader") {
        out.insert("forwardAuthorizationHeader".into(), json!(to_bool(Some(v))));
    }
    if let Some(n) = dict
        .get("keepAliveInterval")
        .and_then(|v| parse_number(&js_string(v)))
        .filter(|n| *n != 0.0)
    {
        out.insert("keepAliveInterval".into(), js_number(n));
    }
    if let Some(Value::Array(names)) = dict.get("omitHeaders")
        && !names.is_empty()
    {
        let names: Vec<Value> = names
            .iter()
            .map(|n| n.as_str().map_or("", js_trim))
            .filter(|n| !n.is_empty())
            .map(|n| json!(n))
            .collect();
        out.insert("omitHeaders".into(), Value::Array(names));
    }
    Value::Object(out)
}

/// JavaScript `parseInt(text, 10)`: optional sign and the leading decimal digits after
/// whitespace, or `None` for `NaN`.
fn parse_int(text: &str) -> Option<f64> {
    let text = text.trim_start_matches(crate::js::is_js_whitespace);
    let (sign, rest) = match text.as_bytes().first() {
        Some(b'-') => ("-", &text[1..]),
        Some(b'+') => ("", &text[1..]),
        _ => ("", text),
    };
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return None;
    }
    format!("{sign}{}", &rest[..digits]).parse().ok()
}

/// `/^(.*?)\s*@contentType\((.*?)\)\s*$/s` applied to a string value: the value before the
/// suffix and the content type, or `None` when there is no suffix.
fn split_content_type(value: &str) -> Option<(&str, &str)> {
    static RE: OnceLock<regress::Regex> = OnceLock::new();
    // A fixed pattern, so a failure to compile is a bug every run hits, not an input error.
    #[allow(clippy::expect_used)]
    let re = RE.get_or_init(|| {
        regress::Regex::with_flags(r"^(.*?)\s*@contentType\((.*?)\)\s*$", "s")
            .expect("the contentType suffix pattern compiles")
    });
    let found = re.find(value)?;
    Some((&value[found.group(1)?], &value[found.group(2)?]))
}

/// The raw text inside `@file(...)`, or `None` when the value is not a file reference. A
/// multipart row splits it on `|` into paths, dropping empty ones where `drop_empty_paths`.
fn file_reference(value: &str) -> Option<&str> {
    value.strip_prefix("@file(")?.strip_suffix(')')
}

/// `mapPairListToKeyValPairsMultipart`: rows typed `text` or `file`, with any content type
/// split off a string value; the request grammar's copy drops empty file paths.
pub(super) fn multipart(pairs: &[Pair], drop_empty_paths: bool) -> Vec<Map<String, Value>> {
    let mut rows = pairs_to_rows(pairs);
    for row in &mut rows {
        row.insert("type".into(), json!("text"));
        let Some(Value::String(value)) = row.get("value").cloned() else {
            continue;
        };
        let (value, content_type) = split_content_type(&value).unwrap_or((&value, ""));
        row.insert("value".into(), json!(value));
        row.insert("contentType".into(), json!(content_type));
        if let Some(paths) = file_reference(value) {
            let paths: Vec<&str> = paths
                .split('|')
                .filter(|p| !drop_empty_paths || !p.is_empty())
                .collect();
            row.insert("type".into(), json!("file"));
            row.insert("value".into(), json!(paths));
        }
    }
    rows
}

/// `mapPairListToKeyValPairsFile`: a file reference row drops `name`, `value` and `enabled`
/// for `filePath` and `selected`; list values were rejected before this runs.
pub(super) fn file_rows(pairs: &[Pair]) -> Vec<Map<String, Value>> {
    let mut rows = pairs_to_rows(pairs);
    for row in &mut rows {
        let Some(Value::String(value)) = row.get("value").cloned() else {
            continue;
        };
        let (value, content_type) = split_content_type(&value)
            .map_or((value.as_str(), ""), |(v, t)| (js_trim(v), js_trim(t)));
        row.insert("value".into(), json!(value));
        row.insert("contentType".into(), json!(content_type));
        if let Some(path) = file_reference(value) {
            let path = path.to_owned();
            row.shift_remove("value");
            row.shift_remove("name");
            let enabled = row.shift_remove("enabled").unwrap_or(Value::Bool(true));
            row.insert("filePath".into(), json!(path));
            row.insert("selected".into(), enabled);
        }
    }
    rows
}

fn text_block(keyword: &str, text: &str) -> Value {
    match keyword {
        "body" => json!({ "http": { "body": "json" }, "body": { "json": text } }),
        "body:json" => json!({ "body": { "json": text } }),
        "body:text" => json!({ "body": { "text": text } }),
        "body:xml" => json!({ "body": { "xml": text } }),
        "body:sparql" => json!({ "body": { "sparql": text } }),
        "script:pre-request" => json!({ "script": { "req": text } }),
        "script:post-response" => json!({ "script": { "res": text } }),
        "script:grpc:before-call-start" => json!({ "script": { "beforeCallStart": text } }),
        "script:grpc:before-message-send" => json!({ "script": { "beforeMessageSend": text } }),
        "script:grpc:after-message-receive" => {
            json!({ "script": { "afterMessageReceive": text } })
        }
        "script:grpc:after-call-end" => json!({ "script": { "afterCallEnd": text } }),
        "body:graphql" => json!({ "body": { "graphql": { "query": text } } }),
        "body:graphql:vars" => json!({ "body": { "graphql": { "variables": text } } }),
        "tests" => json!({ "tests": text }),
        "docs" => json!({ "docs": text }),
        other => unreachable!("{other} is not a text block"),
    }
}

/// `mapRequestParams`: the row shape plus the param `type`.
pub(super) fn params(pairs: &[Pair], kind: &str) -> Vec<Map<String, Value>> {
    let mut rows = pairs_to_rows(pairs);
    for row in &mut rows {
        row.insert("type".into(), json!(kind));
    }
    rows
}

/// Variable rows: a leading `@` (after any `~`) marks the variable local; only pre-request
/// variables take a typed value from their annotations.
pub(super) fn vars(pairs: &[Pair], typed: bool) -> Vec<Map<String, Value>> {
    let mut rows = pairs_to_rows(pairs);
    for (row, pair) in rows.iter_mut().zip(pairs) {
        if typed {
            apply_data_type(row, &pair.annotations);
        }
        let name = row
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let local = match name.strip_prefix('@') {
            Some(rest) => {
                row.insert("name".into(), json!(rest));
                true
            }
            None => false,
        };
        row.insert("local".into(), json!(local));
    }
    rows
}
