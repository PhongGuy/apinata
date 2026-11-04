//! The `req` and `res` objects: host dispatch functions over JSON strings, wrapped by `req_res.js`.

use std::sync::{Arc, PoisonError};

use rquickjs::Function;
use rquickjs::context::EvalOptions;
use serde_json::{Map, Value, json};

use super::ExecutionMode;
use super::engine::Installer;
use super::host::HostState;
use crate::resolve::PreparedRequest;
use crate::response::Response;

const REQ_RES_JS: &str = include_str!("req_res.js");
const QUERY_JS: &str = include_str!("vendor/bruno-query-0.1.0.js");

/// Installs `globalThis.req`, and `globalThis.res` when `with_res`, for one phase over `state`.
pub(crate) fn installer(state: Arc<HostState>, with_res: bool) -> Installer {
    Box::new(move |ctx| {
        let req_state = Arc::clone(&state);
        let req_host = move |op: String, args: String| req_dispatch(&req_state, &op, &args);
        ctx.globals()
            .set("__req", Function::new(ctx.clone(), req_host)?)?;
        if with_res {
            let res_host = move |op: String, args: String| res_dispatch(&state, &op, &args);
            ctx.globals()
                .set("__res", Function::new(ctx.clone(), res_host)?)?;
        }
        if with_res {
            // The query function is evaluated under its own file name for each phase, and
            // `req_res.js` takes it off the global object.
            let mut options = EvalOptions::default();
            options.strict = false;
            options.filename = Some("bruno-query-0.1.0.js".to_owned());
            let query: Function = ctx.eval_with_options(QUERY_JS, options)?;
            ctx.globals().set("__query", query)?;
        }
        ctx.eval::<(), _>(REQ_RES_JS)
    })
}

/// Runs request operation `op` with the JSON array `args` against the prepared request. The
/// answer is `{"v": value}`, `{}` for `undefined`, or `{"u": member}` for an unsupported call.
fn req_dispatch(state: &HostState, op: &str, args: &str) -> String {
    // Arguments that fail to parse are reported to the script, not treated as no arguments.
    let args: Vec<Value> = match serde_json::from_str(args) {
        Ok(args) => args,
        Err(e) => return json!({ "e": format!("invalid arguments for {op}: {e}") }).to_string(),
    };
    let text = |i: usize| args.get(i).and_then(Value::as_str).unwrap_or_default();
    let mut req = state
        .prepared
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let answer = match op {
        "getUrl" => some(req.url().into()),
        "setUrl" => {
            req.set_url(text(0));
            json!({})
        }
        "parts" => some(url_parts(&req)),
        "getMethod" => some(req.method().into()),
        "setMethod" => {
            req.set_method(text(0));
            json!({})
        }
        "getName" => some(req.name().into()),
        "getAuthMode" => some(req.auth_mode().into()),
        "getPathParams" => some(path_params(&req)),
        "getTags" => some(req.tags().into()),
        "getExecutionMode" => some(mode_name(state.mode).into()),
        "getHeaders" => some(headers_object(req.headers())),
        "getHeader" => req
            .header(text(0))
            .map_or_else(|| json!({}), |v| some(v.into())),
        "setHeader" => {
            req.set_header(text(0), text(1));
            json!({})
        }
        "deleteHeader" => {
            req.remove_header(text(0));
            json!({})
        }
        "setHeaders" => {
            let headers = match args.first() {
                Some(Value::Object(map)) => map
                    .iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned()))
                    .collect(),
                _ => Vec::new(),
            };
            req.set_headers(headers);
            json!({})
        }
        "getBody" => match req.body_text() {
            Some(body) => some(json!({ "text": body, "json": req.is_json_body() })),
            None => json!({ "u": "req.getBody" }),
        },
        "setBody" => {
            if req.set_body_text(text(0)) {
                json!({})
            } else {
                json!({ "u": "req.setBody" })
            }
        }
        "getTimeout" => some(req.timeout().map_or(0, |t| t.as_millis() as u64).into()),
        "setTimeout" => {
            // Zero, NaN, negatives and Infinity (null here) mean no limit, as in the file's
            // settings; a positive one is capped like a script send's.
            let ms = args.first().and_then(Value::as_f64).unwrap_or(0.0);
            let timeout = (ms > 0.0).then(|| super::timers::wait(ms));
            req.set_timeout(timeout);
            json!({})
        }
        "setMaxRedirects" => {
            let max = args.first().and_then(Value::as_f64).unwrap_or(0.0);
            req.set_max_redirects(max.max(0.0) as u32);
            json!({})
        }
        _ => json!({ "u": format!("req.{op}") }),
    };
    answer.to_string()
}

/// Runs response operation `op`: `data` answers everything the getters read, and `setBody`
/// records the body later phases and the assertions see.
fn res_dispatch(state: &HostState, op: &str, args: &str) -> String {
    let Some(response) = state.response.get() else {
        return json!({}).to_string();
    };
    let mut body = state
        .response_body
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    match op {
        "setBody" => {
            let args: Vec<Value> = match serde_json::from_str(args) {
                Ok(args) => args,
                Err(e) => {
                    return json!({ "e": format!("invalid arguments for {op}: {e}") }).to_string();
                }
            };
            *body = Some(args.into_iter().next().unwrap_or(Value::Null));
            json!({})
        }
        _ => some(response_data(response, body.as_ref())),
    }
    .to_string()
}

/// What `res` shows: the response's fields with any script-set body in place of its own. A
/// truncated body is left out, so reading it throws.
fn response_data(response: &Response, body: Option<&Value>) -> Value {
    let headers = response.headers_value();
    let (body, body_size) = match body {
        Some(value) => (Some(value.clone()), body_text(value).len() as u64),
        None if response.truncated() => (None, response.size()),
        None => (Some(response.body_value()), response.size()),
    };
    // The header size counts the status line and each header line as HTTP/1.1 writes them.
    let mut lines = vec![format!(
        "HTTP/1.1 {} {}",
        response.status(),
        response.status_text()
    )];
    for (name, value) in &headers {
        match value {
            Value::Array(values) => {
                lines.extend(values.iter().map(|v| format!("{name}: {}", plain(v))));
            }
            other => lines.push(format!("{name}: {}", plain(other))),
        }
    }
    lines.extend([String::new(), String::new()]);
    let header_size = lines.join("\r\n").len() as u64;
    let mut data = json!({
        "status": response.status(),
        "statusText": response.status_text(),
        "headers": headers,
        "responseTime": response.response_time().as_millis() as u64,
        "url": response.url(),
        "size": { "header": header_size, "body": body_size, "total": header_size + body_size },
    });
    if let Some(body) = body {
        data["body"] = body;
    }
    data
}

/// The text a script-set body stands for: a string as is, nothing for null, else its JSON.
pub(crate) fn body_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn plain(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned)
}

/// Host, path and query string of the URL template, as the reference's `parseUrl` splits them,
/// with `:name` path segments replaced by their path params' raw values.
fn url_parts(req: &PreparedRequest) -> Value {
    let raw = req.url();
    let (host, path, query) = split_url(raw);
    let params = req.path_params();
    let path: Vec<String> = path
        .split('/')
        .map(|segment| {
            segment
                .strip_prefix(':')
                .and_then(|name| params.iter().find(|(n, _)| n == name))
                .filter(|(_, value)| !value.trim().is_empty())
                .map_or_else(|| segment.to_owned(), |(_, value)| value.clone())
        })
        .collect();
    json!({ "host": host, "path": path.join("/"), "query": query })
}

/// A WHATWG parse when the URL has no placeholders and a host, else a split that keeps the
/// text as written; an empty URL has no parts.
fn split_url(raw: &str) -> (String, String, String) {
    if raw.is_empty() {
        return Default::default();
    }
    if !raw.contains("{{")
        && let Ok(url) = url::Url::parse(raw)
        && let Some(host) = url.host_str()
    {
        let host = match url.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_owned(),
        };
        let query = url.query().unwrap_or_default().to_owned();
        return (host, url.path().to_owned(), query);
    }
    let rest = match raw.find("://") {
        Some(i) if !raw[..i].contains(['/', '?', '#']) => &raw[i + 3..],
        _ => raw,
    };
    let rest = rest.split('#').next().unwrap_or_default();
    let split = rest.find(['/', '?']).unwrap_or(rest.len());
    let (with_credentials, rest) = rest.split_at(split);
    let host = with_credentials.rsplit('@').next().unwrap_or_default();
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    (host.to_owned(), path.to_owned(), query.to_owned())
}

fn path_params(req: &PreparedRequest) -> Value {
    let params: Vec<Value> = req
        .path_params()
        .into_iter()
        .map(|(name, value)| json!({ "name": name, "value": value, "type": "path" }))
        .collect();
    params.into()
}

/// The headers as an object; a name repeated in the same spelling keeps its last value.
fn headers_object(headers: &[(String, String)]) -> Value {
    let map: Map<String, Value> = headers
        .iter()
        .map(|(name, value)| (name.clone(), Value::String(value.clone())))
        .collect();
    Value::Object(map)
}

fn mode_name(mode: ExecutionMode) -> &'static str {
    match mode {
        ExecutionMode::Standalone => "standalone",
        ExecutionMode::Runner => "runner",
        ExecutionMode::Cli => "cli",
    }
}

fn some(value: Value) -> Value {
    json!({ "v": value })
}
