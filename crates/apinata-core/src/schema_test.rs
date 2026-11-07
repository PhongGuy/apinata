//! Infers a JSON Schema from a sample response body and wraps it in a ready-made tests block.

use serde_json::{Map, Value};

const TEST_NAME: &str = "response matches schema";
/// Levels beyond this accept anything, so a hostile body cannot recurse without bound.
const MAX_DEPTH: usize = 32;
/// Arrays contribute their first items only, which bounds the work on huge lists.
const MAX_ITEMS: usize = 1000;
/// In a JS object literal this key sets the prototype instead of a property, so it is never described.
const PROTO: &str = "__proto__";

/// What the sample shows at one position: the JSON types seen plus object and array detail.
#[derive(Default)]
struct Shape {
    any: bool,
    types: Vec<&'static str>,
    object: Option<ObjectShape>,
    items: Option<Box<Shape>>,
}

struct ObjectShape {
    props: Vec<(String, Shape)>,
    required: Vec<String>,
}

impl Shape {
    fn of(value: &Value, depth: usize) -> Shape {
        if depth >= MAX_DEPTH {
            return Shape {
                any: true,
                ..Shape::default()
            };
        }
        let mut shape = Shape::default();
        match value {
            Value::Null => shape.types.push("null"),
            Value::Bool(_) => shape.types.push("boolean"),
            Value::String(_) => shape.types.push("string"),
            Value::Number(n) => {
                // Lexical form decides: 3 is an integer, 3.0 and 3e0 are not.
                let integral = n.is_i64() || n.is_u64();
                shape
                    .types
                    .push(if integral { "integer" } else { "number" });
            }
            Value::Array(list) => {
                shape.types.push("array");
                shape.items = list
                    .iter()
                    .take(MAX_ITEMS)
                    .map(|v| Shape::of(v, depth + 1))
                    .reduce(|a, b| a.merge(b))
                    .map(Box::new);
            }
            Value::Object(map) => {
                shape.types.push("object");
                shape.object = Some(ObjectShape {
                    props: map
                        .iter()
                        .filter(|(k, _)| *k != PROTO)
                        .map(|(k, v)| (k.clone(), Shape::of(v, depth + 1)))
                        .collect(),
                    required: map.keys().filter(|k| *k != PROTO).cloned().collect(),
                });
            }
        }
        shape
    }

    /// Combine two samples of the same position: union of types and properties,
    /// intersection of required keys.
    fn merge(mut self, other: Shape) -> Shape {
        self.any |= other.any;
        for t in other.types {
            if !self.types.contains(&t) {
                self.types.push(t);
            }
        }
        // A number that is sometimes fractional is simply a number.
        if self.types.contains(&"number") {
            self.types.retain(|t| *t != "integer");
        }
        self.object = match (self.object.take(), other.object) {
            (Some(a), Some(b)) => Some(a.merge(b)),
            (a, b) => a.or(b),
        };
        self.items = match (self.items.take(), other.items) {
            (Some(a), Some(b)) => Some(Box::new(a.merge(*b))),
            (a, b) => a.or(b),
        };
        self
    }

    fn to_schema(&self) -> Value {
        let mut out = Map::new();
        if self.any {
            return Value::Object(out);
        }
        match self.types.as_slice() {
            [] => {}
            [one] => {
                out.insert("type".into(), (*one).into());
            }
            many => {
                out.insert(
                    "type".into(),
                    many.iter().map(|t| Value::from(*t)).collect(),
                );
            }
        }
        if let Some(object) = &self.object {
            let props = object
                .props
                .iter()
                .map(|(k, v)| (k.clone(), v.to_schema()))
                .collect();
            out.insert("properties".into(), Value::Object(props));
            if !object.required.is_empty() {
                out.insert(
                    "required".into(),
                    object.required.iter().cloned().map(Value::from).collect(),
                );
            }
        }
        if let Some(items) = &self.items {
            out.insert("items".into(), items.to_schema());
        }
        Value::Object(out)
    }
}

impl ObjectShape {
    fn merge(mut self, other: ObjectShape) -> ObjectShape {
        self.required.retain(|k| other.required.contains(k));
        for (key, shape) in other.props {
            match self.props.iter().position(|(k, _)| *k == key) {
                Some(i) => {
                    let mine = std::mem::take(&mut self.props[i].1);
                    self.props[i].1 = mine.merge(shape);
                }
                None => self.props.push((key, shape)),
            }
        }
        self
    }
}

/// The full new tests script: `existing` followed by a test that validates the
/// response against a schema inferred from `body`, or `None` when `body` is not JSON.
pub fn schema_test_script(existing: &str, body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    let schema = serde_json::to_string_pretty(&Shape::of(&value, 0).to_schema()).ok()?;
    // serde_json emits these raw, and JS source treats them as line terminators inside strings.
    let schema = schema
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    // The schema sits inside the function body, two spaces deeper than its own indent.
    let schema = schema.replace('\n', "\n  ");

    let name = free_name(existing);
    let mut out = String::from(existing);
    if !out.is_empty() {
        let trailing = out.len() - out.trim_end_matches('\n').len();
        out.push_str(&"\n".repeat(2usize.saturating_sub(trailing)));
    }
    out.push_str(&format!(
        "test(\"{name}\", function () {{\n  \
         const Ajv = require('ajv');\n  \
         const ajv = new Ajv({{ allErrors: true }});\n  \
         const validate = ajv.compile({schema});\n  \
         expect(validate(res.getBody()), JSON.stringify(validate.errors)).to.be.true;\n}});\n"
    ));
    Some(out)
}

/// The test name, suffixed ` (2)`, ` (3)` while the script already declares it.
// The candidate sequence never ends, so `find` returns only once a name is free.
#[allow(clippy::expect_used)]
fn free_name(existing: &str) -> String {
    let taken = |name: &str| existing.contains(&format!("\"{name}\""));
    if !taken(TEST_NAME) {
        return TEST_NAME.to_owned();
    }
    (2..)
        .map(|n| format!("{TEST_NAME} ({n})"))
        .find(|name| !taken(name))
        .expect("an unbounded range has a free name")
}
