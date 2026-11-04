//! Tool arguments, read like rmcp's `Parameters` but refused with a message that names the
//! field path, the kind of problem and what the field takes, never the value sent: serde's
//! own message quotes the input, and arguments carry credentials.
//!
//! The name stays `Parameters`, since rmcp's `#[tool]` macro finds a tool's input type by it.

use rmcp::ErrorData;
use rmcp::handler::server::common::FromContextPart;
use rmcp::handler::server::tool::ToolCallContext;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde_json::Value;
use serde_path_to_error::Segment;

use crate::tagged;

/// A tool's arguments deserialized as `P`, with a value-free error.
pub(crate) struct Parameters<P>(pub P);

// The schema is `P`'s own, byte for byte, as rmcp's wrapper gives it.
impl<P: JsonSchema> JsonSchema for Parameters<P> {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        P::schema_name()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        P::json_schema(generator)
    }
}

impl<S, P: DeserializeOwned> FromContextPart<ToolCallContext<'_, S>> for Parameters<P> {
    fn from_context_part(context: &mut ToolCallContext<'_, S>) -> Result<Self, ErrorData> {
        let arguments = Value::Object(context.arguments.take().unwrap_or_default());
        serde_path_to_error::deserialize(&arguments)
            .map(Parameters)
            .map_err(|e| {
                let (path, inner) = refine(&arguments, e);
                // rmcp turns an invalid-params error with this prefix into an `isError` result.
                let message = format!(
                    "failed to deserialize parameters: {}",
                    describe(&path, &inner)
                );
                ErrorData::invalid_params(message, None)
            })
    }
}

/// The full path and error for `error`: while it stops at a tagged document inside the
/// arguments, the document's variant is read again and the inner path joined on.
fn refine(root: &Value, error: tagged::Error) -> (String, serde_json::Error) {
    let mut path = error.path().to_string();
    let mut doc = resolve(root, error.path());
    let mut key = last_key(error.path());
    let mut error = error.into_inner();
    while let (Some(value @ Value::Object(object)), Some(name)) = (doc, key.as_deref())
        && let Some(inner) = tagged::variant_error(name, object)
    {
        // An error on the document itself (a missing field) keeps the document's path.
        if inner.path().iter().next().is_none() {
            return (path, inner.into_inner());
        }
        path = format!("{path}.{}", inner.path());
        doc = resolve(value, inner.path());
        key = last_key(inner.path());
        error = inner.into_inner();
    }
    (path, error)
}

/// The value at `path` below `value`, when every segment is a key or an index.
fn resolve<'a>(value: &'a Value, path: &serde_path_to_error::Path) -> Option<&'a Value> {
    path.iter().try_fold(value, |v, segment| match segment {
        Segment::Map { key } => v.get(key),
        Segment::Seq { index } => v.get(index),
        _ => None,
    })
}

/// The key `path` ends at, if it ends at one.
fn last_key(path: &serde_path_to_error::Path) -> Option<String> {
    match path.iter().next_back()? {
        Segment::Map { key } => Some(key.clone()),
        _ => None,
    }
}

/// The serde messages whose expected text is kept.
const KINDS: [&str; 4] = [
    "invalid type",
    "invalid value",
    "unknown variant",
    "invalid length",
];

/// The longest field name or path kept in a message.
const NAME_LIMIT: usize = 64;

/// `` `path`: kind, expected what `` for a deserialization error, built only from the field
/// path, a field name serde quotes first, and the static text after serde's last
/// `, expected `. A message of any other shape gives the path and kind alone.
fn describe(path: &str, inner: &serde_json::Error) -> String {
    let message = inner.to_string();
    let message = strip_position(&message);
    let expected = message
        .rsplit_once(", expected ")
        .map(|(_, e)| format!(", expected {e}"))
        .unwrap_or_default();
    let quoted = |prefix: &str| {
        let name = message.strip_prefix(prefix)?.split_once('`')?.0;
        Some(cut(name))
    };
    let kind = if let Some(name) = quoted("missing field `") {
        format!("missing field `{name}`")
    } else if let Some(name) = quoted("unknown field `") {
        format!("unknown field `{name}`{expected}")
    } else if let Some(kind) = KINDS.into_iter().find(|k| message.starts_with(k)) {
        format!("{kind}{expected}")
    } else if inner.is_syntax() || inner.is_eof() {
        "malformed".to_owned()
    } else {
        "invalid value".to_owned()
    };
    if path == "." {
        kind
    } else {
        format!("`{}`: {kind}", cut(path))
    }
}

/// serde_json's ` at line L column C` suffix removed.
fn strip_position(message: &str) -> &str {
    let Some((head, tail)) = message.rsplit_once(" at line ") else {
        return message;
    };
    let numbers = tail
        .split_once(" column ")
        .is_some_and(|(l, c)| [l, c].iter().all(|n| n.bytes().all(|b| b.is_ascii_digit())));
    if numbers { head } else { message }
}

/// `name` cut to [`NAME_LIMIT`] characters.
fn cut(name: &str) -> String {
    name.chars().take(NAME_LIMIT).collect()
}
