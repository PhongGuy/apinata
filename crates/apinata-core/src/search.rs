//! Text search over a collection's loaded requests.

use crate::collection::{Collection, Folder, Sibling};
use crate::edit::{BodySpec, MultipartValue, ParamKind};
use crate::item_path::ItemPath;
use crate::paging::{MAX_PAGE_LIMIT, PageError, effective_limit};
use crate::request::RequestFile;
use crate::script::ScriptPhase;
use regress::Regex;
use std::collections::HashMap;
use std::ops::Range;

/// The most hits one search returns.
pub const SEARCH_HIT_CAP: usize = 500;

/// Which part of a request a hit was found in. Authentication is deliberately absent: its
/// secret fields are never searched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SearchField {
    /// The request's name.
    Name,
    /// The request's URL.
    Url,
    /// A path param's name or value; query params are part of the URL.
    PathParam,
    /// A header's name; disabled headers are searched too.
    HeaderName,
    /// A header's value; disabled headers are searched too.
    HeaderValue,
    /// Body text, form field names and values, or GraphQL query and variables.
    Body,
    /// A script phase's code.
    Script,
    /// The request's documentation text.
    Docs,
}

impl SearchField {
    /// The field's snake_case name, as the CLI and MCP print it.
    pub fn as_str(self) -> &'static str {
        match self {
            SearchField::Name => "name",
            SearchField::Url => "url",
            SearchField::PathParam => "path_param",
            SearchField::HeaderName => "header_name",
            SearchField::HeaderValue => "header_value",
            SearchField::Body => "body",
            SearchField::Script => "script",
            SearchField::Docs => "docs",
        }
    }
}

/// One match in one field of one request.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SearchHit {
    /// The request's path in the collection.
    pub path: ItemPath,
    /// The request's name.
    pub request_name: String,
    /// Where the match is.
    pub field: SearchField,
    /// The header, param or form field name, the script phase or the GraphQL part; `None` when
    /// the field has no such label.
    pub key: Option<String>,
    /// Which of the request's fields sharing this `field` and `key` this is, counting from 0 in
    /// walk order and counting every such field, matching or not.
    pub occurrence: usize,
    /// One line of the field's text around the first match.
    pub snippet: String,
    /// The matched text within `snippet`, as byte offsets on char boundaries.
    pub match_range: Range<usize>,
}

/// The hits of one search.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SearchResults {
    /// The hits in tree order, then field order.
    pub hits: Vec<SearchHit>,
    /// More hits existed than [`SEARCH_HIT_CAP`].
    pub capped: bool,
}

/// One page of a search's hits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SearchPage {
    /// The page's hits, in tree order then field order.
    pub hits: Vec<SearchHit>,
    /// How many hits the whole collection holds, past [`SEARCH_HIT_CAP`] too.
    pub total: usize,
    /// Whether hits follow this page.
    pub has_more: bool,
    /// The page size used, after clamping.
    pub limit: usize,
    /// How many hits were skipped.
    pub offset: usize,
}

/// How a search compares text: the options turned into something that can find a first match.
pub(crate) enum Matcher {
    /// Case-insensitive literal; the needle is already folded.
    Folded(Vec<char>),
    /// Case-sensitive literal.
    Exact(String),
    Pattern(Regex),
}

impl Matcher {
    pub(crate) fn new(query: &str, options: &SearchOptions) -> Result<Self, SearchError> {
        if options.regex {
            let flags = if options.case_sensitive { "" } else { "i" };
            return Regex::with_flags(query, flags)
                .map(Matcher::Pattern)
                .map_err(|e| SearchError::InvalidPattern(e.to_string()));
        }
        let query = query.trim();
        Ok(if options.case_sensitive {
            Matcher::Exact(query.to_owned())
        } else {
            Matcher::Folded(query.chars().flat_map(fold).collect())
        })
    }

    /// Nothing can match: a blank literal, or an empty pattern.
    pub(crate) fn is_blank(&self) -> bool {
        match self {
            Matcher::Folded(n) => n.is_empty(),
            Matcher::Exact(n) => n.is_empty(),
            Matcher::Pattern(_) => false,
        }
    }

    /// The non-overlapping literal matches in one pass; only the first when `first_only`.
    fn find_literal(&self, text: &str, first_only: bool) -> Vec<Range<usize>> {
        let limit = if first_only { 1 } else { usize::MAX };
        match self {
            Matcher::Folded(needle) => find_fold_all(text, needle, limit),
            Matcher::Exact(needle) if !needle.is_empty() => text
                .match_indices(needle.as_str())
                .map(|(i, m)| i..i + m.len())
                .take(limit)
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The byte range of the first non-empty match; empty regex matches never count.
    fn find(&self, text: &str) -> Option<Range<usize>> {
        match self {
            Matcher::Folded(needle) => find_fold_all(text, needle, 1).pop(),
            Matcher::Exact(needle) => text
                .find(needle.as_str())
                .map(|i| i..i + needle.len())
                .filter(|r| !r.is_empty()),
            Matcher::Pattern(re) => re
                .find_iter(text)
                .map(|m| m.range())
                .find(|r| !r.is_empty()),
        }
    }
}

impl Matcher {
    /// `text` with its non-empty matches replaced, and how many were; `None` when nothing
    /// matched. Regex replacements expand `$n`, `${n}`, `$&` and `$$`; literal ones are used
    /// as written. `first_only` stops after the first match.
    pub(crate) fn replace(
        &self,
        text: &str,
        replacement: &str,
        first_only: bool,
    ) -> Option<(String, usize)> {
        let mut spans: Vec<(Range<usize>, String)> = Vec::new();
        if let Matcher::Pattern(re) = self {
            for m in re.find_iter(text).filter(|m| !m.range().is_empty()) {
                spans.push((m.range(), expand(replacement, text, &m)));
                if first_only {
                    break;
                }
            }
        } else {
            for r in self.find_literal(text, first_only) {
                spans.push((r, replacement.to_owned()));
            }
        }
        if spans.is_empty() {
            return None;
        }
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for (range, with) in &spans {
            out.push_str(&text[last..range.start]);
            out.push_str(with);
            last = range.end;
        }
        out.push_str(&text[last..]);
        Some((out, spans.len()))
    }
}

/// Expand `$1`..`$99`, `${n}`, `$&` and `$$` in `template` against one regex match. A group
/// that does not exist or did not take part expands to nothing; any other `$` stays literal.
fn expand(template: &str, text: &str, m: &regress::Match) -> String {
    let group = |n: usize| m.group(n).map_or("", |r| &text[r]);
    let groups = m.captures.len();
    let mut out = String::new();
    let mut rest = template;
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        rest = &rest[i + 1..];
        let digits = rest.bytes().take(2).take_while(u8::is_ascii_digit).count();
        if let Some(tail) = rest.strip_prefix('$') {
            out.push('$');
            rest = tail;
        } else if let Some(tail) = rest.strip_prefix('&') {
            out.push_str(group(0));
            rest = tail;
        } else if let Some((n, tail)) = braced(rest) {
            out.push_str(group(n));
            rest = tail;
        } else if digits == 2 && rest[..2].parse::<usize>().is_ok_and(|n| n <= groups) {
            out.push_str(group(rest[..2].parse().unwrap_or(0)));
            rest = &rest[2..];
        } else if digits > 0 {
            out.push_str(group(usize::from(rest.as_bytes()[0] - b'0')));
            rest = &rest[1..];
        } else {
            out.push('$');
        }
    }
    out.push_str(rest);
    out
}

/// A leading `{digits}` group reference and what follows it.
fn braced(s: &str) -> Option<(usize, &str)> {
    let inner = s.strip_prefix('{')?;
    let end = inner.find('}')?;
    let digits = &inner[..end];
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((digits.parse().unwrap_or(usize::MAX), &inner[end + 1..]))
}

/// Options for [`Collection::search_with`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SearchOptions {
    /// Match letter case exactly; off by default.
    pub case_sensitive: bool,
    /// Treat the query as an ECMAScript regular expression; off by default.
    pub regex: bool,
    /// Hits to skip; read by [`Collection::search_page`] only.
    pub offset: usize,
    /// Page size for [`Collection::search_page`]; unset means [`MAX_PAGE_LIMIT`].
    pub limit: Option<usize>,
}

impl SearchOptions {
    /// Set how many hits a page skips.
    pub fn with_offset(mut self, offset: usize) -> Self {
        self.offset = offset;
        self
    }

    /// Set the page size.
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Set whether letter case must match.
    pub fn with_case_sensitive(mut self, on: bool) -> Self {
        self.case_sensitive = on;
        self
    }

    /// Set whether the query is a regular expression.
    pub fn with_regex(mut self, on: bool) -> Self {
        self.regex = on;
        self
    }
}

/// Why a search could not run.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SearchError {
    /// The regular expression did not compile; carries the engine's message.
    #[error("invalid regular expression: {0}")]
    InvalidPattern(String),
    /// The page request was refused.
    #[error(transparent)]
    Page(#[from] PageError),
}

/// Characters of context kept before and after a match in a snippet.
const BEFORE: usize = 40;
const AFTER: usize = 80;

/// Collects hits for one query and refuses the one past the cap.
struct Sink {
    matcher: Matcher,
    out: SearchResults,
    /// Fields seen so far in the current request, per field and key, for `occurrence`.
    seen: HashMap<(SearchField, Option<String>), usize>,
    /// The hit positions to keep when paging; every hit is then counted and none stops the walk.
    window: Option<Range<usize>>,
    total: usize,
}

/// The request being searched, so each hit carries its identity.
struct Subject<'a> {
    path: &'a ItemPath,
    name: &'a str,
}

impl Sink {
    /// Record a hit when `text` holds the query; false once the cap is reached.
    fn offer(
        &mut self,
        who: &Subject<'_>,
        field: SearchField,
        key: Option<&str>,
        text: &str,
    ) -> bool {
        let slot = self
            .seen
            .entry((field, key.map(str::to_owned)))
            .or_default();
        let occurrence = *slot;
        *slot += 1;
        let Some(found) = self.matcher.find(text) else {
            return true;
        };
        if let Some(window) = &self.window {
            let index = self.total;
            self.total += 1;
            if !window.contains(&index) {
                return true;
            }
        } else if self.out.hits.len() >= SEARCH_HIT_CAP {
            self.out.capped = true;
            return false;
        }
        let (snippet, match_range) = snippet_around(text, found);
        self.out.hits.push(SearchHit {
            path: who.path.clone(),
            request_name: who.name.to_owned(),
            field,
            key: key.map(str::to_owned),
            occurrence,
            snippet,
            match_range,
        });
        true
    }

    fn request(&mut self, who: &Subject<'_>, r: &RequestFile) -> bool {
        use SearchField::*;
        self.seen.clear();
        let mut ok = self.offer(who, Name, None, r.name()) && self.offer(who, Url, None, r.url());
        for p in r.params().iter().filter(|p| p.kind == ParamKind::Path) {
            ok = ok
                && self.offer(who, PathParam, Some(&p.name), &p.name)
                && self.offer(who, PathParam, Some(&p.name), &p.value);
        }
        for h in r.headers() {
            ok = ok
                && self.offer(who, HeaderName, Some(&h.name), &h.name)
                && self.offer(who, HeaderValue, Some(&h.name), &h.value);
        }
        ok = ok && self.body(who, &r.body());
        for phase in [
            ScriptPhase::PreRequest,
            ScriptPhase::PostResponse,
            ScriptPhase::Tests,
        ] {
            ok = ok && self.offer(who, Script, Some(phase.as_str()), &r.script(phase));
        }
        ok && self.offer(who, Docs, None, &r.docs())
    }

    fn body(&mut self, who: &Subject<'_>, body: &BodySpec) -> bool {
        use SearchField::Body;
        match body {
            BodySpec::Json(t) | BodySpec::Xml(t) | BodySpec::Text(t) | BodySpec::Sparql(t) => {
                self.offer(who, Body, None, t)
            }
            BodySpec::FormUrlEncoded(fields) => fields.iter().all(|f| {
                self.offer(who, Body, Some(&f.name), &f.name)
                    && self.offer(who, Body, Some(&f.name), &f.value)
            }),
            BodySpec::Multipart(fields) => fields.iter().all(|f| {
                let text = match &f.value {
                    MultipartValue::Text(t) => t.as_str(),
                    _ => "",
                };
                self.offer(who, Body, Some(&f.name), &f.name)
                    && self.offer(who, Body, Some(&f.name), text)
            }),
            BodySpec::Graphql { query, variables } => {
                self.offer(who, Body, Some("query"), query)
                    && self.offer(who, Body, Some("variables"), variables)
            }
            _ => true,
        }
    }

    /// Walk folders before requests, as the sidebar lists them; false once the cap is hit.
    fn folder(&mut self, c: &Collection, folder: &Folder) -> bool {
        for item in folder.siblings() {
            let ok = match item {
                Sibling::Folder(f) => self.folder(c, f),
                Sibling::Request(r) => match c.item_path(r.path()) {
                    Some(path) => {
                        let who = Subject {
                            path: &path,
                            name: r.name(),
                        };
                        self.request(&who, r)
                    }
                    None => true,
                },
                _ => true,
            };
            if !ok {
                return false;
            }
        }
        true
    }
}

/// The lowercase form of `c`, which may be several chars (or one for most text).
fn fold(c: char) -> impl Iterator<Item = char> {
    c.to_lowercase()
}

/// The byte ranges of up to `limit` non-overlapping case-insensitive occurrences of `needle`
/// (already folded) in `text`. Matching runs on folded chars, built once, and maps back to the
/// original char boundaries.
fn find_fold_all(text: &str, needle: &[char], limit: usize) -> Vec<Range<usize>> {
    let mut folded: Vec<(char, usize, usize)> = Vec::new();
    for (start, c) in text.char_indices() {
        let end = start + c.len_utf8();
        folded.extend(fold(c).map(|f| (f, start, end)));
    }
    let mut out: Vec<Range<usize>> = Vec::new();
    if needle.is_empty() || folded.len() < needle.len() {
        return out;
    }
    let mut i = 0;
    while i + needle.len() <= folded.len() && out.len() < limit {
        let window = &folded[i..i + needle.len()];
        let start = window[0].1;
        // A fold that spans one original char could share it with the previous match.
        if window.iter().map(|f| f.0).eq(needle.iter().copied())
            && out.last().is_none_or(|r| r.end <= start)
        {
            out.push(start..window[needle.len() - 1].2);
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}

/// A single-line window of `text` around `found`, whitespace runs collapsed to one space, with
/// the match range translated into the window. Cut edges are marked with an ellipsis.
fn snippet_around(text: &str, found: Range<usize>) -> (String, Range<usize>) {
    let start = text[..found.start]
        .char_indices()
        .rev()
        .nth(BEFORE - 1)
        .map_or(0, |(i, _)| i);
    let end = text[found.end..]
        .char_indices()
        .nth(AFTER)
        .map_or(text.len(), |(i, _)| found.end + i);
    let mut out = String::new();
    if start > 0 {
        out.push('\u{2026}');
    }
    let (mut from, mut to) = (None, None);
    for (i, c) in text[start..end].char_indices().map(|(i, c)| (i + start, c)) {
        if i == found.start {
            from = Some(out.len());
        }
        if i == found.end {
            to = Some(out.len());
        }
        if c.is_whitespace() {
            if !out.ends_with(' ') {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
    }
    let from = from.unwrap_or(out.len());
    let to = to.unwrap_or(out.len()).max(from);
    if end < text.len() {
        out.push('\u{2026}');
    }
    (out, from..to)
}

impl Collection {
    /// Search the loaded requests for `query`, case-insensitively, in tree order and then field
    /// order. It reads only text held in the request files themselves: variables stay as
    /// `{{name}}` and secret values, `.env` and authentication are never consulted. Unsaved
    /// edits held in memory count. A blank query finds nothing.
    pub fn search(&self, query: &str) -> SearchResults {
        self.search_with(query, &SearchOptions::default())
            .unwrap_or_default()
    }

    /// [`search`](Self::search) with case and regex options. Fails only when regex mode is on
    /// and `query` is not a valid pattern. Regex matches of zero length are not hits.
    pub fn search_with(
        &self,
        query: &str,
        options: &SearchOptions,
    ) -> Result<SearchResults, SearchError> {
        let mut sink = Sink {
            matcher: Matcher::new(query, options)?,
            out: SearchResults::default(),
            seen: HashMap::new(),
            window: None,
            total: 0,
        };
        if !sink.matcher.is_blank() {
            sink.folder(self, self.tree());
        }
        Ok(sink.out)
    }

    /// One page of the hits [`search_with`](Self::search_with) would find, from
    /// `options.offset`, at most `options.limit` (clamped to [`MAX_PAGE_LIMIT`]; unset means
    /// that maximum). The whole tree is walked so `total` counts past [`SEARCH_HIT_CAP`]. An
    /// offset past the end is an empty page; a zero limit is refused.
    pub fn search_page(
        &self,
        query: &str,
        options: &SearchOptions,
    ) -> Result<SearchPage, SearchError> {
        let limit = effective_limit(options.limit)?.unwrap_or(MAX_PAGE_LIMIT);
        let start = options.offset;
        let mut sink = Sink {
            matcher: Matcher::new(query, options)?,
            out: SearchResults::default(),
            seen: HashMap::new(),
            window: Some(start..start.saturating_add(limit)),
            total: 0,
        };
        if !sink.matcher.is_blank() {
            sink.folder(self, self.tree());
        }
        Ok(SearchPage {
            hits: sink.out.hits,
            total: sink.total,
            has_more: start.saturating_add(limit) < sink.total,
            limit,
            offset: start,
        })
    }
}
