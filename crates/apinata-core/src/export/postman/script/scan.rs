//! The exporter's view of the shared JavaScript scanner, plus its argument splitter.

#[cfg(test)]
pub(super) use crate::js_scan::REGEX_AFTER;
pub(super) use crate::js_scan::keyword_regex::{find_close, skip_opaque, template_end};
pub(super) use crate::js_scan::{
    MAX_DEPTH, ident_end, is_ident_byte, is_property_access, prev_significant, skip_trivia, skip_ws,
};

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
