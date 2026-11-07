//! The migration's `stripBruExtInRunRequest`: drops `.bru` from the one string-literal argument
//! of each `bru.runRequest(...)` call in a script, skipping comments, strings and regex literals.
//!
//! The reference walks UTF-16 units; this walks bytes. Every character it tests is ASCII and no
//! UTF-8 continuation byte is ASCII, so both walks rewrite the same calls.

const CALL: &[u8] = b"bru.runRequest";
const EXTENSION: &[u8] = b".bru";

/// Keywords after which a `/` opens a regex literal rather than a division.
const VALUE_EXPECTING_KEYWORDS: [&[u8]; 14] = [
    b"return",
    b"typeof",
    b"delete",
    b"throw",
    b"in",
    b"of",
    b"void",
    b"new",
    b"instanceof",
    b"await",
    b"yield",
    b"do",
    b"else",
    b"case",
];

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$'
}

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0c | 0x0b)
}

/// One scanner frame: code (with the `${` brace depth it closes at) or a template literal.
enum Frame {
    Code { brace_depth: usize },
    Template,
}

/// Index past the closing quote of a quoted string at `start`; a raw line break ends it early.
fn string_end(code: &[u8], start: usize, quote: u8) -> usize {
    let mut j = start + 1;
    while j < code.len() {
        match code[j] {
            b'\\' => j += 2,
            c if c == quote => return j + 1,
            b'\n' | b'\r' => return j,
            _ => j += 1,
        }
    }
    code.len()
}

/// Like `string_end`, but a template literal may span lines.
fn template_or_string_end(code: &[u8], start: usize, quote: u8) -> usize {
    if quote != b'`' {
        return string_end(code, start, quote);
    }
    let mut j = start + 1;
    while j < code.len() {
        match code[j] {
            b'\\' => j += 2,
            b'`' => return j + 1,
            _ => j += 1,
        }
    }
    code.len()
}

/// Index past a regex literal and its flags at `start`, or `None` when the `/` cannot open one.
fn regex_end(code: &[u8], start: usize) -> Option<usize> {
    let mut j = start + 1;
    let mut in_class = false;
    while j < code.len() {
        let c = code[j];
        if c == b'\\' {
            j += 2;
            continue;
        }
        if c == b'\n' || c == b'\r' {
            return None;
        }
        if in_class {
            in_class = c != b']';
        } else if c == b'[' {
            in_class = true;
        } else if c == b'/' {
            j += 1;
            while j < code.len() && b"dgimsuy".contains(&code[j]) {
                j += 1;
            }
            return Some(j);
        }
        j += 1;
    }
    None
}

/// Whether a `/` opens a regex, judged by the last significant output byte and, after an
/// identifier, by whether the whole preceding word is a value-expecting keyword.
fn starts_regex(out: &[u8], prev: Option<u8>) -> bool {
    let Some(prev) = prev else { return true };
    if matches!(prev, b')' | b']' | b'`' | b'"' | b'\'' | b'/') {
        return false;
    }
    if !is_ident(prev) {
        return true;
    }
    let word_start = out.iter().rposition(|&c| !is_ident(c)).map_or(0, |k| k + 1);
    VALUE_EXPECTING_KEYWORDS.contains(&&out[word_start..])
}

fn matches_call_start(code: &[u8], i: usize) -> bool {
    if (i > 0 && is_ident(code[i - 1])) || !code[i..].starts_with(CALL) {
        return false;
    }
    code.get(i + CALL.len())
        .is_some_and(|&after| after == b'(' || is_space(after))
}

/// Rewrites the call at `start` when its argument is exactly one quoted literal ending in `.bru`;
/// returns the rewritten text and the index past the closing parenthesis.
fn rewrite_call(code: &[u8], start: usize) -> Option<(Vec<u8>, usize)> {
    let n = code.len();
    let skip_space = |mut j: usize| {
        while j < n && is_space(code[j]) {
            j += 1;
        }
        j
    };
    let mut j = skip_space(start + CALL.len());
    if code.get(j) != Some(&b'(') {
        return None;
    }
    j = skip_space(j + 1);
    let quote = *code.get(j).filter(|q| matches!(q, b'"' | b'\'' | b'`'))?;
    let str_end = template_or_string_end(code, j, quote);
    // A string cut short by a line break right after its opening quote ends on that quote; its
    // content is empty to the reference, so it has no `.bru` to strip.
    if str_end < j + 2 || code[str_end - 1] != quote {
        return None;
    }
    let close = skip_space(str_end);
    if code.get(close) != Some(&b')') {
        return None;
    }
    let content = &code[j + 1..str_end - 1];
    let stripped = content.strip_suffix(EXTENSION)?;
    let mut text = code[start..=j].to_vec();
    text.extend_from_slice(stripped);
    text.extend_from_slice(&code[str_end - 1..=close]);
    Some((text, close + 1))
}

/// `stripBruExtInRunRequest`.
// The rewrite drops only ASCII text between ASCII bytes, so the bytes stay UTF-8.
#[allow(clippy::expect_used)]
pub(crate) fn strip_bru_ext(code: &str) -> String {
    if !code.contains("runRequest") {
        return code.to_owned();
    }
    let code = code.as_bytes();
    let n = code.len();
    let at = |i: usize| code.get(i).copied();
    let mut out: Vec<u8> = Vec::with_capacity(n);
    let mut i = 0;
    // `${` inside a template pushes a code frame closed by its matching `}`.
    let mut frames = vec![Frame::Code { brace_depth: 0 }];
    let mut prev: Option<u8> = None;

    while i < n {
        let c = code[i];
        if let Some(Frame::Template) = frames.last() {
            if c == b'\\' {
                out.extend_from_slice(&code[i..(i + 2).min(n)]);
                i += 2;
            } else if c == b'`' {
                out.push(c);
                i += 1;
                frames.pop();
                prev = Some(b'`');
            } else if c == b'$' && at(i + 1) == Some(b'{') {
                out.extend_from_slice(b"${");
                i += 2;
                frames.push(Frame::Code { brace_depth: 1 });
                prev = None;
            } else {
                out.push(c);
                i += 1;
            }
            continue;
        }

        let next = at(i + 1);
        if c == b'/' && next == Some(b'/') {
            let end = code[(i + 2).min(n)..]
                .iter()
                .position(|&b| b == b'\n')
                .map_or(n, |k| i + 2 + k);
            out.extend_from_slice(&code[i..end]);
            i = end;
            continue;
        }
        if c == b'/' && next == Some(b'*') {
            let end = code[(i + 2).min(n)..]
                .windows(2)
                .position(|w| w == b"*/")
                .map_or(n, |k| i + 2 + k + 2);
            out.extend_from_slice(&code[i..end]);
            i = end;
            continue;
        }
        if c == b'/'
            && starts_regex(&out, prev)
            && let Some(end) = regex_end(code, i)
        {
            out.extend_from_slice(&code[i..end]);
            i = end;
            prev = Some(b'/');
            continue;
        }
        if c == b'"' || c == b'\'' {
            let end = string_end(code, i, c);
            out.extend_from_slice(&code[i..end]);
            i = end;
            prev = Some(c);
            continue;
        }
        if c == b'`' {
            out.push(c);
            i += 1;
            frames.push(Frame::Template);
            continue;
        }
        if c == b'{' {
            out.push(c);
            i += 1;
            if let Some(Frame::Code { brace_depth }) = frames.last_mut()
                && *brace_depth > 0
            {
                *brace_depth += 1;
            }
            prev = Some(b'{');
            continue;
        }
        if c == b'}' {
            out.push(c);
            i += 1;
            prev = Some(b'}');
            if let Some(Frame::Code { brace_depth }) = frames.last_mut()
                && *brace_depth > 0
            {
                *brace_depth -= 1;
                if *brace_depth == 0 {
                    frames.pop();
                }
            }
            continue;
        }
        if c == b'b'
            && matches_call_start(code, i)
            && let Some((text, end)) = rewrite_call(code, i)
        {
            out.extend_from_slice(&text);
            i = end;
            prev = Some(b')');
            continue;
        }
        out.push(c);
        i += 1;
        if !is_space(c) {
            prev = Some(c);
        }
    }
    String::from_utf8(out).expect("the rewrite only removes ASCII text between ASCII bytes")
}
