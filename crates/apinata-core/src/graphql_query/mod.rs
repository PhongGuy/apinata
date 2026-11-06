//! GraphQL query text: formatting and validation with `{{placeholder}}` support.
//!
//! Placeholders are masked to same-length identifiers before parsing, so every byte
//! offset reported here points into the caller's original text.

mod format;
// The validator calls the rest of masking.
#[allow(dead_code)]
mod mask;
mod validate;

pub use format::format_query;
pub use validate::CompiledSchema;

use std::ops::Range;

/// Largest query, in bytes, that is checked against a schema; larger ones get syntax only.
pub const QUERY_VALIDATE_LIMIT: usize = 256 * 1024;
/// Largest query, in bytes, that is formatted or parsed at all.
pub const QUERY_FORMAT_LIMIT: usize = 4 * 1024 * 1024;
/// Largest formatted output, in bytes; deep nesting multiplies indentation past the input.
pub const MAX_FORMAT_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
/// Most diagnostics returned for one query; the rest are summarised in one line.
pub const QUERY_DIAGNOSTIC_LIMIT: usize = 200;

/// One finding about a query: a byte range in the original text, a severity and a message.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct QueryDiagnostic {
    range: Range<usize>,
    severity: QuerySeverity,
    message: String,
}

impl QueryDiagnostic {
    /// Byte range in the original query text.
    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }

    /// How serious the finding is.
    pub fn severity(&self) -> QuerySeverity {
        self.severity
    }

    /// Human-readable text of the finding.
    pub fn message(&self) -> &str {
        &self.message
    }

    // The validator builds these.
    #[allow(dead_code)]
    pub(crate) fn new(
        range: Range<usize>,
        severity: QuerySeverity,
        message: impl Into<String>,
    ) -> Self {
        Self {
            range,
            severity,
            message: message.into(),
        }
    }
}

/// How serious a [`QueryDiagnostic`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum QuerySeverity {
    /// The query is wrong and the server would reject it.
    Error,
    /// The query works but something deserves attention.
    Warning,
    /// A note about how the query was checked.
    Info,
}

/// Why a query could not be formatted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum QuerySyntaxError {
    /// The query does not parse.
    #[error("syntax error: {message}")]
    Syntax {
        /// Byte range of the problem in the original text.
        range: Range<usize>,
        /// What the parser reported.
        message: String,
    },
    /// The query exceeds the format size limit.
    #[error("the query is larger than the {} MiB format limit", limit / (1024 * 1024))]
    TooLarge {
        /// The limit in bytes.
        limit: usize,
    },
    /// The formatted query would exceed the output size limit.
    #[error("the formatted query would be larger than the {} MiB output limit", limit / (1024 * 1024))]
    OutputTooLarge {
        /// The limit in bytes.
        limit: usize,
    },
}
