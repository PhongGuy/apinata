//! Example values generated from schemas, for request bodies without an example.
//!
//! The rules follow the reference converter's default values, plus `examples`, `const`, `allOf`,
//! `oneOf` and `anyOf`. Each sample stops at a nesting depth, a compact JSON size and a number of
//! schema steps per body and per document; each cut is noted once per body.

use super::Doc;
use super::bounds::{Bounds, approx_size};
use super::refs::RefKind;
use crate::import::{DocumentError, ImportNote, NoteKind};
use crate::js;
use serde_json::{Map, Value};

/// Combinator hops (`allOf`, `oneOf`, `anyOf`, `$ref`) do not deepen a sample, so this caps the
/// recursion they can build up before the sample is cut as too deep.
const MAX_NEST: usize = 128;

/// A sample for `schema`, with each bound it hits noted on `source`. Resolver notes stay on
/// `doc`. Fails only when `doc`'s entry or byte budget runs out.
pub(crate) fn sample<'a, 'v>(
    doc: &mut Doc<'a>,
    schema: &'v Value,
    source: &str,
    notes: &mut Vec<ImportNote>,
) -> Result<Value, DocumentError>
where
    'a: 'v,
{
    let mut sampler = Sampler {
        source,
        notes,
        size: 0,
        steps: 0,
        nest: 0,
        noted: [false; 5],
    };
    let value = sampler.node(doc, schema, 0)?;
    Ok(value.unwrap_or_else(|| Value::String(String::new())))
}

/// Compact JSON length <= bytes, stopping as soon as the sum passes `bytes`.
pub(crate) fn fits(value: &Value, bytes: usize) -> bool {
    json_len(value, bytes).is_some()
}

/// The note detail for a value cut at the size bound of `bytes`.
pub(crate) fn size_cut(bytes: usize) -> String {
    format!("the sample was cut at {} KiB", bytes >> 10)
}

/// The length of `js::stringify_pretty(value)`: the compact text plus each line break, its
/// indent and the space after each colon.
pub(crate) fn pretty_len(value: &Value) -> usize {
    let mut total = json_len(value, usize::MAX).unwrap_or(usize::MAX);
    let mut stack = vec![(value, 0usize)];
    while let Some((v, depth)) = stack.pop() {
        let children = match v {
            Value::Array(items) => {
                stack.extend(items.iter().map(|i| (i, depth + 1)));
                items.len()
            }
            Value::Object(map) => {
                stack.extend(map.values().map(|i| (i, depth + 1)));
                total = total.saturating_add(map.len());
                map.len()
            }
            _ => 0,
        };
        if children > 0 {
            let lines = children * (1 + 2 * (depth + 1)) + 1 + 2 * depth;
            total = total.saturating_add(lines);
        }
    }
    total
}

/// The bounds a sample can hit, each noted at most once per body.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cut {
    Depth,
    Nest,
    Size,
    Steps,
    DocumentSteps,
}

struct Sampler<'s> {
    source: &'s str,
    notes: &'s mut Vec<ImportNote>,
    /// Compact JSON length of everything the sample holds so far.
    size: usize,
    /// Schema steps this body has taken.
    steps: u64,
    /// Current recursion through `node`.
    nest: usize,
    noted: [bool; 5],
}

impl Sampler<'_> {
    /// Sample one schema at `depth`, or `None` when a bound leaves it out.
    fn node<'a, 'v>(
        &mut self,
        doc: &mut Doc<'a>,
        schema: &'v Value,
        depth: usize,
    ) -> Result<Option<Value>, DocumentError>
    where
        'a: 'v,
    {
        if self.nest >= MAX_NEST {
            self.note(doc, Cut::Nest)?;
            return Ok(None);
        }
        if !self.step(doc)? {
            return Ok(None);
        }
        let resolved = doc.resolve_followed(schema, RefKind::Schema, self.source)?;
        // A ref the resolver cut comes back as `{}` with no hops; it stands for an object.
        let is_ref = schema.get("$ref").is_some_and(Value::is_string);
        if is_ref && resolved.followed.is_empty() {
            return self.leaf(doc, Value::Object(Map::new()));
        }
        doc.enter(&resolved.followed);
        self.nest += 1;
        let value = self.shape(doc, &resolved.value, depth);
        self.nest -= 1;
        doc.leave(&resolved.followed);
        value
    }

    /// Sample a resolved schema: a fitting copied value first, else one generated from its shape.
    fn shape<'a, 'v>(
        &mut self,
        doc: &mut Doc<'a>,
        schema: &'v Value,
        depth: usize,
    ) -> Result<Option<Value>, DocumentError>
    where
        'a: 'v,
    {
        let first = |key: &str| schema.get(key).and_then(Value::as_array)?.first();
        let copied = [
            schema.get("example"),
            first("examples"),
            schema.get("const"),
            first("enum"),
        ];
        for candidate in copied.into_iter().flatten() {
            if let Some(value) = self.copy(doc, candidate)? {
                return Ok(Some(value));
            }
        }
        let Some(members) = schema.get("allOf").and_then(Value::as_array) else {
            return self.own(doc, schema, depth);
        };
        // Members merge into one sample, later keys winning; the running size is re-measured
        // afterwards, since overridden keys were counted twice.
        let start = self.size;
        let shaped = ["type", "properties", "items", "oneOf", "anyOf"];
        let mut merged = if shaped.iter().any(|k| schema.get(k).is_some()) {
            self.own(doc, schema, depth)?
        } else {
            None
        };
        for member in members {
            if let Some(value) = self.node(doc, member, depth)? {
                merged = Some(merge(merged, value));
            }
        }
        if let Some(value) = &merged {
            self.size = start + json_len(value, usize::MAX).unwrap_or_default();
        }
        Ok(merged)
    }

    /// Generate from the schema's own type, properties or first `oneOf`/`anyOf` branch.
    fn own<'a, 'v>(
        &mut self,
        doc: &mut Doc<'a>,
        schema: &'v Value,
        depth: usize,
    ) -> Result<Option<Value>, DocumentError>
    where
        'a: 'v,
    {
        let properties = schema.get("properties").and_then(Value::as_object);
        match type_of(schema) {
            Some("object") => self.object(doc, properties, depth),
            None if properties.is_some() => self.object(doc, properties, depth),
            Some("array") => self.array(doc, schema.get("items"), depth),
            Some("integer" | "number") => self.leaf(doc, Value::from(0)),
            Some("boolean") => self.leaf(doc, Value::Bool(false)),
            None => {
                let branch = ["oneOf", "anyOf"]
                    .iter()
                    .find_map(|k| schema.get(k).and_then(Value::as_array)?.first());
                match branch {
                    Some(branch) => self.node(doc, branch, depth),
                    None => self.leaf(doc, Value::String(String::new())),
                }
            }
            Some(_) => self.leaf(doc, Value::String(String::new())),
        }
    }

    /// Every property in order, until a bound leaves the rest out.
    fn object<'a, 'v>(
        &mut self,
        doc: &mut Doc<'a>,
        properties: Option<&'v Map<String, Value>>,
        depth: usize,
    ) -> Result<Option<Value>, DocumentError>
    where
        'a: 'v,
    {
        let Some(Value::Object(mut out)) = self.leaf(doc, Value::Object(Map::new()))? else {
            return Ok(None);
        };
        let Some(properties) = properties.filter(|p| !p.is_empty()) else {
            return Ok(Some(Value::Object(out)));
        };
        if depth >= doc.bounds.sample_depth {
            self.note(doc, Cut::Depth)?;
            return Ok(Some(Value::Object(out)));
        }
        for (key, schema) in properties {
            if self.spent(doc) {
                self.step(doc)?;
                break;
            }
            // The key, its colon and a comma are counted and charged before the value; the size is
            // given back when the value is left out.
            let mark = self.size;
            let key_len = string_len(key) + 1 + usize::from(!out.is_empty());
            if !self.reserve(doc, key_len)? {
                continue;
            }
            match self.node(doc, schema, depth + 1)? {
                Some(value) => {
                    out.insert(key.clone(), value);
                }
                None => self.size = mark,
            }
        }
        Ok(Some(Value::Object(out)))
    }

    /// One item from `items`; an item sampled as `""`, `0` or `false` leaves the array empty.
    fn array<'a, 'v>(
        &mut self,
        doc: &mut Doc<'a>,
        items: Option<&'v Value>,
        depth: usize,
    ) -> Result<Option<Value>, DocumentError>
    where
        'a: 'v,
    {
        if self.leaf(doc, Value::Array(Vec::new()))?.is_none() {
            return Ok(None);
        }
        if depth >= doc.bounds.sample_depth {
            self.note(doc, Cut::Depth)?;
            return Ok(Some(Value::Array(Vec::new())));
        }
        let mark = self.size;
        let item = self.node(doc, items.unwrap_or(&Value::Null), depth + 1)?;
        let empty = |v: &Value| match v {
            Value::String(s) => s.is_empty(),
            Value::Number(n) => n.as_f64() == Some(0.0),
            Value::Bool(b) => !b,
            _ => false,
        };
        match item {
            Some(item) if !empty(&item) => Ok(Some(Value::Array(vec![item]))),
            _ => {
                self.size = mark;
                Ok(Some(Value::Array(Vec::new())))
            }
        }
    }

    /// Take a step against the body's and the document's visit budgets, noting the first one
    /// that is spent.
    fn step(&mut self, doc: &mut Doc<'_>) -> Result<bool, DocumentError> {
        if doc.visits >= doc.bounds.document_visits {
            self.note(doc, Cut::DocumentSteps)?;
            return Ok(false);
        }
        if self.steps >= doc.bounds.sample_visits {
            self.note(doc, Cut::Steps)?;
            return Ok(false);
        }
        doc.visits += 1;
        self.steps += 1;
        Ok(true)
    }

    fn spent(&self, doc: &Doc<'_>) -> bool {
        doc.visits >= doc.bounds.document_visits || self.steps >= doc.bounds.sample_visits
    }

    /// Count `len` bytes against the size bound and the byte budget, or note the cut.
    fn reserve(&mut self, doc: &mut Doc<'_>, len: usize) -> Result<bool, DocumentError> {
        if self.size.saturating_add(len) > doc.bounds.sample_bytes {
            self.note(doc, Cut::Size)?;
            return Ok(false);
        }
        self.size += len;
        doc.budget.charge_bytes(len as u64)?;
        Ok(true)
    }

    /// A generated value, kept when its JSON fits.
    fn leaf(&mut self, doc: &mut Doc<'_>, value: Value) -> Result<Option<Value>, DocumentError> {
        let len = json_len(&value, usize::MAX).unwrap_or_default();
        Ok(self.reserve(doc, len)?.then_some(value))
    }

    /// A copy of a document value, made only when its JSON fits; its deep size is charged first.
    fn copy(&mut self, doc: &mut Doc<'_>, value: &Value) -> Result<Option<Value>, DocumentError> {
        let room = doc.bounds.sample_bytes.saturating_sub(self.size);
        let Some(len) = json_len(value, room) else {
            self.note(doc, Cut::Size)?;
            return Ok(None);
        };
        self.size += len;
        doc.budget.charge_bytes(approx_size(value))?;
        Ok(Some(value.clone()))
    }

    fn note(&mut self, doc: &mut Doc<'_>, cut: Cut) -> Result<(), DocumentError> {
        if std::mem::replace(&mut self.noted[cut as usize], true) {
            return Ok(());
        }
        let detail = cut_detail(&doc.bounds, cut);
        let note = ImportNote::new(self.source, NoteKind::Converted, detail);
        doc.budget.charge_note(&note)?;
        self.notes.push(note);
        Ok(())
    }
}

/// The `Converted` note detail for a body cut at `cut`.
pub(crate) fn cut_detail(b: &Bounds, cut: Cut) -> String {
    match cut {
        Cut::Depth => format!(
            "nesting deeper than {} levels was left out of the sample",
            b.sample_depth
        ),
        Cut::Nest => format!(
            "`allOf`, `oneOf`, `anyOf` and `$ref` nested more than {MAX_NEST} deep were left out of the sample"
        ),
        Cut::Size => size_cut(b.sample_bytes),
        Cut::Steps => format!(
            "the sample needs more than {} schema steps, so it was cut",
            b.sample_visits
        ),
        Cut::DocumentSteps => format!(
            "this document's samples used all {} schema steps, so this one was cut",
            b.document_visits
        ),
    }
}

/// The schema's type, or the first non-`null` entry of a 3.1 type array.
fn type_of(schema: &Value) -> Option<&str> {
    match schema.get("type")? {
        Value::String(t) => Some(t),
        Value::Array(types) => types
            .iter()
            .filter_map(Value::as_str)
            .find(|t| *t != "null"),
        _ => None,
    }
}

/// Objects merge with later keys winning; an empty string never replaces a value; otherwise the
/// later value wins.
fn merge(earlier: Option<Value>, later: Value) -> Value {
    match (earlier, later) {
        (Some(Value::Object(mut into)), Value::Object(from)) => {
            into.extend(from);
            Value::Object(into)
        }
        (Some(earlier), Value::String(s)) if s.is_empty() => earlier,
        (_, later) => later,
    }
}

/// Compact JSON length of `value`, or `None` as soon as it passes `cap`.
fn json_len(value: &Value, cap: usize) -> Option<usize> {
    let mut total = 0usize;
    let mut stack = vec![value];
    while let Some(v) = stack.pop() {
        // Containers add their brackets and separators, and one byte per child as a floor,
        // before their children are queued, so the queue never outgrows the cap.
        let len = match v {
            Value::Null | Value::Bool(true) => 4,
            Value::Bool(false) => 5,
            Value::Number(n) => n
                .as_f64()
                .map_or_else(|| n.to_string().len(), |f| js::number_to_string(f).len()),
            Value::String(s) if s.len() + 2 > cap - total => return None,
            Value::String(s) => string_len(s),
            Value::Array(items) => {
                let own = 2 + items.len().saturating_sub(1);
                if total.saturating_add(own + items.len()) > cap {
                    return None;
                }
                stack.extend(items);
                own
            }
            Value::Object(map) => {
                // Each entry is at least an empty key, a colon and a one-byte value.
                let own = 2 + map.len().saturating_sub(1);
                if total.saturating_add(own + 4 * map.len()) > cap {
                    return None;
                }
                let mut len = own;
                for (key, item) in map {
                    len += string_len(key) + 1;
                    stack.push(item);
                }
                len
            }
        };
        total = total.saturating_add(len);
        if total > cap {
            return None;
        }
    }
    Some(total)
}

/// Length of `text` as a JSON string literal, quotes and escapes included.
fn string_len(text: &str) -> usize {
    let escaped: usize = text
        .bytes()
        .map(|b| match b {
            b'"' | b'\\' | 0x08 | 0x0c | b'\n' | b'\r' | b'\t' => 2,
            0..0x20 => 6,
            _ => 1,
        })
        .sum();
    escaped + 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::openapi::bounds::Bounds;
    use crate::import::openapi::version::Version;
    use crate::import::{DocumentError, NoteKind};
    use serde_json::{Map, Value, json};
    use std::time::{Duration, Instant};

    fn with(root: &Value, bounds: Bounds, schema: &Value) -> (Value, Vec<ImportNote>) {
        let mut doc = Doc::new(root, Version::V31, bounds);
        let mut notes = Vec::new();
        let value = sample(&mut doc, schema, "body", &mut notes).expect("within budget");
        notes.extend(doc.take_ref_notes());
        (value, notes)
    }

    fn plain(schema: &Value) -> Value {
        with(&json!({}), Bounds::DEFAULT, schema).0
    }

    fn table(cases: &[(Value, Value)]) {
        for (schema, want) in cases {
            assert_eq!(&plain(schema), want, "{schema}");
        }
    }

    fn converted(notes: &[ImportNote], part: &str) -> usize {
        notes
            .iter()
            .filter(|n| n.kind == NoteKind::Converted && n.detail.contains(part))
            .count()
    }

    #[test]
    fn types_examples_and_enums_give_bruno_style_values() {
        let cases = [
            (json!({"type": "string"}), json!("")),
            (json!({"type": "integer"}), json!(0)),
            (json!({"type": "number"}), json!(0)),
            (json!({"type": "boolean"}), json!(false)),
            (json!({}), json!("")),
            (
                json!({"type": "string", "example": "ex", "enum": ["a"]}),
                json!("ex"),
            ),
            (json!({"type": "string", "enum": ["a", "b"]}), json!("a")),
            (json!({"type": ["null", "integer"]}), json!(0)),
            (
                json!({"type": "array", "items": {"type": "string"}}),
                json!([]),
            ),
            (
                json!({"type": "array", "items": {"type": "string", "enum": ["x"]}}),
                json!(["x"]),
            ),
            (
                json!({"type": "array", "items": {"type": "object", "properties": {"a": {"type": "integer"}}}}),
                json!([{"a": 0}]),
            ),
            (
                json!({"properties": {"b": {"type": "boolean"}, "s": {"type": "string", "example": "v"}}}),
                json!({"b": false, "s": "v"}),
            ),
        ];
        table(&cases);
    }

    #[test]
    fn const_and_3_1_examples_feed_samples() {
        table(&[
            (json!({"const": "c", "enum": ["a"]}), json!("c")),
            (json!({"examples": [7, 8], "const": 1}), json!(7)),
            (json!({"example": 1, "examples": [7]}), json!(1)),
        ]);
    }

    #[test]
    fn all_of_members_merge_in_samples() {
        let root = json!({"components": {"schemas": {"Base": {"type": "object", "properties": {
            "id": {"type": "integer"}, "kind": {"type": "string", "example": "base"}}}}}});
        let schema = json!({"allOf": [{"$ref": "#/components/schemas/Base"},
            {"type": "object", "properties": {"kind": {"type": "string", "example": "pet"}, "name": {"type": "string"}}}]});
        assert_eq!(
            with(&root, Bounds::DEFAULT, &schema).0,
            json!({"id": 0, "kind": "pet", "name": ""})
        );
    }

    #[test]
    fn one_of_and_any_of_take_their_first_branch() {
        let any = json!({"anyOf": [{"properties": {"x": {"type": "boolean"}}}]});
        table(&[
            (
                json!({"oneOf": [{"type": "integer"}, {"type": "string"}]}),
                json!(0),
            ),
            (any, json!({"x": false})),
        ]);
    }

    #[test]
    fn recursive_schemas_stop_at_the_cycle_with_a_note() {
        let root = json!({"components": {"schemas": {"Node": {"type": "object", "properties": {
            "name": {"type": "string"}, "next": {"$ref": "#/components/schemas/Node"}}}}}});
        let (value, notes) = with(
            &root,
            Bounds::DEFAULT,
            &json!({"$ref": "#/components/schemas/Node"}),
        );
        assert_eq!(value, json!({"name": "", "next": {}}));
        assert_eq!(converted(&notes, ""), 1, "{notes:?}");
    }

    #[test]
    fn deep_and_large_samples_are_cut_with_a_note() {
        let mut deep = json!({"type": "string"});
        for _ in 0..20 {
            deep = json!({"type": "object", "properties": {"n": deep}});
        }
        let (value, notes) = with(&json!({}), Bounds::DEFAULT, &deep);
        let (mut depth, mut cur) = (0, &value);
        while let Some(next) = cur.get("n") {
            depth += 1;
            cur = next;
        }
        assert_eq!(depth, 16);
        assert_eq!(converted(&notes, "16"), 1, "{notes:?}");

        let mut props = Map::new();
        for i in 0..100 {
            props.insert(
                format!("p{i:03}"),
                json!({"type": "string", "example": "x".repeat(1024)}),
            );
        }
        let (value, notes) = with(
            &json!({}),
            Bounds::DEFAULT,
            &json!({"type": "object", "properties": props}),
        );
        let len = crate::js::stringify(&value).len();
        assert!(len <= 64 << 10 && len > 60 << 10, "{len}");
        assert_eq!(converted(&notes, "64 KiB"), 1, "{notes:?}");
    }

    #[test]
    fn an_example_over_the_size_bound_is_replaced_by_a_generated_sample() {
        let big = json!({"type": "string", "example": "y".repeat(70 << 10)});
        let (value, notes) = with(&json!({}), Bounds::DEFAULT, &big);
        assert_eq!(value, json!(""));
        assert_eq!(converted(&notes, "64 KiB"), 1, "{notes:?}");
    }

    #[test]
    fn all_of_fan_out_stops_at_the_visit_budget() {
        let mut schemas = Map::new();
        for i in 0..12 {
            let member = json!({"$ref": format!("#/components/schemas/s{}", i + 1)});
            schemas.insert(format!("s{i}"), json!({"allOf": vec![member; 10]}));
        }
        schemas.insert(
            "s12".into(),
            json!({"type": "object", "properties": {"x": {"type": "integer"}}}),
        );
        let root = json!({"components": {"schemas": schemas}});
        let started = Instant::now();
        let (value, notes) = with(
            &root,
            Bounds::DEFAULT,
            &json!({"$ref": "#/components/schemas/s0"}),
        );
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{:?}",
            started.elapsed()
        );
        assert!(value.is_object());
        assert_eq!(converted(&notes, "100000"), 1, "{notes:?}");
    }

    #[test]
    fn the_document_visit_budget_spans_bodies() {
        let mut bounds = Bounds::DEFAULT;
        bounds.document_visits = 50;
        let root = json!({});
        let mut doc = Doc::new(&root, Version::V31, bounds);
        let props: Map<String, Value> = (0..30)
            .map(|i| (format!("p{i}"), json!({"type": "integer"})))
            .collect();
        let schema = json!({"type": "object", "properties": props});
        let mut notes = Vec::new();
        let first = sample(&mut doc, &schema, "first", &mut notes).expect("fits");
        assert_eq!(first.as_object().map(Map::len), Some(30));
        let second = sample(&mut doc, &schema, "second", &mut notes).expect("fits");
        assert!(second.as_object().is_some_and(|o| o.len() < 30), "{second}");
        assert!(
            notes
                .iter()
                .any(|n| n.source == "second" && n.kind == NoteKind::Converted),
            "{notes:?}"
        );
    }

    #[test]
    fn samples_charge_the_byte_budget() {
        let bounds = Bounds {
            bytes: 1 << 10,
            ..Bounds::DEFAULT
        };
        let root = json!({});
        let mut doc = Doc::new(&root, Version::V31, bounds);
        let schema = json!({"type": "string", "example": "x".repeat(2 << 10)});
        assert_eq!(
            sample(&mut doc, &schema, "body", &mut Vec::new()).unwrap_err(),
            DocumentError::TooLarge {
                limit: 1 << 10,
                unit: "imported bytes"
            }
        );
    }

    #[test]
    fn fits_holds_exactly_at_the_room_and_fails_one_byte_over() {
        let room = 64 << 10;
        // A string's compact length is its text plus two quotes.
        assert!(fits(&json!("x".repeat(room - 2)), room));
        assert!(!fits(&json!("x".repeat(room - 1)), room));
        let list = json!(["x".repeat(room - 4)]);
        assert!(fits(&list, room));
        assert!(!fits(&list, room - 1));
    }

    #[test]
    fn pretty_len_is_the_pretty_text_length() {
        for value in [
            json!({"a": 1, "b": [true, {"c": "x\n"}], "d": {}, "e": []}),
            json!([[1, 2], [[]]]),
            json!("plain"),
        ] {
            assert_eq!(
                pretty_len(&value),
                crate::js::stringify_pretty(&value).len(),
                "{value}"
            );
        }
    }

    #[test]
    fn combinator_nesting_is_cut_with_its_own_note() {
        let mut nested = json!({"type": "integer"});
        for _ in 0..200 {
            nested = json!({"allOf": [nested]});
        }
        let (_, notes) = with(&json!({}), Bounds::DEFAULT, &nested);
        assert_eq!(converted(&notes, "nested more than 128"), 1, "{notes:?}");
        assert_eq!(converted(&notes, "16 levels"), 0, "{notes:?}");
    }
}
