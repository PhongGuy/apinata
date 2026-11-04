//! The Compare tab's view model: a core `Comparison` flattened into display rows, a summary line
//! and the tab badge. It holds no UI state, so the list renders it row by row.

use apinata_core::{
    BodyComparison, Comparison, HeaderChange, HeaderChangeKind, JsonChangeKind, Side, StatusChange,
    StatusLine, UnavailableReason,
};

use crate::body_text::format_size;
use crate::compare_source::HeaderValue;
use crate::diff_doc::{self, DiffHunk, DiffRow};

/// One line of the flattened comparison.
#[derive(Clone, Debug, PartialEq)]
pub enum CompareRow {
    /// A section title: "Status", "Headers" or "Body".
    Group(&'static str),
    /// A muted line of prose.
    Note(String),
    /// A changed status, both sides as "200 OK".
    Status {
        /// The baseline status line.
        old: String,
        /// The current status line.
        new: String,
    },
    /// A changed header, values joined with ", ".
    Header {
        /// How it differs.
        kind: HeaderChangeKind,
        /// The header name.
        name: String,
        /// The baseline values.
        old: String,
        /// The current values.
        new: String,
        /// The row's eye, when the shown text hides something.
        eye: Option<HeaderEye>,
    },
    /// The fold over the headers that change on every send.
    Volatile {
        /// How many headers it hides.
        count: usize,
        /// Whether they are shown.
        open: bool,
    },
    /// A changed JSON value.
    Json {
        /// How it differs.
        kind: JsonChangeKind,
        /// The path to the value.
        path: String,
        /// The baseline value, if any.
        old: Option<String>,
        /// The current value, if any.
        new: Option<String>,
    },
    /// A line of the text diff.
    Diff(DiffRow),
}

/// The eye of a header row whose shown text hides something.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderEye {
    /// Masked; a click reveals both sides.
    Hidden,
    /// Revealed; a click masks both sides again.
    Revealed,
    /// A side holds only its masked text, so there is nothing to reveal.
    Locked,
}

/// What the header rows show: each side's values, and the header names revealed (lowercase).
#[derive(Clone, Copy, Default)]
pub struct Reveal<'a> {
    /// The baseline's header values.
    pub old: &'a [HeaderValue],
    /// The current side's header values.
    pub new: &'a [HeaderValue],
    /// The names whose rows show real values.
    pub revealed: &'a [String],
}

fn status_text(s: &StatusLine) -> String {
    if s.text.is_empty() {
        s.code.to_string()
    } else {
        format!("{} {}", s.code, s.text)
    }
}

// A row never carries a real value unless its eye is open: the compared values are mapped back
// to each side's values of that name, shown masked unless the name is revealed. Without side
// values (a bare `Comparison`), the compared text is shown as it is.
fn header_row(h: &HeaderChange, reveal: &Reveal) -> CompareRow {
    let of = |side: &[HeaderValue]| -> Vec<HeaderValue> {
        (side.iter())
            .filter(|v| v.name.eq_ignore_ascii_case(&h.name))
            .cloned()
            .collect()
    };
    let (old, new) = (of(reveal.old), of(reveal.new));
    if old.is_empty() && new.is_empty() {
        return CompareRow::Header {
            kind: h.kind,
            name: h.name.clone(),
            old: h.old.join(", "),
            new: h.new.join(", "),
            eye: None,
        };
    }
    let both = || old.iter().chain(&new);
    let eye = if !both().any(HeaderValue::masked) {
        None
    } else if both().any(|v| v.raw.is_none()) {
        Some(HeaderEye::Locked)
    } else if (reveal.revealed.iter()).any(|n| n.eq_ignore_ascii_case(&h.name)) {
        Some(HeaderEye::Revealed)
    } else {
        Some(HeaderEye::Hidden)
    };
    let text = |values: &[HeaderValue]| {
        (values.iter())
            .map(|v| match (eye, &v.raw) {
                (Some(HeaderEye::Revealed), Some(raw)) => raw.as_str(),
                _ => v.shown.as_str(),
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    CompareRow::Header {
        kind: h.kind,
        name: h.name.clone(),
        old: text(&old),
        new: text(&new),
        eye,
    }
}

fn unavailable(side: Side, reason: UnavailableReason) -> String {
    let side = match side {
        Side::Old => "baseline",
        _ => "current",
    };
    let reason = match reason {
        UnavailableReason::Binary => "is binary",
        UnavailableReason::Truncated => "was cut at the size limit",
        UnavailableReason::Dropped => "was not kept",
        _ => "is absent",
    };
    format!("Cannot compare bodies: the {side} body {reason}")
}

/// Flattens `c` into rows; `volatile_open` shows the folded headers.
pub fn rows(c: &Comparison, volatile_open: bool) -> Vec<CompareRow> {
    rows_revealing(c, volatile_open, &Reveal::default())
}

/// [`rows`] with each header row's values read from `reveal`'s sides, masked unless revealed.
pub fn rows_revealing(c: &Comparison, volatile_open: bool, reveal: &Reveal) -> Vec<CompareRow> {
    let same = || CompareRow::Note("Same".into());
    let mut out = vec![CompareRow::Group("Status")];
    out.push(match &c.status {
        StatusChange::Changed { old, new } => CompareRow::Status {
            old: status_text(old),
            new: status_text(new),
        },
        _ => same(),
    });

    out.push(CompareRow::Group("Headers"));
    if c.headers.is_empty() {
        out.push(same());
    } else {
        out.extend(
            c.headers
                .iter()
                .filter(|h| !h.volatile)
                .map(|h| header_row(h, reveal)),
        );
        let volatile: Vec<_> = c.headers.iter().filter(|h| h.volatile).collect();
        if !volatile.is_empty() {
            out.push(CompareRow::Volatile {
                count: volatile.len(),
                open: volatile_open,
            });
            if volatile_open {
                out.extend(volatile.into_iter().map(|h| header_row(h, reveal)));
            }
        }
    }

    out.push(CompareRow::Group("Body"));
    match &c.body {
        BodyComparison::Json {
            changes,
            truncated_changes,
            ..
        } => {
            if changes.is_empty() && *truncated_changes == 0 {
                out.push(CompareRow::Note("Same apart from key order".into()));
            }
            out.extend(changes.iter().map(|j| CompareRow::Json {
                kind: j.kind,
                path: j.path.clone(),
                old: j.old.clone(),
                new: j.new.clone(),
            }));
            if *truncated_changes > 0 {
                out.push(CompareRow::Note(more_changes_note(*truncated_changes)));
            }
        }
        BodyComparison::Text { hunks, exact } => {
            if !exact {
                out.push(CompareRow::Note("Approximate: the search timed out".into()));
            }
            let hunks: Vec<DiffHunk> = hunks
                .iter()
                .map(|h| DiffHunk {
                    old_start: h.old_start,
                    new_start: h.new_start,
                    lines: h.lines.iter().map(|l| (l.kind, l.text.clone())).collect(),
                    unterminated: diff_doc::unterminated(&h.lines),
                })
                .collect();
            out.extend(diff_doc::rows(&hunks).into_iter().map(CompareRow::Diff));
        }
        BodyComparison::TooLarge {
            limit,
            old_len,
            new_len,
        } => out.push(CompareRow::Note(format!(
            "Too large to compare: {}, limit {}",
            format_size(*old_len.max(new_len) as u64),
            format_size(*limit as u64)
        ))),
        BodyComparison::Unavailable { side, reason } => {
            out.push(CompareRow::Note(unavailable(*side, *reason)));
        }
        _ => out.push(same()),
    }
    out
}

/// The one-line summary above the list.
pub fn summary(c: &Comparison) -> String {
    if c.is_identical() {
        return "Identical".into();
    }
    let status = match c.status {
        StatusChange::Changed { .. } => "changed",
        _ => "same",
    };
    let headers = match c.headers.iter().filter(|h| !h.volatile).count() {
        0 => "headers same".to_string(),
        1 => "1 header".to_string(),
        n => format!("{n} headers"),
    };
    let counted = |k: usize| match k {
        0 => "body same".to_string(),
        1 => "1 body change".to_string(),
        k => format!("{k} body changes"),
    };
    let body = match &c.body {
        BodyComparison::Json {
            changes,
            truncated_changes,
            ..
        } if changes.is_empty() && *truncated_changes == 0 => "key order differs".to_string(),
        BodyComparison::Json {
            changes,
            truncated_changes,
            ..
        } => counted(changes.len() + truncated_changes),
        BodyComparison::Text { hunks, .. } => counted(hunks.len()),
        BodyComparison::TooLarge { .. } => "body too large".into(),
        BodyComparison::Unavailable { .. } => "body unavailable".into(),
        _ => "body same".into(),
    };
    format!("Status {status}, {headers}, {body}")
}

/// The count the Compare tab's badge shows; `None` when nothing changed.
pub fn badge(c: &Comparison) -> Option<usize> {
    Some(c.change_count()).filter(|&n| n > 0)
}

/// The note under a capped JSON change list, singular for one.
pub fn more_changes_note(n: usize) -> String {
    match n {
        1 => "1 more change not shown".into(),
        n => format!("{n} more changes not shown"),
    }
}
