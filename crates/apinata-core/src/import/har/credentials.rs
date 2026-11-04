//! Captured credentials to valueless secret variables of a `Capture` environment (plan 4k
//! Decisions 8 to 12). Values live only in `Credentials` for the length of the parse.

use super::budget::{Budget, Prefixed};
use super::secret_names;
use crate::edit::{AuthSpec, KeyValue};
use crate::env_file::EnvVariable;
use crate::import::{DocumentError, ImportNote, ImportedEnvironment, NoteKind};
use crate::url_params::split_on_first;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use percent_encoding::percent_decode_str;
use std::collections::HashMap;
use url::Url;

/// Variable names for captured credentials; values live here only for the parse.
pub(super) struct Credentials {
    /// `(base, value)` to the variable holding that value.
    named: HashMap<(String, String), String>,
    /// The last suffix used per base; the base itself counts as 1.
    suffixes: HashMap<String, u32>,
    /// Variable names in first-use order with the input path that first used each.
    order: Vec<(String, String)>,
}

impl Credentials {
    pub(super) fn new() -> Credentials {
        Credentials {
            named: HashMap::new(),
            suffixes: HashMap::new(),
            order: Vec::new(),
        }
    }

    /// The variable for `value` under `<slug(host)>_<kind>`: the existing one for a known value,
    /// else `base`, `base_2`, ... from a per-base counter (never a scan).
    /// A new variable is charged one entry to `budget`; a reused one is free.
    pub(super) fn name_for(
        &mut self,
        host: &str,
        kind: &str,
        value: &str,
        source: &str,
        budget: &mut Budget,
    ) -> Result<String, DocumentError> {
        let base = format!("{}_{kind}", slug(host));
        let key = (base, value.to_owned());
        if let Some(name) = self.named.get(&key) {
            return Ok(name.clone());
        }
        budget.entry()?;
        // A suffixed name ends in a digit and no base does, so suffixes never meet a base.
        let n = self.suffixes.entry(key.0.clone()).or_insert(0);
        *n += 1;
        let name = match *n {
            1 => key.0.clone(),
            n => format!("{}_{n}", key.0),
        };
        self.order.push((name.clone(), source.to_owned()));
        self.named.insert(key, name.clone());
        Ok(name)
    }

    /// The `Capture` environment, or `None` when nothing was captured.
    pub(super) fn environment(self) -> Option<ImportedEnvironment> {
        if self.order.is_empty() {
            return None;
        }
        let (names, sources): (Vec<String>, Vec<String>) = self.order.into_iter().unzip();
        Some(ImportedEnvironment {
            name: "Capture".to_owned(),
            variables: names.into_iter().map(EnvVariable::secret).collect(),
            notes: Vec::new(),
            sources,
        })
    }
}

/// `host` with each run of characters that are not ASCII alphanumeric as one `_`.
pub(super) fn slug(host: &str) -> String {
    let mut out = String::with_capacity(host.len());
    for c in host.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out
}

/// What an `Authorization` header maps to: Basic credentials or a Bearer token.
enum Scheme {
    Basic(String, String),
    Bearer(String),
}

/// A Basic header (base64 of UTF-8 text holding a colon, split at the first) or a Bearer one
/// with a token; the scheme matches ignoring ASCII case.
fn scheme(value: &str) -> Option<Scheme> {
    let (name, rest) = value.trim().split_once(char::is_whitespace)?;
    let rest = rest.trim();
    if name.eq_ignore_ascii_case("bearer") && !rest.is_empty() {
        return Some(Scheme::Bearer(rest.to_owned()));
    }
    if !name.eq_ignore_ascii_case("basic") {
        return None;
    }
    let text = String::from_utf8(STANDARD.decode(rest).ok()?).ok()?;
    let (user, password) = text.split_once(':')?;
    Some(Scheme::Basic(user.to_owned(), password.to_owned()))
}

/// One request's place in the capture: the host its variables are named for, the note
/// prefix `<host> / <name> / ` and the entry path `entries[i].`.
pub(super) struct Scrub<'a> {
    pub(super) host: &'a str,
    pub(super) at: &'a str,
    pub(super) entry: &'a str,
    pub(super) creds: &'a mut Credentials,
}

impl Scrub<'_> {
    /// `{{name}}` for `value` under `<slug(host)>_<kind>` (`Credentials::name_for`), plus a
    /// Converted note at `{at}{path}` naming `what`; a new variable's source is `{entry}{path}`.
    pub(super) fn reference(
        &mut self,
        path: &str,
        kind: &str,
        what: &str,
        value: &str,
        budget: &mut Budget,
        notes: &mut Vec<ImportNote>,
    ) -> Result<String, DocumentError> {
        let source = format!("{}{path}", self.entry);
        let name = self
            .creds
            .name_for(self.host, kind, value, &source, budget)?;
        let detail = converted(what, &name);
        notes.push(budget.note(self.at, path, NoteKind::Converted, detail)?);
        Ok(format!("{{{{{name}}}}}"))
    }
}

/// Rewrite one request's headers and cookies by Decisions 10 and 11; returns its auth.
/// `headers` pairs each header with its input index, for note sources. A credential header
/// with a blank value is kept as captured, since there is nothing to hide.
pub(super) fn apply(
    scrub: &mut Scrub,
    headers: &mut Vec<(usize, KeyValue)>,
    cookies: &[(String, String)],
    budget: &mut Budget,
    notes: &mut Vec<ImportNote>,
) -> Result<AuthSpec, DocumentError> {
    let mut auth = AuthSpec::None;
    let mut mapped = None;
    let mut removed = Vec::new();
    let mut has_cookie = false;
    for (pos, (k, header)) in headers.iter_mut().enumerate() {
        let path = format!("request.headers[{k}]");
        let lower = header.name.to_ascii_lowercase();
        if lower == "referer" {
            if !header.value.trim().is_empty()
                && !referer(scrub, &path, &mut header.value, budget, notes)?
            {
                removed.push(pos);
            }
            continue;
        }
        let kind = match lower.as_str() {
            "authorization" => Some(("authorization", "Authorization")),
            "proxy-authorization" => Some(("proxy_authorization", "Proxy-Authorization")),
            "cookie" => {
                has_cookie = true;
                Some(("cookie", "Cookie"))
            }
            _ => None,
        };
        let credential = kind.is_some() || secret_names::is_header(&header.name);
        if !credential || header.value.trim().is_empty() {
            continue;
        }
        let mut secret = |kind: &str, value: &str, what: &str| {
            scrub.reference(&path, kind, what, value, budget, notes)
        };
        // Only the first Basic or Bearer `Authorization` becomes the request's auth; the header
        // is then removed, and any other one keeps a reference in place of its value.
        if lower == "authorization" && mapped.is_none() {
            match scheme(&header.value) {
                Some(Scheme::Basic(user, password)) => {
                    auth = AuthSpec::Basic {
                        username: part(user, |u| secret("username", &u, "Authorization"))?,
                        password: part(password, |p| secret("password", &p, "Authorization"))?,
                    };
                    mapped = Some(pos);
                    continue;
                }
                Some(Scheme::Bearer(token)) => {
                    auth = AuthSpec::Bearer {
                        token: secret("token", &token, "Authorization")?,
                    };
                    mapped = Some(pos);
                    continue;
                }
                None => {}
            }
        }
        header.value = match kind {
            Some((kind, what)) => secret(kind, &header.value, what)?,
            None => secret(&slug(&header.name), &header.value, &header.name)?,
        };
    }
    // Removing from the highest position down keeps the lower positions valid.
    removed.extend(mapped);
    removed.sort_unstable();
    for pos in removed.into_iter().rev() {
        headers.remove(pos);
    }
    if !has_cookie && !cookies.is_empty() {
        let joined: Vec<String> = cookies.iter().map(|(n, v)| format!("{n}={v}")).collect();
        let value = joined.join("; ");
        let reference = scrub.reference(
            "request.cookies",
            "cookie",
            "cookies",
            &value,
            budget,
            notes,
        )?;
        let index = headers.len();
        headers.push((index, KeyValue::new("Cookie", reference)));
    }
    Ok(auth)
}

/// Scrub a `Referer` value like the request URL (Decision 7), with notes at `path`: userinfo
/// is cut, then query and fragment credentials become references named for the Referer's own
/// host, which a partial URI takes from the request. Returns false when userinfo survives the
/// cut, so the caller removes the header.
fn referer(
    scrub: &mut Scrub,
    path: &str,
    value: &mut String,
    budget: &mut Budget,
    notes: &mut Vec<ImportNote>,
) -> Result<bool, DocumentError> {
    let shape = RefererShape::of(value);
    let (cut, userinfo) = shape.cut(value);
    if userinfo.is_some() {
        let detail = "the Referer user name and password are not kept";
        notes.push(budget.note(scrub.at, path, NoteKind::Dropped, detail)?);
    }
    // The URL parser strips TAB and newlines that the textual cut does not see, so userinfo
    // found in the stripped text, or by a parse, means the cut missed it and the header goes.
    // The textual recheck also covers a value the parser rejects.
    let squeezed: String = cut
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    let missed = RefererShape::of(&squeezed).cut(&squeezed).1.is_some();
    let parsed = match RefererShape::of(&cut) {
        RefererShape::Scheme => Url::parse(&cut).ok(),
        _ => Url::parse(&format!("https://{}/", scrub.host))
            .and_then(|base| base.join(&cut))
            .ok(),
    };
    if missed
        || parsed
            .as_ref()
            .is_some_and(|u| !u.username().is_empty() || u.password().is_some())
    {
        let detail = "Referer credentials could not be removed";
        notes.push(budget.note(scrub.at, path, NoteKind::Dropped, detail)?);
        return Ok(false);
    }
    let host = parsed
        .as_ref()
        .and_then(Url::host_str)
        .unwrap_or(scrub.host);
    let mut own = Scrub {
        host,
        at: scrub.at,
        entry: scrub.entry,
        creds: &mut *scrub.creds,
    };
    *value = rewrite_url(&mut own, path, &cut, budget, notes)?;
    Ok(true)
}

/// Where a `Referer` value's authority is, decided from its text, never from a parse that a
/// bad port or host would fail.
#[derive(Clone, Copy)]
enum RefererShape {
    /// An RFC 3986 scheme then `:`; the authority follows it.
    Scheme,
    /// A network-path reference: `//` (or `\\`) then the authority.
    NetworkPath,
    /// A path, query or fragment with no authority.
    Path,
}

impl RefererShape {
    /// The shape of `text` after the leading spaces and controls the URL parser trims, with TAB
    /// and newlines ignored as the parser ignores them.
    fn of(text: &str) -> RefererShape {
        let t: String = text
            .trim_start_matches(|c: char| c <= ' ')
            .chars()
            .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
            .collect();
        if t.starts_with(['/', '\\']) && t[1..].starts_with(['/', '\\']) {
            return RefererShape::NetworkPath;
        }
        let scheme = t.split_once(':').is_some_and(|(s, _)| {
            s.starts_with(|c: char| c.is_ascii_alphabetic())
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        });
        if scheme {
            RefererShape::Scheme
        } else {
            RefererShape::Path
        }
    }

    /// `cut_userinfo` by shape: a network path is cut behind a placeholder scheme placed after
    /// its leading spaces, and a bare path has no authority to cut.
    fn cut(self, text: &str) -> (String, Option<(String, String)>) {
        match self {
            RefererShape::Scheme => cut_userinfo(text),
            RefererShape::Path => (text.to_owned(), None),
            RefererShape::NetworkPath => {
                let lead = text.len() - text.trim_start_matches(|c: char| c <= ' ').len();
                let (cut, userinfo) =
                    cut_userinfo(&format!("{}x:{}", &text[..lead], &text[lead..]));
                (format!("{}{}", &cut[..lead], &cut[lead + 2..]), userinfo)
            }
        }
    }
}

/// The URL with its userinfo cut textually (the authority read as `Url` reads a special
/// scheme) and the percent-decoded (username, password) when an `@` was found; no budget, no
/// variables. The captured text is cut in place rather than rebuilt through `Url`, which would
/// lower-case the host and re-encode the rest.
pub(super) fn cut_userinfo(url: &str) -> (String, Option<(String, String)>) {
    // The authority follows the scheme's `:` and any run of `/` or `\`, and ends at the first
    // `/`, `\`, `?` or `#`.
    let Some(colon) = url.find(':') else {
        return (url.to_owned(), None);
    };
    let start = url.len() - url[colon + 1..].trim_start_matches(['/', '\\']).len();
    let end = url[start..]
        .find(['/', '\\', '?', '#'])
        .map_or(url.len(), |i| start + i);
    let Some(at_sign) = url[start..end].rfind('@').map(|i| start + i) else {
        return (url.to_owned(), None);
    };
    let (user, password) = split_on_first(&url[start..at_sign], ':');
    let decode = |s: &str| percent_decode_str(s).decode_utf8_lossy().into_owned();
    let userinfo = (decode(user), decode(password.unwrap_or_default()));
    (
        format!("{}{}", &url[..start], &url[at_sign + 1..]),
        Some(userinfo),
    )
}

/// Names the userinfo variables (empty parts make none) and rewrites the credential pairs of
/// the query and fragment by `rewrite_url`; notes at `request.url`. Returns the URL to write
/// and the userinfo Basic auth, if any.
pub(super) fn apply_url(
    scrub: &mut Scrub,
    cut: String,
    userinfo: Option<(String, String)>,
    budget: &mut Budget,
    notes: &mut Vec<ImportNote>,
) -> Result<(String, Option<AuthSpec>), DocumentError> {
    let mut secret = |kind: &str, value: &str, what: &str| {
        scrub.reference("request.url", kind, what, value, budget, notes)
    };
    let auth = match userinfo {
        Some((user, password)) if !user.is_empty() || !password.is_empty() => {
            Some(AuthSpec::Basic {
                username: part(user, |u| secret("username", &u, "URL username"))?,
                password: part(password, |p| secret("password", &p, "URL password"))?,
            })
        }
        _ => None,
    };
    let rewritten = rewrite_url(scrub, "request.url", &cut, budget, notes)?;
    Ok((rewritten, auth))
}

/// `url` with each credential pair of its query and fragment replaced by a `scrub` reference
/// noted at `path` (Decision 6); every other byte stays as captured.
pub(super) fn rewrite_url(
    scrub: &mut Scrub,
    path: &str,
    url: &str,
    budget: &mut Budget,
    notes: &mut Vec<ImportNote>,
) -> Result<String, DocumentError> {
    // The fragment starts at the first `#`; the query is the text between the first `?` before
    // it and that `#`. A hash router puts its own `?` inside the fragment and a hand-edited
    // capture may repeat `#`, so the fragment also splits at both.
    let hash = url.find('#').unwrap_or(url.len());
    let mut out = String::with_capacity(url.len());
    match url[..hash].find('?') {
        Some(q) => {
            out.push_str(&url[..=q]);
            rewrite_pairs(
                scrub,
                path,
                &url[q + 1..hash],
                &['&'],
                budget,
                notes,
                &mut out,
            )?;
        }
        None => out.push_str(&url[..hash]),
    }
    if hash < url.len() {
        out.push('#');
        rewrite_pairs(
            scrub,
            path,
            &url[hash + 1..],
            &['&', '?', '#'],
            budget,
            notes,
            &mut out,
        )?;
    }
    Ok(out)
}

/// Appends `segment` to `out` with each pair whose name satisfies `secret_names::is_query` and
/// whose value is non-empty replaced by a reference. Pairs end at any of `separators` and split
/// at their first `=`; the separators are kept, so an unmatched segment is copied unchanged.
fn rewrite_pairs(
    scrub: &mut Scrub,
    path: &str,
    segment: &str,
    separators: &[char],
    budget: &mut Budget,
    notes: &mut Vec<ImportNote>,
    out: &mut String,
) -> Result<(), DocumentError> {
    for piece in segment.split_inclusive(separators) {
        let pair = piece.strip_suffix(separators).unwrap_or(piece);
        // A name is matched as the server reads it: the URL parser strips TAB, LF and CR, and
        // the server decodes `+` as a space before percent-decoding, so `%74oken` and
        // `to<TAB>ken` are both `token` while `%2B` stays a literal `+`.
        let (raw, value) = split_on_first(pair, '=');
        let name: String = percent_decode_str(&raw.replace('+', " "))
            .decode_utf8_lossy()
            .chars()
            .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
            .collect();
        match value {
            Some(value) if !value.is_empty() && secret_names::is_query(&name) => {
                let kind = slug(secret_names::leaf(&name));
                let reference = scrub.reference(path, &kind, &name, value, budget, notes)?;
                out.push_str(raw);
                out.push('=');
                out.push_str(&reference);
                out.push_str(&piece[pair.len()..]);
            }
            _ => out.push_str(piece),
        }
    }
    Ok(())
}

/// An empty Basic part stays empty and makes no variable.
fn part(
    value: String,
    secret: impl FnOnce(String) -> Result<String, DocumentError>,
) -> Result<String, DocumentError> {
    if value.is_empty() {
        Ok(value)
    } else {
        secret(value)
    }
}

/// The detail of a replacement note; it names the variable, never the value.
fn converted(what: &str, name: &str) -> String {
    format!(
        "the captured `{what}` value is not kept; the request uses the secret `{name}` of the Capture environment"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::budget::{ENTRIES, NOTE_BYTES};

    #[test]
    fn a_reference_notes_at_the_prefix_and_sources_at_the_entry() {
        let mut creds = Credentials::new();
        let mut budget = Budget::new(NOTE_BYTES, ENTRIES);
        let mut notes = Vec::new();
        let mut scrub = Scrub {
            host: "X.test",
            at: "x.test / GET /a / ",
            entry: "entries[3].",
            creds: &mut creds,
        };
        let first = scrub
            .reference(
                "request.headers[1]",
                "x_api_key",
                "X-Api-Key",
                "v1",
                &mut budget,
                &mut notes,
            )
            .expect("fits");
        let again = scrub
            .reference(
                "request.headers[2]",
                "x_api_key",
                "X-Api-Key",
                "v1",
                &mut budget,
                &mut notes,
            )
            .expect("fits");
        assert_eq!(first, "{{x_test_x_api_key}}");
        assert_eq!(again, first);
        let at: Vec<&str> = notes.iter().map(|n| n.source.as_str()).collect();
        assert_eq!(
            at,
            [
                "x.test / GET /a / request.headers[1]",
                "x.test / GET /a / request.headers[2]"
            ]
        );
        let env = creds.environment().expect("one variable");
        assert_eq!(env.sources, ["entries[3].request.headers[1]"]);
    }

    #[test]
    fn slug_folds_each_run_of_other_characters_into_one_underscore() {
        assert_eq!(slug("my-api.example.com"), "my_api_example_com");
        assert_eq!(slug("127.0.0.1"), "127_0_0_1");
        assert_eq!(slug("a--.b"), "a_b");
    }
}
