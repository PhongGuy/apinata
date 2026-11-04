//! JSON layout by whitespace alone: key order, number text and escapes stay as written.

/// Re-indents JSON as `JSON.stringify(v, null, 2)` lays it out, keeping
/// every token as sent; `None` when the text is not valid JSON.
pub fn reindent_json(text: &str) -> Option<String> {
    serde_json::from_str::<serde::de::IgnoredAny>(text).ok()?;
    layout_json(text, true)
}

/// Lays out request-body JSON by whitespace alone, so key order, number
/// text and escapes stay as written. A `{{name}}` placeholder outside a
/// string counts as a value; `Err` is a short message naming the line and
/// column where the text stops being JSON.
pub fn beautify_json(text: &str) -> Result<String, String> {
    let probe = placeholders_as_values(text);
    serde_json::from_str::<serde::de::IgnoredAny>(&probe)
        .map_err(|e| format!("Invalid JSON: {e}"))?;
    layout_json(text, true).ok_or_else(|| "Invalid JSON: not valid UTF-8".to_string())
}

/// Strips the whitespace between JSON tokens, keeping everything else as
/// written; validates exactly as [`beautify_json`] does and shares its error.
pub fn minify_json(text: &str) -> Result<String, String> {
    let probe = placeholders_as_values(text);
    serde_json::from_str::<serde::de::IgnoredAny>(&probe)
        .map_err(|e| format!("Invalid JSON: {e}"))?;
    layout_json(text, false).ok_or_else(|| "Invalid JSON: not valid UTF-8".to_string())
}

// The byte length of the `{{...}}` placeholder starting `bytes`, which
// stays on one line and ends at the first `}}`.
fn placeholder_len(bytes: &[u8]) -> Option<usize> {
    if !bytes.starts_with(b"{{") {
        return None;
    }
    let end = bytes[2..]
        .windows(2)
        .position(|w| w == b"}}")
        .map(|n| n + 4)?;
    (!bytes[..end].contains(&b'\n')).then_some(end)
}

// A copy of `text` for validation with each outside-string placeholder
// swapped for a same-width value, so error positions match the original.
// A placeholder glued to a neighbouring token becomes a letter, which keeps
// that text invalid.
fn placeholders_as_values(text: &str) -> String {
    let bytes = text.as_bytes();
    let glue = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+' | b'_');
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                let start = i;
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
                i = (i + 1).min(bytes.len());
                out.push_str(&text[start..i]);
            }
            b'{' if let Some(n) = placeholder_len(&bytes[i..]) => {
                let touching =
                    (i > 0 && glue(bytes[i - 1])) || bytes.get(i + n).is_some_and(|b| glue(*b));
                out.push(if touching { 'x' } else { '0' });
                out.extend(std::iter::repeat_n(' ', n - 1));
                i += n;
            }
            _ => {
                let c = text[i..].chars().next().unwrap_or(' ');
                out.push(c);
                i += c.len_utf8();
            }
        }
    }
    out
}

// Re-lays out text already known to be JSON (placeholders allowed): indented
// when `pretty`, otherwise with no whitespace between tokens.
fn layout_json(text: &str, pretty: bool) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() * 2);
    let mut depth = 0usize;
    let newline = |out: &mut Vec<u8>, depth: usize| {
        if !pretty {
            return;
        }
        out.push(b'\n');
        out.resize(out.len() + depth * 2, b' ');
    };
    let mut i = 0;
    // Walk the validated text token by token: strings are copied whole with
    // their escapes, whitespace outside them is dropped, and structure
    // characters carry the layout.
    while i < bytes.len() {
        let b = bytes[i];
        i += 1;
        match b {
            b'"' => {
                out.push(b);
                while i < bytes.len() {
                    let c = bytes[i];
                    out.push(c);
                    i += 1;
                    if c == b'\\' {
                        out.push(bytes[i]);
                        i += 1;
                    } else if c == b'"' {
                        break;
                    }
                }
            }
            b'{' if let Some(n) = placeholder_len(&bytes[i - 1..]) => {
                out.extend_from_slice(&bytes[i - 1..i - 1 + n]);
                i += n - 1;
            }
            b'{' | b'[' => {
                out.push(b);
                let next = bytes[i..].iter().position(|c| !c.is_ascii_whitespace());
                match next.map(|n| (n, bytes[i + n])) {
                    Some((n, close @ (b'}' | b']'))) => {
                        out.push(close);
                        i += n + 1;
                    }
                    _ => {
                        depth += 1;
                        newline(&mut out, depth);
                    }
                }
            }
            b'}' | b']' => {
                depth = depth.saturating_sub(1);
                newline(&mut out, depth);
                out.push(b);
            }
            b',' => {
                out.push(b);
                newline(&mut out, depth);
            }
            b':' => out.extend_from_slice(if pretty { b": " } else { b":" }),
            b' ' | b'\t' | b'\n' | b'\r' => {}
            _ => out.push(b),
        }
    }
    String::from_utf8(out).ok()
}
