//! XML request bodies, built from a schema's shape and an example object.

use super::Doc;
use super::params::text;
use super::refs::{RefKind, Resolved};
use super::sample::{Cut, cut_detail};
use crate::import::{DocumentError, ImportNote, NoteKind};
use serde_json::{Map, Value};
use std::borrow::Cow;

const DECLARATION: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n";

/// Bruno's buildXmlBody: a string example verbatim; else a declaration and a root element
/// (`xml.name`, else `root`) with `xml.attribute` properties as attributes and the other
/// properties as elements, filled from the example object, empty otherwise. The text stops at
/// the sample bounds (depth, size, schema steps), each cut noted once on `source`.
pub(crate) fn render<'a, 'v>(
    doc: &mut Doc<'a>,
    schema: &'v Value,
    example: Option<&Value>,
    source: &str,
    notes: &mut Vec<ImportNote>,
) -> Result<String, DocumentError>
where
    'a: 'v,
{
    if let Some(Value::String(raw)) = example {
        doc.budget.charge_text(raw)?;
        return Ok(raw.clone());
    }
    let values = example.and_then(Value::as_object);
    let properties = schema.get("properties").and_then(Value::as_object);
    if properties.is_none() && values.is_none() {
        return Ok(String::new());
    }
    let root = xml_name(schema).unwrap_or("root");
    let mut w = Writer {
        out: String::new(),
        reserved: 0,
        steps: 0,
        source,
        notes,
        noted: Vec::new(),
    };
    w.push(doc, DECLARATION)?;
    w.push(doc, "<")?;
    w.push(doc, root)?;
    // The closing tag is held back from the size bound, so children cannot crowd it out.
    w.reserved = root.len() + 4;
    if let Some(properties) = properties {
        w.attributes(doc, properties, values)?;
    }
    w.push(doc, ">")?;
    let any = match properties {
        Some(properties) => w.schema_children(doc, properties, values, 1)?,
        None => w.value_children(doc, values.into_iter().flatten(), None, 1)?,
    };
    if any {
        w.push(doc, "\n")?;
    }
    w.push(doc, "</")?;
    w.push(doc, root)?;
    w.push(doc, ">")?;
    Ok(w.out)
}

/// The XML text being written, with the room the open elements' closing tags need.
struct Writer<'s> {
    out: String,
    /// Bytes the closing tags of the open elements will take.
    reserved: usize,
    /// Schema steps this body has taken.
    steps: u64,
    source: &'s str,
    notes: &'s mut Vec<ImportNote>,
    noted: Vec<Cut>,
}

impl Writer<'_> {
    /// `name="value"` for each `xml.attribute` property, valued from the example.
    fn attributes<'a, 'v>(
        &mut self,
        doc: &mut Doc<'a>,
        properties: &'v Map<String, Value>,
        values: Option<&Map<String, Value>>,
    ) -> Result<(), DocumentError>
    where
        'a: 'v,
    {
        for (name, raw) in properties {
            let Some(prop) = self.resolve(doc, raw)? else {
                continue;
            };
            if !is_attribute(&prop.value) {
                continue;
            }
            let attr = xml_name(&prop.value).unwrap_or(name);
            let value = values.and_then(|v| v.get(name.as_str()));
            let value = match value {
                None | Some(Value::Null) => Cow::Borrowed(""),
                Some(Value::String(s)) => Cow::Borrowed(s.as_str()),
                Some(other) => Cow::Owned(text(doc, other)?),
            };
            if !self.fits(doc, 1 + attr.len() + 3 + escaped_len(&value))? {
                continue;
            }
            self.push(doc, " ")?;
            self.push(doc, attr)?;
            self.push(doc, "=\"")?;
            self.escaped(doc, &value)?;
            self.push(doc, "\"")?;
        }
        Ok(())
    }

    /// One element per schema property, valued from `values`; true when any was written.
    fn schema_children<'a, 'v>(
        &mut self,
        doc: &mut Doc<'a>,
        properties: &'v Map<String, Value>,
        values: Option<&Map<String, Value>>,
        depth: usize,
    ) -> Result<bool, DocumentError>
    where
        'a: 'v,
    {
        let mut any = false;
        for (name, raw) in properties {
            let value = values.and_then(|v| v.get(name.as_str()));
            any |= self.element(doc, name, Some(raw), value, depth)?;
        }
        Ok(any)
    }

    /// One element per example entry, typed by the matching property of `properties`.
    fn value_children<'a, 'v, 'e>(
        &mut self,
        doc: &mut Doc<'a>,
        values: impl Iterator<Item = (&'e String, &'e Value)>,
        properties: Option<&'v Map<String, Value>>,
        depth: usize,
    ) -> Result<bool, DocumentError>
    where
        'a: 'v,
    {
        let mut any = false;
        for (name, value) in values {
            let raw = properties.and_then(|p| p.get(name.as_str()));
            any |= self.element(doc, name, raw, Some(value), depth)?;
        }
        Ok(any)
    }

    /// Write `\n` plus one indented element at `depth`, unless it is an attribute or a bound
    /// leaves it out; true when it was written.
    fn element<'a, 'v>(
        &mut self,
        doc: &mut Doc<'a>,
        name: &str,
        raw: Option<&'v Value>,
        value: Option<&Value>,
        depth: usize,
    ) -> Result<bool, DocumentError>
    where
        'a: 'v,
    {
        let empty = Value::Object(Map::new());
        let resolved = match raw {
            Some(raw) => match self.resolve(doc, raw)? {
                Some(resolved) => Some(resolved),
                None => return Ok(false),
            },
            None => None,
        };
        let prop = resolved.as_ref().map_or(&empty, |r| r.value.as_ref());
        if is_attribute(prop) {
            return Ok(false);
        }
        let tag = xml_name(prop).unwrap_or(name);
        let indent = 2 * depth;
        // A newline and indent before the open tag, then the close tag with room for a newline
        // and indent of its own before it.
        let close = 1 + indent + tag.len() + 3;
        if !self.fits(doc, 1 + indent + tag.len() + 2 + close)? {
            return Ok(false);
        }
        self.push(doc, "\n")?;
        self.indent(doc, indent)?;
        self.push(doc, "<")?;
        self.push(doc, tag)?;
        self.push(doc, ">")?;
        self.reserved += close;
        let followed = resolved.as_ref().map(|r| r.followed.as_slice());
        doc.enter(followed.unwrap_or_default());
        let nested = self.content(doc, prop, value, depth);
        doc.leave(followed.unwrap_or_default());
        self.reserved -= close;
        if nested? {
            self.push(doc, "\n")?;
            self.indent(doc, indent)?;
        }
        self.push(doc, "</")?;
        self.push(doc, tag)?;
        self.push(doc, ">")?;
        Ok(true)
    }

    /// An element's children from an object value or an object schema, else its escaped text;
    /// true when children were written.
    fn content<'a, 'v>(
        &mut self,
        doc: &mut Doc<'a>,
        prop: &'v Value,
        value: Option<&Value>,
        depth: usize,
    ) -> Result<bool, DocumentError>
    where
        'a: 'v,
    {
        let properties = prop.get("properties").and_then(Value::as_object);
        let object = value.and_then(Value::as_object);
        let is_object = prop.get("type").and_then(Value::as_str) == Some("object");
        if object.is_some() || is_object || properties.is_some() {
            let has_children = match object {
                Some(object) => !object.is_empty(),
                None => properties.is_some_and(|p| !p.is_empty()),
            };
            if has_children && depth >= doc.bounds.sample_depth {
                self.note(doc, Cut::Depth)?;
                return Ok(false);
            }
            return match object {
                Some(object) => self.value_children(doc, object.iter(), properties, depth + 1),
                None => {
                    self.schema_children(doc, properties.unwrap_or(&Map::new()), None, depth + 1)
                }
            };
        }
        let value = match value {
            None | Some(Value::Null) => return Ok(false),
            Some(Value::String(s)) => Cow::Borrowed(s.as_str()),
            Some(other) => Cow::Owned(text(doc, other)?),
        };
        if self.fits(doc, escaped_len(&value))? {
            self.escaped(doc, &value)?;
        }
        Ok(false)
    }

    /// Resolve a property schema as one schema step, or `None` when a step budget is spent.
    fn resolve<'a, 'v>(
        &mut self,
        doc: &mut Doc<'a>,
        raw: &'v Value,
    ) -> Result<Option<Resolved<'v>>, DocumentError>
    where
        'a: 'v,
    {
        if doc.visits >= doc.bounds.document_visits {
            self.note(doc, Cut::DocumentSteps)?;
            return Ok(None);
        }
        if self.steps >= doc.bounds.sample_visits {
            self.note(doc, Cut::Steps)?;
            return Ok(None);
        }
        doc.visits += 1;
        self.steps += 1;
        Ok(Some(doc.resolve_followed(
            raw,
            RefKind::Schema,
            self.source,
        )?))
    }

    /// Whether `len` more bytes stay within the size bound, noting the cut when not.
    fn fits(&mut self, doc: &mut Doc<'_>, len: usize) -> Result<bool, DocumentError> {
        let total = self.out.len() + self.reserved + len;
        if total > doc.bounds.sample_bytes {
            self.note(doc, Cut::Size)?;
            return Ok(false);
        }
        Ok(true)
    }

    /// Append `text`, charging its bytes first.
    fn push(&mut self, doc: &mut Doc<'_>, text: &str) -> Result<(), DocumentError> {
        doc.budget.charge_bytes(text.len() as u64)?;
        self.out.push_str(text);
        Ok(())
    }

    fn indent(&mut self, doc: &mut Doc<'_>, width: usize) -> Result<(), DocumentError> {
        doc.budget.charge_bytes(width as u64)?;
        self.out.extend(std::iter::repeat_n(' ', width));
        Ok(())
    }

    /// Append `text` with `&`, `<`, `>` and `"` escaped, charging the escaped length first.
    fn escaped(&mut self, doc: &mut Doc<'_>, text: &str) -> Result<(), DocumentError> {
        doc.budget.charge_bytes(escaped_len(text) as u64)?;
        for c in text.chars() {
            match escape(c) {
                Some(entity) => self.out.push_str(entity),
                None => self.out.push(c),
            }
        }
        Ok(())
    }

    fn note(&mut self, doc: &mut Doc<'_>, cut: Cut) -> Result<(), DocumentError> {
        if self.noted.contains(&cut) {
            return Ok(());
        }
        self.noted.push(cut);
        let note = ImportNote::new(
            self.source,
            NoteKind::Converted,
            cut_detail(&doc.bounds, cut),
        );
        doc.budget.charge_note(&note)?;
        self.notes.push(note);
        Ok(())
    }
}

fn escape(c: char) -> Option<&'static str> {
    match c {
        '&' => Some("&amp;"),
        '<' => Some("&lt;"),
        '>' => Some("&gt;"),
        '"' => Some("&quot;"),
        _ => None,
    }
}

fn escaped_len(text: &str) -> usize {
    text.chars()
        .map(|c| escape(c).map_or(c.len_utf8(), str::len))
        .sum()
}

fn xml(schema: &Value) -> Option<&Value> {
    schema.get("xml")
}

/// A non-empty `xml.name`.
fn xml_name(schema: &Value) -> Option<&str> {
    xml(schema)?
        .get("name")
        .and_then(Value::as_str)
        .filter(|n| !n.is_empty())
}

fn is_attribute(schema: &Value) -> bool {
    xml(schema).and_then(|x| x.get("attribute")) == Some(&Value::Bool(true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::openapi::bounds::Bounds;
    use crate::import::openapi::version::Version;
    use serde_json::json;

    fn with(root: &Value, schema: &Value, example: Option<&Value>) -> (String, Vec<ImportNote>) {
        let mut doc = Doc::new(root, Version::V31, Bounds::DEFAULT);
        let mut notes = Vec::new();
        let text = render(&mut doc, schema, example, "body", &mut notes).expect("within budget");
        notes.extend(doc.take_ref_notes());
        (text, notes)
    }

    fn cuts(notes: &[ImportNote], part: &str) -> usize {
        let cut = |n: &&ImportNote| n.kind == NoteKind::Converted && n.detail.contains(part);
        notes.iter().filter(cut).count()
    }

    #[test]
    fn nesting_past_the_depth_bound_is_left_out() {
        let mut deep = json!({"type": "string"});
        for _ in 0..20 {
            deep = json!({"type": "object", "properties": {"n": deep}});
        }
        let (text, notes) = with(&json!({}), &deep, None);
        assert_eq!(text.matches("<n>").count(), 16, "{text}");
        assert!(text.ends_with("</n>\n</root>"), "{text}");
        assert_eq!(cuts(&notes, "16"), 1, "{notes:?}");
    }

    #[test]
    fn a_large_example_is_cut_at_the_size_bound_and_stays_well_formed() {
        let values: Map<String, Value> = (0..100)
            .map(|i| (format!("p{i:03}"), json!("x".repeat(1024))))
            .collect();
        let example = Value::Object(values);
        let (text, notes) = with(&json!({}), &json!({}), Some(&example));
        assert!(
            text.len() <= 64 << 10 && text.len() > 60 << 10,
            "{}",
            text.len()
        );
        assert!(text.ends_with("</root>"));
        assert_eq!(text.matches("<p").count(), text.matches("</p").count());
        assert_eq!(cuts(&notes, "64 KiB"), 1, "{notes:?}");
    }

    #[test]
    fn a_recursive_schema_stops_at_the_cycle() {
        let root = json!({"components": {"schemas": {"A": {"type": "object", "properties": {
            "b": {"$ref": "#/components/schemas/A"}}}}}});
        let schema =
            json!({"type": "object", "properties": {"a": {"$ref": "#/components/schemas/A"}}});
        let (text, _) = with(&root, &schema, None);
        let want = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<root>\n  <a>\n    <b></b>\n  </a>\n</root>";
        assert_eq!(text, want);
    }
}
