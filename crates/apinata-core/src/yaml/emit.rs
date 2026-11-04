//! The document emitter: Bruno's `stringifyYml`, which is `YAML.stringify` from `yaml` 2.3.4
//! followed by a pass that puts a blank line before some top-level keys.

use super::scalar::{INDENT_STEP, is_js_space, stringify_number, stringify_string};
use yaml_serde::{Mapping, Sequence, Value};

/// Top-level keys that Bruno separates from the line above with a blank line.
const BLOCK_KEYS: [&str; 13] = [
    "info",
    "http",
    "graphql",
    "grpc",
    "websocket",
    "mock",
    "routes",
    "runtime",
    "settings",
    "examples",
    "docs",
    "items",
    "request",
];

/// `yaml` writes a key longer than this many UTF-16 units in the explicit `? key` form.
const MAX_IMPLICIT_KEY_LENGTH: usize = 1024;

/// Serialize a document to the bytes Bruno's writer produces for it.
pub(crate) fn to_bruno_yaml(doc: &Mapping) -> String {
    let yaml = block_map(doc, "") + "\n";
    space_block_keys(&yaml)
}

/// Port of the `stringifyYml` line loop: a top-level line whose text before the first `:` is a
/// block key gets a blank line above it, unless the line above is already blank after `trim()`.
fn space_block_keys(yaml: &str) -> String {
    let mut result: Vec<&str> = Vec::new();
    for (i, line) in yaml.split('\n').enumerate() {
        let top_level = i > 0 && !line.starts_with([' ', '\t']);
        let key = line.split(':').next().unwrap_or_default();
        let after_text = result
            .last()
            .is_some_and(|prev| !prev.trim_matches(is_js_space).is_empty());
        if top_level && BLOCK_KEYS.contains(&key) && after_text {
            result.push("");
        }
        result.push(line);
    }
    result.join("\n")
}

/// Stringify any node whose own lines continue at `indent`; `implicit_key` marks a `key:` slot.
fn node(value: &Value, indent: &str, implicit_key: bool) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => stringify_number(n),
        Value::String(s) => stringify_string(s, indent, implicit_key),
        Value::Sequence(seq) => block_seq(seq, indent),
        Value::Mapping(map) => block_map(map, indent),
        Value::Tagged(_) => unreachable!("tagged values are rejected at load"),
    }
}

/// Port of `stringifyBlockCollection` for a mapping: one pair per line, all at `indent`.
fn block_map(map: &Mapping, indent: &str) -> String {
    if map.is_empty() {
        return "{}".to_owned();
    }
    let lines: Vec<String> = map.iter().map(|(k, v)| pair(k, v, indent)).collect();
    lines.join(&format!("\n{indent}"))
}

/// Port of `stringifyBlockCollection` for a sequence: `- ` items whose content sits one step in.
fn block_seq(seq: &Sequence, indent: &str) -> String {
    if seq.is_empty() {
        return "[]".to_owned();
    }
    let item_indent = format!("{indent}{INDENT_STEP}");
    let lines: Vec<String> = seq
        .iter()
        .map(|item| format!("- {}", node(item, &item_indent, false)))
        .collect();
    lines.join(&format!("\n{indent}"))
}

/// Port of `stringifyPair` without comments, anchors or flow: `key: value`, with a non-empty
/// collection value starting on the next line one step in.
fn pair(key: &Value, value: &Value, indent: &str) -> String {
    let inner = format!("{indent}{INDENT_STEP}");
    let is_collection = |v: &Value| matches!(v, Value::Mapping(_) | Value::Sequence(_));
    let mut explicit = is_collection(key);
    let key = node(key, &inner, !explicit);
    explicit |= key.encode_utf16().count() > MAX_IMPLICIT_KEY_LENGTH;

    let value_str = node(value, &inner, false);
    let non_empty_collection = match value {
        Value::Mapping(m) => !m.is_empty(),
        Value::Sequence(s) => !s.is_empty(),
        _ => false,
    };
    let ws = if !explicit && non_empty_collection {
        format!("\n{inner}")
    } else {
        " ".to_owned()
    };
    if explicit {
        format!("? {key}\n{indent}:{ws}{value_str}")
    } else {
        format!("{key}:{ws}{value_str}")
    }
}
