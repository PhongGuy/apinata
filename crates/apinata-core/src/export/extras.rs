//! Postman events from scripts, collection variables, and responses from saved examples.

use serde_json::{Map, Value, json};

use super::ExportNote;
use super::body::body;
use super::postman::{header, url};
use crate::config_file::CollectionFile;
use crate::edit::VarValue;
use crate::example::Example;
use crate::js::script_code;
use crate::script::ScriptPhase;

/// The Postman events for one item's scripts, given a reader of its script per phase.
/// Tests join the post-response script in one `test` event, as Postman has no tests phase.
pub(super) fn events(script: impl Fn(ScriptPhase) -> String) -> Vec<Value> {
    let lines = |phase| -> Vec<String> {
        let code = script(phase);
        let code = script_code(&code);
        if code.is_empty() {
            Vec::new()
        } else {
            code.split('\n').map(str::to_owned).collect()
        }
    };
    let mut out = Vec::new();
    let pre = lines(ScriptPhase::PreRequest);
    if !pre.is_empty() {
        out.push(event("prerequest", pre));
    }
    let mut test = lines(ScriptPhase::PostResponse);
    let tests = lines(ScriptPhase::Tests);
    if !tests.is_empty() {
        if !test.is_empty() {
            test.push(String::new());
        }
        test.push("// Tests".to_owned());
        test.extend(tests);
    }
    if !test.is_empty() {
        out.push(event("test", test));
    }
    out
}

fn event(listen: &str, exec: Vec<String>) -> Value {
    json!({
        "listen": listen,
        "script": {"type": "text/javascript", "packages": {}, "requests": {}, "exec": exec},
    })
}

/// Declared collection variables, then post-response variables under names not yet taken,
/// each with its text value; the first of a name wins.
pub(super) fn collection_variables(
    file: &CollectionFile,
    notes: &mut Vec<ExportNote>,
) -> Vec<Value> {
    let mut seen = Vec::new();
    let mut out = Vec::new();
    let mut push = |key: &str, value: &str, enabled: bool| {
        if seen.iter().any(|s: &String| s == key) {
            return;
        }
        seen.push(key.to_owned());
        let mut entry = Map::new();
        entry.insert("key".into(), key.into());
        entry.insert("value".into(), value.into());
        entry.insert("type".into(), "default".into());
        if !enabled {
            entry.insert("disabled".into(), true.into());
        }
        out.push(Value::Object(entry));
    };
    for var in file.vars() {
        let value = match &var.value {
            VarValue::Text(text) => text,
            VarValue::Typed { kind, data } => {
                let detail = format!("the {kind} variable is written as text");
                let source = format!("collection / variables / {}", var.name);
                notes.push(ExportNote::new(source, detail));
                data
            }
        };
        push(&var.name, value, var.enabled);
    }
    for var in file.post_response_vars() {
        push(var.name(), var.expression(), var.enabled());
    }
    out
}

/// One Postman response per saved example.
pub(super) fn responses(examples: &[Example]) -> Vec<Value> {
    examples.iter().map(response).collect()
}

fn response(example: &Example) -> Value {
    let original = match &example.request {
        Some(req) => {
            let mut out = Map::new();
            out.insert("method".into(), req.method.as_str().into());
            let headers: Vec<Value> = req.headers.iter().map(header).collect();
            out.insert("header".into(), headers.into());
            out.insert("url".into(), url(&req.url, &req.params));
            if let Some(body) = body(&req.body) {
                out.insert("body".into(), body);
            }
            Value::Object(out)
        }
        None => json!({
            "method": "GET",
            "header": [],
            "url": {"raw": "", "protocol": "https", "host": [], "path": []},
        }),
    };
    let res = example.response.as_ref();
    let headers = res.map_or(&[][..], |r| &r.headers);
    let header: Vec<Value> = headers
        .iter()
        .map(|(k, v)| json!({"key": k, "value": v, "name": k, "description": "", "type": "text"}))
        .collect();
    let mut out = Map::new();
    out.insert("name".into(), example.name.as_str().into());
    out.insert("originalRequest".into(), original);
    let status = res.and_then(|r| r.status_text.as_deref()).unwrap_or("OK");
    out.insert("status".into(), status.into());
    let code = res
        .and_then(|r| r.status)
        .filter(|c| *c != 0)
        .unwrap_or(200);
    out.insert("code".into(), code.into());
    out.insert("header".into(), header.into());
    out.insert("cookie".into(), json!([]));
    let data = res.and_then(|r| r.body.as_ref()).map_or("", |b| &b.data);
    out.insert("body".into(), data.into());
    let content_type = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.as_str());
    if let Some(language) = content_type.and_then(preview_language) {
        out.insert("_postman_previewlanguage".into(), language.into());
    }
    Value::Object(out)
}

/// Postman's preview language for a content type, matched by substring as the reference
/// converter does.
fn preview_language(content_type: &str) -> Option<&'static str> {
    if content_type.contains("application/json") {
        Some("json")
    } else if content_type.contains("application/xml") || content_type.contains("text/xml") {
        Some("xml")
    } else if content_type.contains("text/html") {
        Some("html")
    } else if content_type.contains("text/plain") {
        Some("text")
    } else {
        None
    }
}
