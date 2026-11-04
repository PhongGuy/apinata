//! Live JSON syntax check for the request body editor.

/// Where and why a JSON body fails to parse.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct JsonSyntaxError {
    /// 1-based line of the error.
    pub line: usize,
    /// 1-based column in characters on that line.
    pub column: usize,
    /// A short reason, such as `expected value`.
    pub message: String,
}

/// The first syntax error in `text` read as a JSON body, or `None` when it parses or is blank.
///
/// A `{{name}}` placeholder counts as a valid value (or valid string content), so a body
/// that is only waiting for its variables to resolve is not flagged. Columns count characters.
pub fn json_syntax_error(text: &str) -> Option<JsonSyntaxError> {
    if text.trim().is_empty() {
        return None;
    }
    let masked = mask_placeholders(text);
    let err = serde_json::from_str::<serde::de::IgnoredAny>(&masked).err()?;
    // serde puts an unexpected end on the last char; the marker belongs just past it.
    let line = if err.is_eof() {
        masked.split('\n').count()
    } else {
        err.line().max(1)
    };
    let line_text = masked.split('\n').nth(line - 1).unwrap_or("");
    // serde reports a 1-based byte column; round it down to a char boundary and count chars.
    let wanted = if err.is_eof() {
        line_text.len()
    } else {
        err.column().saturating_sub(1)
    };
    let mut at = wanted.min(line_text.len());
    while !line_text.is_char_boundary(at) {
        at -= 1;
    }
    let full = err.to_string();
    let message = full
        .rfind(" at line ")
        .map_or(full.as_str(), |i| &full[..i])
        .to_string();
    Some(JsonSyntaxError {
        line,
        column: line_text[..at].chars().count() + 1,
        message,
    })
}

// Swaps each single-line placeholder for filler of the same char count, so every line and
// column survives: `x` inside a JSON string, `1` elsewhere (a valid number of any length).
fn mask_placeholders(text: &str) -> String {
    let mut spans = crate::placeholders(text)
        .into_iter()
        .map(|(range, _)| range)
        .filter(|range| !text[range.clone()].contains('\n'))
        .peekable();
    let mut out = String::with_capacity(text.len());
    let (mut in_string, mut escaped) = (false, false);
    let mut chars = text.char_indices();
    while let Some((at, c)) = chars.next() {
        if let Some(span) = spans.next_if(|s| s.start == at) {
            let filler = if in_string { 'x' } else { '1' };
            out.extend(std::iter::repeat_n(
                filler,
                text[span.clone()].chars().count(),
            ));
            // Resume after the placeholder.
            while chars.clone().next().is_some_and(|(i, _)| i < span.end) {
                chars.next();
            }
            continue;
        }
        if in_string {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => in_string = false,
                _ => {}
            }
        } else if c == '"' {
            in_string = true;
        }
        out.push(c);
    }
    out
}
