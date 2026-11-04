//! `parseBruRequest`: the reader's parse reshaped into the reference's item, followed by the
//! migration's strip of `.bru` from `bru.runRequest` calls in the request scripts and tests.

use super::run_request::strip_bru_ext;
use super::value::{ensure_string, get, js_or, number, to_number, truthy};
use crate::js::number_to_string;
use serde_json::{Map, Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ItemKind {
    Http,
    Graphql,
    Grpc,
    Ws,
    App,
}

/// The reference item a document writer reads; `None` and absent keys stand for `undefined`.
pub(super) struct Item {
    pub(super) kind: ItemKind,
    pub(super) name: Option<Value>,
    pub(super) seq: f64,
    pub(super) tags: Value,
    pub(super) settings: Value,
    pub(super) app: Option<Value>,
    pub(super) request: Value,
    /// The examples `parseBruRequest` builds; empty for app items.
    pub(super) examples: Vec<Value>,
}

/// The `oauth2_additional_parameters_*` lists, grouped as `getOauth2AdditionalParameters` does.
const ADDITIONAL_SOURCES: [(&str, &str, &str); 8] = [
    ("authorization", "headers", "auth_req_headers"),
    ("authorization", "queryparams", "auth_req_queryparams"),
    ("token", "headers", "access_token_req_headers"),
    ("token", "queryparams", "access_token_req_queryparams"),
    ("token", "body", "access_token_req_bodyvalues"),
    ("refresh", "headers", "refresh_token_req_headers"),
    ("refresh", "queryparams", "refresh_token_req_queryparams"),
    ("refresh", "body", "refresh_token_req_bodyvalues"),
];

/// lodash `_.get(value, path, default)`: the default replaces only `undefined`.
fn get_or(value: &Value, path: &str, default: Value) -> Value {
    get(value, path).cloned().unwrap_or(default)
}

/// `getOauth2AdditionalParameters(parsed)` when `auth.oauth2.grantType` is truthy and any list is
/// non-empty; shared by `parse_item` and the folder and collection parse.
pub(super) fn oauth2_additional_parameters(parsed: &Value) -> Option<Value> {
    let grant = get(parsed, "auth.oauth2.grantType");
    if !truthy(grant) {
        return None;
    }
    let grant = grant.and_then(Value::as_str);
    let mut out = Map::new();
    for kind in ["authorization", "token", "refresh"] {
        let included = match kind {
            "authorization" => matches!(grant, Some("authorization_code" | "implicit")),
            _ => grant != Some("implicit"),
        };
        if !included {
            continue;
        }
        let mut params = Vec::new();
        for (_, send_in, source) in ADDITIONAL_SOURCES.iter().filter(|s| s.0 == kind) {
            let key = format!("oauth2_additional_parameters_{source}");
            let rows = parsed.get(&key).and_then(Value::as_array);
            // `{...param, sendIn}`: a row's own `sendIn` keeps its place with the new value.
            params.extend(rows.into_iter().flatten().map(|row| {
                let mut param = row.as_object().cloned().unwrap_or_default();
                param.insert("sendIn".into(), json!(send_in));
                Value::Object(param)
            }));
        }
        if !params.is_empty() {
            out.insert(kind.into(), Value::Array(params));
        }
    }
    (!out.is_empty()).then_some(Value::Object(out))
}

/// Sets `object.key`, as JavaScript assignment does on an object; a missing object starts empty.
fn assign(target: Value, key: &str, value: Value) -> Value {
    match target {
        Value::Object(mut map) => {
            map.insert(key.into(), value);
            Value::Object(map)
        }
        Value::Null => Value::Object(Map::from_iter([(key.to_owned(), value)])),
        other => other,
    }
}

/// The migration's strip, applied to string scripts only; anything else passes through.
pub(super) fn strip(value: &mut Value) {
    if let Value::String(code) = value {
        *code = strip_bru_ext(code);
    }
}

fn kind_of(parsed: &Value) -> ItemKind {
    match get(parsed, "meta.type").and_then(Value::as_str) {
        Some("app") => ItemKind::App,
        Some("graphql") => ItemKind::Graphql,
        Some("grpc") => ItemKind::Grpc,
        Some("ws") => ItemKind::Ws,
        _ => ItemKind::Http,
    }
}

/// `{name: 'message 1', content: '{}'}` under `mode`, the default gRPC and WebSocket body.
fn default_message_body(mode: &str) -> Value {
    json!({"mode": mode, mode: [{"name": "message 1", "content": "{}"}]})
}

/// `parseBruRequest(parsed)` and the migration's strip of `script.req`, `script.res` and `tests`.
pub(super) fn parse_item(parsed: &Value) -> Item {
    let kind = kind_of(parsed);
    let tags = get(parsed, "meta.tags")
        .filter(|t| t.is_array())
        .cloned()
        .unwrap_or_else(|| json!([]));
    let mut item = Item {
        kind,
        name: get(parsed, "meta.name").cloned(),
        seq: to_number(get(parsed, "meta.seq")),
        tags,
        settings: get_or(parsed, "settings", json!({})),
        app: None,
        request: Value::Null,
        examples: Vec::new(),
    };
    if kind == ItemKind::App {
        item.app = Some(json!({"code": get_or(parsed, "app.code", Value::Null)}));
        return item;
    }
    item.app = get(parsed, "app").filter(|a| truthy(Some(a))).map(|app| {
        json!({
            "code": get_or(app, "code", Value::Null),
            "enabled": get(app, "enabled") == Some(&Value::Bool(true)),
        })
    });

    let mut request = Map::new();
    let method = match kind {
        ItemKind::Grpc => get_or(parsed, "grpc.method", json!("")),
        _ => json!(ensure_string(get(parsed, "http.method")).to_uppercase()),
    };
    request.insert("method".into(), method);
    let url = match kind {
        ItemKind::Grpc => get(parsed, "grpc.url"),
        ItemKind::Ws => get(parsed, "ws.url"),
        _ => None,
    }
    .or_else(|| get(parsed, "http.url"));
    if let Some(url) = url {
        request.insert("url".into(), url.clone());
    }
    let headers = if kind == ItemKind::Grpc {
        "metadata"
    } else {
        "headers"
    };
    request.insert("headers".into(), get_or(parsed, headers, json!([])));
    let auth_mode = |block: &str| get_or(parsed, &format!("{block}.auth"), json!("none"));
    let auth = get_or(parsed, "auth", json!({}));
    let mut script = get_or(parsed, "script", json!({}));
    if let Value::Object(script) = &mut script {
        for key in ["req", "res"] {
            if let Some(code) = script.get_mut(key) {
                strip(code);
            }
        }
    }
    let mut tests = get_or(parsed, "tests", json!(""));
    strip(&mut tests);
    let (auth, body) = match kind {
        ItemKind::Grpc => (
            assign(auth, "mode", auth_mode("grpc")),
            get(parsed, "body")
                .cloned()
                .unwrap_or_else(|| default_message_body("grpc")),
        ),
        ItemKind::Ws => (
            assign(auth, "mode", auth_mode("ws")),
            get(parsed, "body")
                .cloned()
                .unwrap_or_else(|| default_message_body("ws")),
        ),
        _ => (
            assign(auth, "mode", auth_mode("http")),
            assign(
                get_or(parsed, "body", json!({})),
                "mode",
                get_or(parsed, "http.body", json!("none")),
            ),
        ),
    };
    request.insert("auth".into(), auth);
    request.insert("body".into(), body);
    request.insert("script".into(), script);
    request.insert("vars".into(), get_or(parsed, "vars", json!({})));
    request.insert("assertions".into(), get_or(parsed, "assertions", json!([])));
    request.insert("tests".into(), tests);
    request.insert("docs".into(), get_or(parsed, "docs", json!("")));
    match kind {
        ItemKind::Grpc => {
            for (from, to) in [
                ("grpc.methodType", "methodType"),
                ("grpc.protoPath", "protoPath"),
            ] {
                if let Some(value) = get(parsed, from).filter(|v| truthy(Some(v))) {
                    request.insert(to.into(), value.clone());
                }
            }
        }
        ItemKind::Ws => {}
        _ => {
            request.insert("params".into(), get_or(parsed, "params", json!([])));
        }
    }
    if let Some(params) = oauth2_additional_parameters(parsed)
        && let Some(Value::Object(oauth2)) = request
            .get_mut("auth")
            .and_then(|auth| auth.get_mut("oauth2"))
    {
        oauth2.insert("additionalParameters".into(), params);
    }
    item.request = Value::Object(request);
    let parent_method = get(parsed, "http.method");
    item.examples = match get(parsed, "examples") {
        Some(Value::Array(examples)) => examples
            .iter()
            .map(|example| example_item(example, parent_method))
            .collect(),
        _ => Vec::new(),
    };
    item
}

/// A JavaScript number as JSON; a non-finite one is carried as the text JavaScript prints.
fn js_number(n: f64) -> Value {
    if n.is_finite() {
        number(n)
    } else {
        Value::String(number_to_string(n))
    }
}

/// `bruExampleToJson(example, true, parentType, parentMethod)` with `parent_method` the raw
/// `http.method`; only the fields the HTTP writer reads are built.
pub(super) fn example_item(example: &Value, parent_method: Option<&Value>) -> Value {
    let method = js_or(parent_method, get_or(example, "http.method", json!("GET")));
    let mut status = get_or(example, "response.status", json!("200"));
    let mut status_text = get_or(example, "response.statusText", json!("OK"));
    // Older imports stored the code and the text swapped; a numeric text swaps them back.
    if to_number(Some(&status)).is_nan() && !to_number(Some(&status_text)).is_nan() {
        std::mem::swap(&mut status, &mut status_text);
    }
    let status = match to_number(Some(&status)) {
        n if n == 0.0 || n.is_nan() => 200.0,
        n => n,
    };

    let mut request = Map::new();
    request.insert(
        "method".into(),
        js_or(get(example, "request.method"), method),
    );
    if let Some(url) = get(example, "request.url") {
        request.insert("url".into(), url.clone());
    }
    request.insert(
        "headers".into(),
        get_or(example, "request.headers", json!([])),
    );
    request.insert(
        "body".into(),
        get_or(example, "request.body", json!({"mode": "none"})),
    );
    request.insert(
        "params".into(),
        get_or(example, "request.params", json!([])),
    );

    // `{name: header.name, value: header.value}`: an undefined field stays absent.
    let headers = match get_or(example, "response.headers", json!([])) {
        Value::Array(rows) => Value::Array(
            rows.iter()
                .map(|row| {
                    let mut out = Map::new();
                    for key in ["name", "value"] {
                        if let Some(v) = get(row, key) {
                            out.insert(key.into(), v.clone());
                        }
                    }
                    Value::Object(out)
                })
                .collect(),
        ),
        other => other,
    };
    let response = json!({
        "headers": headers,
        "status": js_number(status),
        "statusText": js_or(Some(&status_text), json!("OK")),
        "body": {
            "type": get_or(example, "response.body.type", json!("json")),
            "content": get_or(example, "response.body.content", json!("")),
        },
    });

    let mut out = Map::new();
    if let Some(name) = get(example, "name") {
        out.insert("name".into(), name.clone());
    }
    out.insert(
        "description".into(),
        get_or(example, "description", json!("")),
    );
    out.insert("request".into(), Value::Object(request));
    out.insert("response".into(), response);
    Value::Object(out)
}
