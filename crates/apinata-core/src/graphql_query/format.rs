//! `format_query`: prints a query's significant tokens in one canonical layout.

use std::collections::{HashMap, HashSet};

use apollo_parser::cst::CstNode;
use apollo_parser::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};

use super::mask::{is_blank, mask, parse, snap};
use super::{MAX_FORMAT_OUTPUT_BYTES, QUERY_FORMAT_LIMIT, QuerySyntaxError};

/// Formats GraphQL query text into the canonical layout, keeping `{{placeholders}}`,
/// strings and comments as written. Blank input comes back unchanged.
///
/// # Errors
/// [`QuerySyntaxError::TooLarge`] over [`QUERY_FORMAT_LIMIT`] bytes, otherwise
/// [`QuerySyntaxError::Syntax`] for the first parse error, and
/// [`QuerySyntaxError::OutputTooLarge`] when the result would pass [`MAX_FORMAT_OUTPUT_BYTES`].
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
    Printer::new(text, &root).print(&root)
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
/// begins each definition, selection or list item, or opens or closes a list.
struct Printer<'a> {
    text: &'a str,
    definitions: HashSet<usize>,
    selections: HashSet<usize>,
    /// Each list item after the first, mapped to its list's opener.
    later_items: HashMap<usize, usize>,
    /// Openers and closers of lists holding a comment; these print one item per line.
    broken: HashSet<usize>,
}

impl<'a> Printer<'a> {
    fn new(text: &'a str, root: &SyntaxNode) -> Self {
        let mut printer = Self {
            text,
            definitions: HashSet::new(),
            selections: HashSet::new(),
            later_items: HashMap::new(),
            broken: HashSet::new(),
        };
        let mut openers = HashSet::new();
        let mut closers = HashSet::new();
        for node in root.descendants() {
            let starts = node.children().filter_map(|child| first_start(&child));
            match node.kind() {
                SyntaxKind::DOCUMENT => printer.definitions.extend(starts),
                SyntaxKind::SELECTION_SET => printer.selections.extend(starts),
                SyntaxKind::ARGUMENTS
                | SyntaxKind::VARIABLE_DEFINITIONS
                | SyntaxKind::LIST_VALUE
                | SyntaxKind::OBJECT_VALUE => {
                    let mut delimiters = node
                        .children_with_tokens()
                        .filter_map(SyntaxElement::into_token)
                        .filter(|t| is_opener(t.kind()) || is_closer(t.kind()))
                        .map(|t| usize::from(t.text_range().start()));
                    let (Some(open), Some(close)) = (delimiters.next(), delimiters.last()) else {
                        continue;
                    };
                    openers.insert(open);
                    closers.insert(close);
                    printer
                        .later_items
                        .extend(starts.skip(1).map(|item| (item, open)));
                }
                _ => {}
            }
        }
        printer.mark_broken(root, &openers, &closers);
        printer
    }

    /// One pass over the tokens with a stack of open lists: a comment breaks every open
    /// list, marked top down until one already broken (those below it are too).
    fn mark_broken(
        &mut self,
        root: &SyntaxNode,
        openers: &HashSet<usize>,
        closers: &HashSet<usize>,
    ) {
        let mut open: Vec<(usize, bool)> = Vec::new();
        for tok in tokens(root) {
            let start = usize::from(tok.text_range().start());
            if openers.contains(&start) {
                open.push((start, false));
            } else if closers.contains(&start) {
                if let Some((opener, true)) = open.pop() {
                    self.broken.extend([opener, start]);
                }
            } else if tok.kind() == SyntaxKind::COMMENT {
                for entry in open.iter_mut().rev() {
                    if entry.1 {
                        break;
                    }
                    entry.1 = true;
                }
            }
        }
    }

    /// Walks the tokens iteratively, so nesting depth never touches the call stack, and
    /// stops once indentation has grown the output past the cap.
    fn print(&self, root: &SyntaxNode) -> Result<String, QuerySyntaxError> {
        let mut out = String::with_capacity(self.text.len() + self.text.len() / 4);
        let mut indent = 0usize;
        // Open blocks that indent their contents: true for a selection set, false for a
        // broken list.
        let mut blocks: Vec<bool> = Vec::new();
        let mut prev: Option<SyntaxToken> = None;
        let mut after_top_comment = false;
        for tok in tokens(root) {
            let range = tok.text_range();
            let (start, end) = (usize::from(range.start()), usize::from(range.end()));
            let kind = tok.kind();
            let parent = parent_kind(&tok);
            let opens_selection =
                kind == SyntaxKind::L_CURLY && parent == Some(SyntaxKind::SELECTION_SET);
            let closes_selection =
                kind == SyntaxKind::R_CURLY && parent == Some(SyntaxKind::SELECTION_SET);
            let closes_block =
                closes_selection || (is_closer(kind) && self.broken.contains(&start));
            if closes_block {
                indent = indent.saturating_sub(1);
                blocks.pop();
            }
            let top_comment = kind == SyntaxKind::COMMENT && parent == Some(SyntaxKind::DOCUMENT);
            // The first token counts as starting its own line.
            let newlines = prev.as_ref().map_or(1, |p| self.newlines_between(p, start));
            if let Some(p) = &prev {
                if kind == SyntaxKind::COMMENT && newlines == 0 {
                    out.push(' ');
                } else if self.definitions.contains(&start) || top_comment {
                    // Top level: a blank line parts definitions and comment blocks, but a
                    // comment stays with what follows it unless the source parted them.
                    out.push_str(if after_top_comment && newlines < 2 {
                        "\n"
                    } else {
                        "\n\n"
                    });
                    out.push_str(&"  ".repeat(indent));
                } else if self.own_line(p, &tok, start, closes_block) {
                    let in_selection = blocks.last() == Some(&true);
                    let opened = p.kind() == SyntaxKind::L_CURLY
                        && parent_kind(p) == Some(SyntaxKind::SELECTION_SET);
                    let keeps_blank = newlines >= 2
                        && in_selection
                        && !opened
                        && !closes_block
                        && (self.selections.contains(&start) || kind == SyntaxKind::COMMENT);
                    out.push_str(if keeps_blank { "\n\n" } else { "\n" });
                    out.push_str(&"  ".repeat(indent));
                } else if self.later_items.contains_key(&start) {
                    out.push_str(", ");
                } else if spaced(p, &tok) {
                    out.push(' ');
                }
            }
            after_top_comment = top_comment && newlines > 0;
            let source = &self.text[start..end];
            if kind == SyntaxKind::COMMENT {
                out.push_str(source.trim_end());
            } else {
                out.push_str(source);
            }
            if out.len() > MAX_FORMAT_OUTPUT_BYTES {
                return Err(QuerySyntaxError::OutputTooLarge {
                    limit: MAX_FORMAT_OUTPUT_BYTES,
                });
            }
            if opens_selection || (is_opener(kind) && self.broken.contains(&start)) {
                indent += 1;
                blocks.push(opens_selection);
            }
            prev = Some(tok);
        }
        out.push('\n');
        Ok(out)
    }

    /// Whether the token at `start` begins a new line: after a comment or a block opener,
    /// at a selection, a full-line comment, a block closer, or an item of a broken list.
    fn own_line(
        &self,
        prev: &SyntaxToken,
        tok: &SyntaxToken,
        start: usize,
        closes_block: bool,
    ) -> bool {
        let prev_start = usize::from(prev.text_range().start());
        let after_opener = (prev.kind() == SyntaxKind::L_CURLY
            && parent_kind(prev) == Some(SyntaxKind::SELECTION_SET))
            || (is_opener(prev.kind()) && self.broken.contains(&prev_start));
        prev.kind() == SyntaxKind::COMMENT
            || after_opener
            || closes_block
            || tok.kind() == SyntaxKind::COMMENT
            || self.selections.contains(&start)
            || self
                .later_items
                .get(&start)
                .is_some_and(|open| self.broken.contains(open))
    }

    /// Line ends in the source between `prev` and the token at `start`, counted as the
    /// lexer does: `\n`, `\r\n` and a lone `\r` each end one line.
    fn newlines_between(&self, prev: &SyntaxToken, start: usize) -> usize {
        let gap = &self.text.as_bytes()[usize::from(prev.text_range().end())..start];
        gap.iter()
            .enumerate()
            .filter(|&(i, &b)| b == b'\n' || (b == b'\r' && gap.get(i + 1) != Some(&b'\n')))
            .count()
    }
}

/// The significant tokens under `root`, in source order.
fn tokens(root: &SyntaxNode) -> impl Iterator<Item = SyntaxToken> {
    root.descendants_with_tokens()
        .filter_map(SyntaxElement::into_token)
        .filter(significant)
}

fn is_opener(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::L_PAREN | SyntaxKind::L_BRACK | SyntaxKind::L_CURLY
    )
}

fn is_closer(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::R_PAREN | SyntaxKind::R_BRACK | SyntaxKind::R_CURLY
    )
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
