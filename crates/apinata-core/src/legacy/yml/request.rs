//! Request document writers: ports of the reference's `stringify*Request`, each building the
//! document's keys in the reference's insertion order.

use super::auth::map_auth;
use super::item::{Item, ItemKind, parse_item};
use super::rows::{self, GRPC_SCRIPT_KEYS, HTTP_SCRIPT_KEYS};
use super::value::{get, is_non_empty_string, js_or, number, to_number, truthy};
use crate::js::{js_trim, stringify_pretty};
use serde_json::{Map, Value, json};
use yaml_serde::Mapping;

/// A JSON section as YAML, keeping its key order.
// Every JSON value has a YAML form, so the conversion cannot fail.
#[allow(clippy::expect_used)]
fn yaml(value: Value) -> yaml_serde::Value {
    yaml_serde::to_value(value).expect("JSON converts to YAML")
}

/// A JavaScript number as YAML; a non-finite one is written as YAML's own `.inf` or `.nan`.
fn yaml_number(n: f64) -> yaml_serde::Value {
    if n.is_finite() {
        yaml(number(n))
    } else {
        yaml_serde::Value::Number(n.into())
    }
}

/// The `info` block: `seq` is a JavaScript number, so a non-finite one is written as YAML's own.
fn info(item: &Item, kind: &str) -> yaml_serde::Value {
    let mut info = Mapping::new();
    let name = item
        .name
        .clone()
        .filter(|n| is_non_empty_string(Some(n)))
        .unwrap_or_else(|| json!("Untitled Request"));
    info.insert("name".into(), yaml(name));
    info.insert("type".into(), kind.into());
    // `if (item.seq)`: zero and NaN are falsy.
    if item.seq != 0.0 && !item.seq.is_nan() {
        info.insert("seq".into(), yaml_number(item.seq));
    }
    if item.tags.as_array().is_some_and(|t| !t.is_empty()) {
        info.insert("tags".into(), yaml(item.tags.clone()));
    }
    yaml_serde::Value::Mapping(info)
}

/// Inserts `value` under `key` when the mapper produced one.
fn put(out: &mut Map<String, Value>, key: &str, value: Option<Value>) {
    if let Some(value) = value {
        out.insert(key.into(), value);
    }
}

/// `resolveTimeoutSetting`: "inherit" or a positive finite number, else 0.
fn timeout(value: Option<&Value>) -> Value {
    match value {
        Some(v @ Value::String(s)) if s == "inherit" => v.clone(),
        Some(Value::Number(n)) if n.as_f64().is_some_and(|n| n.is_finite() && n > 0.0) => {
            Value::Number(n.clone())
        }
        _ => json!(0),
    }
}

/// `toMaxRedirects`: a non-negative finite number truncated, else 5.
fn max_redirects(value: Option<&Value>) -> Value {
    match value.and_then(Value::as_f64) {
        Some(n) if n.is_finite() && n >= 0.0 => number(n.trunc()),
        _ => json!(5),
    }
}

/// The HTTP and GraphQL `settings` block; booleans other than `false` fall back to `true`.
/// Only HTTP writes `omitHeaders`.
fn http_settings(settings: &Value, omit_headers: bool) -> Value {
    let field = |key| get(settings, key);
    let flag = |key| field(key) != Some(&Value::Bool(false));
    let mut out = Map::new();
    out.insert("encodeUrl".into(), json!(flag("encodeUrl")));
    out.insert("timeout".into(), timeout(field("timeout")));
    out.insert("followRedirects".into(), json!(flag("followRedirects")));
    out.insert("maxRedirects".into(), max_redirects(field("maxRedirects")));
    let forward = match field("forwardAuthorizationHeader") {
        None | Some(Value::Null) => json!(true),
        Some(v) => v.clone(),
    };
    out.insert("forwardAuthorizationHeader".into(), forward);
    if omit_headers {
        put(
            &mut out,
            "omitHeaders",
            rows::omit_headers(field("omitHeaders")),
        );
    }
    Value::Object(out)
}

/// `stringifyHttpRequest`.
pub(super) fn http_document(item: &Item) -> Mapping {
    let request = &item.request;
    let field = |key| get(request, key);
    let mut doc = Mapping::new();
    doc.insert("info".into(), info(item, "http"));

    let mut http = Map::new();
    let text_or = |key, fallback: &str| {
        field(key)
            .filter(|v| is_non_empty_string(Some(v)))
            .cloned()
            .unwrap_or_else(|| json!(fallback))
    };
    http.insert("method".into(), text_or("method", "GET"));
    http.insert("url".into(), text_or("url", ""));
    put(&mut http, "headers", rows::http_headers(field("headers")));
    put(&mut http, "params", rows::params(field("params")));
    put(&mut http, "body", rows::body(field("body")));
    put(&mut http, "auth", map_auth(field("auth")));
    doc.insert("http".into(), yaml(Value::Object(http)));

    let mut runtime = Map::new();
    put(&mut runtime, "variables", rows::variables(field("vars")));
    put(
        &mut runtime,
        "scripts",
        rows::scripts(Some(request), HTTP_SCRIPT_KEYS),
    );
    put(
        &mut runtime,
        "assertions",
        rows::assertions(field("assertions")),
    );
    put(
        &mut runtime,
        "actions",
        rows::actions(get(request, "vars.res")),
    );
    if !runtime.is_empty() {
        doc.insert("runtime".into(), yaml(Value::Object(runtime)));
    }

    doc.insert("settings".into(), yaml(http_settings(&item.settings, true)));
    let examples: Vec<Value> = item.examples.iter().map(example).collect();
    if !examples.is_empty() {
        doc.insert("examples".into(), yaml(Value::Array(examples)));
    }
    if let Some(docs) = field("docs").filter(|d| is_non_empty_string(Some(d))) {
        doc.insert("docs".into(), yaml(docs.clone()));
    }
    if let Some(app) = rows::app(item.app.as_ref()) {
        doc.insert("app".into(), yaml(app));
    }
    doc
}

/// One entry of the HTTP `examples` block.
fn example(example: &Value) -> Value {
    let field = |key| get(example, key);
    let mut out = Map::new();
    out.insert(
        "name".into(),
        js_or(field("name"), json!("Untitled Example")),
    );
    if let Some(description) = field("description").filter(|d| is_non_empty_string(Some(d))) {
        out.insert("description".into(), description.clone());
    }
    if let Some(request) = field("request").filter(|r| truthy(Some(r))) {
        let field = |key| get(request, key);
        let mut out_request = Map::new();
        out_request.insert("url".into(), js_or(field("url"), json!("")));
        out_request.insert("method".into(), js_or(field("method"), json!("GET")));
        put(
            &mut out_request,
            "headers",
            rows::http_headers(field("headers")),
        );
        put(&mut out_request, "params", rows::params(field("params")));
        put(&mut out_request, "body", rows::body(field("body")));
        out.insert("request".into(), Value::Object(out_request));
    }
    if let Some(response) = field("response").filter(|r| truthy(Some(r))) {
        let field = |key| get(response, key);
        let mut out_response = Map::new();
        let status = to_number(field("status"));
        if status.is_finite() && status.fract() == 0.0 && status > 0.0 {
            out_response.insert("status".into(), number(status));
        }
        if let Some(text) = field("statusText").filter(|t| is_non_empty_string(Some(t))) {
            out_response.insert("statusText".into(), text.clone());
        }
        put(
            &mut out_response,
            "headers",
            rows::response_headers(field("headers")),
        );
        let body = field("body").filter(|b| truthy(Some(b)));
        let kind = body
            .and_then(|b| get(b, "type"))
            .filter(|t| truthy(Some(t)));
        if let (Some(kind), Some(content)) = (kind, body.and_then(|b| get(b, "content"))) {
            let data = match content {
                Value::String(text) => text.clone(),
                other => stringify_pretty(other),
            };
            out_response.insert("body".into(), json!({"type": kind, "data": data}));
        }
        out.insert("response".into(), Value::Object(out_response));
    }
    Value::Object(out)
}

/// The `runtime` block shared by the GraphQL, gRPC and WebSocket writers; `actions` is read
/// only by GraphQL and `assertions` not by WebSocket.
fn runtime(
    request: &Value,
    script_keys: &[&str],
    assertions: bool,
    actions: bool,
) -> Option<Value> {
    let field = |key| get(request, key);
    let mut runtime = Map::new();
    put(&mut runtime, "variables", rows::variables(field("vars")));
    put(
        &mut runtime,
        "scripts",
        rows::scripts(Some(request), script_keys),
    );
    if assertions {
        put(
            &mut runtime,
            "assertions",
            rows::assertions(field("assertions")),
        );
    }
    if actions {
        put(
            &mut runtime,
            "actions",
            rows::actions(get(request, "vars.res")),
        );
    }
    (!runtime.is_empty()).then_some(Value::Object(runtime))
}

/// The request's `docs` when its text is non-blank.
fn docs(doc: &mut Mapping, request: &Value) {
    if let Some(docs) = get(request, "docs").filter(|d| is_non_empty_string(Some(d))) {
        doc.insert("docs".into(), yaml(docs.clone()));
    }
}

/// `value` when it is a non-blank string, else `fallback`.
fn text_or(value: Option<&Value>, fallback: &str) -> Value {
    value
        .filter(|v| is_non_empty_string(Some(v)))
        .cloned()
        .unwrap_or_else(|| json!(fallback))
}

/// `stringifyGraphQLRequest`: the HTTP shape with a GraphQL body, without examples or app.
fn graphql_document(item: &Item) -> Mapping {
    let request = &item.request;
    let field = |key| get(request, key);
    let mut doc = Mapping::new();
    doc.insert("info".into(), info(item, "graphql"));

    let mut graphql = Map::new();
    graphql.insert("method".into(), text_or(field("method"), "POST"));
    graphql.insert("url".into(), text_or(field("url"), ""));
    put(
        &mut graphql,
        "headers",
        rows::http_headers(field("headers")),
    );
    put(&mut graphql, "params", rows::params(field("params")));
    let source = field("body")
        .filter(|b| get(b, "mode").and_then(Value::as_str) == Some("graphql"))
        .and_then(|b| get(b, "graphql"))
        .filter(|g| truthy(Some(g)));
    if let Some(source) = source {
        let mut body = Map::new();
        for key in ["query", "variables"] {
            if let Some(text) = get(source, key).filter(|t| is_non_empty_string(Some(t))) {
                body.insert(key.into(), text.clone());
            }
        }
        if !body.is_empty() {
            graphql.insert("body".into(), Value::Object(body));
        }
    }
    put(&mut graphql, "auth", map_auth(field("auth")));
    doc.insert("graphql".into(), yaml(Value::Object(graphql)));

    if let Some(runtime) = runtime(request, HTTP_SCRIPT_KEYS, true, true) {
        doc.insert("runtime".into(), yaml(runtime));
    }
    doc.insert(
        "settings".into(),
        yaml(http_settings(&item.settings, false)),
    );
    docs(&mut doc, request);
    doc
}

/// `stringifyGrpcRequest`: no settings, and gRPC script phases.
fn grpc_document(item: &Item) -> Mapping {
    let request = &item.request;
    let field = |key| get(request, key);
    let mut doc = Mapping::new();
    doc.insert("info".into(), info(item, "grpc"));

    let mut grpc = Map::new();
    grpc.insert("url".into(), text_or(field("url"), ""));
    grpc.insert("method".into(), text_or(field("method"), ""));
    if let Some(kind) = field("methodType").filter(|k| truthy(Some(k))) {
        grpc.insert("methodType".into(), kind.clone());
    }
    if let Some(path) = field("protoPath").filter(|p| is_non_empty_string(Some(p))) {
        grpc.insert("protoFilePath".into(), path.clone());
    }
    put(&mut grpc, "metadata", rows::http_headers(field("headers")));
    if let Some(messages) = body_messages(field("body"), "grpc") {
        let messages = messages
            .iter()
            .enumerate()
            .map(|(i, msg)| {
                json!({
                    "title": js_or(get(msg, "name"), json!(format!("message {}", i + 1))),
                    "message": js_or(get(msg, "content"), json!("")),
                })
            })
            .collect();
        grpc.insert("message".into(), Value::Array(messages));
    }
    put(&mut grpc, "auth", map_auth(field("auth")));
    doc.insert("grpc".into(), yaml(Value::Object(grpc)));

    if let Some(runtime) = runtime(request, GRPC_SCRIPT_KEYS, true, false) {
        doc.insert("runtime".into(), yaml(runtime));
    }
    docs(&mut doc, request);
    doc
}

/// The non-empty message list of a body in `mode`.
fn body_messages<'a>(body: Option<&'a Value>, mode: &str) -> Option<&'a Vec<Value>> {
    body.filter(|b| get(b, "mode").and_then(Value::as_str) == Some(mode))
        .and_then(|b| get(b, mode))
        .and_then(Value::as_array)
        .filter(|m| !m.is_empty())
}

/// A message's text field, JS-trimmed, is non-empty.
fn has_text(msg: &Value, key: &str) -> bool {
    get(msg, key)
        .and_then(Value::as_str)
        .is_some_and(|t| !js_trim(t).is_empty())
}

/// `stringifyWebsocketRequest`: one unnamed message with content is written flat, any other
/// list as titled variants; settings fall back to 0 where `Number()` gives NaN.
fn ws_document(item: &Item) -> Mapping {
    let request = &item.request;
    let field = |key| get(request, key);
    let mut doc = Mapping::new();
    doc.insert("info".into(), info(item, "websocket"));

    let mut websocket = Map::new();
    websocket.insert("url".into(), text_or(field("url"), ""));
    put(
        &mut websocket,
        "headers",
        rows::http_headers(field("headers")),
    );
    if let Some(messages) = body_messages(field("body"), "ws") {
        let message = |msg: &Value| {
            json!({
                "type": js_or(get(msg, "type"), json!("text")),
                "data": js_or(get(msg, "content"), json!("")),
            })
        };
        let single = (messages.len() == 1).then(|| &messages[0]);
        let message = match single {
            Some(msg) if !has_text(msg, "name") && has_text(msg, "content") => message(msg),
            _ => messages
                .iter()
                .enumerate()
                .map(|(i, msg)| {
                    json!({
                        "title": js_or(get(msg, "name"), json!(format!("message {}", i + 1))),
                        "selected": js_or(get(msg, "selected"), json!(false)),
                        "message": message(msg),
                    })
                })
                .collect(),
        };
        websocket.insert("message".into(), message);
    }
    put(&mut websocket, "auth", map_auth(field("auth")));
    doc.insert("websocket".into(), yaml(Value::Object(websocket)));

    if let Some(runtime) = runtime(request, HTTP_SCRIPT_KEYS, false, false) {
        doc.insert("runtime".into(), yaml(runtime));
    }
    if truthy(Some(&item.settings)) {
        let mut settings = Mapping::new();
        for key in ["timeout", "keepAliveInterval"] {
            let n = to_number(get(&item.settings, key));
            settings.insert(key.into(), yaml_number(if n.is_nan() { 0.0 } else { n }));
        }
        doc.insert("settings".into(), yaml_serde::Value::Mapping(settings));
    }
    docs(&mut doc, request);
    doc
}

/// `stringifyApp`: `seq` is always a number after `Number()`, so NaN is written as `.nan`.
fn app_document(item: &Item) -> Mapping {
    let mut info = Mapping::new();
    let name = item
        .name
        .clone()
        .filter(|n| is_non_empty_string(Some(n)))
        .unwrap_or_else(|| json!("App"));
    info.insert("name".into(), yaml(name));
    info.insert("type".into(), "app".into());
    info.insert("seq".into(), yaml_number(item.seq));
    if item.tags.as_array().is_some_and(|t| !t.is_empty()) {
        info.insert("tags".into(), yaml(item.tags.clone()));
    }
    let mut doc = Mapping::new();
    doc.insert("info".into(), yaml_serde::Value::Mapping(info));
    let code = item.app.as_ref().and_then(|app| get(app, "code"));
    if let Some(code) = code.filter(|c| is_non_empty_string(Some(c))) {
        doc.insert("code".into(), yaml(code.clone()));
    }
    doc
}

/// The YAML document the migration writes for a request `.bru` parse, every item kind.
pub(crate) fn map_request(parsed: &Value) -> Mapping {
    let item = parse_item(parsed);
    match item.kind {
        ItemKind::Http => http_document(&item),
        ItemKind::Graphql => graphql_document(&item),
        ItemKind::Grpc => grpc_document(&item),
        ItemKind::Ws => ws_document(&item),
        ItemKind::App => app_document(&item),
    }
}
