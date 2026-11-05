//! Find in the response body: the key context, the action its chord dispatches, and the
//! matcher for long bodies shown as rows.

use std::ops::Range;

use crate::body_text::Chunked;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{App, Hsla, Rgba, SharedString, actions};

/// The response body's key context.
pub const KEY_CONTEXT: &str = "ResponseBody";

actions!(
    apinata,
    [
        /// Opens the find bar over the response body, or refocuses its field.
        FindInResponse,
        /// Steps the response body's find to the next match.
        FindNextInResponse,
        /// Steps the response body's find to the previous match.
        FindPreviousInResponse,
        /// Closes the response body's find bar; with no bar open the key passes on.
        CloseFindInResponse,
        /// Switches the open find bar between literal and regular-expression queries.
        ToggleFindRegex,
        /// Switches the open find bar's whole-word matching.
        ToggleFindWholeWord,
        /// Moves the rows list's current row up one.
        RowUp,
        /// Moves the rows list's current row down one.
        RowDown,
        /// Moves the rows list's current row up a screenful.
        RowPageUp,
        /// Moves the rows list's current row down a screenful.
        RowPageDown,
        /// Moves the rows list's current row to the first row.
        RowFirst,
        /// Moves the rows list's current row to the last row.
        RowLast
    ]
);

/// The find bar's toggles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindOptions {
    /// Letters match only in the query's case.
    pub case_sensitive: bool,
    /// The query is an ECMAScript pattern.
    pub regex: bool,
    /// A hit counts only between non-word characters; ignored while `regex` is on.
    pub whole_word: bool,
}

/// The find query is a pattern that does not compile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidPattern;

/// A find query compiled under its options, searched one line at a time.
#[derive(Debug)]
pub struct Finder(Search);

#[derive(Debug)]
enum Search {
    Literal {
        needle: String,
        case_sensitive: bool,
    },
    Pattern(regress::Regex),
}

// An ASCII word character, as ECMAScript's `\w` and `\b` count them.
const NOT_WORD_BEFORE: &str = "(?<![A-Za-z0-9_])";
const NOT_WORD_AFTER: &str = "(?![A-Za-z0-9_])";

impl Finder {
    /// Compiles `query`: `Ok(None)` for an empty query, `Err` for a regex that does not
    /// compile. Whole-word wraps the escaped query in non-word lookarounds.
    pub fn new(query: &str, options: FindOptions) -> Result<Option<Self>, InvalidPattern> {
        if query.is_empty() {
            return Ok(None);
        }
        let pattern = if options.regex {
            query.to_owned()
        } else if options.whole_word {
            format!(
                "{NOT_WORD_BEFORE}{}{NOT_WORD_AFTER}",
                regress::escape(query)
            )
        } else {
            return Ok(Some(Finder(Search::Literal {
                needle: fold(query, options.case_sensitive),
                case_sensitive: options.case_sensitive,
            })));
        };
        let flags = if options.case_sensitive { "" } else { "i" };
        regress::Regex::with_flags(&pattern, flags)
            .map(|re| Some(Finder(Search::Pattern(re))))
            .map_err(|_| InvalidPattern)
    }

    /// Every non-empty, non-overlapping hit in `line`, left to right, as byte ranges.
    /// Empty-width pattern hits are dropped.
    pub fn find_in_line(&self, line: &str) -> Vec<Range<usize>> {
        match &self.0 {
            Search::Literal {
                needle,
                case_sensitive,
            } => fold(line, *case_sensitive)
                .match_indices(needle.as_str())
                .map(|(start, _)| start..start + needle.len())
                .collect(),
            Search::Pattern(re) => re
                .find_iter(line)
                .map(|m| m.range())
                .filter(|r| !r.is_empty())
                .collect(),
        }
    }

    /// [`Finder::find_in_line`] over each `\n`-separated line of `text`, as byte ranges into
    /// it, so `^` and `$` anchor at every line.
    pub fn find_in_text(&self, text: &str) -> Vec<Range<usize>> {
        let mut out = Vec::new();
        let mut at = 0;
        for line in text.split('\n') {
            out.extend(
                self.find_in_line(line)
                    .into_iter()
                    .map(|r| r.start + at..r.end + at),
            );
            at += line.len() + 1;
        }
        out
    }
}

/// Every non-overlapping occurrence of `query` in each row, left to right, as (row, byte range);
/// without `case_sensitive` letters fold case where the lowercase keeps the char's byte
/// length. An empty query matches nothing.
pub fn find_matches(
    rows: &[SharedString],
    query: &str,
    case_sensitive: bool,
) -> Vec<(usize, Range<usize>)> {
    if query.is_empty() {
        return Vec::new();
    }
    let needle = fold(query, case_sensitive);
    let mut matches = Vec::new();
    for (ix, row) in rows.iter().enumerate() {
        let hay = fold(row, case_sensitive);
        matches.extend(
            hay.match_indices(needle.as_str())
                .map(|(start, _)| (ix, start..start + needle.len())),
        );
    }
    matches
}

// Each char folds only to a lowercase of the same UTF-8 length, so every byte stays where
// it was and ranges in the folded text hold in the original; other chars compare exactly.
fn fold(s: &str, case_sensitive: bool) -> String {
    if case_sensitive {
        return s.to_owned();
    }
    s.chars()
        .map(|c| {
            let mut lower = c.to_lowercase();
            match (lower.next(), lower.next()) {
                (Some(l), None) if l.len_utf8() == c.len_utf8() => l,
                _ => c,
            }
        })
        .collect()
}

/// The matches of a long body: each match once, with its first row and range, plus the
/// pieces of a match that runs on into later rows of the same line.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RowMatches {
    /// Per match, its row and byte range there (clipped to that row), in order.
    pub matches: Vec<(usize, Range<usize>)>,
    /// Per match that crosses a chunk seam, (match index, row, range) of each later piece.
    pub tails: Vec<(usize, usize, Range<usize>)>,
}

/// Like [`find_matches`] over `chunked`, but a line cut into several rows is searched whole,
/// so a match straddling a seam is found once.
pub fn find_in_chunked(chunked: &Chunked, query: &str, case_sensitive: bool) -> RowMatches {
    let options = FindOptions {
        case_sensitive,
        ..FindOptions::default()
    };
    match Finder::new(query, options) {
        Ok(Some(finder)) => find_in_chunked_with(chunked, &finder),
        _ => RowMatches::default(),
    }
}

/// Like [`find_in_chunked`], with a compiled [`Finder`].
pub fn find_in_chunked_with(chunked: &Chunked, finder: &Finder) -> RowMatches {
    let mut out = RowMatches::default();
    let rows = &chunked.rows;
    let mut first = 0;
    while first < rows.len() {
        let mut end = first + 1;
        while chunked.line_nos.get(end).is_some_and(|n| *n == 0) {
            end += 1;
        }
        let hay = rows[first..end]
            .iter()
            .map(|r| r.as_ref())
            .collect::<String>();
        // Where each row of the line starts within it.
        let mut starts = Vec::with_capacity(end - first + 1);
        let mut at = 0;
        for row in &rows[first..end] {
            starts.push(at);
            at += row.len();
        }
        starts.push(at);
        for Range { start, end: stop } in finder.find_in_line(&hay) {
            let piece =
                |k: usize| starts[k].max(start) - starts[k]..starts[k + 1].min(stop) - starts[k];
            let k = starts.partition_point(|s| *s <= start) - 1;
            let k = k.min(end - first - 1);
            out.matches.push((first + k, piece(k)));
            let index = out.matches.len() - 1;
            let later = (k + 1..end - first).take_while(|l| starts[*l] < stop);
            out.tails
                .extend(later.map(|l| (index, first + l, piece(l))));
        }
        first = end;
    }
    out
}

/// The selection colour of the response body editor. The kit paints every find match in it
/// at saturation 0.1 and the current match in it again on top, so it is the theme's primary
/// at an alpha that keeps both visible over the editor and the text readable.
pub fn find_selection(cx: &App) -> Hsla {
    let theme = cx.theme();
    let alpha = if theme.mode.is_dark() { 0.45 } else { 0.3 };
    theme.primary.opacity(alpha)
}

/// The (current, other) match backgrounds for rows, which take one background per range:
/// the kit's two paints composed into one.
pub fn match_colours(cx: &App) -> (Hsla, Hsla) {
    let selection = find_selection(cx);
    let other = Hsla {
        s: 0.1,
        ..selection
    };
    let (top, bottom) = (selection.to_rgb(), other.to_rgb());
    let a = top.a + bottom.a * (1. - top.a);
    let mix = |t: f32, b: f32| (t * top.a + b * bottom.a * (1. - top.a)) / a;
    let current = Rgba {
        r: mix(top.r, bottom.r),
        g: mix(top.g, bottom.g),
        b: mix(top.b, bottom.b),
        a,
    };
    (current.into(), other)
}
