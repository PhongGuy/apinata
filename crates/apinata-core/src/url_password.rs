//! The url password rule: which part of a url's userinfo a send would use as a password, and
//! the url without it. Drafts, kept sends and saved examples all read userinfo through here.

use crate::collection::DraftSnapshot;
use crate::collection::snapshot::visit_strings;
use crate::encode_url::authority_span;
use crate::interpolate::placeholders;
use crate::resolve::{URL_PADDING, with_scheme};
use yaml_serde::Value;

/// The request `url` field without its userinfo password, read as the executor sends it: a
/// schemeless url is sent as `http://`. The user splits from the password at the first `:` or
/// `%3A`; a password that is one `{{name}}` placeholder stays, and an empty user drops the whole
/// userinfo.
pub fn url_without_password(url: &str) -> String {
    without_password(url, Reading::Request)
}

/// Any other text without the userinfo password of a url it holds, by the same rule as
/// [`url_without_password`]; text with no `scheme://` (a JSON body, say) is returned as is.
pub fn text_without_password(text: &str) -> String {
    without_password(text, Reading::FreeText)
}

impl DraftSnapshot {
    /// Drops every url password in the snapshot: the request's `url` by the request reading,
    /// every other string by the free-text one, never the `type` or `method` tokens. Whether
    /// anything changed.
    pub fn drop_url_passwords(&mut self) -> bool {
        let key = self.request_key();
        let mut changed = false;
        let mut apply = |s: &mut String, reading: Reading| {
            let kept = without_password(s, reading);
            if kept != *s {
                *s = kept;
                changed = true;
            }
        };
        for (name, value) in self.root_mut().iter_mut() {
            match value {
                Value::Mapping(request) if name.as_str() == Some(key) => {
                    for (field, v) in request.iter_mut() {
                        match (field.as_str(), v) {
                            (Some("url"), Value::String(url)) => apply(url, Reading::Request),
                            (Some("type" | "method"), Value::String(_)) => {}
                            (_, v) => visit_strings(v, &mut |s| apply(s, Reading::FreeText)),
                        }
                    }
                }
                v => visit_strings(v, &mut |s| apply(s, Reading::FreeText)),
            }
        }
        changed
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Reading {
    /// A request's `url` field, schemeless allowed.
    Request,
    /// Any other string; only a `scheme://` url in it is rewritten by the text rule.
    FreeText,
}

/// The text rule first, keeping an ordinary url's bytes; then the parser, reading the text as
/// is and as the executor would send it, so a password it still finds is cleared too; then a
/// hide-only pass that ends the authority only at `/?#`, as a non-special scheme reads it.
fn without_password(text: &str, reading: Reading) -> String {
    // A url parser skips leading spaces and control characters, so the rule does too.
    let body = text.trim_start_matches(|c: char| c.is_whitespace() || c.is_control());
    let lead = &text[..text.len() - body.len()];
    let schemed = reading == Reading::Request || has_scheme(body);
    let kept = match schemed.then(|| text_rule(body, reading)).flatten() {
        Some(Rule::Placeholder) => return text.to_owned(),
        Some(Rule::Dropped(dropped)) => dropped,
        None => body.to_owned(),
    };
    let mut forms = vec![kept.clone()];
    if schemed {
        forms.push(with_scheme(kept.clone()));
    }
    let judged = forms
        .iter()
        .find_map(|form| parser_rule(form))
        .unwrap_or(kept);
    let wide = match schemed.then(|| rule_at(&judged, span(&judged, WIDE_AUTHORITY_END), reading)) {
        Some(Some(Rule::Dropped(dropped))) => dropped,
        _ => judged,
    };
    format!("{lead}{wide}")
}

/// What the text rule made of the userinfo.
enum Rule {
    /// The password, a lone placeholder, stays.
    Placeholder,
    /// The url without its password.
    Dropped(String),
}

/// The url as the bytes show it without its password; `None` when there is nothing to drop.
/// The authority ends at `\` as a special scheme's parser reads it. A placeholder stays only
/// when the `/?#` reading of the authority agrees; otherwise the parser judges.
fn text_rule(url: &str, reading: Reading) -> Option<Rule> {
    let exact = rule_at(url, span(url, AUTHORITY_END), reading);
    let agree = span(url, AUTHORITY_END) == span(url, WIDE_AUTHORITY_END);
    match exact {
        Some(Rule::Placeholder) if !agree => None,
        exact => exact,
    }
}

/// The text rule over the userinfo at `span` of `url`.
fn rule_at(url: &str, span: Option<(usize, usize)>, reading: Reading) -> Option<Rule> {
    let (start, at) = span?;
    let (user, password) = split_userinfo(&url[start..at])?;
    if password.is_empty() {
        return None;
    }
    if is_lone_placeholder(password) {
        return Some(Rule::Placeholder);
    }
    // In free text, whitespace in the user or host may mark prose, which the parser then judges.
    let host = &url[at + 1..];
    let host = &host[..host.find(['/', '?', '#', '\\']).unwrap_or(host.len())];
    if reading == Reading::FreeText
        && (user.contains(char::is_whitespace) || host.contains(char::is_whitespace))
    {
        return None;
    }
    let userinfo = if user.is_empty() {
        String::new()
    } else {
        format!("{user}@")
    };
    Some(Rule::Dropped(format!(
        "{}{userinfo}{}",
        &url[..start],
        &url[at + 1..]
    )))
}

/// The user and password of `userinfo`, split at its first colon after one percent-decode (a
/// literal `:` or `%3A`); `None` when it has no colon.
fn split_userinfo(userinfo: &str) -> Option<(&str, &str)> {
    let at = colon(userinfo)?;
    let width = if userinfo[at..].starts_with(':') {
        1
    } else {
        3
    };
    Some((&userinfo[..at], &userinfo[at + width..]))
}

/// The byte index of the first `:` or `%3A` (either case) in `text`.
fn colon(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    (0..bytes.len()).find(|&i| {
        bytes[i] == b':' || (bytes[i..].len() >= 3 && bytes[i..i + 3].eq_ignore_ascii_case(b"%3a"))
    })
}

/// The parsed url without the password the parser finds, in its password or after a decoded
/// colon in its username; `None` when it finds none or `text` is no url.
fn parser_rule(text: &str) -> Option<String> {
    let mut url = url::Url::parse(text).ok()?;
    let username = url.username().to_owned();
    let user = match split_userinfo(&username) {
        Some((user, password)) if !password.is_empty() => user.to_owned(),
        _ if url.password().is_some() => username,
        _ => return None,
    };
    url.set_password(None).ok()?;
    url.set_username(&user).ok()?;
    Some(url.to_string())
}

/// Whether `text` starts with `scheme://`, a literal scheme or one built from placeholders.
fn has_scheme(text: &str) -> bool {
    if authority_span(text).is_some() {
        return true;
    }
    text.find("://").is_some_and(|sep| {
        let scheme = &text[..sep];
        scheme.contains("{{") && !scheme.contains(|c: char| c.is_whitespace() || "/?#@".contains(c))
    })
}

/// Where the userinfo of the authority the executor sends to lies in `url`: its start and the
/// byte index of the `@` that ends it; `None` when the authority has no `@`. The userinfo is
/// everything before the authority's last `@`, as the URL standard reads it.
pub(crate) fn userinfo_span(url: &str) -> Option<(usize, usize)> {
    span(url, AUTHORITY_END)
}

/// Where a special scheme's parser ends the authority.
const AUTHORITY_END: [char; 4] = ['/', '?', '#', '\\'];
/// Where any other scheme's parser ends it.
const WIDE_AUTHORITY_END: [char; 3] = ['/', '?', '#'];

/// [`userinfo_span`] with the authority ending at the first of `end`.
fn span<const N: usize>(url: &str, end: [char; N]) -> Option<(usize, usize)> {
    let lead = url.len() - url.trim_start_matches(URL_PADDING).len();
    let start = lead + authority_start(&url[lead..]);
    let rest = &url[start..];
    let authority = &rest[..rest.find(end).unwrap_or(rest.len())];
    authority.rfind('@').map(|at| (start, start + at))
}

/// Where the authority of `url` starts: past `://` when what precedes it is a scheme, a literal
/// one or text such as `{{proto}}` holding none of `/?#@`, and otherwise at the start.
fn authority_start(url: &str) -> usize {
    if let Some(span) = authority_span(url) {
        return span.start;
    }
    match url.find("://") {
        Some(sep) if sep > 0 && !url[..sep].contains(['/', '?', '#', '@']) => sep + 3,
        _ => 0,
    }
}

pub(crate) fn is_lone_placeholder(text: &str) -> bool {
    matches!(placeholders(text).as_slice(), [(range, _)] if *range == (0..text.len()))
}
