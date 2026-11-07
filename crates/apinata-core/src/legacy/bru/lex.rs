//! The lexical layer shared by every `.bru` grammar: a backtracking cursor that tracks the
//! rightmost failure the way ohm does, the dictionary, list, multiline and text-block rules,
//! and the block-sequence start rule. Each rule is a transcription of the reference grammar.

use super::BruError;
use std::borrow::Cow;
use std::sync::OnceLock;

/// One `@name(arg)` decorator line above a pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Annotation {
    pub(crate) name: String,
    pub(crate) value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PairValue {
    Text(String),
    List(Vec<String>),
}

/// One `key: value` line of a dictionary, after the reference's `pair` semantic action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Pair {
    pub(crate) key: String,
    pub(crate) value: PairValue,
    pub(crate) annotations: Vec<Annotation>,
}

/// How a block's body is shaped after its keyword.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockKind {
    Dictionary,
    AssertDictionary,
    Text,
    /// `st* "{" nl* (~tagend any)* tagend`: the raw content, parsed later by its own grammar.
    Example,
}

/// The grammar whose dictionary rules apply. The collection grammar allows no spaces after
/// `{` and has no `@contentType(...)` suffix on a multiline value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dialect {
    Request,
    Collection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BlockBody {
    Pairs(Vec<Pair>),
    /// The text block's source, before `outdentString`.
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Block {
    pub(crate) keyword: &'static str,
    /// The 1-based line of the keyword, for errors raised after the parse.
    pub(crate) line: usize,
    pub(crate) body: BlockBody,
}

/// A PEG cursor over the input. A failed terminal records its start offset unless it ran
/// inside a lookahead or space skipping; the error line is taken from the rightmost one.
pub(crate) struct Cursor<'a> {
    src: &'a str,
    pos: usize,
    furthest: usize,
    quiet: u32,
}

impl<'a> Cursor<'a> {
    pub(super) fn new(src: &'a str) -> Self {
        Cursor {
            src,
            pos: 0,
            furthest: 0,
            quiet: 0,
        }
    }

    fn fail(&mut self, at: usize) -> bool {
        if self.quiet == 0 && at > self.furthest {
            self.furthest = at;
        }
        false
    }

    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    pub(super) fn lit(&mut self, text: &str) -> bool {
        if self.src[self.pos..].starts_with(text) {
            self.pos += text.len();
            true
        } else {
            self.fail(self.pos)
        }
    }

    fn char_if(&mut self, accept: impl Fn(char) -> bool) -> bool {
        match self.peek() {
            Some(c) if accept(c) => {
                self.pos += c.len_utf8();
                true
            }
            _ => self.fail(self.pos),
        }
    }

    pub(super) fn any(&mut self) -> bool {
        self.char_if(|_| true)
    }

    /// `~X`: succeeds without consuming when `X` fails; a matching `X` is the failure.
    pub(super) fn not(&mut self, rule: impl FnOnce(&mut Self) -> bool) -> bool {
        let start = self.pos;
        self.quiet += 1;
        let matched = rule(self);
        self.quiet -= 1;
        self.pos = start;
        if matched { self.fail(start) } else { true }
    }

    /// Runs a sequence, rewinding to the start when it fails.
    pub(super) fn seq(&mut self, rule: impl FnOnce(&mut Self) -> bool) -> bool {
        let start = self.pos;
        let ok = rule(self);
        if !ok {
            self.pos = start;
        }
        ok
    }

    pub(super) fn attempt<T>(&mut self, rule: impl FnOnce(&mut Self) -> Option<T>) -> Option<T> {
        let start = self.pos;
        let out = rule(self);
        if out.is_none() {
            self.pos = start;
        }
        out
    }

    /// `X*`, each iteration rewinding on failure; returns how many matched.
    pub(super) fn star(&mut self, mut rule: impl FnMut(&mut Self) -> bool) -> usize {
        let mut count = 0;
        while self.seq(&mut rule) {
            count += 1;
        }
        count
    }

    pub(super) fn plus(&mut self, rule: impl FnMut(&mut Self) -> bool) -> bool {
        self.star(rule) > 0
    }

    fn since(&self, start: usize) -> &'a str {
        &self.src[start..self.pos]
    }

    /// Runs `rule` and returns the text it consumed.
    pub(super) fn capture(&mut self, rule: impl FnOnce(&mut Self)) -> &'a str {
        let start = self.pos;
        rule(self);
        self.since(start)
    }

    /// The syntactic start rule's implicit skip of `"\x00".." "`, whose failures never count.
    pub(super) fn skip_spaces(&mut self) {
        self.quiet += 1;
        self.star(space);
        self.quiet -= 1;
    }

    /// The start rule's end: trailing spaces, then end of input or the rightmost failure.
    pub(super) fn expect_end(&mut self) -> Result<(), BruError> {
        self.skip_spaces();
        if self.pos < self.src.len() {
            self.fail(self.pos);
            return Err(self.error());
        }
        Ok(())
    }

    fn error(&self) -> BruError {
        let line = 1 + self.src[..self.furthest].matches('\n').count();
        let message = match self.src[self.furthest..].chars().next() {
            Some(c) => format!("unexpected \"{}\"", c.escape_debug()),
            None => "unexpected end of file".to_owned(),
        };
        BruError { line, message }
    }
}

pub(super) fn space(c: &mut Cursor) -> bool {
    c.char_if(|ch| ch <= ' ')
}

pub(super) fn nl(c: &mut Cursor) -> bool {
    c.seq(|c| {
        c.lit("\r");
        c.lit("\n")
    })
}

pub(super) fn st(c: &mut Cursor) -> bool {
    c.lit(" ") || c.lit("\t")
}

pub(super) fn stnl(c: &mut Cursor) -> bool {
    st(c) || nl(c)
}

pub(super) fn tagend(c: &mut Cursor) -> bool {
    c.seq(|c| nl(c) && c.lit("}"))
}

pub(super) fn optionalnl(c: &mut Cursor) -> bool {
    c.not(tagend) && nl(c)
}

fn keychar(c: &mut Cursor) -> bool {
    c.not(|c| tagend(c) || st(c) || nl(c) || c.lit(":")) && c.any()
}

pub(super) fn valuechar(c: &mut Cursor) -> bool {
    c.not(|c| nl(c) || tagend(c)) && c.any()
}

/// `'''...'''`: returns the raw content between the delimiters.
pub(super) fn triple_quoted<'a>(c: &mut Cursor<'a>) -> Option<&'a str> {
    c.attempt(|c| {
        if !c.lit("'''") {
            return None;
        }
        let start = c.pos;
        c.star(|c| c.not(|c| c.lit("'''")) && c.any());
        let content = c.since(start);
        c.lit("'''").then_some(content)
    })
}

/// `multilinetextblock`'s syntax: the raw content between the delimiters and the
/// `@contentType(...)` suffix, empty when absent.
pub(super) fn multiline_parts<'a>(c: &mut Cursor<'a>) -> Option<(&'a str, &'a str)> {
    c.attempt(|c| {
        let content = triple_quoted(c)?;
        c.star(st);
        let start = c.pos;
        c.seq(|c| {
            c.lit("@contentType(") && {
                c.star(|c| c.not(|c| c.lit(")")) && c.any());
                c.lit(")")
            }
        });
        Some((content, c.since(start)))
    })
}

/// A multiline value drops four characters per line and, in the request dialect, keeps an
/// `@contentType(...)` suffix.
fn multilinetextblock(c: &mut Cursor, dialect: Dialect) -> Option<String> {
    c.attempt(|c| {
        let (content, content_type) = match dialect {
            Dialect::Request => multiline_parts(c)?,
            Dialect::Collection => (triple_quoted(c)?, ""),
        };
        let text = slice_lines(content);
        Some(if content_type.is_empty() {
            text
        } else {
            format!("{text} {content_type}")
        })
    })
}

/// `split('\n').map((line) => line.slice(4)).join('\n')`.
fn slice_lines(content: &str) -> String {
    content
        .split('\n')
        .map(|line| js_slice(line, 4))
        .collect::<Vec<_>>()
        .join("\n")
}

/// JavaScript `String.prototype.slice(n)`: drops `n` UTF-16 code units. A cut inside a
/// surrogate pair leaves a lone low surrogate, which a Rust string cannot hold, so it
/// becomes U+FFFD.
pub(super) fn js_slice(line: &str, units: usize) -> Cow<'_, str> {
    let mut seen = 0;
    for (index, ch) in line.char_indices() {
        if seen >= units {
            return Cow::Borrowed(&line[index..]);
        }
        seen += ch.len_utf16();
        if seen > units {
            return Cow::Owned(format!("\u{fffd}{}", &line[index + ch.len_utf8()..]));
        }
    }
    Cow::Borrowed("")
}

fn annotationchar(c: &mut Cursor) -> bool {
    c.not(|c| {
        ["(", ")", " ", "\t", "\r", "\n", ":"]
            .iter()
            .any(|t| c.lit(t))
    }) && c.any()
}

fn annotation_arg(c: &mut Cursor) -> String {
    if let Some(content) = triple_quoted(c) {
        return outdent_annotation_block(content);
    }
    let quoted = c.attempt(|c| {
        if !c.lit("'") {
            return None;
        }
        let start = c.pos;
        c.star(|c| c.not(|c| c.lit("'")) && c.any());
        let text = c.since(start);
        c.lit("'").then(|| text.to_owned())
    });
    if let Some(text) = quoted {
        return text;
    }
    let double = c.attempt(|c| {
        if !c.lit("\"") {
            return None;
        }
        let start = c.pos;
        c.star(|c| c.seq(|c| c.lit("\\") && c.any()) || (c.not(|c| c.lit("\"")) && c.any()));
        let text = c.since(start);
        c.lit("\"").then(|| unescape_double_quoted(text))
    });
    if let Some(text) = double {
        return text;
    }
    let start = c.pos;
    c.star(|c| c.not(|c| c.lit(")")) && c.any());
    c.since(start).to_owned()
}

fn annotation(c: &mut Cursor) -> Option<Annotation> {
    c.attempt(|c| {
        if !c.lit("@") {
            return None;
        }
        let start = c.pos;
        if !c.plus(annotationchar) {
            return None;
        }
        let name = c.since(start).to_owned();
        let value = c.attempt(|c| {
            if !c.lit("(") {
                return None;
            }
            let value = annotation_arg(c);
            c.lit(")").then_some(value)
        });
        Some(Annotation { name, value })
    })
}

pub(super) fn pairannotations(c: &mut Cursor) -> Vec<Annotation> {
    let mut out = Vec::new();
    while let Some(found) = c.attempt(|c| {
        c.star(st);
        let found = annotation(c)?;
        (c.not(|c| c.lit(":")) && {
            c.star(st);
            nl(c)
        })
        .then_some(found)
    }) {
        out.push(found);
    }
    out
}

fn quoted_key(c: &mut Cursor) -> Option<String> {
    c.attempt(|c| {
        let disabled = c.lit("~");
        if !c.lit("\"") {
            return None;
        }
        let mut key = String::from(if disabled { "~" } else { "" });
        loop {
            if c.seq(|c| c.lit("\\") && c.lit("\"")) {
                key.push('"');
                continue;
            }
            let start = c.pos;
            if c.not(|c| c.lit("\"") || c.seq(|c| c.lit("\\") && c.lit("\"")) || nl(c)) && c.any() {
                key.push_str(c.since(start));
                continue;
            }
            break;
        }
        c.lit("\"").then_some(key)
    })
}

pub(super) fn key(c: &mut Cursor) -> String {
    let start = c.pos;
    c.star(keychar);
    crate::js::js_trim(c.since(start)).to_owned()
}

fn is_alnum(ch: char) -> bool {
    static LETTER: OnceLock<regress::Regex> = OnceLock::new();
    // A fixed pattern, so a failure to compile is a bug every run hits, not an input error.
    #[allow(clippy::expect_used)]
    let letter = LETTER.get_or_init(|| {
        regress::Regex::with_flags(r"^\p{L}$", "u").expect("the letter pattern compiles")
    });
    // ohm matches one UTF-16 unit, and a surrogate half is never a letter.
    ch.len_utf16() == 1
        && (ch.is_ascii_digit() || letter.find(ch.encode_utf8(&mut [0; 4])).is_some())
}

fn list(c: &mut Cursor) -> Option<Vec<String>> {
    c.attempt(|c| {
        c.star(st);
        if !(c.lit("[") && c.plus(nl)) {
            return None;
        }
        let mut items = Vec::new();
        if let Some(first) = listitem(c) {
            items.push(first);
            while let Some(next) = c.attempt(|c| if c.plus(nl) { listitem(c) } else { None }) {
                items.push(next);
            }
        }
        c.star(st);
        if !c.plus(nl) {
            return None;
        }
        c.star(st);
        c.lit("]").then_some(items)
    })
}

fn listitem(c: &mut Cursor) -> Option<String> {
    c.attempt(|c| {
        if !c.plus(st) {
            return None;
        }
        let start = c.pos;
        if !c.plus(|c| c.char_if(is_alnum) || c.lit("_") || c.lit("-")) {
            return None;
        }
        let item = c.since(start).to_owned();
        c.star(st);
        Some(item)
    })
}

/// `value`: a list, a multiline block, or the rest of the line.
fn value(c: &mut Cursor, dialect: Dialect) -> PairValue {
    if let Some(items) = list(c) {
        return PairValue::List(items);
    }
    if let Some(text) = multilinetextblock(c, dialect) {
        return PairValue::Text(text);
    }
    let start = c.pos;
    c.star(valuechar);
    let line = c.since(start);
    // The collection grammar's value action treats any source wrapped in `'''` as a multiline
    // block, so an unclosed `'''` or `''''` on one line strips to nothing.
    if dialect == Dialect::Collection && line.starts_with("'''") && line.ends_with("'''") {
        let inner = &line[3..];
        return PairValue::Text(slice_lines(inner.strip_suffix("'''").unwrap_or(inner)));
    }
    PairValue::Text(crate::js::js_trim(line).to_owned())
}

/// `pair` or `assertpair`, which differ only in the key rule.
fn pair(c: &mut Cursor, dialect: Dialect, assert: bool) -> Option<Pair> {
    c.attempt(|c| {
        c.star(st);
        let annotations = pairannotations(c);
        c.star(st);
        let key = if assert {
            if !c.not(tagend) {
                return None;
            }
            let start = c.pos;
            c.star(|c| c.not(|c| tagend(c) || nl(c) || c.lit(":")) && c.any());
            crate::js::js_trim(c.since(start)).to_owned()
        } else {
            quoted_key(c).unwrap_or_else(|| key(c))
        };
        c.star(st);
        if !c.lit(":") {
            return None;
        }
        c.star(st);
        let value = match value(c, dialect) {
            PairValue::Text(text) => PairValue::Text(crate::js::js_trim(&text).to_owned()),
            list => list,
        };
        c.star(st);
        Some(Pair {
            key,
            value,
            annotations,
        })
    })
}

fn pairlist(c: &mut Cursor, dialect: Dialect, assert: bool) -> Option<Vec<Pair>> {
    c.attempt(|c| {
        c.star(optionalnl);
        let mut pairs = vec![pair(c, dialect, assert)?];
        while let Some(next) = c.attempt(|c| {
            if !c.not(tagend) {
                return None;
            }
            c.star(stnl);
            pair(c, dialect, assert)
        }) {
            pairs.push(next);
        }
        c.star(|c| c.not(tagend) && space(c));
        Some(pairs)
    })
}

/// `dictionary` and `assertdictionary`; only the request dialect's `dictionary` allows spaces
/// after `{`.
fn dictionary(c: &mut Cursor, dialect: Dialect, assert: bool) -> Option<Vec<Pair>> {
    c.attempt(|c| {
        c.star(st);
        if !c.lit("{") {
            return None;
        }
        if !assert && dialect == Dialect::Request {
            c.star(st);
        }
        let pairs = pairlist(c, dialect, assert).unwrap_or_default();
        tagend(c).then_some(pairs)
    })
}

/// `st* "{" nl* textblock tagend`, returning the text block's source.
pub(super) fn text_block(c: &mut Cursor) -> Option<String> {
    c.attempt(|c| {
        c.star(st);
        if !c.lit("{") {
            return None;
        }
        c.star(nl);
        let start = c.pos;
        let textline = |c: &mut Cursor| {
            c.star(|c| c.not(nl) && c.any());
        };
        textline(c);
        c.star(|c| {
            c.not(tagend) && nl(c) && {
                textline(c);
                true
            }
        });
        let text = c.since(start).to_owned();
        tagend(c).then_some(text)
    })
}

/// `example`'s body: everything up to the first line that starts with `}`.
fn example_body(c: &mut Cursor) -> Option<String> {
    c.attempt(|c| {
        c.star(st);
        if !c.lit("{") {
            return None;
        }
        c.star(nl);
        let start = c.pos;
        c.star(|c| c.not(tagend) && c.any());
        let content = c.since(start).to_owned();
        tagend(c).then_some(content)
    })
}

/// The example sub-grammars' `dictionary`: no annotations, no space after `{`, and exactly
/// one line break between pairs.
pub(super) fn example_dictionary(c: &mut Cursor) -> Option<Vec<Pair>> {
    c.attempt(|c| {
        c.star(st);
        if !c.lit("{") {
            return None;
        }
        let pairs = c
            .attempt(|c| {
                c.star(optionalnl);
                let mut pairs = vec![example_pair(c)?];
                while let Some(next) =
                    c.attempt(|c| (c.not(tagend) && nl(c)).then(|| example_pair(c)).flatten())
                {
                    pairs.push(next);
                }
                Some(pairs)
            })
            .unwrap_or_default();
        tagend(c).then_some(pairs)
    })
}

fn example_pair(c: &mut Cursor) -> Option<Pair> {
    c.attempt(|c| {
        c.star(st);
        let key = quoted_key(c).unwrap_or_else(|| key(c));
        c.star(st);
        if !c.lit(":") {
            return None;
        }
        c.star(st);
        let value = match value(c, Dialect::Request) {
            PairValue::Text(text) => PairValue::Text(crate::js::js_trim(&text).to_owned()),
            list => list,
        };
        c.star(st);
        Some(Pair {
            key,
            value,
            annotations: Vec::new(),
        })
    })
}

/// The start rule `(block)*` then end of input, trying `table`'s keywords in order.
pub(crate) fn parse_blocks(
    text: &str,
    table: &[(&'static str, BlockKind)],
    dialect: Dialect,
) -> Result<Vec<Block>, BruError> {
    let mut c = Cursor::new(text);
    let mut blocks = Vec::new();
    loop {
        let start = c.pos;
        c.skip_spaces();
        let start_of_block = c.pos;
        let found = table.iter().find_map(|&(keyword, kind)| {
            c.attempt(|c| {
                if !c.lit(keyword) {
                    return None;
                }
                let body = match kind {
                    BlockKind::Dictionary => BlockBody::Pairs(dictionary(c, dialect, false)?),
                    BlockKind::AssertDictionary => BlockBody::Pairs(dictionary(c, dialect, true)?),
                    BlockKind::Text => BlockBody::Text(text_block(c)?),
                    BlockKind::Example => BlockBody::Text(example_body(c)?),
                };
                let line = 1 + text[..start_of_block].matches('\n').count();
                Some(Block {
                    keyword,
                    line,
                    body,
                })
            })
        });
        match found {
            Some(block) => blocks.push(block),
            None => {
                c.pos = start;
                break;
            }
        }
    }
    c.expect_end()?;
    Ok(blocks)
}

/// The reference `outdentString`: split on any line break, drop two leading spaces, join `\n`.
pub(crate) fn outdent(text: &str) -> String {
    outdent_by(text, "  ")
}

/// `outdentString(text, n)` with `indent` as the `n` spaces to drop from each line.
pub(super) fn outdent_by(text: &str, indent: &str) -> String {
    split_lines(text)
        .map(|line| line.strip_prefix(indent).unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// JavaScript `split(/\r\n|\r|\n/)`.
pub(super) fn split_lines(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = Some(text);
    std::iter::from_fn(move || {
        let current = rest?;
        match current.find(['\r', '\n']) {
            Some(at) => {
                let skip = if current[at..].starts_with("\r\n") {
                    2
                } else {
                    1
                };
                rest = Some(&current[at + skip..]);
                Some(&current[..at])
            }
            None => {
                rest = None;
                Some(current)
            }
        }
    })
}

/// The reference `parseAnnotationMultilineTextBlock`: drop the blank first and last lines,
/// remove the common indent, and rejoin with the content's own line ending.
fn outdent_annotation_block(content: &str) -> String {
    if !content.contains(['\n', '\r']) {
        return content.to_owned();
    }
    let ending = if content.contains("\r\n") {
        "\r\n"
    } else if content.contains('\r') {
        "\r"
    } else {
        "\n"
    };
    let mut lines: Vec<&str> = split_lines(content).collect();
    if lines.first() == Some(&"") {
        lines.remove(0);
    }
    if lines
        .last()
        .is_some_and(|l| crate::js::js_trim(l).is_empty())
    {
        lines.pop();
    }
    let indent = |l: &str| l.len() - l.trim_start_matches([' ', '\t']).len();
    let min = lines
        .iter()
        .filter(|l| !crate::js::js_trim(l).is_empty())
        .map(|l| indent(l))
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .map(|l| {
            if crate::js::js_trim(l).is_empty() {
                ""
            } else {
                &l[min..]
            }
        })
        .collect::<Vec<_>>()
        .join(ending)
}

/// The reference `unescapeAnnotationDoubleQuotedArg`: `\r \n \t \" \\` only.
pub(super) fn unescape_double_quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        let escaped = match (ch, chars.peek()) {
            ('\\', Some('r')) => Some('\r'),
            ('\\', Some('n')) => Some('\n'),
            ('\\', Some('t')) => Some('\t'),
            ('\\', Some('"')) => Some('"'),
            ('\\', Some('\\')) => Some('\\'),
            _ => None,
        };
        match escaped {
            Some(e) => {
                chars.next();
                out.push(e);
            }
            None => out.push(ch),
        }
    }
    out
}
