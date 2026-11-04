//! String and number scalar styling, ported from `yaml` 2.3.4 `src/stringify/stringifyString.ts`
//! and `stringifyNumber.ts`.
//!
//! Bruno's options (`lineWidth: 0`, `blockQuote: true`, `defaultStringType: 'PLAIN'`) make line
//! folding the identity and block style always literal, so those branches are not ported.
//! Every scalar sits at least one indent step deep, because the document root is a mapping.

use yaml_serde::Number;

/// One level of indentation, Bruno's `indent: 2`.
pub(super) const INDENT_STEP: &str = "  ";

/// `doubleQuotedMinMultiLineLength`: shorter JSON strings never break at an escaped newline.
const DOUBLE_QUOTED_MIN_MULTI_LINE_LENGTH: usize = 40;

/// Style a string scalar at `indent`, as a mapping key when `implicit_key` is set.
/// Port of `stringifyString` with no explicit scalar type, so `PLAIN` is the starting style.
pub(super) fn stringify_string(value: &str, indent: &str, implicit_key: bool) -> String {
    // Control characters (but not tab or newline) force double quotes before anything else.
    let forces_double = value
        .chars()
        .any(|c| matches!(c, '\x00'..='\x08' | '\x0b'..='\x1f' | '\x7f'..='\u{9f}'));
    if forces_double {
        double_quoted(value, indent, implicit_key)
    } else {
        plain(value, indent, implicit_key)
    }
}

/// Port of `stringifyNumber`: integers keep their exact digits, floats print as JavaScript would.
pub(super) fn stringify_number(n: &Number) -> String {
    if let Some(u) = n.as_u64() {
        return u.to_string();
    }
    if let Some(i) = n.as_i64() {
        return i.to_string();
    }
    match n.as_f64() {
        Some(f) if f.is_finite() => ryu_js::Buffer::new().format_finite(f).to_owned(),
        Some(f) if f.is_nan() => ".nan".to_owned(),
        Some(f) if f < 0.0 => "-.inf".to_owned(),
        _ => ".inf".to_owned(),
    }
}

/// Port of `plainString`: write the string bare unless a YAML reader would misread it.
fn plain(value: &str, indent: &str, implicit_key: bool) -> String {
    let multiline = value.contains('\n');
    if implicit_key && multiline {
        return quoted(value, indent, implicit_key);
    }
    if value.is_empty() || plain_is_ambiguous(value) {
        return if implicit_key || !multiline {
            quoted(value, indent, implicit_key)
        } else {
            block(value, indent, implicit_key)
        };
    }
    if !implicit_key && multiline {
        return block(value, indent, implicit_key);
    }
    // A key at the first level that starts a line with `---` or `...` would end the document.
    if implicit_key && indent == INDENT_STEP && contains_document_marker(value) {
        return quoted(value, indent, implicit_key);
    }
    if resolves_as_non_string(value) {
        return quoted(value, indent, implicit_key);
    }
    value.to_owned()
}

/// The `plainString` test for strings that cannot be written plain:
/// `/^[\n\t ,[\]{}#&*!|>'"%@`]|^[?-]$|^[?-][ \t]|[\n:][ \t]|[ \t]\n|[\n\t ]#|[\n\t :]$/`.
/// Every pattern is ASCII, so scanning bytes is exact.
fn plain_is_ambiguous(value: &str) -> bool {
    let b = value.as_bytes();
    let (Some(&first), Some(&last)) = (b.first(), b.last()) else {
        return false;
    };
    let starts_with_indicator = b"\n\t ,[]{}#&*!|>'\"%@`".contains(&first);
    let lone_or_spaced_indicator =
        matches!(first, b'?' | b'-') && (b.len() == 1 || matches!(b[1], b' ' | b'\t'));
    let bad_pair = b.windows(2).any(|w| {
        matches!(
            (w[0], w[1]),
            (b'\n' | b':', b' ' | b'\t') | (b' ' | b'\t', b'\n') | (b'\n' | b'\t' | b' ', b'#')
        )
    });
    let bad_end = matches!(last, b'\n' | b'\t' | b' ' | b':');
    starts_with_indicator || lone_or_spaced_indicator || bad_pair || bad_end
}

/// `containsDocumentMarker`: `/^(%|---|\.\.\.)/m`, where a JavaScript line starts after
/// `\n`, `\r`, U+2028 or U+2029.
fn contains_document_marker(value: &str) -> bool {
    value
        .split(['\n', '\r', '\u{2028}', '\u{2029}'])
        .any(|line| line.starts_with('%') || line.starts_with("---") || line.starts_with("..."))
}

/// True when the core schema would resolve the plain string as null, a boolean or a number.
/// Hand ports of the `test` regexes in `src/schema/common/null.ts` and `src/schema/core/{bool,int,float}.ts`;
/// `yaml_serde`'s resolver disagrees on `0b11` and `017`.
fn resolves_as_non_string(s: &str) -> bool {
    let all_digits = |t: &str| !t.is_empty() && t.bytes().all(|c| c.is_ascii_digit());
    let unsigned = s.strip_prefix(['+', '-']).unwrap_or(s);

    let null = matches!(s, "" | "~" | "null" | "Null" | "NULL");
    let bool = matches!(s, "true" | "True" | "TRUE" | "false" | "False" | "FALSE");
    let int = all_digits(unsigned);
    let oct = s
        .strip_prefix("0o")
        .is_some_and(|t| !t.is_empty() && t.bytes().all(|c| matches!(c, b'0'..=b'7')));
    let hex = s
        .strip_prefix("0x")
        .is_some_and(|t| !t.is_empty() && t.bytes().all(|c| c.is_ascii_hexdigit()));
    let nan = matches!(
        unsigned,
        ".inf" | ".Inf" | ".INF" | ".nan" | ".NaN" | ".NAN"
    );
    let float = decimal_prefix(unsigned) == Some(("", true));
    let exp = decimal_prefix(unsigned)
        .and_then(|(rest, _)| rest.strip_prefix(['e', 'E']))
        .is_some_and(|e| all_digits(e.strip_prefix(['+', '-']).unwrap_or(e)));
    null || bool || int || oct || hex || nan || float || exp
}

/// Match `\.[0-9]+|[0-9]+(?:\.[0-9]*)?` at the start of `s`.
/// Returns the unmatched rest and whether the match contained a dot.
fn decimal_prefix(s: &str) -> Option<(&str, bool)> {
    fn split_digits(t: &str) -> (&str, &str) {
        t.split_at(t.bytes().take_while(u8::is_ascii_digit).count())
    }
    let (int, rest) = split_digits(s);
    match rest.strip_prefix('.') {
        Some(frac) => {
            let (frac, rest) = split_digits(frac);
            (!int.is_empty() || !frac.is_empty()).then_some((rest, true))
        }
        None => (!int.is_empty()).then_some((rest, false)),
    }
}

/// Port of `quotedString` with `singleQuote: null`: single quotes only for a string that holds
/// double quotes and no single quotes.
fn quoted(value: &str, indent: &str, implicit_key: bool) -> String {
    if value.contains('"') && !value.contains('\'') {
        single_quoted(value, indent, implicit_key)
    } else {
        double_quoted(value, indent, implicit_key)
    }
}

/// Port of `singleQuotedString`. Whitespace beside a newline cannot survive single quotes.
fn single_quoted(value: &str, indent: &str, implicit_key: bool) -> String {
    let space_by_newline = value
        .as_bytes()
        .windows(2)
        .any(|w| matches!((w[0], w[1]), (b' ' | b'\t', b'\n') | (b'\n', b' ' | b'\t')));
    if (implicit_key && value.contains('\n')) || space_by_newline {
        return double_quoted(value, indent, implicit_key);
    }
    // Each newline run gains one more newline, since a single line break folds to a space.
    let escaped = value.replace('\'', "''");
    let extra = format!("\n{indent}");
    format!("'{}'", after_newline_runs(&escaped, &extra, false))
}

/// Port of `doubleQuotedString`: start from `JSON.stringify`, swap JSON's `\u00xx` escapes for
/// YAML's short forms, and turn escaped newlines in long values into real line breaks.
fn double_quoted(value: &str, indent: &str, implicit_key: bool) -> String {
    // serde_json escapes exactly as JSON.stringify does, with lowercase hex and non-ASCII left raw.
    let json = serde_json::to_string(value).expect("a str always serializes");
    let short = json.encode_utf16().count() < DOUBLE_QUOTED_MIN_MULTI_LINE_LENGTH;
    let b = json.as_bytes();
    let at = |i: usize| b.get(i).copied();
    let mut out = String::with_capacity(json.len());
    let mut start = 0;
    let mut i = 0;
    while i < b.len() {
        let mut ch = b[i];
        // A space before a newline is escaped so a reader does not fold it away.
        if ch == b' ' && at(i + 1) == Some(b'\\') && at(i + 2) == Some(b'n') {
            out.push_str(&json[start..i]);
            out.push_str("\\ ");
            i += 1;
            start = i;
            ch = b'\\';
        }
        if ch == b'\\' {
            match at(i + 1) {
                // JSON.stringify only writes `\u` for characters below U+0020.
                Some(b'u') => {
                    out.push_str(&json[start..i]);
                    match &json[i + 2..i + 6] {
                        "0000" => out.push_str("\\0"),
                        "0007" => out.push_str("\\a"),
                        "000b" => out.push_str("\\v"),
                        "001b" => out.push_str("\\e"),
                        code => {
                            out.push_str("\\x");
                            out.push_str(&code[2..]);
                        }
                    }
                    i += 5;
                    start = i + 1;
                }
                Some(b'n') if implicit_key || at(i + 2) == Some(b'"') || short => i += 1,
                Some(b'n') => {
                    // The blank line stands for the escaped newline; with folding off, both stay.
                    out.push_str(&json[start..i]);
                    out.push_str("\n\n");
                    while at(i + 2) == Some(b'\\')
                        && at(i + 3) == Some(b'n')
                        && at(i + 4) != Some(b'"')
                    {
                        out.push('\n');
                        i += 2;
                    }
                    out.push_str(indent);
                    if at(i + 2) == Some(b' ') {
                        out.push('\\');
                    }
                    i += 1;
                    start = i + 1;
                }
                _ => i += 1,
            }
        }
        i += 1;
    }
    out.push_str(&json[start..]);
    out
}

/// Port of `blockString` for the literal style: a `|` header with chomping and indent
/// indicators, then the lines indented by `indent`.
fn block(value: &str, indent: &str, implicit_key: bool) -> String {
    // A block cannot end in whitespace after its last newline; blank strings read better quoted.
    let trimmed = value.trim_end_matches(['\t', ' ']);
    let ends_in_spaced_line = trimmed.len() < value.len() && trimmed.ends_with('\n');
    if ends_in_spaced_line || value.chars().all(is_js_space) {
        return quoted(value, indent, implicit_key);
    }

    // Chomping follows the trailing whitespace: no newline strips, one clips, more keep.
    let body_len = value.trim_end_matches(['\n', '\t', ' ']).len();
    let (body, end) = value.split_at(body_len);
    let chomp = match end.find('\n') {
        None => "-",
        Some(nl) if nl != end.len() - 1 => "+",
        Some(_) => "",
    };
    let end = after_newline_runs(end.strip_suffix('\n').unwrap_or(end), indent, true);

    // Leading spaces need an explicit indent indicator; leading blank lines are kept as `start`.
    let lead_len = body
        .bytes()
        .take_while(|c| matches!(c, b' ' | b'\n'))
        .count();
    let lead = &body[..lead_len];
    let starts_with_space = lead.contains(' ');
    let start_len = lead.rfind('\n').map_or(0, |nl| nl + 1);
    let (start, body) = body.split_at(start_len);

    format!(
        "|{}{chomp}\n{indent}{}{}{end}",
        if starts_with_space { "2" } else { "" },
        after_newline_runs(start, indent, false),
        after_newline_runs(body, indent, false),
    )
}

/// Insert `insert` after every run of newlines, as `s.replace(/\n+/g, '$&' + insert)` does.
/// With `skip_final_run`, a run that ends the string is left alone (`blockEndNewlines`).
fn after_newline_runs(s: &str, insert: &str, skip_final_run: bool) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        let run_ends_here = c == '\n' && chars.peek() != Some(&'\n');
        if run_ends_here && !(skip_final_run && chars.peek().is_none()) {
            out.push_str(insert);
        }
    }
    out
}

/// JavaScript's `\s` class, which also covers Unicode spaces and the BOM.
pub(super) fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\x0b' | '\x0c' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}
