//! `{{process.env.NAME}}` references rewritten to `{{NAME}}` for Postman, which has no
//! process environment, with one note per name telling where to define it.

use serde_json::{Map, Value};

use super::ExportNote;

const OPEN: &str = "{{process.env.";

/// The names already noted, so each one gets a single note however often it appears.
#[derive(Default)]
pub(super) struct Rewrites {
    seen: Vec<String>,
}

impl Rewrites {
    /// Rewrite every template string in a Postman collection, skipping script code (`event`),
    /// docs (`description`) and saved response bodies, the only plain-string `body`.
    pub(super) fn collection(&mut self, root: &mut Value, notes: &mut Vec<ExportNote>) {
        self.walk(root, "collection", notes);
    }

    /// Rewrite `text` in place, noting each name first met at `source`.
    pub(super) fn text(&mut self, text: &mut String, source: &str, notes: &mut Vec<ExportNote>) {
        if !text.contains(OPEN) {
            return;
        }
        let mut out = String::with_capacity(text.len());
        let mut rest = text.as_str();
        while let Some(at) = rest.find(OPEN) {
            out.push_str(&rest[..at]);
            let after = &rest[at + OPEN.len()..];
            let name = after.split("}}").next().filter(|_| after.contains("}}"));
            match name.filter(|n| plain_name(n)) {
                Some(name) => {
                    out.push_str("{{");
                    out.push_str(name);
                    out.push_str("}}");
                    self.note(name, source, notes);
                    rest = &after[name.len() + 2..];
                }
                None => {
                    out.push_str(OPEN);
                    rest = after;
                }
            }
        }
        out.push_str(rest);
        *text = out;
    }

    fn note(&mut self, name: &str, source: &str, notes: &mut Vec<ExportNote>) {
        if self.seen.iter().any(|n| n == name) {
            return;
        }
        self.seen.push(name.to_owned());
        notes.push(ExportNote::changed(
            source,
            format!(
                "{{{{process.env.{name}}}}} was written as {{{{{name}}}}}, since Postman cannot read the process environment; define {name} in a Postman environment"
            ),
        ));
    }

    // `source` is the item path of the nearest named item, so a note says where it was met.
    fn walk(&mut self, value: &mut Value, source: &str, notes: &mut Vec<ExportNote>) {
        match value {
            Value::String(text) => self.text(text, source, notes),
            Value::Array(list) => {
                for v in list {
                    self.walk(v, source, notes);
                }
            }
            Value::Object(map) => self.walk_object(map, source, notes),
            _ => {}
        }
    }

    fn walk_object(
        &mut self,
        map: &mut Map<String, Value>,
        source: &str,
        notes: &mut Vec<ExportNote>,
    ) {
        for (key, v) in map.iter_mut() {
            let skipped = matches!(key.as_str(), "event" | "description" | "info" | "name")
                || (key == "body" && v.is_string());
            if skipped {
                continue;
            }
            if key != "item" {
                self.walk(v, source, notes);
                continue;
            }
            // Items name the path the notes inside them report.
            let Value::Array(items) = v else { continue };
            for item in items {
                let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
                let path = if source == "collection" {
                    name.to_owned()
                } else {
                    format!("{source} / {name}")
                };
                self.walk(item, &path, notes);
            }
        }
    }
}

// A name as a `.env` key spells it; anything else (brackets, spaces) stays as written.
fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}
