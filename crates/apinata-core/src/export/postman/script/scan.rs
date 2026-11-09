//! Byte-level JavaScript scanning: strings, comments, regexes, templates and brackets.

/// Deepest nesting of argument and template rewrites; beyond it text passes through.
pub(super) const MAX_DEPTH: usize = 32;
/// Longest regex literal recognised; a longer run is read as division.
pub(super) const MAX_REGEX: usize = 256;

pub(super) fn is_ident_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c >= 0x80
}

pub(super) fn ident_end(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && is_ident_byte(b[i]) {
        i += 1;
    }
    i
}

pub(super) fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// The last non-whitespace byte before `i`, with its index.
pub(super) fn prev_significant(b: &[u8], i: usize) -> Option<(usize, u8)> {
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
pub(super) fn is_property_access(b: &[u8], i: usize) -> bool {
    match prev_significant(b, i) {
        Some((k, b'.')) => !(k >= 2 && b[k - 1] == b'.' && b[k - 2] == b'.'),
        _ => false,
    }
}

/// End of the string, comment, template or regex literal starting at `i`, if one does.
pub(super) fn skip_opaque(src: &str, i: usize) -> Option<usize> {
    let b = src.as_bytes();
    match b[i] {
        q @ (b'\'' | b'"') => {
            let mut k = i + 1;
            while k < b.len() && b[k] != q {
                k += if b[k] == b'\\' { 2 } else { 1 };
            }
            Some((k + 1).min(b.len()))
        }
        b'`' => Some(template_end(src, i)),
        b'/' if b.get(i + 1) == Some(&b'/') => Some(src[i..].find('\n').map_or(b.len(), |n| i + n)),
        b'/' if b.get(i + 1) == Some(&b'*') => {
            Some(src[i + 2..].find("*/").map_or(b.len(), |n| i + 2 + n + 2))
        }
        b'/' => regex_end(b, i),
        _ => None,
    }
}

/// A `/` starts a regex literal only where an expression may begin.
pub(super) fn regex_end(b: &[u8], i: usize) -> Option<usize> {
    let starts_expr = match prev_significant(b, i) {
        None => true,
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

thread_local!(static NESTING: std::cell::Cell<usize> = const { std::cell::Cell::new(0) });

/// One level of template nesting; dropping it, also while unwinding, gives the level back.
pub(super) struct NestingGuard;

impl NestingGuard {
    fn enter() -> Option<NestingGuard> {
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
pub(super) fn template_end(src: &str, i: usize) -> usize {
    // Templates nested through `${}` recurse via find_close; past the cap the rest of
    // the input is taken as one opaque template.
    let Some(_level) = NestingGuard::enter() else {
        return src.len();
    };
    template_scan(src, i)
}

pub(super) fn template_scan(src: &str, i: usize) -> usize {
    let b = src.as_bytes();
    let mut k = i + 1;
    while k < b.len() {
        match b[k] {
            b'\\' => k += 2,
            b'`' => return k + 1,
            b'$' if b.get(k + 1) == Some(&b'{') => {
                k = find_close(src, k + 1).map_or(b.len(), |c| c + 1);
            }
            _ => k += 1,
        }
    }
    b.len()
}

/// Index of the bracket closing the one at `open`, skipping literals and comments.
pub(super) fn find_close(src: &str, open: usize) -> Option<usize> {
    let b = src.as_bytes();
    let mut depth = 0usize;
    let mut i = open;
    while i < b.len() {
        if let Some(end) = skip_opaque(src, i) {
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
pub(super) fn skip_trivia(src: &str, mut i: usize, end: usize) -> usize {
    let b = src.as_bytes();
    loop {
        i = skip_ws(&b[..end], i);
        match (b.get(i), b.get(i + 1)) {
            (Some(b'/'), Some(b'/' | b'*')) if i < end => {
                i = skip_opaque(src, i).unwrap_or(end).min(end);
            }
            _ => return i,
        }
    }
}

/// Absolute ranges of the top-level comma-separated parts of `src[from..to]`; empty parts
/// dropped. Leading comments and blanks are trimmed. A part ending in a line comment keeps
/// the line break after it, so text appended to the part is not commented out.
pub(super) fn parts(src: &str, from: usize, to: usize) -> Vec<(usize, usize)> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut push = |s: usize, e: usize, line_comment_end: Option<usize>| {
        let s = skip_trivia(src, s, e);
        let mut e = (s..e)
            .rev()
            .find(|&k| !b[k].is_ascii_whitespace())
            .map_or(s, |k| k + 1);
        if line_comment_end == Some(e) && b.get(e) == Some(&b'\n') {
            e += 1;
        }
        if s < e {
            out.push((s, e));
        }
    };
    let (mut start, mut depth, mut i) = (from, 0usize, from);
    let mut line_comment_end = None;
    while i < to {
        if let Some(end) = skip_opaque(src, i) {
            if b[i] == b'/' && b.get(i + 1) == Some(&b'/') {
                line_comment_end = Some(end);
            }
            i = end;
            continue;
        }
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                push(start, i, line_comment_end);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    push(start, to, line_comment_end);
    out
}
