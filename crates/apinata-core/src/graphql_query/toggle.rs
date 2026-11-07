//! `toggle_field`: the one text edit that ticks a field path into a query or unticks it.
//!
//! Edits are computed on the masked text, whose offsets match the original, so a newline
//! inside a placeholder is never mistaken for a line break.

use std::ops::Range;

use super::mask::mask;
use super::outline::{Cut, FieldPath, QueryOutline, Set, keyword, query_outline, resolve};
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
/// ancestors, or unticks it, removing one copy and every parent that would be left empty.
/// `None` when the query does not outline, the path is locked, a tick names a path that is
/// not a leaf field of the schema or is already selected, or an untick names one that is
/// not selected.
pub fn toggle_field(query: &str, schema: &Schema, path: &FieldPath, on: bool) -> Option<QueryEdit> {
    let outline = query_outline(query, schema)?;
    if outline.locked(path).is_some() {
        return None;
    }
    if on {
        tick(query, &outline, path)
    } else {
        untick(query, &outline, path)
    }
}

fn untick(query: &str, outline: &QueryOutline<'_>, path: &FieldPath) -> Option<QueryEdit> {
    if !outline.selects(path) {
        return None;
    }
    let node = match outline.cut(path)? {
        Cut::Field(sel) => sel.range.clone(),
        Cut::Operation(op) => op.range.clone(),
        Cut::Locked(_) => return None,
    };
    Some(QueryEdit {
        range: removal_range(&mask(query).text, node),
        text: String::new(),
    })
}

fn tick(query: &str, outline: &QueryOutline<'_>, path: &FieldPath) -> Option<QueryEdit> {
    if outline.selects(path) {
        return None;
    }
    let schema = outline.schema();
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
        return new_operation(&text, keyword(schema, root)?, rest);
    };
    // Descend through the first copy of each ancestor that has a selection set; the
    // first missing level is where the rest of the chain is written. Any copy already
    // selecting the path returned above.
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
            None => return insertion(&text, set, &rest[i..]),
        }
    }
    None
}

// Whole lines when the node stands alone on them (a trailing comma or comment may follow),
// else the node plus the separator after it, or before it when nothing follows on the line.
// A line's `\r\n` goes with the line, and a CRLF pair is never split.
fn removal_range(text: &str, node: Range<usize>) -> Range<usize> {
    let (s, e) = (node.start, node.end);
    let b = text.as_bytes();
    let line_start = text[..s].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[e..].find('\n').map_or(text.len(), |i| e + i);
    let alone_before = text[line_start..s]
        .bytes()
        .all(|c| matches!(c, b' ' | b'\t' | b'\r'));
    let after = text[e..line_end].trim_start_matches([' ', '\t', '\r', ',']);
    if alone_before && (after.is_empty() || after.starts_with('#')) {
        return line_start..(line_end + 1).min(text.len());
    }
    // Alone on its line but followed by a closer: take the preceding line break instead, so
    // the closer joins the previous line rather than keeping a stray indent. Skipped when the
    // previous line may end in a comment, which would swallow the closer.
    let prev_start = text[..line_start.saturating_sub(1)]
        .rfind('\n')
        .map_or(0, |i| i + 1);
    if alone_before
        && after.starts_with('}')
        && line_start > 0
        && !text[prev_start..line_start].contains('#')
    {
        let crlf = line_start >= 2 && b[line_start - 2] == b'\r';
        return line_start - if crlf { 2 } else { 1 }..e;
    }
    let skip = |mut j: usize| {
        while j < b.len() && matches!(b[j], b' ' | b'\t') {
            j += 1;
        }
        j
    };
    let mut j = skip(e);
    if j < b.len() && b[j] == b',' {
        j = skip(j + 1);
    }
    if j < b.len() && !matches!(b[j], b'}' | b'\n' | b'\r' | b'#') {
        return s..j;
    }
    let mut i = s;
    while i > 0 && matches!(b[i - 1], b' ' | b'\t' | b',') {
        i -= 1;
    }
    i..e
}

// The whitespace that starts the line holding byte `at`, copied verbatim (tabs stay tabs).
fn leading_ws(text: &str, at: usize) -> &str {
    let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let line = &text[start..];
    &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
}

// `names` nested as fields; on one line when `indent` is None, else one per line, each
// deeper level two spaces in from `indent`, the first name's own indent. None for no names.
fn chain(names: &[String], indent: Option<&str>) -> Option<String> {
    let (first, rest) = names.split_first()?;
    if rest.is_empty() {
        return Some(first.clone());
    }
    Some(match indent {
        None => format!("{first} {{ {} }}", chain(rest, None)?),
        Some(outer) => {
            let inner = format!("{outer}  ");
            format!(
                "{first} {{\n{inner}{}\n{outer}}}",
                chain(rest, Some(&inner))?
            )
        }
    })
}

// Appends `names` to `set`: inside a one-line set after its last selection, else on a new
// line after the line holding the last selection's end, at that selection's indent, with
// the set's own line break (CRLF stays CRLF). None for an empty set or no names.
fn insertion(text: &str, set: &Set, names: &[String]) -> Option<QueryEdit> {
    let last = set.items.last()?;
    let end = last.range.end;
    if !text[set.open..set.close].contains('\n') {
        return Some(QueryEdit {
            range: end..end,
            text: format!(" {}", chain(names, None)?),
        });
    }
    let indent = leading_ws(text, last.range.start);
    let crlf = text[set.open..set.close].contains("\r\n");
    let mut at = text[end..set.close].find('\n').map_or(end, |i| end + i);
    if crlf && text[..at].ends_with('\r') {
        at -= 1;
    }
    let lines = format!("\n{indent}{}", chain(names, Some(indent))?);
    Some(QueryEdit {
        range: at..at,
        text: if crlf {
            lines.replace('\n', "\r\n")
        } else {
            lines
        },
    })
}

// A new operation after any existing text, one blank line between; blank text is replaced.
// None for no names.
fn new_operation(text: &str, keyword: &str, names: &[String]) -> Option<QueryEdit> {
    let op = format!("{keyword} {{\n  {}\n}}", chain(names, Some("  "))?);
    let kept = text.trim_end().len();
    Some(if kept == 0 {
        QueryEdit {
            range: 0..text.len(),
            text: op,
        }
    } else {
        QueryEdit {
            range: kept..text.len(),
            text: format!("\n\n{op}"),
        }
    })
}
