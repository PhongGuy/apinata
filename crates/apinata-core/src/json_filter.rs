//! JSONPath filtering of response bodies, behind an apinata-owned API.

use std::cell::Cell;

use serde::ser::{Error as _, Serialize, SerializeMap as _, SerializeSeq as _, Serializer};
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
    /// A newer filter replaced this one before it finished.
    #[error("The filter was replaced by a newer one")]
    Cancelled,
    /// The matches could not be written out.
    #[error("The matches could not be written: {message}")]
    Output {
        /// The writer's complaint.
        message: String,
    },
}

/// Runs the RFC 9535 JSONPath `expression` over the JSON `body` and returns the matched values as a
/// JSON array laid out like `JSON.stringify(matches, null, 2)`. A missing leading `$` reads from the root.
pub fn filter_json(body: &str, expression: &str) -> Result<String, FilterError> {
    let filter = JsonFilter::compile(expression)?;
    filter.run(&ParsedJson::parse(body)?, || false)
}

/// A JSON body parsed once, so several filters can run over it without parsing it again.
#[derive(Debug)]
#[non_exhaustive]
pub struct ParsedJson {
    value: Value,
}

impl ParsedJson {
    /// Parses `body`, skipping a leading byte order mark; fails with [`FilterError::Body`].
    pub fn parse(body: &str) -> Result<Self, FilterError> {
        let value =
            serde_json::from_str(body.strip_prefix('\u{feff}').unwrap_or(body)).map_err(|e| {
                FilterError::Body {
                    message: e.to_string(),
                }
            })?;
        Ok(ParsedJson { value })
    }
}

/// A compiled JSONPath filter, as [`filter_json`] reads the expression.
#[derive(Debug)]
#[non_exhaustive]
pub struct JsonFilter {
    path: JsonPath,
}

impl JsonFilter {
    /// Compiles `expression`; fails with [`FilterError::Expression`] for an empty or invalid one.
    pub fn compile(expression: &str) -> Result<Self, FilterError> {
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
        Ok(JsonFilter { path })
    }

    /// Runs the filter over `body`, printed as [`filter_json`] prints it. `cancelled` is asked
    /// before each value written; once it says yes the run stops with [`FilterError::Cancelled`].
    pub fn run(
        &self,
        body: &ParsedJson,
        cancelled: impl Fn() -> bool,
    ) -> Result<String, FilterError> {
        if cancelled() {
            return Err(FilterError::Cancelled);
        }
        // The write learns of a cancel only as an error, so the answer is noted where it is given.
        let stopped = Cell::new(false);
        let check = || {
            let stop = cancelled();
            stopped.set(stopped.get() || stop);
            stop
        };
        let matches = self.path.query(&body.value).all();
        let checked: Vec<_> = matches.into_iter().map(|v| Checked(v, &check)).collect();
        // Only a write the cancel stopped is a cancel; any other failure keeps its own message.
        crate::js::write_pretty(&checked).map_err(|e| {
            if stopped.get() {
                FilterError::Cancelled
            } else {
                FilterError::Output {
                    message: e.to_string(),
                }
            }
        })
    }
}

/// A value that serializes like itself but asks `cancelled` before every element on the way,
/// failing the write once it says yes.
struct Checked<'a, F>(&'a Value, &'a F);

impl<F: Fn() -> bool> Serialize for Checked<'_, F> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if (self.1)() {
            return Err(S::Error::custom("cancelled"));
        }
        match self.0 {
            Value::Array(items) => {
                let mut seq = s.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(&Checked(item, self.1))?;
                }
                seq.end()
            }
            Value::Object(map) => {
                let mut out = s.serialize_map(Some(map.len()))?;
                for (key, value) in map {
                    out.serialize_entry(key, &Checked(value, self.1))?;
                }
                out.end()
            }
            scalar => scalar.serialize(s),
        }
    }
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
