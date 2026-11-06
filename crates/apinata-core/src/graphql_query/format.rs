//! `format_query`: prints a query's significant tokens in one canonical layout.

use std::collections::HashSet;

use apollo_parser::cst::CstNode;
use apollo_parser::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};

use super::mask::{is_blank, mask, parse, snap};
use super::{QUERY_FORMAT_LIMIT, QuerySyntaxError};

/// Formats GraphQL query text into the canonical layout, keeping `{{placeholders}}`,
/// strings and comments as written. Blank input comes back unchanged.
///
/// # Errors
/// [`QuerySyntaxError::TooLarge`] over [`QUERY_FORMAT_LIMIT`] bytes, otherwise
/// [`QuerySyntaxError::Syntax`] for the first parse error.
pub fn format_query(text: &str) -> Result<String, QuerySyntaxError> {
    if text.len() > QUERY_FORMAT_LIMIT {
        return Err(QuerySyntaxError::TooLarge {
            limit: QUERY_FORMAT_LIMIT,
        });
    }
    let masked = mask(text);
    let tree = parse(&masked.text);
    if is_blank(&tree) {
        return Ok(text.to_owned());
    }
    if let Some(err) = tree.errors().next() {
        let start = err.index();
        return Err(QuerySyntaxError::Syntax {
            range: snap(text, start..start + err.data().len()),
            message: err.message().to_owned(),
        });
    }
    let root = tree.document().syntax().clone();
    Ok(Printer::new(text, &root).print(&root))
}

fn significant(tok: &SyntaxToken) -> bool {
    !matches!(
        tok.kind(),
        SyntaxKind::WHITESPACE | SyntaxKind::COMMA | SyntaxKind::EOF
    )
}

fn parent_kind(tok: &SyntaxToken) -> Option<SyntaxKind> {
    tok.parent().map(|p| p.kind())
}

/// Start offset of a node's first significant token; leading trivia may belong to the node.
fn first_start(node: &SyntaxNode) -> Option<usize> {
    node.descendants_with_tokens()
        .filter_map(SyntaxElement::into_token)
        .find(significant)
        .map(|tok| usize::from(tok.text_range().start()))
}

/// Where lines break and separators go, keyed by the start offset of the token that
/// begins each definition, selection or list item.
struct Printer<'a> {
    text: &'a str,
    definitions: HashSet<usize>,
    selections: HashSet<usize>,
    later_items: HashSet<usize>,
}

impl<'a> Printer<'a> {
    fn new(text: &'a str, root: &SyntaxNode) -> Self {
        let mut printer = Self {
            text,
            definitions: HashSet::new(),
            selections: HashSet::new(),
            later_items: HashSet::new(),
        };
        for node in root.descendants() {
            let starts = node.children().filter_map(|child| first_start(&child));
            match node.kind() {
                SyntaxKind::DOCUMENT => printer.definitions.extend(starts),
                SyntaxKind::SELECTION_SET => printer.selections.extend(starts),
                SyntaxKind::ARGUMENTS
                | SyntaxKind::VARIABLE_DEFINITIONS
                | SyntaxKind::LIST_VALUE
                | SyntaxKind::OBJECT_VALUE => printer.later_items.extend(starts.skip(1)),
                _ => {}
            }
        }
        printer
    }

    /// Walks the tokens iteratively, so nesting depth never touches the call stack.
    fn print(&self, root: &SyntaxNode) -> String {
        let mut out = String::with_capacity(self.text.len() + self.text.len() / 4);
        let mut indent = 0usize;
        let mut prev: Option<SyntaxToken> = None;
        for tok in root
            .descendants_with_tokens()
            .filter_map(SyntaxElement::into_token)
            .filter(significant)
        {
            let range = tok.text_range();
            let (start, end) = (usize::from(range.start()), usize::from(range.end()));
            let kind = tok.kind();
            let parent = parent_kind(&tok);
            let closes_selection =
                kind == SyntaxKind::R_CURLY && parent == Some(SyntaxKind::SELECTION_SET);
            if closes_selection {
                indent = indent.saturating_sub(1);
            }
            match &prev {
                None => {}
                Some(_) if self.definitions.contains(&start) => out.push_str("\n\n"),
                Some(p) => {
                    let own_line = p.kind() == SyntaxKind::COMMENT
                        || closes_selection
                        || self.selections.contains(&start)
                        || (kind == SyntaxKind::COMMENT && self.newline_before(p, start));
                    if own_line {
                        out.push('\n');
                        out.push_str(&"  ".repeat(indent));
                    } else if self.later_items.contains(&start) {
                        out.push_str(", ");
                    } else if spaced(p, &tok) {
                        out.push(' ');
                    }
                }
            }
            let source = &self.text[start..end];
            if kind == SyntaxKind::COMMENT {
                out.push_str(source.trim_end());
            } else {
                out.push_str(source);
            }
            if kind == SyntaxKind::L_CURLY && parent == Some(SyntaxKind::SELECTION_SET) {
                indent += 1;
            }
            prev = Some(tok);
        }
        out.push('\n');
        out
    }

    /// True when the source has a line break between `prev` and the token at `start`.
    fn newline_before(&self, prev: &SyntaxToken, start: usize) -> bool {
        let gap = usize::from(prev.text_range().end())..start;
        self.text[gap].contains('\n')
    }
}

/// Whether one space separates two tokens printed on the same line.
fn spaced(prev: &SyntaxToken, cur: &SyntaxToken) -> bool {
    let open = match prev.kind() {
        SyntaxKind::L_PAREN | SyntaxKind::L_BRACK | SyntaxKind::DOLLAR | SyntaxKind::AT => true,
        SyntaxKind::L_CURLY => parent_kind(prev) == Some(SyntaxKind::OBJECT_VALUE),
        SyntaxKind::SPREAD => parent_kind(prev) == Some(SyntaxKind::FRAGMENT_SPREAD),
        _ => false,
    };
    let close = match cur.kind() {
        SyntaxKind::R_PAREN
        | SyntaxKind::R_BRACK
        | SyntaxKind::COLON
        | SyntaxKind::BANG
        | SyntaxKind::L_PAREN => true,
        SyntaxKind::R_CURLY => parent_kind(cur) == Some(SyntaxKind::OBJECT_VALUE),
        _ => false,
    };
    !open && !close
}
