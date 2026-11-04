//! Finds a header the HTTP stack would reject, so it can be named before the request is built.

use http::{HeaderName, HeaderValue};

/// Describes what is wrong with one resolved header, if anything, by the `http` crate's rules.
/// `label` is how the message names the header (the file's spelling, never a secret) and
/// `masked` says the name holds secret text, so no character of it is revealed. The value is
/// never quoted.
pub(crate) fn header_problem(label: &str, name: &str, value: &str, masked: bool) -> Option<String> {
    let shown = label.escape_debug();
    if name.is_empty() {
        return Some(format!("header `{shown}` has an empty name"));
    }
    if HeaderName::from_bytes(name.as_bytes()).is_err() {
        if masked {
            return Some(format!(
                "header `{shown}` has a name that header names cannot hold"
            ));
        }
        let problem = match name.chars().find(|c| !is_token(*c)) {
            Some(' ') => "contains a space in its name".to_owned(),
            Some(c) if c.is_control() => "contains a control character in its name".to_owned(),
            Some(c) if !c.is_ascii() => "contains a non-ASCII character in its name".to_owned(),
            Some(c) => {
                format!("contains the character `{c}` in its name, which header names cannot hold")
            }
            None => "is not a valid header name".to_owned(),
        };
        return Some(format!("header `{shown}` {problem}"));
    }
    if HeaderValue::from_bytes(value.as_bytes()).is_err() {
        let problem = if value.contains(['\r', '\n']) {
            "a line break"
        } else {
            "a control character"
        };
        return Some(format!("value of header `{shown}` contains {problem}"));
    }
    None
}

/// The characters RFC 9110 allows in a header name token.
fn is_token(c: char) -> bool {
    c.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(c)
}

/// Why a header name as written in a file can never be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum HeaderNameProblem {
    /// The name holds this character, which is not an RFC 9110 token character.
    Char(char),
}

/// Judges a header name as written: every well-formed `{{...}}` is dropped first, since it is
/// filled in at send time, and the literal characters left must be token characters. A blank
/// name, or one that is only placeholders, is not judged.
pub fn invalid_header_name(name: &str) -> Option<HeaderNameProblem> {
    let mut literal = String::new();
    let mut from = 0;
    for (range, _) in crate::interpolate::placeholders(name) {
        literal.push_str(&name[from..range.start]);
        from = range.end;
    }
    literal.push_str(&name[from..]);
    literal
        .chars()
        .find(|c| !is_token(*c))
        .map(HeaderNameProblem::Char)
}
