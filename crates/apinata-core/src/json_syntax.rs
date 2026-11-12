//! Live JSON syntax check for the request body editor.

/// Where and why a JSON body fails to parse.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct JsonSyntaxError {
    /// 1-based line of the error.
    pub line: usize,
    /// 1-based column in characters on that line.
    pub column: usize,
    /// A short reason in plain words, such as `Remove the comma before }`.
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
    let raw = full
        .rfind(" at line ")
        .map_or(full.as_str(), |i| &full[..i]);
    let offset = masked
        .split('\n')
        .take(line - 1)
        .map(|l| l.len() + 1)
        .sum::<usize>()
        + at;
    let message = plain_message(raw, &masked, offset, err.is_eof());
    Some(JsonSyntaxError {
        line,
        column: line_text[..at].chars().count() + 1,
        message,
    })
}

// Rewords the parser's `raw` reason in plain words. The character at byte `offset` and the
// one before it name the common slips (a trailing comma, single quotes, a bare word) that the
// parser only reports as a generic expectation.
fn plain_message(raw: &str, text: &str, offset: usize, eof: bool) -> String {
    let here = (!eof).then(|| text[offset..].chars().next()).flatten();
    let before = text[..offset].trim_end().chars().next_back();
    let message = match (raw, here) {
        (_, Some(close @ ('}' | ']'))) if before == Some(',') => {
            return format!("Remove the comma before {close}");
        }
        (_, Some('\'')) => "Use double quotes instead of single quotes",
        ("EOF while parsing an object", _) => "The JSON ends before the closing }",
        ("EOF while parsing a list", _) => "The JSON ends before the closing ]",
        ("EOF while parsing a string", _) => "The JSON ends inside a string; add the closing \"",
        ("EOF while parsing a value", _) => "The JSON ends where a value should be",
        _ if eof => "The JSON ends too early",
        ("key must be a string", _) => "Put the key in double quotes",
        ("expected `,` or `}`", _) => "Add a comma between items, or close the object with }",
        ("expected `,` or `]`", _) => "Add a comma between items, or close the list with ]",
        ("expected `:`", _) => "Add a colon after the key",
        ("expected value", Some(c)) if c.is_alphabetic() => "Put text in double quotes",
        ("expected value", _) => "A value is missing here",
        ("expected ident", _) => "Use true, false or null, or put text in double quotes",
        ("trailing characters", _) => "Remove the text after the end of the JSON",
        _ => "This is not valid JSON",
    };
    message.to_owned()
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
