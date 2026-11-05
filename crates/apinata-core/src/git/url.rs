//! Rules for the repository URL and folder name of a clone, and URL redaction for display.

use super::clone::CloneError;

/// The kind of address a clone URL is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CloneUrlKind {
    /// An `https://` URL.
    Https,
    /// An `http://` URL.
    Http,
    /// An `ssh://` URL or the scp form `user@host:path`.
    Ssh,
    /// A `file://` URL.
    File,
    /// An absolute filesystem path.
    Path,
}

fn bad_url(reason: &str) -> CloneError {
    CloneError::InvalidUrl {
        reason: reason.to_owned(),
    }
}

/// Accept a URL git may clone and say what kind it is. Anything that could be read as a git
/// option, a transport helper (`ext::`) or a host starting with `-` is refused before git runs.
pub fn validate_clone_url(url: &str) -> Result<CloneUrlKind, CloneError> {
    if url.trim().is_empty() {
        return Err(bad_url("the URL is empty"));
    }
    if url.starts_with('-') {
        return Err(bad_url("the URL starts with '-'"));
    }
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(bad_url(
            "the URL contains whitespace or a control character",
        ));
    }
    if let Some((scheme, _)) = url.split_once("::")
        && !scheme.is_empty()
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
    {
        return Err(bad_url("transport helpers such as ext:: are not allowed"));
    }
    if std::path::Path::new(url).is_absolute() {
        return Ok(CloneUrlKind::Path);
    }
    if let Some((scheme, rest)) = url.split_once("://") {
        let kind = match scheme.to_ascii_lowercase().as_str() {
            "https" => CloneUrlKind::Https,
            "http" => CloneUrlKind::Http,
            "ssh" => CloneUrlKind::Ssh,
            "file" => return Ok(CloneUrlKind::File),
            _ => return Err(bad_url("only https, http, ssh and file URLs are supported")),
        };
        let authority = rest.split('/').next().unwrap_or("");
        if authority.starts_with('-') {
            return Err(bad_url("the host starts with '-'"));
        }
        let host = authority.rsplit('@').next().unwrap_or("");
        if host.is_empty() {
            return Err(bad_url("the URL has no host"));
        }
        if host.starts_with('-') {
            return Err(bad_url("the host starts with '-'"));
        }
        return Ok(kind);
    }
    // The scp form: `user@host:path`, with no `/` before the colon.
    match url.split_once(':') {
        Some((left, path)) if !left.contains('/') && left.contains('@') => {
            let host = left.rsplit('@').next().unwrap_or("");
            if host.is_empty() || path.is_empty() {
                Err(bad_url("the URL has no host or path"))
            } else if host.starts_with('-') {
                Err(bad_url("the host starts with '-'"))
            } else {
                Ok(CloneUrlKind::Ssh)
            }
        }
        _ => Err(bad_url(
            "expected an https, http, ssh or file URL, user@host:path, or an absolute path",
        )),
    }
}

/// Accept `name` only when it is one plain path component.
pub fn validate_folder_name(name: &str) -> Result<(), CloneError> {
    let reason = if name.is_empty() {
        "the name is empty"
    } else if name == "." || name == ".." {
        "the name is a relative path marker"
    } else if name.starts_with('-') {
        "the name starts with '-'"
    } else if name.contains(['/', '\\', '\0']) {
        "the name contains a path separator or NUL"
    } else {
        return Ok(());
    };
    Err(CloneError::InvalidName {
        reason: reason.to_owned(),
    })
}

/// The folder a clone of `url` would get by default: its last path segment without `.git`.
/// `None` when that is empty or not a valid folder name.
pub fn default_folder_name(url: &str) -> Option<String> {
    let path = if let Some((_, rest)) = url.split_once("://") {
        rest.find('/').map_or("", |i| &rest[i..])
    } else if std::path::Path::new(url).is_absolute() {
        url
    } else {
        url.split_once(':').map_or("", |(_, path)| path)
    };
    let segment = path.trim_end_matches('/').rsplit('/').next()?;
    let name = segment.strip_suffix(".git").unwrap_or(segment);
    validate_folder_name(name).ok()?;
    Some(name.to_owned())
}

/// `text` with the userinfo of every URL-shaped word removed: `scheme://userinfo@host` of any
/// scheme, and the scp form `user:password@host:path`. A bare `user@host:path` holds no secret
/// and stays. Works on a single URL or on a sentence of git output.
///
/// The userinfo of a `scheme://` URL is what precedes the last `@` of the whitespace-free token,
/// so a password may hold `/`, `?`, `#`, `@` or `:`. That `@` is not userinfo when a `/`, `?` or
/// `#` precedes the first `:` (a path or query holding an `@`) or the token starts with `[` (an
/// IPv6 host, unless an `@` sits inside the brackets, which marks userinfo of an invalid URL).
/// It fails closed: `host:8080/a?e=a@b` reads as userinfo `host:8080/a?e=a`.
/// That suits text a user typed, such as git output; a url that was parsed takes
/// [`redact_url_userinfo`] instead.
pub fn redact_url(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for word in text.split_inclusive(char::is_whitespace) {
        let body = word.trim_end_matches(char::is_whitespace);
        let tail = &word[body.len()..];
        if body.contains("://") {
            out.push_str(&redact_schemes(body));
        } else {
            out.push_str(&redact_scp(body));
        }
        out.push_str(tail);
    }
    out
}

/// Every `scheme://` in `word` (which holds no whitespace) loses its userinfo.
fn redact_schemes(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    let mut rest = word;
    while let Some(at) = rest.find("://") {
        let (before, after) = rest.split_at(at + 3);
        out.push_str(before);
        // A nested URL in this one's query is found by the next pass of the loop.
        match userinfo_end(after) {
            Some(end) => {
                out.push_str(&after[end + 1..]);
                return out;
            }
            None => match bracketed_userinfo_end(after) {
                Some(end) => {
                    out.push('[');
                    rest = &after[end + 1..];
                }
                None => rest = after,
            },
        }
    }
    out.push_str(rest);
    out
}

/// The index of the last `@` inside a leading `[...]` of `after` (to its end when unclosed).
/// An IPv6 host never holds `@`, so the text before it is userinfo of an invalid URL.
fn bracketed_userinfo_end(after: &str) -> Option<usize> {
    let inner = after.strip_prefix('[')?;
    let close = inner.find(']').unwrap_or(inner.len());
    inner[..close].rfind('@').map(|at| at + 1)
}

/// `text` with the userinfo of every `scheme://` url removed, where the userinfo is what
/// precedes the last `@` of the authority (which ends at the first `/`, `?`, `#` or `\`), as a
/// url parser reads it. For a url that was parsed, such as one a request was sent to; a path or
/// query `@` stays.
pub fn redact_url_userinfo(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for word in text.split_inclusive(char::is_whitespace) {
        out.push_str(&redact_authorities(word));
    }
    redact_nested_userinfo(&out)
}

/// A url-valued header's text without userinfo: the authority-only rule of
/// [`redact_url_userinfo`], then the scp form (keeping a leading `url=` key), then
/// scheme-relative urls.
pub(crate) fn redact_header_url(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for word in text.split_inclusive(char::is_whitespace) {
        let body = word.trim_end_matches(char::is_whitespace);
        let tail = &word[body.len()..];
        if body.contains("://") {
            out.push_str(&redact_authorities(body));
        } else {
            out.push_str(&redact_scp_after_key(body));
        }
        out.push_str(tail);
    }
    redact_nested_userinfo(&redact_network_paths(&out))
}

/// `text` without the userinfo of each segment that starts after `//` (not `://`, whose
/// authority the passes before handled) or `\`, when a `:` comes before an `@` that comes
/// before the segment's end. Display only; a colon-less `//a@b` stays.
fn redact_nested_userinfo(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while let Some(c) = text[i..].chars().next() {
        let before = &text[..i];
        let starts = (before.ends_with("//") && !before.ends_with("://")) || before.ends_with('\\');
        if starts && let Some(at) = nested_userinfo_end(&text[i..]) {
            i += at + 1;
            continue;
        }
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// The index of the `@` closing a password-holding userinfo at the start of `segment`.
fn nested_userinfo_end(segment: &str) -> Option<usize> {
    let end = segment
        .find(|c: char| matches!(c, '/' | '?' | '#' | '\\') || c.is_whitespace())
        .unwrap_or(segment.len());
    let at = segment[..end].rfind('@')?;
    segment[..at].contains(':').then_some(at)
}

/// Every `scheme://` in `word` loses the userinfo of its authority; a url nested in the query of
/// another is found by the next pass of the loop.
fn redact_authorities(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    let mut rest = word;
    while let Some(at) = rest.find("://") {
        let (before, after) = rest.split_at(at + 3);
        out.push_str(before);
        rest = match authority_userinfo_end(after) {
            Some(end) => &after[end + 1..],
            None => after,
        };
    }
    out.push_str(rest);
    out
}

/// The index of the `@` closing the userinfo inside the authority that starts `after`.
fn authority_userinfo_end(after: &str) -> Option<usize> {
    let authority = &after[..after.find(['/', '?', '#', '\\']).unwrap_or(after.len())];
    let at = authority.rfind('@')?;
    (at > 0 && !authority.starts_with('[')).then_some(at)
}

/// `text` with the userinfo of every scheme-relative url (`//userinfo@host`) removed. Such a
/// url starts a word or follows `=` or a quote, as in a Refresh `url=`; a `//` inside a path
/// is left alone. One in a query or fragment is taken for a url only when its userinfo holds a
/// password, so `?next=//a@b` stays.
fn redact_network_paths(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for word in text.split_inclusive(char::is_whitespace) {
        let mut rest = word;
        let mut prev: Option<char> = None;
        let mut in_query = false;
        while let Some(at) = rest.find("//") {
            let starts_url = match rest[..at].chars().next_back().or(prev) {
                None => true,
                Some(c) => matches!(c, '=' | '\'' | '"') || c.is_whitespace(),
            };
            in_query |= rest[..at].contains(['?', '#']);
            let (before, after) = rest.split_at(at + 2);
            out.push_str(before);
            prev = Some('/');
            rest = after;
            if starts_url
                && let Some(end) = authority_userinfo_end(after)
                && (!in_query || after[..end].contains(':'))
            {
                rest = &after[end + 1..];
                prev = Some('@');
            }
        }
        out.push_str(rest);
    }
    out
}

/// The index of the `@` closing the userinfo of `after` (the text following `://`), if it has any.
fn userinfo_end(after: &str) -> Option<usize> {
    let at = after.rfind('@')?;
    let candidate = &after[..at];
    let user = candidate.split(':').next().unwrap_or("");
    if candidate.is_empty() || candidate.starts_with('[') || user.contains(['/', '?', '#']) {
        return None;
    }
    Some(at)
}

/// [`redact_scp`], keeping a leading `url=` key of a Refresh value and one quote right after it,
/// or else a lone leading quote, found before the userinfo's first `:`. Any other `=` or quote
/// is part of the username and goes with it.
fn redact_scp_after_key(word: &str) -> String {
    let shown = redact_scp(word);
    if shown == word {
        return shown;
    }
    let user = &word[..word.find(':').unwrap_or(0)];
    let key_len = match user.find('=') {
        Some(eq) if is_url_key(&user[..eq]) => {
            let quoted = matches!(word.as_bytes().get(eq + 1), Some(b'\'' | b'"'));
            eq + 1 + usize::from(quoted)
        }
        _ => usize::from(user.starts_with(['\'', '"'])),
    };
    format!("{}{shown}", &word[..key_len])
}

/// Whether `before`, the text before a word's first `=`, is a Refresh `url` key: `url` in any
/// case, alone, after one quote, or after a delay (`0;`, `1.5;'`), as in `0;url=` or `'URL=`.
/// Any other text before it makes it part of a username.
fn is_url_key(before: &str) -> bool {
    let Some(start) = before.len().checked_sub(3) else {
        return false;
    };
    let lead = &before.as_bytes()[..start];
    before.as_bytes()[start..].eq_ignore_ascii_case(b"url")
        && (matches!(lead, [] | [b'\'' | b'"']) || is_delay(lead))
}

/// Whether `lead` is a Refresh delay before its `;`: digits, an optional `.` fraction, `;` and
/// an optional quote.
fn is_delay(lead: &[u8]) -> bool {
    let lead = lead
        .strip_suffix(b"'")
        .or(lead.strip_suffix(b"\""))
        .unwrap_or(lead);
    let Some(number) = lead.strip_suffix(b";") else {
        return false;
    };
    let (whole, fraction) = match number.iter().position(|&b| b == b'.') {
        Some(dot) => (&number[..dot], Some(&number[dot + 1..])),
        None => (number, None),
    };
    !whole.is_empty()
        && whole.iter().all(u8::is_ascii_digit)
        && fraction.is_none_or(|f| f.iter().all(u8::is_ascii_digit))
}

/// `user:password@host:path` becomes `host:path`; anything else is returned unchanged.
fn redact_scp(word: &str) -> String {
    if let Some(at) = word.rfind('@') {
        let (userinfo, host_path) = (&word[..at], &word[at + 1..]);
        let host_ok = host_path
            .split_once(':')
            .is_some_and(|(host, _)| !host.is_empty() && !host.contains('/'));
        let user_ok = userinfo
            .split_once(':')
            .is_some_and(|(user, _)| !user.is_empty() && !user.contains('/'));
        if host_ok && user_ok {
            return host_path.to_owned();
        }
    }
    word.to_owned()
}
