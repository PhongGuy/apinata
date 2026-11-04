//! The body half of a comparison.

use std::time::{Duration, Instant};

use super::json::compare_json;
use super::*;
use crate::git::hunks_within;

/// Compares two bodies as text. A side that is not held in full, or has no body, is reported
/// as unavailable (old side first); a body past `body_limit` bytes is `TooLarge`; otherwise
/// JSON is compared structurally and anything else by the shared line diff under `timeout`.
pub(super) fn compare_body(
    old: &ResponseSnapshot,
    new: &ResponseSnapshot,
    timeout: Duration,
    body_limit: usize,
) -> BodyComparison {
    for (side, snap) in [(Side::Old, old), (Side::New, new)] {
        let reason = match snap.body_state() {
            BodyState::Text => continue,
            BodyState::Binary => UnavailableReason::Binary,
            BodyState::Truncated => UnavailableReason::Truncated,
            BodyState::Dropped => UnavailableReason::Dropped,
        };
        return BodyComparison::Unavailable { side, reason };
    }
    let (a, b) = match (old.body(), new.body()) {
        (None, None) => return BodyComparison::Identical,
        (None, Some(_)) => return absent(Side::Old),
        (Some(_), None) => return absent(Side::New),
        (Some(a), Some(b)) => (a, b),
    };
    if a.len() > body_limit || b.len() > body_limit {
        return BodyComparison::TooLarge {
            limit: body_limit,
            old_len: a.len(),
            new_len: b.len(),
        };
    }
    if a == b {
        return BodyComparison::Identical;
    }
    if let Some(json) = compare_json(a, b) {
        return json;
    }
    let started = Instant::now();
    let hunks = hunks_within(a, b, timeout);
    BodyComparison::Text {
        hunks,
        exact: started.elapsed() < timeout,
    }
}

fn absent(side: Side) -> BodyComparison {
    BodyComparison::Unavailable {
        side,
        reason: UnavailableReason::Absent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::LineKind;

    const T: Duration = Duration::from_millis(250);

    fn snap(text: &str) -> ResponseSnapshot {
        ResponseSnapshot::new(200).with_body(text)
    }

    #[test]
    fn equal_text_is_identical() {
        assert_eq!(
            compare_body(&snap("a\nb\n"), &snap("a\nb\n"), T, usize::MAX),
            BodyComparison::Identical
        );
    }

    #[test]
    fn a_changed_line_is_one_hunk_with_context() {
        let found = compare_body(
            &snap("one\ntwo\nthree\n"),
            &snap("one\n2\nthree\n"),
            T,
            usize::MAX,
        );
        let BodyComparison::Text { hunks, exact } = found else {
            panic!("{found:?}")
        };
        assert!(exact);
        assert_eq!(hunks.len(), 1);
        let shown: Vec<(LineKind, &str)> = hunks[0]
            .lines
            .iter()
            .map(|l| (l.kind, l.text.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                (LineKind::Context, "one"),
                (LineKind::Removed, "two"),
                (LineKind::Added, "2"),
                (LineKind::Context, "three"),
            ]
        );
    }

    #[test]
    fn whitespace_only_text_differences_are_reported() {
        let found = compare_body(&snap("a b\n"), &snap("a  b\n"), T, usize::MAX);
        assert!(matches!(found, BodyComparison::Text { .. }), "{found:?}");
    }

    #[test]
    fn a_zero_timeout_still_returns_valid_hunks_marked_approximate() {
        let old: String = (0..40).map(|i| format!("{}\n", i % 7)).collect();
        let new: String = (0..40).map(|i| format!("{}\n", i * 3 % 5)).collect();
        let found = compare_body(&snap(&old), &snap(&new), Duration::ZERO, usize::MAX);
        let BodyComparison::Text { hunks, exact } = found else {
            panic!("{found:?}")
        };
        assert!(!exact);
        let side = |keep: LineKind| -> String {
            hunks
                .iter()
                .flat_map(|h| &h.lines)
                .filter(|l| l.kind == LineKind::Context || l.kind == keep)
                .map(|l| format!("{}\n", l.text))
                .collect()
        };
        assert_eq!((side(LineKind::Removed), side(LineKind::Added)), (old, new));
    }

    #[test]
    fn each_unavailable_state_names_its_side_and_reason() {
        for (state, reason) in [
            (BodyState::Binary, UnavailableReason::Binary),
            (BodyState::Truncated, UnavailableReason::Truncated),
            (BodyState::Dropped, UnavailableReason::Dropped),
        ] {
            let odd = ResponseSnapshot::new(200)
                .with_body("x")
                .with_body_state(state);
            assert_eq!(
                compare_body(&odd, &snap("x"), T, usize::MAX),
                BodyComparison::Unavailable {
                    side: Side::Old,
                    reason
                }
            );
            assert_eq!(
                compare_body(&snap("x"), &odd, T, usize::MAX),
                BodyComparison::Unavailable {
                    side: Side::New,
                    reason
                }
            );
        }
    }

    #[test]
    fn with_both_sides_unavailable_the_old_side_is_named() {
        let binary = ResponseSnapshot::new(200).with_body_state(BodyState::Binary);
        let dropped = ResponseSnapshot::new(200).with_body_state(BodyState::Dropped);
        assert_eq!(
            compare_body(&binary, &dropped, T, usize::MAX),
            BodyComparison::Unavailable {
                side: Side::Old,
                reason: UnavailableReason::Binary
            }
        );
    }

    #[test]
    fn a_missing_body_is_absent_unless_both_are_missing() {
        let none = ResponseSnapshot::new(204);
        assert_eq!(
            compare_body(&none, &none, T, usize::MAX),
            BodyComparison::Identical
        );
        assert_eq!(
            compare_body(&none, &snap("x"), T, usize::MAX),
            BodyComparison::Unavailable {
                side: Side::Old,
                reason: UnavailableReason::Absent
            }
        );
        assert_eq!(
            compare_body(&snap("x"), &none, T, usize::MAX),
            BodyComparison::Unavailable {
                side: Side::New,
                reason: UnavailableReason::Absent
            }
        );
    }

    #[test]
    fn invalid_json_on_one_side_falls_through_to_text() {
        let found = compare_body(&snap("{\"a\":1}"), &snap("{\"a\":"), T, usize::MAX);
        assert!(matches!(found, BodyComparison::Text { .. }), "{found:?}");
    }

    #[test]
    fn a_json_pair_differing_in_one_value_is_json() {
        let found = compare_body(&snap("{\"a\":1}"), &snap("{\"a\":2}"), T, usize::MAX);
        let BodyComparison::Json { changes, .. } = found else {
            panic!("{found:?}")
        };
        assert_eq!(changes.len(), 1);
    }
}
