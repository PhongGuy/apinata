//! Credentials told apart by name and shape rather than by value: the rule a history snapshot
//! and the app's Timeline share for literal secrets no variable list knows.

use crate::collection::DraftSnapshot;
use crate::interpolate::placeholders;
use crate::url_password::is_lone_placeholder;
use percent_encoding::percent_decode_str;
use yaml_serde::Value;

const REDACTED: &str = "[redacted]";

/// Names that are credentials whole, matched in any case.
const NAMES: [&str; 5] = [
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
];

/// Words that make a name a credential when they end it on a word boundary.
const SUFFIXES: [&str; 4] = ["token", "secret", "key", "password"];

/// The schemes whose following word is a credential wherever they appear.
const SCHEMES: [&str; 2] = ["bearer", "basic"];

/// Whether a header, query or form field `name` holds a credential: one of the listed header
/// names, or a name whose last word is token, secret, key or password. That word starts after
/// `-` or `_`, as a capitalised word (`apiKey`, `APIKey`) or as an all-caps word after a
/// lower-case letter or digit (`apiKEY`), so `monkey`, `X-APIKEY` and `X-Keyboard` do not match.
pub fn is_credential_name(name: &str) -> bool {
    let name = name.trim();
    if NAMES.iter().any(|n| name.eq_ignore_ascii_case(n)) {
        return true;
    }
    SUFFIXES.iter().any(|word| {
        let Some(at) = name.len().checked_sub(word.len()) else {
            return false;
        };
        let (head, tail) = (&name.as_bytes()[..at], &name.as_bytes()[at..]);
        if !tail.eq_ignore_ascii_case(word.as_bytes()) {
            return false;
        }
        match head.last() {
            None | Some(b'-' | b'_') => true,
            // A case step: a capitalised word after any letter or digit, or an all-caps word
            // after a lower-case letter or digit; an all-caps run alone shows no boundary.
            Some(prev) => {
                let capitalised =
                    tail[0].is_ascii_uppercase() && tail[1..].iter().all(u8::is_ascii_lowercase);
                let all_caps = tail.iter().all(u8::is_ascii_uppercase);
                (capitalised && prev.is_ascii_alphanumeric())
                    || (all_caps && (prev.is_ascii_lowercase() || prev.is_ascii_digit()))
            }
        }
    })
}

/// The value of a credential-named field as history keeps it: unchanged when it holds nothing
/// but `{{var}}` placeholders (and perhaps a Bearer or Basic scheme), else `[redacted]`, keeping
/// a leading Bearer or Basic scheme word.
pub fn mask_credential_value(value: &str) -> String {
    if !holds_literal(value) {
        return value.to_owned();
    }
    match value.split_once(' ') {
        Some((scheme, _)) if is_scheme(scheme) => format!("{scheme} {REDACTED}"),
        _ => REDACTED.to_owned(),
    }
}

/// `value` with the word after each Bearer or Basic scheme (any case, a whole word followed by
/// spaces) replaced by `[redacted]`, unless that word is a single `{{var}}` placeholder.
pub fn mask_credential_shape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    let mut prev_word_char = false;
    while let Some(c) = rest.chars().next() {
        let scheme = SCHEMES.iter().find_map(|s| {
            let head = rest.get(..s.len())?;
            let after = &rest[s.len()..];
            let spaced = after.len() - after.trim_start_matches(' ').len();
            (!prev_word_char && head.eq_ignore_ascii_case(s) && spaced > 0)
                .then_some(s.len() + spaced)
        });
        if let Some(lead) = scheme {
            let after = &rest[lead..];
            let end = after
                .find(|c: char| c.is_whitespace() || c == ',')
                .unwrap_or(after.len());
            let word = &after[..end];
            out.push_str(&rest[..lead]);
            if word.is_empty() || is_lone_placeholder(word) {
                out.push_str(word);
            } else {
                out.push_str(REDACTED);
            }
            rest = &after[end..];
            prev_word_char = true;
            continue;
        }
        prev_word_char = c.is_alphanumeric();
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// `url` with the value of each query pair replaced by `[redacted]` when its name (decoded) is a
/// credential name and the value is not empty or only placeholders, or when the decoded value
/// holds a Bearer or Basic credential.
pub fn mask_credential_query(url: &str) -> String {
    let (before, fragment) = match url.split_once('#') {
        Some((before, fragment)) => (before, Some(fragment)),
        None => (url, None),
    };
    let Some((path, query)) = before.split_once('?') else {
        return url.to_owned();
    };
    let pairs: Vec<String> = query
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some((name, value)) => {
                let decode = |text: &str| {
                    percent_decode_str(&text.replace('+', " "))
                        .decode_utf8_lossy()
                        .into_owned()
                };
                let shaped = decode(value);
                if (is_credential_name(&decode(name)) && holds_literal(value))
                    || mask_credential_shape(&shaped) != shaped
                {
                    format!("{name}={REDACTED}")
                } else {
                    pair.to_owned()
                }
            }
            None => pair.to_owned(),
        })
        .collect();
    let mut out = format!("{path}?{}", pairs.join("&"));
    if let Some(fragment) = fragment {
        out.push('#');
        out.push_str(fragment);
    }
    out
}

impl DraftSnapshot {
    /// Hides literal credentials found by name and shape: credential-named header, query
    /// parameter and form field values, query values in the url, and any Bearer or Basic
    /// credential in any of those values. Whether anything changed.
    pub fn mask_credentials_by_name(&mut self) -> bool {
        let key = self.request_key();
        let Some(Value::Mapping(request)) = self.root_mut().get_mut(key) else {
            return false;
        };
        let mut changed = false;
        let mut set = |slot: &mut String, masked: String| {
            if *slot != masked {
                *slot = masked;
                changed = true;
            }
        };
        if let Some(Value::String(url)) = request.get_mut("url") {
            let masked = mask_credential_query(url);
            set(url, masked);
        }
        for_rows(request.get_mut("headers"), |name, value| {
            set(value, mask_named_or_shaped(name, value));
        });
        for_rows(request.get_mut("params"), |name, value| {
            set(value, mask_named_or_shaped(name, value));
        });
        // Path parameters are rows too, but only a query row's value reaches the query string.
        let form = request.get_mut("body").and_then(|body| {
            let kind = body.get("type").and_then(Value::as_str);
            matches!(kind, Some("form-urlencoded" | "multipart-form"))
                .then(|| body.get_mut("data"))
                .flatten()
        });
        for_rows(form, |name, value| {
            set(value, mask_named_or_shaped(name, value));
        });
        changed
    }
}

// A field's value with its name rule and then the Bearer or Basic shape rule applied.
fn mask_named_or_shaped(name: &str, value: &str) -> String {
    let named = if is_credential_name(name) {
        mask_credential_value(value)
    } else {
        value.to_owned()
    };
    mask_credential_shape(&named)
}

// Calls `f` with the name and string value of each row in a sequence of name/value rows,
// skipping path parameters, whose names never reach the wire.
fn for_rows(rows: Option<&mut Value>, mut f: impl FnMut(&str, &mut String)) {
    let Some(Value::Sequence(rows)) = rows else {
        return;
    };
    for row in rows {
        let Value::Mapping(row) = row else { continue };
        if row.get("type").and_then(Value::as_str) == Some("path") {
            continue;
        }
        let Some(name) = row.get("name").and_then(Value::as_str).map(str::to_owned) else {
            continue;
        };
        if let Some(Value::String(value)) = row.get_mut("value") {
            f(&name, value);
        }
    }
}

// Whether `value` holds text beyond `{{var}}` placeholders, spaces and a lone scheme word.
fn holds_literal(value: &str) -> bool {
    let mut rest = value.to_owned();
    for (range, _) in placeholders(value).into_iter().rev() {
        rest.replace_range(range, " ");
    }
    let rest = rest.trim();
    !(rest.is_empty() || is_scheme(rest))
}

fn is_scheme(word: &str) -> bool {
    SCHEMES.iter().any(|s| word.eq_ignore_ascii_case(s))
}
