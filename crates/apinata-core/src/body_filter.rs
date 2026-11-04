//! Filtering a response body by its content type, shared by the CLI and the MCP server.

use crate::json_filter::{FilterError, filter_json};
use crate::xml_filter::filter_xml;

/// Why a body filter could not be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum BodyFilterError {
    /// The expression is neither JSONPath nor XPath.
    #[error("invalid filter '{expression}': {reason}")]
    InvalidExpression {
        /// The expression as typed.
        expression: String,
        /// What the JSONPath parser objected to.
        reason: String,
    },
    /// The engine rejected the expression or the body.
    #[error(transparent)]
    Filter(#[from] FilterError),
    /// The body is neither JSON nor XML by its content type.
    #[error("the body is {}, not JSON or XML", .content_type.as_deref().unwrap_or("of unknown type"))]
    Unfilterable {
        /// The media type, without parameters; `None` when the response named none.
        content_type: Option<String>,
    },
}

/// Applies `expression` to `text`: JSONPath for a content type containing `json`, XPath for one
/// containing `xml`. A JSONPath that can match only one value (`$[0].name`) prints that value;
/// every other path prints the matches as an array, as [`filter_json`] does.
pub fn filter_body(
    content_type: Option<&str>,
    text: &str,
    expression: &str,
) -> Result<String, BodyFilterError> {
    let media = content_type
        .map(|t| {
            t.split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
        })
        .filter(|t| !t.is_empty());
    match media.as_deref() {
        Some(m) if m.contains("json") => {
            let matched = filter_json(text, expression)?;
            Ok(if is_singular(expression) {
                unwrap_single(&matched).unwrap_or(matched)
            } else {
                matched
            })
        }
        Some(m) if m.contains("xml") => Ok(filter_xml(text, expression)?),
        _ => Err(BodyFilterError::Unfilterable {
            content_type: media,
        }),
    }
}

/// Checks `expression` before any request: it must parse as JSONPath or as XPath, because the
/// engine is chosen only once the response arrives.
pub fn validate_filter(expression: &str) -> Result<(), BodyFilterError> {
    let rejected = |e: &FilterError| {
        matches!(
            e,
            FilterError::Expression { .. } | FilterError::Invalid { .. }
        )
    };
    let Err(json) = filter_json("null", expression) else {
        return Ok(());
    };
    if !rejected(&json) {
        return Ok(());
    }
    match filter_xml("<a/>", expression) {
        Err(xml) if rejected(&xml) => Err(BodyFilterError::InvalidExpression {
            expression: expression.to_owned(),
            reason: json.to_string(),
        }),
        _ => Ok(()),
    }
}

/// The one element of a pretty JSON array of length one, as pretty JSON.
fn unwrap_single(array: &str) -> Option<String> {
    let serde_json::Value::Array(items) = serde_json::from_str(array).ok()? else {
        return None;
    };
    let [only] = items.as_slice() else {
        return None;
    };
    Some(crate::js::stringify_pretty(only))
}

/// True for an RFC 9535 singular query: the root followed only by member names and integer
/// indexes, so it selects at most one value. Anything this scan does not recognise counts as
/// not singular, which keeps the array form.
fn is_singular(expression: &str) -> bool {
    let typed = expression.trim();
    let rest = typed.strip_prefix('$').unwrap_or(typed);
    // Without a `$`, a leading name reads from the root like `$.name`.
    let rest = if typed.starts_with('$') || rest.starts_with(['[', '.']) {
        rest.to_owned()
    } else {
        format!(".{rest}")
    };
    let mut s = rest.as_str();
    while !s.is_empty() {
        if let Some(after) = s.strip_prefix('.') {
            let end = after.find(['.', '[']).unwrap_or(after.len());
            let name = &after[..end];
            let plain = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || !c.is_ascii());
            if !plain {
                return false;
            }
            s = &after[end..];
        } else if let Some(after) = s.strip_prefix('[') {
            let Some(end) = bracket_end(after) else {
                return false;
            };
            let inner = &after[..end];
            let index = inner
                .strip_prefix('-')
                .unwrap_or(inner)
                .chars()
                .all(|c| c.is_ascii_digit())
                && !inner.is_empty()
                && inner != "-";
            let quoted = single_literal(inner);
            if !(index || quoted) {
                return false;
            }
            s = &after[end + 1..];
        } else {
            return false;
        }
    }
    true
}

/// The offset of the `]` closing a bracket whose content starts `s`, skipping quoted text.
fn bracket_end(s: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        match quote {
            Some(_) if escaped => escaped = false,
            Some(_) if c == '\\' => escaped = true,
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '\'' || c == '"' => quote = Some(c),
            None if c == ']' => return Some(i),
            None => {}
        }
    }
    None
}

/// True when `inner` is exactly one quoted string, so `'a'` but not `'a','b'` or `'a'x`.
fn single_literal(inner: &str) -> bool {
    let Some(q) = inner.chars().next().filter(|c| matches!(c, '\'' | '"')) else {
        return false;
    };
    let mut escaped = false;
    for (i, c) in inner.char_indices().skip(1) {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            c if c == q => return i + c.len_utf8() == inner.len(),
            _ => {}
        }
    }
    false
}
