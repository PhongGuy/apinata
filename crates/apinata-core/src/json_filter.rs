//! JSONPath filtering of response bodies, behind an apinata-owned API.

use serde_json::Value;
use serde_json_path::JsonPath;

/// Why a response filter could not run.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum FilterError {
    /// The expression is empty or not valid JSONPath; `position` is a 0-based character offset into the trimmed expression as typed.
    #[error("Invalid filter at character {}: {message}", .position + 1)]
    Expression {
        /// What is wrong with the expression.
        message: String,
        /// 0-based character offset of the problem.
        position: usize,
    },
    /// The body does not parse as JSON.
    #[error("The body is not JSON: {message}")]
    Body {
        /// The JSON parser's complaint.
        message: String,
    },
    /// The expression is not valid XPath; the engine reports no position.
    #[error("Invalid filter: {message}")]
    Invalid {
        /// What is wrong with the expression.
        message: String,
    },
    /// The body does not parse as XML, or trips a parser safety limit.
    #[error("The body is not XML: {message}")]
    Xml {
        /// The XML parser's complaint.
        message: String,
    },
    /// The expression needs more work on this body than one evaluation may spend.
    #[error("The filter needs more work than allowed on this body")]
    Limit,
}

/// Runs the RFC 9535 JSONPath `expression` over the JSON `body` and returns the matched values as a
/// JSON array laid out like `JSON.stringify(matches, null, 2)`. A missing leading `$` reads from the root.
pub fn filter_json(body: &str, expression: &str) -> Result<String, FilterError> {
    let typed = expression.trim();
    if typed.is_empty() {
        return Err(FilterError::Expression {
            message: "the filter is empty".to_owned(),
            position: 0,
        });
    }

    // A rootless path reads from the root: segments starting with `[` or `.` follow a bare `$`,
    // a leading name needs `$.` first. The prefix length is subtracted from error positions.
    let prefix = if typed.starts_with('$') {
        ""
    } else if typed.starts_with(['[', '.']) {
        "$"
    } else {
        "$."
    };
    let path = JsonPath::parse(&format!("{prefix}{typed}")).map_err(|e| {
        // The parser reports a byte offset; the UI counts characters.
        let byte = e.position().saturating_sub(prefix.len());
        FilterError::Expression {
            message: reason(e.message(), typed.get(byte..).unwrap_or_default()),
            position: typed.char_indices().take_while(|(i, _)| *i < byte).count(),
        }
    })?;

    let value: Value = serde_json::from_str(body.strip_prefix('\u{feff}').unwrap_or(body))
        .map_err(|e| FilterError::Body {
            message: e.to_string(),
        })?;
    let matches = path.query(&value).all().into_iter().cloned().collect();
    Ok(crate::js::stringify_pretty(&Value::Array(matches)))
}

/// The parser's `message` as a user reads it: without the grammar rule it was in, and with
/// the bare "parser error" replaced by the character it stopped at (`rest` starts there).
fn reason(message: &str, rest: &str) -> String {
    let message = message
        .strip_prefix("in ")
        .and_then(|m| m.split_once(", "))
        .map_or(message, |(_, m)| m);
    if message != "parser error" {
        return message.to_owned();
    }
    match rest.chars().next() {
        Some(c) => format!("unexpected \"{c}\""),
        None => "unexpected end of filter".to_owned(),
    }
}
