//! `toggle_field`: the one text edit that ticks a field path into a query.
//!
//! Edits are computed on the masked text, whose offsets match the original, so a newline
//! inside a placeholder is never mistaken for a line break.

use std::ops::Range;

use super::mask::mask;
use super::outline::{FieldPath, Set, keyword, query_outline, resolve};
use crate::{Schema, TypeKind};

/// One replacement in the query text: `range` is replaced by `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct QueryEdit {
    /// Byte range in the original query text.
    pub range: Range<usize>,
    /// The text that replaces the range.
    pub text: String,
}

/// The edit that ticks (`on`) `path` in `query`, writing the field and any missing
/// ancestors. `None` when the query does not outline, the path is locked or not a leaf
/// field of the schema, or it is already selected.
pub fn toggle_field(query: &str, schema: &Schema, path: &FieldPath, on: bool) -> Option<QueryEdit> {
    let outline = query_outline(query, schema)?;
    if !on || outline.locked(path).is_some() {
        return None;
    }
    // Only a scalar or enum field can be written without a selection set of its own.
    let last = resolve(schema, path)?.pop()?;
    if schema
        .type_named(last.ty().named())
        .is_some_and(|t| !matches!(t.kind(), TypeKind::Scalar | TypeKind::Enum))
    {
        return None;
    }
    let text = mask(query).text;
    let (root, rest) = path.names().split_first()?;
    let Some(op) = outline.op() else {
        return Some(new_operation(&text, keyword(schema, root)?, rest));
    };
    // Descend through the first copy of each ancestor that has a selection set; the
    // first missing level is where the rest of the chain is written.
    let mut set = &op.set;
    for (i, name) in rest.iter().enumerate() {
        let leaf = i + 1 == rest.len();
        let found = set
            .items
            .iter()
            .find(|s| s.name.as_ref() == Some(name) && (leaf || s.set.is_some()));
        match found {
            Some(_) if leaf => return None,
            Some(sel) => set = sel.set.as_ref()?,
            None => return Some(insertion(&text, set, &rest[i..])),
        }
    }
    None
}

// The whitespace that starts the line holding byte `at`, copied verbatim (tabs stay tabs).
fn leading_ws(text: &str, at: usize) -> &str {
    let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let line = &text[start..];
    &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
}

// `names` nested as fields; on one line when `indent` is None, else one per line, each
// deeper level two spaces in from `indent`, the first name's own indent.
fn chain(names: &[String], indent: Option<&str>) -> String {
    let (first, rest) = names.split_first().expect("at least one name");
    if rest.is_empty() {
        return first.clone();
    }
    match indent {
        None => format!("{first} {{ {} }}", chain(rest, None)),
        Some(outer) => {
            let inner = format!("{outer}  ");
            format!(
                "{first} {{\n{inner}{}\n{outer}}}",
                chain(rest, Some(&inner))
            )
        }
    }
}

// Appends `names` to `set`: inside a one-line set after its last selection, else on a new
// line after the line holding the last selection's end, at that selection's indent.
fn insertion(text: &str, set: &Set, names: &[String]) -> QueryEdit {
    let last = set.items.last().expect("a parsed set is never empty");
    let end = last.range.end;
    if !text[set.open..set.close].contains('\n') {
        return QueryEdit {
            range: end..end,
            text: format!(" {}", chain(names, None)),
        };
    }
    let indent = leading_ws(text, last.range.start);
    let at = text[end..set.close].find('\n').map_or(end, |i| end + i);
    QueryEdit {
        range: at..at,
        text: format!("\n{indent}{}", chain(names, Some(indent))),
    }
}

// A new operation after any existing text, one blank line between; blank text is replaced.
fn new_operation(text: &str, keyword: &str, names: &[String]) -> QueryEdit {
    let op = format!("{keyword} {{\n  {}\n}}", chain(names, Some("  ")));
    let kept = text.trim_end().len();
    if kept == 0 {
        QueryEdit {
            range: 0..text.len(),
            text: op,
        }
    } else {
        QueryEdit {
            range: kept..text.len(),
            text: format!("\n\n{op}"),
        }
    }
}
