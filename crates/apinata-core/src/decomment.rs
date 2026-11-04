//! Port of the `decomment` 0.9.5 package, as bruno-cli applies it to JSON bodies.
//! Models only esprima's unterminated string or comment and raw line break in a string; other rejections are not modelled.
//! Regex-literal inputs are not modelled either.

/// The text bruno-cli sends for a JSON body: decomment 0.9.5's output, or the raw text where decomment throws.
pub(crate) fn decomment(code: &str) -> String {
    // decomment switches to HTML mode when `<` is the first non-blank symbol; JSON never is.
    let first = code
        .bytes()
        .find(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'));
    if first == Some(b'<') {
        return code.to_owned();
    }
    let b = code.as_bytes();
    let len = b.len();
    let eol = eol_of(code);
    let find = |needle: &str, from: usize| {
        code.get(from..)
            .and_then(|rest| rest.find(needle))
            .map(|i| i + from)
    };
    let mut out: Vec<u8> = Vec::with_capacity(len);
    let mut empty_line = true;
    let mut empty_letters: Vec<u8> = Vec::new();
    let mut idx = 0;
    while idx < len {
        let c = b[idx];
        if c == b'/' && idx < len - 1 && (idx == 0 || b[idx - 1] != b'\\') {
            if b[idx + 1] == b'/' {
                // A line comment runs to the line break; on an otherwise empty line the break goes too.
                let Some(lb) = find(eol, idx + 2) else { break };
                if empty_line {
                    empty_letters.clear();
                    idx = lb + eol.len() - 1;
                } else {
                    idx = lb - 1;
                }
                idx += 1;
                continue;
            }
            if b[idx + 1] == b'*' {
                // Esprima rejects an unterminated block comment before decomment runs, so Bruno sends the raw text.
                let Some(end) = find("*/", idx + 2) else {
                    return code.to_owned();
                };
                idx = end + 1;
                if empty_line {
                    empty_letters.clear();
                }
                if let Some(lb) = find(eol, idx + 1) {
                    let mut gap = lb - 1;
                    while matches!(b[gap], b' ' | b'\t') {
                        gap -= 1;
                        if gap <= idx {
                            break;
                        }
                    }
                    if gap == idx && empty_line {
                        idx = lb + eol.len() - 1;
                    }
                }
                idx += 1;
                continue;
            }
        }
        let is_space = c == b' ' || c == b'\t';
        if c == b'\r' || c == b'\n' {
            if code[idx..].starts_with(eol) {
                empty_line = true;
            }
        } else if !is_space {
            empty_line = false;
            out.append(&mut empty_letters);
        }
        if empty_line && is_space {
            empty_letters.push(c);
        } else {
            out.push(c);
        }
        if matches!(c, b'\'' | b'"' | b'`') && (idx == 0 || b[idx - 1] != b'\\') {
            // A quoted run is copied verbatim; it closes at the next same quote behind an even number of backslashes.
            let mut close = idx;
            let closed = loop {
                match find(&code[idx..=idx], close + 1) {
                    None => break None,
                    Some(at) => {
                        close = at;
                        let mut sh = at;
                        while sh > 0 && b[sh - 1] == b'\\' {
                            sh -= 1;
                        }
                        if (at - sh) % 2 == 0 {
                            break Some(at);
                        }
                    }
                }
            };
            // Esprima rejects an unterminated string too.
            let Some(close) = closed else {
                return code.to_owned();
            };
            // Esprima also rejects a raw line break inside a quoted string (not a template),
            // unless a backslash escapes it as a line continuation; the LF of a CRLF follows its CR.
            if c != b'`' && has_raw_break(&b[idx + 1..close]) {
                return code.to_owned();
            }
            out.extend_from_slice(&b[idx + 1..=close]);
            idx = close;
        }
        idx += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| code.to_owned())
}

/// True when a string body holds a line break not preceded by an odd run of backslashes.
fn has_raw_break(body: &[u8]) -> bool {
    body.iter().enumerate().any(|(i, &x)| {
        let is_break = x == b'\r' || (x == b'\n' && !(i > 0 && body[i - 1] == b'\r'));
        let slashes = body[..i].iter().rev().take_while(|&&y| y == b'\\').count();
        is_break && slashes % 2 == 0
    })
}

/// decomment's `getEOL`: the majority line break; on a tie Node uses `os.EOL`, so `\n` matches bruno-cli on Linux and macOS.
fn eol_of(text: &str) -> &'static str {
    let b = text.as_bytes();
    let windows = b.windows(2).filter(|w| w == b"\r\n").count();
    let unix = b.iter().filter(|&&c| c == b'\n').count() - windows;
    if windows > unix { "\r\n" } else { "\n" }
}

#[cfg(test)]
mod tests {
    use super::decomment;

    #[test]
    fn decomment_matches_node_decomment_0_9_5() {
        let cases = [
            (
                "line_end",
                "{\n  \"a\": 1, // trailing\n  \"b\": 2\n}",
                "{\n  \"a\": 1, \n  \"b\": 2\n}",
            ),
            (
                "line_own",
                "{\n  // whole line\n  \"a\": 1\n}",
                "{\n  \"a\": 1\n}",
            ),
            (
                "line_indented_own",
                "{\n    // indented whole line\n  \"a\": 1\n}",
                "{\n  \"a\": 1\n}",
            ),
            ("block_inline", "{\"a\": /* note */ 1}", "{\"a\":  1}"),
            (
                "block_own_line",
                "{\n  /* block */\n  \"a\": 1\n}",
                "{\n  \"a\": 1\n}",
            ),
            (
                "block_multiline",
                "{\n  /*\n   * many\n   */\n  \"a\": 1\n}",
                "{\n  \"a\": 1\n}",
            ),
            (
                "block_then_text",
                "{\n  /* x */ \"a\": 1\n}",
                "{\n \"a\": 1\n}",
            ),
            (
                "url_in_string",
                "{\"u\": \"https://example.test/a//b\"}",
                "{\"u\": \"https://example.test/a//b\"}",
            ),
            (
                "comment_text_in_string",
                "{\"s\": \"/* not */ // not\"}",
                "{\"s\": \"/* not */ // not\"}",
            ),
            (
                "escaped_quote",
                "{\"s\": \"a\\\"// still string\", \"t\": 1} // gone",
                "{\"s\": \"a\\\"// still string\", \"t\": 1} ",
            ),
            ("single_quote", "{'s': '// kept'}", "{'s': '// kept'}"),
            (
                "placeholder",
                "{\n  \"n\": {{n}}, // number\n  \"s\": \"{{s}}\"\n}",
                "{\n  \"n\": {{n}}, \n  \"s\": \"{{s}}\"\n}",
            ),
            (
                "crlf",
                "{\r\n  \"a\": 1, // c\r\n  \"b\": 2\r\n}",
                "{\r\n  \"a\": 1, \r\n  \"b\": 2\r\n}",
            ),
            ("no_newline_line", "{\"a\": 1} // tail", "{\"a\": 1} "),
            (
                "unterminated_block",
                "{\"a\": 1} /* open",
                "{\"a\": 1} /* open",
            ),
            ("unterminated_string", "{\"a\": \"open}", "{\"a\": \"open}"),
            (
                "regex_like",
                "{\"a\": [/x\\/\\/y/]}",
                "{\"a\": [/x\\/\\/y/]}",
            ),
            ("leading_lt", "<a>\n// c\n</a>", "<a>\n// c\n</a>"),
            ("empty", "", ""),
            ("whitespace_only", "\t ", ""),
            ("lf_only", "\n", "\n"),
            ("crlf_only", "\r\n", "\r\n"),
            (
                "continuation_lf",
                "{\"a\": \"l1\\\nl2\"} // c",
                "{\"a\": \"l1\\\nl2\"} ",
            ),
            (
                "continuation_crlf",
                "{\"a\": \"l1\\\r\nl2\"} // c",
                "{\"a\": \"l1\\\r\nl2\"} ",
            ),
            ("breaks_then_tab", "\n\r\n\t", "\n\r\n"),
            (
                "raw_break_in_string",
                "{\"a\": \"l1\nl2\"} // c",
                "{\"a\": \"l1\nl2\"} // c",
            ),
            (
                "trailing_spaces_before_comment",
                "{\n  \"a\": 1,   // c\n  \"b\": 2\n}",
                "{\n  \"a\": 1,   \n  \"b\": 2\n}",
            ),
        ];
        for (name, input, expected) in cases {
            assert_eq!(decomment(input), expected, "{name}");
        }
    }

    #[test]
    fn decomment_keeps_multibyte_text() {
        let input = "{\"é\": 1, // ü\n  \"b\": \"ä//ö\" /* ß */}";
        assert_eq!(decomment(input), "{\"é\": 1, \n  \"b\": \"ä//ö\" }");
    }
}
