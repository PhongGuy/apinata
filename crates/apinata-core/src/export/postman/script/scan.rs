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
