//! Byte-level JavaScript scanning shared by the Postman script translators: identifiers,
//! strings, comments, regexes, templates and brackets.
//!
//! The scanners differ in one rule, whether a `/` after a keyword such as `return` opens a
//! regex. That rule is the `KEYWORDS` parameter; `keyword_regex` and `punct_regex` pin it.

/// Deepest nesting of argument and template rewrites; beyond it text passes through.
pub(crate) const MAX_DEPTH: usize = 32;
/// Longest regex literal recognised; a longer run is read as division.
pub(crate) const MAX_REGEX: usize = 256;

/// Words after which a `/` starts an expression, so it opens a regex.
pub(crate) const REGEX_AFTER: &[&str] = &[
    "return",
    "typeof",
    "case",
    "do",
    "else",
    "in",
    "of",
    "new",
    "delete",
    "void",
    "throw",
    "instanceof",
    "yield",
    "await",
];

/// Scanning where a `/` after a keyword in `REGEX_AFTER` opens a regex.
pub(crate) mod keyword_regex {
    pub(crate) fn skip_opaque(src: &str, i: usize) -> Option<usize> {
        super::skip_opaque::<true>(src, i)
    }
    pub(crate) fn template_end(src: &str, i: usize) -> usize {
        super::template_end::<true>(src, i)
    }
    pub(crate) fn find_close(src: &str, open: usize) -> Option<usize> {
        super::find_close::<true>(src, open)
    }
}

/// Scanning where only punctuation before a `/` makes it a regex, so `return /a/` is division.
pub(crate) mod punct_regex {
    pub(crate) fn skip_opaque(src: &str, i: usize) -> Option<usize> {
        super::skip_opaque::<false>(src, i)
    }
    pub(crate) fn template_end(src: &str, i: usize) -> usize {
        super::template_end::<false>(src, i)
    }
    pub(crate) fn find_close(src: &str, open: usize) -> Option<usize> {
        super::find_close::<false>(src, open)
    }
}

pub(crate) fn is_ident_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c >= 0x80
}

pub(crate) fn ident_end(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && is_ident_byte(b[i]) {
        i += 1;
    }
    i
}

pub(crate) fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// The last non-whitespace byte before `i`, with its index.
pub(crate) fn prev_significant(b: &[u8], i: usize) -> Option<(usize, u8)> {
    let mut k = i;
    while k > 0 {
        k -= 1;
        if !b[k].is_ascii_whitespace() {
            return Some((k, b[k]));
        }
    }
    None
}

/// True when the identifier at `i` is a property (`a.b`, not a spread `...b`).
pub(crate) fn is_property_access(b: &[u8], i: usize) -> bool {
    match prev_significant(b, i) {
        Some((k, b'.')) => !(k >= 2 && b[k - 1] == b'.' && b[k - 2] == b'.'),
        _ => false,
    }
}

/// End of the string, comment, template or regex literal starting at `i`, if one does.
fn skip_opaque<const KEYWORDS: bool>(src: &str, i: usize) -> Option<usize> {
    let b = src.as_bytes();
    match b[i] {
        q @ (b'\'' | b'"') => {
            let mut k = i + 1;
            while k < b.len() && b[k] != q {
                k += if b[k] == b'\\' { 2 } else { 1 };
            }
            Some((k + 1).min(b.len()))
        }
        b'`' => Some(template_end::<KEYWORDS>(src, i)),
        b'/' if b.get(i + 1) == Some(&b'/') => Some(src[i..].find('\n').map_or(b.len(), |n| i + n)),
        b'/' if b.get(i + 1) == Some(&b'*') => {
            Some(src[i + 2..].find("*/").map_or(b.len(), |n| i + 2 + n + 2))
        }
        b'/' => regex_end::<KEYWORDS>(b, i),
        _ => None,
    }
}

/// A `/` starts a regex literal only where an expression may begin.
fn regex_end<const KEYWORDS: bool>(b: &[u8], i: usize) -> Option<usize> {
    let starts_expr = match prev_significant(b, i) {
        None => true,
        Some((k, c)) if KEYWORDS && is_ident_byte(c) => {
            let from = (0..=k)
                .rev()
                .take_while(|&j| is_ident_byte(b[j]))
                .last()
                .unwrap_or(k);
            let word = std::str::from_utf8(&b[from..=k]).unwrap_or("");
            REGEX_AFTER.contains(&word) && !is_property_access(b, from)
        }
        Some((_, c)) => b"(,=:[!&|?{};+-*%<>~^".contains(&c),
    };
    if !starts_expr {
        return None;
    }
    let mut k = i + 1;
    let mut class = false;
    while k < b.len() && b[k] != b'\n' && k - i <= MAX_REGEX {
        match b[k] {
            b'\\' => k += 1,
            b'[' => class = true,
            b']' => class = false,
            b'/' if !class => {
                k += 1;
                while k < b.len() && b[k].is_ascii_alphabetic() {
                    k += 1;
                }
                return Some(k);
            }
            _ => {}
        }
        k += 1;
    }
    None
}

thread_local!(pub(crate) static NESTING: std::cell::Cell<usize> = const { std::cell::Cell::new(0) });

/// One level of template nesting; dropping it, also while unwinding, gives the level back.
pub(crate) struct NestingGuard;

impl NestingGuard {
    pub(crate) fn enter() -> Option<NestingGuard> {
        let depth = NESTING.get();
        (depth < MAX_DEPTH).then(|| {
            NESTING.set(depth + 1);
            NestingGuard
        })
    }
}

impl Drop for NestingGuard {
    fn drop(&mut self) {
        NESTING.set(NESTING.get().saturating_sub(1));
    }
}

/// End of the template literal starting at `i`, descending into `${}` holes.
fn template_end<const KEYWORDS: bool>(src: &str, i: usize) -> usize {
    // Templates nested through `${}` recurse via find_close; past the cap the rest of
    // the input is taken as one opaque template.
    let Some(_level) = NestingGuard::enter() else {
        return src.len();
    };
    template_scan::<KEYWORDS>(src, i)
}

fn template_scan<const KEYWORDS: bool>(src: &str, i: usize) -> usize {
    let b = src.as_bytes();
    let mut k = i + 1;
    while k < b.len() {
        match b[k] {
            b'\\' => k += 2,
            b'`' => return k + 1,
            b'$' if b.get(k + 1) == Some(&b'{') => {
                k = find_close::<KEYWORDS>(src, k + 1).map_or(b.len(), |c| c + 1);
            }
            _ => k += 1,
        }
    }
    b.len()
}

/// Index of the bracket closing the one at `open`, skipping literals and comments.
fn find_close<const KEYWORDS: bool>(src: &str, open: usize) -> Option<usize> {
    let b = src.as_bytes();
    let mut depth = 0usize;
    let mut i = open;
    while i < b.len() {
        if let Some(end) = skip_opaque::<KEYWORDS>(src, i) {
            i = end;
            continue;
        }
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The first byte at or after `i`, before `end`, that is not whitespace or a comment.
/// Comments never reach the regex rule, so either scanner gives the same answer.
pub(crate) fn skip_trivia(src: &str, mut i: usize, end: usize) -> usize {
    let b = src.as_bytes();
    loop {
        i = skip_ws(&b[..end], i);
        match (b.get(i), b.get(i + 1)) {
            (Some(b'/'), Some(b'/' | b'*')) if i < end => {
                i = skip_opaque::<true>(src, i).unwrap_or(end).min(end);
            }
            _ => return i,
        }
    }
}
