//! Response comparison: snapshots in, a structured `Comparison` out. No UI types.

mod body;
mod headers;
mod json;

use std::time::Duration;

use crate::git::Hunk;

/// The largest body, in bytes, either side may hold for a body comparison to run.
/// Set where the worst case stays interactive; see tests/fixtures/reference/2026-10-18-m11b-compare-cap.md.
pub const COMPARE_BODY_LIMIT: usize = 256 * 1024;

/// How long the text diff may run before it settles for an approximate result.
pub(crate) const COMPARE_TIMEOUT: Duration = Duration::from_millis(250);

/// How much of a response body a snapshot holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BodyState {
    /// The body is text and held in full.
    Text,
    /// The body is binary and not held.
    Binary,
    /// The body was cut at the size limit.
    Truncated,
    /// The body was not kept.
    Dropped,
}

/// One response reduced to what a comparison reads.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResponseSnapshot {
    status: u16,
    status_text: String,
    headers: Vec<(String, String)>,
    body: Option<String>,
    body_state: BodyState,
}

impl ResponseSnapshot {
    /// A snapshot with a status code, no headers and no body.
    pub fn new(status: u16) -> Self {
        Self {
            status,
            status_text: String::new(),
            headers: Vec::new(),
            body: None,
            body_state: BodyState::Text,
        }
    }

    /// Sets the status text shown beside the code.
    pub fn with_status_text(mut self, t: impl Into<String>) -> Self {
        self.status_text = t.into();
        self
    }

    /// Appends one header.
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Sets a text body.
    pub fn with_body(mut self, text: impl Into<String>) -> Self {
        self.body = Some(text.into());
        self.body_state = BodyState::Text;
        self
    }

    /// Sets the body state; any state but `Text` drops the body.
    pub fn with_body_state(mut self, s: BodyState) -> Self {
        if s != BodyState::Text {
            self.body = None;
        }
        self.body_state = s;
        self
    }

    /// The status code.
    pub fn status(&self) -> u16 {
        self.status
    }

    /// The status text, empty when there is none.
    pub fn status_text(&self) -> &str {
        &self.status_text
    }

    /// The headers in received order.
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    /// The text body, if held.
    pub fn body(&self) -> Option<&str> {
        self.body.as_deref()
    }

    /// The body state.
    pub fn body_state(&self) -> BodyState {
        self.body_state
    }
}

/// Which side of a comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Side {
    /// The older response.
    Old,
    /// The newer response.
    New,
}

/// Why a body cannot be compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnavailableReason {
    /// The body is binary.
    Binary,
    /// The body was truncated.
    Truncated,
    /// The body was not kept.
    Dropped,
    /// There is no body.
    Absent,
}

/// A status code with its text.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct StatusLine {
    /// The status code.
    pub code: u16,
    /// The status text, empty when there is none.
    pub text: String,
}

/// Whether the status code changed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StatusChange {
    /// The codes match.
    Same,
    /// The codes differ.
    Changed {
        /// The older status.
        old: StatusLine,
        /// The newer status.
        new: StatusLine,
    },
}

/// How a header differs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HeaderChangeKind {
    /// Only the new side has it.
    Added,
    /// Only the old side has it.
    Removed,
    /// Both have it with different values.
    Changed,
}

/// One header that differs.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct HeaderChange {
    /// The header name, in its first-seen spelling.
    pub name: String,
    /// How it differs.
    pub kind: HeaderChangeKind,
    /// The old values in order.
    pub old: Vec<String>,
    /// The new values in order.
    pub new: Vec<String>,
    /// True for headers that change on every send.
    pub volatile: bool,
}

/// How a JSON value differs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum JsonChangeKind {
    /// Only the new side has it.
    Added,
    /// Only the old side has it.
    Removed,
    /// Both have it with different values.
    Changed,
}

/// One JSON value that differs.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct JsonChange {
    /// The path to the value.
    pub path: String,
    /// How it differs.
    pub kind: JsonChangeKind,
    /// The old value rendered, if any.
    pub old: Option<String>,
    /// The new value rendered, if any.
    pub new: Option<String>,
}

/// The body half of a comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BodyComparison {
    /// The bodies match.
    Identical,
    /// Both bodies are JSON and differ structurally.
    Json {
        /// The changes kept.
        changes: Vec<JsonChange>,
        /// Changes counted but not kept.
        truncated_changes: usize,
        /// True when only key order differs.
        order_only: bool,
    },
    /// The bodies differ as text.
    Text {
        /// The line hunks.
        hunks: Vec<Hunk>,
        /// False when the diff hit its time limit.
        exact: bool,
    },
    /// A body is past the size limit.
    TooLarge {
        /// The limit in bytes.
        limit: usize,
        /// The old body length in bytes.
        old_len: usize,
        /// The new body length in bytes.
        new_len: usize,
    },
    /// A side has no comparable body.
    Unavailable {
        /// The side that lacks it.
        side: Side,
        /// Why.
        reason: UnavailableReason,
    },
}

/// The result of comparing two responses.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Comparison {
    /// The status comparison.
    pub status: StatusChange,
    /// The headers that differ.
    pub headers: Vec<HeaderChange>,
    /// The body comparison.
    pub body: BodyComparison,
}

impl Comparison {
    /// The number of differences worth showing: a status change, each non-volatile header
    /// change, and the JSON changes or text hunks. Volatile headers and key reordering count 0.
    pub fn change_count(&self) -> usize {
        let status = usize::from(matches!(self.status, StatusChange::Changed { .. }));
        let headers = self.headers.iter().filter(|h| !h.volatile).count();
        let body = match &self.body {
            BodyComparison::Json {
                changes,
                truncated_changes,
                ..
            } => changes.len() + truncated_changes,
            BodyComparison::Text { hunks, .. } => hunks.len(),
            _ => 0,
        };
        status + headers + body
    }

    /// True only when the status matches, no header differs at all (volatile included) and the
    /// bodies are identical.
    pub fn is_identical(&self) -> bool {
        self.status == StatusChange::Same
            && self.headers.is_empty()
            && self.body == BodyComparison::Identical
    }
}

/// Compares two responses with the standard text-diff timeout and body cap.
pub fn compare(old: &ResponseSnapshot, new: &ResponseSnapshot) -> Comparison {
    compare_with(old, new, COMPARE_TIMEOUT, COMPARE_BODY_LIMIT)
}

/// Compares two responses with an explicit text-diff `timeout` and `body_limit` in bytes.
pub(crate) fn compare_with(
    old: &ResponseSnapshot,
    new: &ResponseSnapshot,
    timeout: Duration,
    body_limit: usize,
) -> Comparison {
    let line = |s: &ResponseSnapshot| StatusLine {
        code: s.status(),
        text: s.status_text().to_owned(),
    };
    let status = if old.status() == new.status() {
        StatusChange::Same
    } else {
        StatusChange::Changed {
            old: line(old),
            new: line(new),
        }
    };
    Comparison {
        status,
        headers: headers::compare_headers(old.headers(), new.headers()),
        body: body::compare_body(old, new, timeout, body_limit),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(text: &str) -> ResponseSnapshot {
        ResponseSnapshot::new(200).with_body(text)
    }

    #[test]
    fn a_body_exactly_at_the_limit_is_compared() {
        let c = compare_with(
            &snap("aaaaaaaaaa"),
            &snap("bbbbbbbbbb"),
            COMPARE_TIMEOUT,
            10,
        );
        assert!(
            matches!(c.body, BodyComparison::Text { .. }),
            "{:?}",
            c.body
        );
    }

    #[test]
    fn a_body_one_byte_over_the_limit_is_too_large() {
        let c = compare_with(
            &snap("aaaaaaaaaaa"),
            &snap("bbbbbbbbbbb"),
            COMPARE_TIMEOUT,
            10,
        );
        assert_eq!(
            c.body,
            BodyComparison::TooLarge {
                limit: 10,
                old_len: 11,
                new_len: 11
            }
        );
    }

    #[test]
    fn a_zero_timeout_marks_the_text_diff_approximate() {
        let old: String = (0..40).map(|i| format!("{}\n", i % 7)).collect();
        let new: String = (0..40).map(|i| format!("{}\n", i * 3 % 5)).collect();
        let c = compare_with(&snap(&old), &snap(&new), Duration::ZERO, usize::MAX);
        assert!(
            matches!(c.body, BodyComparison::Text { exact: false, .. }),
            "{:?}",
            c.body
        );
    }

    #[test]
    #[ignore = "measurement; run in release with --nocapture"]
    fn measure_worst_cases() {
        use std::time::Instant;
        let kib = [256usize, 512, 1024, 2048, 5120];
        for size in kib.map(|k| k * 1024) {
            let cases: [(&str, String, String); 3] = [
                (
                    "json-many-small-changes",
                    array_of_objects(size, 0),
                    array_of_objects(size, 1),
                ),
                (
                    "text-every-line-different",
                    lines(size, 'a'),
                    lines(size, 'b'),
                ),
                (
                    "one-line-minified-json",
                    one_line_json(size, 0),
                    one_line_json(size, 1),
                ),
            ];
            for (name, old, new) in cases {
                let (o, n) = (
                    ResponseSnapshot::new(200).with_body(old),
                    ResponseSnapshot::new(200).with_body(new),
                );
                let start = Instant::now();
                // A long timeout and no cap, so the figure is the real cost, not the backstop.
                let _ = compare_with(&o, &n, Duration::from_secs(30), usize::MAX);
                eprintln!(
                    "{name} {} KiB {} ms",
                    size / 1024,
                    start.elapsed().as_millis()
                );
            }
        }
    }

    fn array_of_objects(size: usize, v: u32) -> String {
        let each = format!("{{\"id\":0,\"v\":{v}}}").len() + 1;
        format!(
            "[{}]",
            (0..size / each)
                .map(|i| format!("{{\"id\":{i},\"v\":{v}}}"))
                .collect::<Vec<_>>()
                .join(",")
        )
    }

    fn lines(size: usize, tag: char) -> String {
        (0..size / 80).map(|i| format!("{tag}{i:0>78}\n")).collect()
    }

    fn one_line_json(size: usize, v: u32) -> String {
        format!(
            "[{}]",
            (0..size / 4)
                .map(|i| if i == size / 8 {
                    v.to_string()
                } else {
                    "0".to_owned()
                })
                .collect::<Vec<_>>()
                .join(",")
        )
    }
}
