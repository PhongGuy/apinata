//! Scrubbing of secret values from text and URLs before the app writes them to disk.

use apinata_core::{FormKind, FormPolicy};
use percent_encoding::percent_decode_str;

const REDACTED: &str = "[redacted]";

/// Whether `text` holds the placeholder a scrub leaves where a secret was.
pub(crate) fn holds_placeholder(text: &str) -> bool {
    text.contains(REDACTED)
}

/// `text` with every non-empty secret replaced by `[redacted]`, in each form it may take
/// on the wire: raw, JSON-escaped, percent-encoded (also with `/` literal or space as `+`),
/// and the further forms of `apinata_core::encoded_forms`.
pub fn scrub_text(text: &str, secrets: &[String]) -> String {
    Forms::new(secrets).scrub(text)
}

/// `snap` with every url password dropped, as `DraftSnapshot::drop_url_passwords` does, and
/// every string value scrubbed of `secrets`, building the forms once for all.
pub fn scrub_snapshot(snap: &mut apinata_core::DraftSnapshot, secrets: &[String]) {
    snap.drop_url_passwords();
    let forms = Forms::new(secrets);
    snap.map_strings(|s| forms.scrub(s));
}

/// `text` without the userinfo password of a url it holds, by core's free-text reading.
pub fn without_password(text: &str) -> String {
    apinata_core::text_without_password(text)
}

/// Every wire form of a secret list, built once so a caller scrubbing many fields reuses it.
pub struct Forms {
    /// Longest first; Text needles are stored upper-cased, since they match in any ASCII case.
    needles: Vec<(String, FormKind)>,
    /// The secrets themselves, for the base64 run scan that finds short values.
    values: Vec<String>,
}

impl Forms {
    /// The generator's forms of every non-empty secret, ordered longest first.
    pub fn new(secrets: &[String]) -> Forms {
        #[cfg(test)]
        tests::BUILT.with(|n| n.set(n.get() + 1));
        let mut needles: Vec<(String, FormKind)> = Vec::new();
        let mut values = Vec::new();
        for secret in secrets.iter().filter(|s| !s.is_empty()) {
            values.push(secret.clone());
            for form in apinata_core::wire_forms(secret, FormPolicy::new(4, 6)) {
                let text = match form.kind {
                    FormKind::Text => form.text.to_ascii_uppercase(),
                    _ => form.text,
                };
                let entry = (text, form.kind);
                if !needles.contains(&entry) {
                    needles.push(entry);
                }
            }
        }
        // Longest first over every form, so a short secret's form cannot split a longer one.
        needles.sort_by_key(|(f, _)| std::cmp::Reverse(f.len()));
        Forms { needles, values }
    }

    /// `text` with every form replaced by `[redacted]`, then each base64 run decoding to
    /// text that holds a secret (the only way to see a 4 byte value inside a credential).
    pub fn scrub(&self, text: &str) -> String {
        // Each written placeholder is parked as a stand-in char while later needles run, so a
        // short secret such as `acted` cannot match inside it; none free means no parking.
        let parked = self.stand_in(text);
        let mark = parked.map_or(REDACTED.to_owned(), String::from);
        let out = self
            .needles
            .iter()
            .fold(text.to_owned(), |t, (form, kind)| match kind {
                FormKind::Text => apinata_core::replace_ascii_ci(&t, form, &mark),
                FormKind::Base64Fragment => apinata_core::replace_base64_fragment(&t, form, &mark),
                _ => t.replace(form.as_str(), &mark),
            });
        let values: Vec<&str> = self.values.iter().map(String::as_str).collect();
        let out = apinata_core::redact_base64_runs(&out, &values, REDACTED);
        match parked {
            Some(c) => out.replace(c, REDACTED),
            None => out,
        }
    }

    /// A private-use char found in neither `text` nor any form, to stand for a placeholder.
    fn stand_in(&self, text: &str) -> Option<char> {
        ('\u{E000}'..='\u{F8FF}').find(|c| {
            !text.contains(*c)
                && !self.needles.iter().any(|(f, _)| f.contains(*c))
                && !self.values.iter().any(|v| v.contains(*c))
        })
    }

    /// Whether `bytes` contain any form's bytes or a base64 run holding a secret, for a body
    /// that may not be UTF-8. Never reports absence where `scrub` would change the text.
    pub fn in_bytes(&self, bytes: &[u8]) -> bool {
        let folded = bytes.to_ascii_uppercase();
        let found = self.needles.iter().any(|(form, kind)| {
            let haystack = if *kind == FormKind::Text {
                &folded
            } else {
                bytes
            };
            !form.is_empty()
                && haystack
                    .windows(form.len())
                    .any(|window| window == form.as_bytes())
        });
        found || {
            let text = String::from_utf8_lossy(bytes);
            self.scrub(&text) != text
        }
    }
}

/// `url` with its userinfo password dropped and every component carrying a secret,
/// raw or percent-encoded, replaced by `[redacted]`; other text is kept byte for byte.
pub fn scrub_url(url: &str, secrets: &[String]) -> String {
    let mut out = String::with_capacity(url.len());
    let has_scheme = scheme_end(url).is_some();
    let rest = match scheme_end(url) {
        Some(end) => {
            let (scheme, rest) = url.split_at(end);
            out.push_str(scheme);
            // Split by hand rather than through `url::Url`, which would normalise the string.
            let (authority, rest) = split_authority(rest);
            // The password is dropped whatever it holds; the username is kept if harmless.
            match authority.rsplit_once('@') {
                Some((userinfo, host)) => {
                    let user = userinfo.split_once(':').map_or(userinfo, |(user, _)| user);
                    out.push_str(&component(user, secrets, false));
                    out.push('@');
                    out.push_str(&redact_host(host, secrets));
                }
                None => out.push_str(&redact_host(authority, secrets)),
            }
            rest
        }
        None => url,
    };
    let (rest, fragment) = match rest.split_once('#') {
        Some((before, fragment)) => (before, Some(fragment)),
        None => (rest, None),
    };
    let (path, query) = match rest.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (rest, None),
    };
    // Without a scheme the "path" may be free text, so only a URL's segments go whole.
    if !has_scheme {
        out.push_str(path);
    } else {
        let raw: Vec<&str> = path.split('/').collect();
        let covered = spanning_segments(&raw, secrets);
        let segments: Vec<String> = raw
            .iter()
            .zip(covered)
            .map(|(s, covered)| {
                if covered {
                    REDACTED.to_owned()
                } else {
                    component(s, secrets, false)
                }
            })
            .collect();
        out.push_str(&segments.join("/"));
    }
    if let Some(query) = query {
        let parts: Vec<String> = query
            .split('&')
            .map(|part| match part.split_once('=') {
                Some((name, value)) => format!(
                    "{}={}",
                    component(name, secrets, true),
                    component(value, secrets, true)
                ),
                None => component(part, secrets, true),
            })
            .collect();
        out.push('?');
        out.push_str(&parts.join("&"));
    }
    if let Some(fragment) = fragment {
        out.push('#');
        out.push_str(&component(fragment, secrets, false));
    }
    // A secret the structural pass could not isolate (the host, say) is still caught here.
    scrub_text(&out, secrets)
}

/// `host` with each secret of 4 or more characters replaced regardless of ASCII case, since
/// a parser lowercases the host and a short secret has no lowercase form of its own.
fn redact_host(host: &str, secrets: &[String]) -> String {
    let mut out = host.to_owned();
    for secret in secrets.iter().filter(|s| s.chars().count() >= 4) {
        let needle = secret.to_ascii_lowercase();
        // ASCII lowercasing keeps byte offsets, so a match in the lowered copy indexes `out`.
        // Searching resumes past each placeholder so it cannot be matched itself.
        let mut from = 0;
        while let Some(found) = out.to_ascii_lowercase()[from..].find(&needle) {
            let at = from + found;
            out.replace_range(at..at + needle.len(), REDACTED);
            from = at + REDACTED.len();
        }
    }
    out
}

/// For each encoded path segment, whether a secret found in the decoded joined path
/// overlaps it, so a secret spanning `/` redacts every segment it touches.
fn spanning_segments(segments: &[&str], secrets: &[String]) -> Vec<bool> {
    let decoded: Vec<String> = segments
        .iter()
        .map(|s| percent_decode_str(s).decode_utf8_lossy().into_owned())
        .collect();
    let joined = decoded.join("/");
    // Byte range of each decoded segment within the joined path.
    let mut ranges = Vec::with_capacity(decoded.len());
    let mut start = 0;
    for segment in &decoded {
        ranges.push(start..start + segment.len());
        start += segment.len() + 1;
    }
    let mut covered = vec![false; segments.len()];
    for secret in secrets.iter().filter(|s| !s.is_empty()) {
        for (at, _) in joined.match_indices(secret.as_str()) {
            let end = at + secret.len();
            for (flag, range) in covered.iter_mut().zip(&ranges) {
                *flag |= at < range.end && end > range.start;
            }
        }
    }
    covered
}

/// The authority at the start of `rest` (the text after `scheme://`) and what follows it.
fn split_authority(rest: &str) -> (&str, &str) {
    rest.split_at(rest.find(['/', '?', '#']).unwrap_or(rest.len()))
}

/// The byte offset just past `scheme://` when `url` starts with a valid URL scheme.
fn scheme_end(url: &str) -> Option<usize> {
    let (scheme, _) = url.split_once("://")?;
    let mut chars = scheme.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    valid.then_some(scheme.len() + 3)
}

/// The encoded URL component kept as is, or `[redacted]` whole when it carries a secret
/// raw or once percent-decoded; a `form` (query) component is also read with `+` as space.
fn component(encoded: &str, secrets: &[String], form: bool) -> String {
    let decoded = percent_decode_str(encoded).decode_utf8_lossy();
    let form_decoded = form.then(|| {
        percent_decode_str(&encoded.replace('+', " "))
            .decode_utf8_lossy()
            .into_owned()
    });
    let carries = secrets.iter().filter(|s| !s.is_empty()).any(|s| {
        encoded.contains(s.as_str())
            || decoded.contains(s.as_str())
            || form_decoded
                .as_deref()
                .is_some_and(|f| f.contains(s.as_str()))
    });
    if carries {
        REDACTED.to_owned()
    } else {
        encoded.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::Forms;

    thread_local! {
        /// How many `Forms` this thread has built.
        pub(super) static BUILT: Cell<usize> = const { Cell::new(0) };
    }

    #[test]
    fn a_snapshot_with_several_strings_builds_the_forms_once() {
        let json = r#"{"http":{"method":"GET","url":"https://x/a-secret","headers":[{"name":"a","value":"a-secret"},{"name":"b","value":"b-secret"}]}}"#;
        let mut snap = apinata_core::DraftSnapshot::from_json(json).unwrap();
        let before = BUILT.with(Cell::get);
        super::scrub_snapshot(&mut snap, &["a-secret".to_owned(), "b-secret".to_owned()]);
        assert_eq!(BUILT.with(Cell::get) - before, 1);
        assert!(!snap.to_json().contains("-secret"));
    }

    #[test]
    fn in_bytes_sees_a_base64_run_that_only_decoding_reveals() {
        let forms = Forms::new(&["abcd".to_owned()]);
        assert!(forms.in_bytes(b"Basic dXNlcjphYmNk"));
        assert!(!forms.in_bytes(b"nothing to see"));
    }

    #[test]
    fn in_bytes_sees_a_mixed_case_percent_echo() {
        let forms = Forms::new(&["a/b c".to_owned()]);
        assert!(forms.in_bytes(b"x a%2Fb%20C y"));
        assert!(forms.in_bytes(b"x A%2fB%20c y"));
        assert!(!forms.in_bytes(b"nothing to see"));
    }
}
