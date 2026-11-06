//! Pure helpers for live GraphQL query checks: job numbering, editor
//! diagnostics from core findings, the status label and the format error text.

use std::time::Duration;

use apinata_core::{QueryDiagnostic, QuerySeverity, QuerySyntaxError};
use gpui_kit::base::input::{Diagnostic, DiagnosticSeverity};
use gpui_kit::component::input::{Position, Rope, RopeExt};

use crate::graphql_schema_store::Lookup;

/// How long typing must pause before a check starts.
pub const CHECK_DELAY: Duration = Duration::from_millis(250);
/// Queries at least this large are formatted off the UI thread.
pub const FORMAT_OFF_THREAD_BYTES: usize = 64 * 1024;
/// Core's note for a structural placeholder.
pub const SKIP_NOTE: &str = "Schema checks skipped: a {{placeholder}} stands for query structure";

/// Numbers background jobs so a stale result can be recognised and dropped.
#[derive(Debug, Default)]
pub struct Jobs {
    latest: u64,
}

impl Jobs {
    /// Starts a new job and returns its number.
    pub fn start(&mut self) -> u64 {
        self.latest += 1;
        self.latest
    }

    /// Whether `job` is the most recently started one.
    pub fn is_latest(&self, job: u64) -> bool {
        self.latest == job
    }
}

/// What a finished check was measured against, for the status row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckBasis {
    /// Checked against a schema from `source`.
    Schema {
        /// Where the schema came from (a file name or the URL).
        source: String,
    },
    /// Syntax only: no schema is loaded.
    NoSchema,
    /// Syntax only: the loaded schema failed to compile.
    Unusable(String),
    /// Schema checks were skipped because a placeholder stands for structure.
    Skipped,
}

impl CheckBasis {
    /// Derives the basis from the schema lookup and core's findings.
    pub fn of(lookup: &Lookup, source: &str, found: &[QueryDiagnostic]) -> CheckBasis {
        match lookup {
            Lookup::Ready { .. } if found.iter().any(|d| d.message() == SKIP_NOTE) => {
                CheckBasis::Skipped
            }
            Lookup::Ready { .. } => CheckBasis::Schema {
                source: source.to_string(),
            },
            Lookup::Unusable { reason, .. } => CheckBasis::Unusable(reason.clone()),
            _ => CheckBasis::NoSchema,
        }
    }

    /// The status row text.
    pub fn label(&self) -> String {
        match self {
            CheckBasis::Schema { source } => format!("Checked against the schema from {source}"),
            CheckBasis::NoSchema => "Syntax only: no schema loaded".to_string(),
            CheckBasis::Unusable(reason) => {
                format!("Syntax only: the schema cannot be used for checking ({reason})")
            }
            CheckBasis::Skipped => SKIP_NOTE.to_string(),
        }
    }
}

/// Core diagnostics as editor diagnostics: skip note dropped, empty ranges
/// widened to one char so they draw, sorted by start for the editor's index.
pub fn editor_diagnostics(text: &Rope, found: &[QueryDiagnostic]) -> Vec<Diagnostic> {
    let plain = text.to_string();
    let mut out: Vec<Diagnostic> = found
        .iter()
        .filter(|d| d.message() != SKIP_NOTE)
        .map(|d| {
            let range = d.range();
            let start = range.start.min(plain.len());
            let end = range.end.clamp(start, plain.len());
            let (start, end) = widen(&plain, start, end);
            let severity = match d.severity() {
                QuerySeverity::Error => DiagnosticSeverity::Error,
                QuerySeverity::Warning => DiagnosticSeverity::Warning,
                QuerySeverity::Info => DiagnosticSeverity::Info,
                _ => DiagnosticSeverity::Info,
            };
            Diagnostic::new(
                text.offset_to_position(start)..text.offset_to_position(end),
                d.message().to_string(),
            )
            .with_severity(severity)
        })
        .collect();
    out.sort_by_key(|d| (d.range.start.line, d.range.start.character));
    out
}

/// Grows an empty byte range over the next char, or the previous one at the end.
fn widen(text: &str, start: usize, end: usize) -> (usize, usize) {
    if start != end {
        return (start, end);
    }
    if let Some(c) = text.get(start..).and_then(|rest| rest.chars().next()) {
        return (start, start + c.len_utf8());
    }
    match text.get(..start).and_then(|head| head.chars().next_back()) {
        Some(c) => (start - c.len_utf8(), start),
        None => (start, end),
    }
}

/// A format failure for the inline error: `Line L, column C: message` for a
/// syntax error, the error's own text otherwise.
pub fn syntax_message(text: &str, err: &QuerySyntaxError) -> String {
    match err {
        QuerySyntaxError::Syntax { range, message } => {
            let rope = Rope::from(text);
            let at: Position = rope.offset_to_position(range.start.min(text.len()));
            format!(
                "Line {}, column {}: {message}",
                at.line + 1,
                at.character + 1
            )
        }
        other => other.to_string(),
    }
}
