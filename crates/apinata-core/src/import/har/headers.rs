//! Headers a browser adds for the transport, which a replayed request must not copy.

use super::budget::{Budget, Prefixed};
use crate::edit::KeyValue;
use crate::import::{DocumentError, ImportNote, NoteKind};
use std::collections::HashSet;

/// Headers the HTTP client sets itself; compared ignoring ASCII case.
const TRANSPORT: [&str; 7] = [
    "Host",
    "Content-Length",
    "Connection",
    "Keep-Alive",
    "Transfer-Encoding",
    "Upgrade",
    "Accept-Encoding",
];

const REPEAT: &str = "header repeats an earlier header with the same value";

/// Drop pseudo, hop-by-hop and transport headers silently, and exact repeats with a `Dropped`
/// note at `{at}request.headers[k]` (k the input index). Order and case are kept.
pub(super) fn filter(
    headers: Vec<(usize, KeyValue)>,
    at: &str,
    budget: &mut Budget,
    notes: &mut Vec<ImportNote>,
) -> Result<Vec<(usize, KeyValue)>, DocumentError> {
    let mut kept: Vec<(usize, KeyValue)> = Vec::with_capacity(headers.len());
    // Kept headers keyed by lower-cased name and value, so a hostile request with many headers
    // stays linear.
    let mut seen: HashSet<(String, String)> = HashSet::new();
    for (k, h) in headers {
        if h.name.starts_with(':') || TRANSPORT.iter().any(|t| h.name.eq_ignore_ascii_case(t)) {
            continue;
        }
        if !seen.insert((h.name.to_ascii_lowercase(), h.value.clone())) {
            notes.push(budget.note(
                at,
                &format!("request.headers[{k}]"),
                NoteKind::Dropped,
                REPEAT,
            )?);
            continue;
        }
        kept.push((k, h));
    }
    Ok(kept)
}
