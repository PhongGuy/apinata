//! Schemas inferred from JSON bodies, hoisted into `components/schemas` under names taken from
//! the request, so each use is a `$ref`.

use serde_json::{Map, Value, json};

/// The hoisted schemas in first-use order; a schema is never shared between uses.
#[derive(Default)]
pub(super) struct Schemas(Map<String, Value>);

impl Schemas {
    /// Store `schema` under a free name made from `base` and return the `$ref` to it.
    pub(super) fn hoist(&mut self, base: &str, schema: Value) -> Value {
        let name = self.free_name(base);
        let reference = json!({"$ref": format!("#/components/schemas/{name}")});
        self.0.insert(name, schema);
        reference
    }

    /// The `components.schemas` object, or `None` when nothing was hoisted.
    pub(super) fn into_value(self) -> Option<Value> {
        (!self.0.is_empty()).then(|| self.0.into())
    }

    /// `base` sanitised, or with the first free `_1`, `_2`, ... suffix when taken.
    fn free_name(&self, base: &str) -> String {
        let base = component_name(base);
        let mut name = base.clone();
        let mut n = 1;
        while self.0.contains_key(&name) {
            name = format!("{base}_{n}");
            n += 1;
        }
        name
    }
}

/// `text` lower-cased with every run of characters outside `[a-zA-Z0-9._-]` as one `_`,
/// trimmed of `_`; `request` when nothing is left.
pub(super) fn component_name(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        "request".into()
    } else {
        trimmed.into()
    }
}

/// The JSON Schema of a parsed JSON value.
pub(super) fn infer_schema(value: &Value) -> Value {
    match value {
        Value::Null => json!({"type": "null"}),
        Value::Bool(_) => json!({"type": "boolean"}),
        Value::String(_) => json!({"type": "string"}),
        Value::Number(n) => {
            let integral = n.is_i64() || n.is_u64() || n.as_f64().is_some_and(|f| f.fract() == 0.0);
            json!({"type": if integral { "integer" } else { "number" }})
        }
        Value::Array(items) => {
            let items = items.first().map_or_else(|| json!({}), infer_schema);
            json!({"type": "array", "items": items})
        }
        Value::Object(fields) => {
            let properties: Map<String, Value> = fields
                .iter()
                .map(|(k, v)| (k.clone(), infer_schema(v)))
                .collect();
            json!({"type": "object", "properties": properties})
        }
    }
}
