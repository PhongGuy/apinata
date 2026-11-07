//! Filters over apollo's diagnostics for masked queries, and the final sort, dedupe and cap.

use std::collections::HashSet;
use std::ops::Range;

use apollo_parser::SyntaxTree;
use apollo_parser::cst::{CstNode, Field};

use super::{QUERY_DIAGNOSTIC_LIMIT, QueryDiagnostic, QuerySeverity};

/// Whether to keep one apollo diagnostic, by its `unstable_error_name()` and range.
///
/// A missing subselection on a field whose source has one is the echo of an undefined field
/// inside it, and value checks over a mask judge `_`.
pub(crate) fn keep(
    name: Option<&str>,
    range: &Range<usize>,
    spans: &[Range<usize>],
    filled_fields: &HashSet<usize>,
) -> bool {
    match name {
        Some("MissingSubselection") => !filled_fields.contains(&range.start),
        Some("UnsupportedValueType" | "UndefinedEnumValue" | "ConflictingFieldArgument") => !spans
            .iter()
            .any(|s| range.start < s.end && s.start < range.end),
        _ => true,
    }
}

/// Start offsets of FIELD nodes whose source selection set holds at least one selection.
pub(crate) fn filled_fields(tree: &SyntaxTree) -> HashSet<usize> {
    tree.document()
        .syntax()
        .descendants()
        .filter_map(Field::cast)
        .filter(|f| {
            f.selection_set()
                .is_some_and(|set| set.selections().next().is_some())
        })
        .map(|f| usize::from(f.syntax().text_range().start()))
        .collect()
}

/// Stable sort by range start then end, exact duplicates removed, cut to
/// QUERY_DIAGNOSTIC_LIMIT plus a final `and N more` (Info, 0..0).
pub(crate) fn finish(mut diags: Vec<QueryDiagnostic>) -> Vec<QueryDiagnostic> {
    diags.sort_by_key(|d| (d.range.start, d.range.end));
    let mut seen = HashSet::new();
    diags.retain(|d| seen.insert(d.clone()));
    if diags.len() > QUERY_DIAGNOSTIC_LIMIT {
        let more = diags.len() - QUERY_DIAGNOSTIC_LIMIT;
        diags.truncate(QUERY_DIAGNOSTIC_LIMIT);
        diags.push(QueryDiagnostic::new(
            0..0,
            QuerySeverity::Info,
            format!("and {more} more"),
        ));
    }
    diags
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphql_query::{QUERY_DIAGNOSTIC_LIMIT, QueryDiagnostic, QuerySeverity};
    use QuerySeverity::{Error, Info, Warning};

    fn d(r: Range<usize>, s: QuerySeverity, m: &str) -> QueryDiagnostic {
        QueryDiagnostic::new(r, s, m)
    }

    #[test]
    fn sorted_by_start_without_duplicates() {
        let got = finish(vec![
            d(5..6, Error, "b"),
            d(0..0, Warning, "no location"),
            d(5..6, Error, "b"),
            d(2..3, Error, "a"),
            d(5..6, Error, "c"),
        ]);
        assert_eq!(
            got,
            vec![
                d(0..0, Warning, "no location"),
                d(2..3, Error, "a"),
                d(5..6, Error, "b"),
                d(5..6, Error, "c")
            ]
        );
    }

    #[test]
    fn past_the_limit_a_summary_comes_last() {
        let got = finish((0..250).map(|i| d(i + 1..i + 2, Error, "x")).collect());
        assert_eq!(got.len(), QUERY_DIAGNOSTIC_LIMIT + 1);
        assert_eq!(got[199].range(), 200..201);
        assert_eq!(got[200], d(0..0, Info, "and 50 more"));
        assert_eq!(
            finish((0..200).map(|i| d(i..i + 1, Error, "x")).collect()).len(),
            200
        );
    }
}
