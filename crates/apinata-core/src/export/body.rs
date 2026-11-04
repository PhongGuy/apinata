//! Request bodies as Postman body objects.

use serde_json::{Map, Value, json};

use crate::edit::{BodySpec, KeyValue, MultipartField, MultipartValue};

/// The Postman body for a request body, `None` when there is none. An unsupported mode
/// becomes an empty raw body, which the caller notes.
pub(crate) fn body(spec: &BodySpec) -> Option<Value> {
    Some(match spec {
        BodySpec::None => return None,
        BodySpec::Json(text) => raw(text, "json"),
        BodySpec::Xml(text) => raw(text, "xml"),
        // Postman has no SPARQL language; its plain text keeps the query.
        BodySpec::Text(text) | BodySpec::Sparql(text) => raw(text, "text"),
        BodySpec::FormUrlEncoded(fields) => {
            let fields: Vec<Value> = fields.iter().map(urlencoded).collect();
            json!({"mode": "urlencoded", "urlencoded": fields})
        }
        BodySpec::Multipart(fields) => {
            let fields: Vec<Value> = fields.iter().map(formdata).collect();
            json!({"mode": "formdata", "formdata": fields})
        }
        BodySpec::File(files) => {
            let src = files.iter().find(|f| f.selected).map_or("", |f| &f.path);
            json!({"mode": "file", "file": {"src": src}})
        }
        BodySpec::Graphql { query, variables } => {
            json!({"mode": "graphql", "graphql": {"query": query, "variables": variables}})
        }
        BodySpec::Unsupported { .. } => {
            json!({"mode": "raw", "raw": ""})
        }
    })
}

fn raw(text: &str, language: &str) -> Value {
    json!({"mode": "raw", "raw": text, "options": {"raw": {"language": language}}})
}

fn urlencoded(kv: &KeyValue) -> Value {
    json!({
        "key": kv.name,
        "value": kv.value,
        "disabled": !kv.enabled,
        "type": "default",
        "description": kv.description.clone().unwrap_or_default(),
    })
}

/// A form-data entry; a file field's `src` is null, one path, or a list as Postman allows.
fn formdata(field: &MultipartField) -> Value {
    let mut out = Map::new();
    out.insert("key".into(), field.name.as_str().into());
    out.insert("disabled".into(), (!field.enabled).into());
    let kind = match field.value {
        MultipartValue::Text(_) => "text",
        _ => "file",
    };
    out.insert("type".into(), kind.into());
    let description = field.description.clone().unwrap_or_default();
    out.insert("description".into(), description.into());
    match &field.value {
        MultipartValue::Text(text) => {
            out.insert("value".into(), text.as_str().into());
        }
        MultipartValue::Files(paths) => {
            let src = match paths.as_slice() {
                [] => Value::Null,
                [one] => one.as_str().into(),
                many => many.into(),
            };
            out.insert("src".into(), src);
        }
    }
    if !field.content_type.is_empty() {
        out.insert("contentType".into(), field.content_type.as_str().into());
    }
    Value::Object(out)
}
