//! Names the secret values a resolved request used, so a saved copy of the exchange can
//! swap each one for the placeholder that produced it.

use std::fmt;

use base64::Engine;
use base64::alphabet;
use base64::engine::DecodePaddingMode;
use base64::engine::general_purpose::{
    GeneralPurpose, GeneralPurposeConfig, STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD,
};

use crate::context::VarContext;
use crate::error::ResolveError;
use crate::interpolate::{Vars, interpolate};
use crate::js;
use crate::request::RequestFile;
use crate::resolve::{SendAuth, prepare, resolve_prepared};
use crate::secret_forms::{
    FormKind, FormPolicy, base64_forms, json_escaped, replace_ascii_ci, wire_forms,
};
use yaml_serde::Mapping;

/// Labels for values no variable names: an auth credential, or any other sensitive value
/// (proxy credentials, certificate passphrases, a secret with dynamic values).
const AUTH_LABEL: &str = "auth credentials";
const PLAIN_LABEL: &str = "secret value";
const REDACTED: &str = "[redacted]";
/// Shorter values would match inside unrelated text and shred it.
pub(crate) const MIN_CHARS: usize = 4;
const DERIVED_LABEL: &str = "derived credential";
/// Encoded forms under this many characters would match inside unrelated text.
const MIN_FORM_CHARS: usize = 6;

/// Whether a header's whole value is a credential, matched in any case. The one list the
/// session log, the export notes and the MCP results share.
pub fn is_credential_header(name: &str) -> bool {
    [
        "authorization",
        "proxy-authorization",
        "cookie",
        "set-cookie",
        "x-wsse",
        "x-amz-security-token",
    ]
    .iter()
    .any(|h| name.eq_ignore_ascii_case(h))
}

/// A response header's value as it may be shown: a Set-Cookie keeps only the cookie name (or
/// nothing, when its pair has none), any other credential header is hidden whole, a url-valued
/// header loses its userinfo, and the rest is unchanged.
pub fn mask_response_header(name: &str, value: &str) -> String {
    if name.eq_ignore_ascii_case("set-cookie") {
        // The name-value pair ends at the first `;`. A pair with no `=`, or an empty name, is
        // a nameless cookie whose whole pair is the value, so nothing of it is shown.
        let pair = value.split(';').next().unwrap_or_default();
        match pair.split_once('=') {
            Some((cookie, _)) if !cookie.trim().is_empty() => {
                format!("{}={REDACTED}", cookie.trim())
            }
            _ => REDACTED.to_owned(),
        }
    } else if is_credential_header(name) {
        REDACTED.to_owned()
    } else if ["location", "content-location", "refresh"]
        .iter()
        .any(|h| name.eq_ignore_ascii_case(h))
    {
        // The text rules keep the common shapes readable; the parser then catches every other
        // shape it would read credentials out of, and the value is hidden whole.
        let shown = crate::git::redact_header_url(value);
        let target = if name.eq_ignore_ascii_case("refresh") {
            refresh_url(&shown).unwrap_or(&shown)
        } else {
            &shown
        };
        if parser_reads_credentials(target) {
            REDACTED.to_owned()
        } else {
            shown
        }
    } else {
        value.to_owned()
    }
}

/// Whether `text`, resolved against an http or an https page, has a username or password,
/// or fails to resolve. Both bases count: `http:u:p@h` is a path under one, an authority
/// under the other.
fn parser_reads_credentials(text: &str) -> bool {
    ["http://base.invalid/", "https://base.invalid/"]
        .iter()
        .any(
            |base| match url::Url::parse(base).and_then(|b| b.join(text)) {
                Ok(url) => !url.username().is_empty() || url.password().is_some(),
                Err(_) => true,
            },
        )
}

/// The url of a Refresh value as a browser reads it: after the delay, a `;` or `,`, an
/// optional `url =` in any case and an optional quote that ends it. `None` when it has none.
fn refresh_url(value: &str) -> Option<&str> {
    let ws = |c: char| c.is_ascii_whitespace();
    let rest = value
        .trim_start_matches(ws)
        .trim_start_matches(|c: char| c.is_ascii_digit() || c == '.')
        .trim_start_matches(ws);
    let rest = rest.strip_prefix([';', ',']).unwrap_or(rest);
    let rest = rest.trim_start_matches(ws);
    let rest = match rest.get(..3) {
        Some(word) if word.eq_ignore_ascii_case("url") => rest[3..]
            .trim_start_matches(ws)
            .strip_prefix('=')
            .map_or(rest, |after| after.trim_start_matches(ws)),
        _ => rest,
    };
    let url = match rest.chars().next() {
        Some(quote @ ('\'' | '"')) => rest[1..].split(quote).next().unwrap_or_default(),
        _ => rest,
    };
    (!url.is_empty()).then_some(url)
}

/// One secret value a request used, with the name it came from and its stand-in text.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq)]
pub struct NamedSecret {
    label: String,
    placeholder: String,
    value: String,
}

impl NamedSecret {
    /// What the value is: a variable name such as `token` or `process.env.API_KEY`, else
    /// `auth credentials` or `secret value`.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The text that replaces the value: `{{name}}` for a variable, else `[redacted]`.
    pub fn placeholder(&self) -> &str {
        &self.placeholder
    }

    /// The secret value itself.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// This secret with `[redacted]` as its stand-in, for a record of what was sent.
    pub(crate) fn unnamed_copy(&self) -> NamedSecret {
        unnamed(&self.label, &self.value)
    }

    /// A secret that came from the variable `name`, as resolving a request would find it.
    #[cfg(test)]
    pub(crate) fn variable(name: &str, value: &str) -> NamedSecret {
        NamedSecret {
            label: name.to_owned(),
            placeholder: format!("{{{{{name}}}}}"),
            value: value.to_owned(),
        }
    }

    fn named(&self) -> bool {
        self.placeholder != REDACTED
    }
}

// The value is the secret; Debug shows only what it would be replaced with.
impl fmt::Debug for NamedSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NamedSecret")
            .field("label", &self.label)
            .field("placeholder", &self.placeholder)
            .finish_non_exhaustive()
    }
}

/// The secret values resolving `req` substitutes, named, deduplicated by value, longest first
/// with named before unnamed; values under 4 characters are left out.
///
/// Fails with the [`ResolveError`] resolving `req` would fail with.
pub fn named_secrets(
    req: &RequestFile,
    ctx: &VarContext,
) -> Result<Vec<NamedSecret>, ResolveError> {
    let prepared = prepare(req, ctx)?;
    let mode = prepared.auth_mode().to_owned();
    let resolved = resolve_prepared(prepared, ctx)?;
    let candidates = candidates(req, ctx);

    let mut found: Vec<NamedSecret> = Vec::new();
    let mut add = |secret: NamedSecret| {
        if secret.value.chars().count() >= MIN_CHARS
            && !found.iter().any(|seen| seen.value == secret.value)
        {
            found.push(secret);
        }
    };
    for value in resolved.sensitive_values() {
        let secret = match candidates.iter().find(|(_, text)| text == value) {
            Some((name, _)) => NamedSecret {
                label: name.clone(),
                placeholder: format!("{{{{{name}}}}}"),
                value: value.clone(),
            },
            None => unnamed(PLAIN_LABEL, value),
        };
        add(secret);
    }
    // Auth values come last so a credential that is a secret variable keeps its name. These kinds
    // name the effective (possibly inherited) auth's values, the SigV4 secret key included.
    if matches!(mode.as_str(), "basic" | "bearer" | "apikey" | "awsv4") {
        for value in resolved.auth_secrets() {
            add(unnamed(AUTH_LABEL, value));
        }
    }

    // Stable sort: named before unnamed, longer before shorter, otherwise first-seen order.
    found.sort_by_key(|s| (!s.named(), std::cmp::Reverse(s.value.chars().count())));
    Ok(found)
}

/// [`named_secrets`] joined by every other secret value `req`'s context knows: each sensitive
/// variable (environment, globals, collection) and each `.env` value, named, in the same
/// order and under the same 4-character floor. A global or environment secret is included
/// even when a folder, request, iteration or captured variable shadows its name. Secrets of
/// environments other than the selected one, and OS environment values the request never
/// substituted, are not included. For text that may echo a secret the request never
/// substituted, such as a saved example.
///
/// Fails with the [`ResolveError`] resolving `req` would fail with.
pub fn known_secrets(
    req: &RequestFile,
    ctx: &VarContext,
) -> Result<Vec<NamedSecret>, ResolveError> {
    let mut found = named_secrets(req, ctx)?;
    for (name, value) in logged_candidates(req, ctx) {
        if value.chars().count() >= MIN_CHARS && !found.iter().any(|seen| seen.value == value) {
            found.push(NamedSecret {
                placeholder: format!("{{{{{name}}}}}"),
                label: name,
                value,
            });
        }
    }
    found.sort_by_key(|s| (!s.named(), std::cmp::Reverse(s.value.chars().count())));
    Ok(found)
}

/// Every secret an example of `req` must not hold: [`known_secrets`] followed by the
/// [`derived_secrets`] of `derived`, so a variable keeps its placeholder.
///
/// Fails with the [`ResolveError`] resolving `req` would fail with.
pub fn request_secrets(
    req: &RequestFile,
    ctx: &VarContext,
    derived: &[String],
) -> Result<Vec<NamedSecret>, ResolveError> {
    let mut found = known_secrets(req, ctx)?;
    found.extend(derived_secrets(derived));
    Ok(found)
}

/// Wraps values the executor derived per send (signatures, nonces, tokens) for redaction:
/// deduplicated, longest first, and without values under 4 characters, as every redactor does.
pub fn derived_secrets(values: &[String]) -> Vec<NamedSecret> {
    let mut found: Vec<NamedSecret> = Vec::new();
    for value in values {
        if value.chars().count() >= MIN_CHARS && !found.iter().any(|seen| seen.value == *value) {
            found.push(unnamed(DERIVED_LABEL, value));
        }
    }
    found.sort_by_key(|s| std::cmp::Reverse(s.value.chars().count()));
    found
}

/// Every rendered sensitive-variable candidate, every rendered `.env` value, and the runtime
/// store's tainted values: what a log entry of `req` must not show, short ones included.
pub(crate) fn log_values(req: &RequestFile, ctx: &VarContext) -> Vec<String> {
    let mut values: Vec<String> = logged_candidates(req, ctx)
        .into_iter()
        .map(|(_, text)| text)
        .collect();
    values.extend(ctx.runtime_vars().tainted());
    values
}

// The candidates less the OS environment: its values reach an exchange only by substitution
// or a script read, which already mark them, and matching all of them would shred the text.
// Global and environment secrets join even when a closer layer shadows their name, since the
// shadowed value can still reach an exchange through a script or a response echo.
fn logged_candidates(req: &RequestFile, ctx: &VarContext) -> Vec<(String, String)> {
    let dotenv = ctx.dotenv_keys();
    let mut out: Vec<(String, String)> = candidates(req, ctx)
        .into_iter()
        .filter(|(label, _)| {
            label
                .strip_prefix("process.env.")
                .is_none_or(|key| dotenv.iter().any(|k| k == key))
        })
        .collect();
    for pair in ctx.layer_secrets() {
        if !out.contains(&pair) {
            out.push(pair);
        }
    }
    out
}

/// Every sensitive variable and process env value rendered as the resolver would, with the
/// name that produces it, so a substituted value can be traced back to its name.
fn candidates(req: &RequestFile, ctx: &VarContext) -> Vec<(String, String)> {
    let (vars, sensitive) = ctx.lookup_for(req);
    let mut candidates = Vec::new();
    for name in sensitive.iter().filter(|name| *name != "process") {
        if let Some(value) = vars.get(name) {
            candidates.extend(render(value, &vars).map(|text| (name.clone(), text)));
        }
    }
    for (key, value) in ctx.process_env().into_iter().flatten() {
        candidates.extend(render(value, &vars).map(|text| (format!("process.env.{key}"), text)));
    }
    candidates
}

/// `bytes` with every UTF-16LE and UTF-16BE copy of each searched value (see
/// [`wide_searched`]) replaced by `[redacted]` in the same encoding, or `None` when there is none. Text
/// redaction cannot see such a copy, though ASCII written as UTF-16 is valid UTF-8.
pub(crate) fn redact_wide<'a>(
    bytes: &[u8],
    values: impl IntoIterator<Item = &'a str>,
) -> Option<Vec<u8>> {
    // Every copy of every value in both orders is found in the original bytes, then
    // overlapping copies merge into one span, so no copy can split another or leave a tail.
    let mut spans: Vec<(usize, usize, bool)> = Vec::new();
    for value in values {
        if !wide_searched(value) {
            continue;
        }
        for big in [false, true] {
            let needle = utf16_bytes(value, big);
            for_each_match(bytes, &needle, |start| {
                spans.push((start, start + needle.len(), big));
                true
            });
        }
    }
    if spans.is_empty() {
        return None;
    }
    spans.sort_unstable();
    // A merged span takes the byte order of the copy that starts it.
    let mut out = Vec::with_capacity(bytes.len());
    let mut kept = 0;
    let mut spans = spans.into_iter().peekable();
    while let Some((start, mut end, big)) = spans.next() {
        while let Some(&(next, next_end, _)) = spans.peek() {
            if next >= end {
                break;
            }
            end = end.max(next_end);
            spans.next();
        }
        out.extend_from_slice(&bytes[kept..start]);
        out.extend_from_slice(&utf16_bytes(REDACTED, big));
        kept = end;
    }
    out.extend_from_slice(&bytes[kept..]);
    Some(out)
}

/// Whether any of `secrets` appears in `bytes`.
///
/// The bytes are read through a lossy UTF-8 copy and matched as [`replace_secrets`] matches
/// text; each secret of at least the minimum length is also matched exactly as UTF-16LE and
/// UTF-16BE. Use it for a binary body, where a replacement would corrupt the bytes: refuse to
/// write the body instead.
pub fn bytes_hold_secret(bytes: &[u8], secrets: &[NamedSecret]) -> bool {
    let lossy = String::from_utf8_lossy(bytes);
    if replace_secrets(&lossy, secrets) != lossy {
        return true;
    }
    secrets
        .iter()
        .filter(|s| wide_searched(&s.value))
        .any(|s| {
            contains_bytes(bytes, &utf16_bytes(&s.value, false))
                || contains_bytes(bytes, &utf16_bytes(&s.value, true))
        })
}

/// `value` as UTF-16 bytes in big or little endian order.
pub(crate) fn utf16_bytes(value: &str, big: bool) -> Vec<u8> {
    value
        .encode_utf16()
        .flat_map(|u| {
            if big {
                u.to_be_bytes()
            } else {
                u.to_le_bytes()
            }
        })
        .collect()
}

/// Whether a value gets UTF-16 copies searched: any non-ASCII value, as its narrow forms are,
/// and an ASCII one only from the minimum length, since a short one matches unrelated text.
pub(crate) fn wide_searched(value: &str) -> bool {
    !value.is_ascii() || value.chars().count() >= MIN_CHARS
}

pub(crate) fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    let mut found = false;
    for_each_match(haystack, needle, |_| {
        found = true;
        false
    });
    found
}

// Knuth-Morris-Pratt, so a search stays linear in the haystack whatever the needle repeats.
// Calls `hit` with each match's start, overlapping ones included, until it returns false.
fn for_each_match(haystack: &[u8], needle: &[u8], mut hit: impl FnMut(usize) -> bool) {
    if needle.is_empty() {
        hit(0);
        return;
    }
    let mut fail = vec![0usize; needle.len()];
    let mut k = 0;
    for i in 1..needle.len() {
        while k > 0 && needle[i] != needle[k] {
            k = fail[k - 1];
        }
        if needle[i] == needle[k] {
            k += 1;
        }
        fail[i] = k;
    }
    let mut k = 0;
    for (i, &b) in haystack.iter().enumerate() {
        while k > 0 && b != needle[k] {
            k = fail[k - 1];
        }
        if b == needle[k] {
            k += 1;
            if k == needle.len() {
                if !hit(i + 1 - k) {
                    return;
                }
                k = fail[k - 1];
            }
        }
    }
}

/// `text` with each secret (raw and JSON-escaped form) replaced by its placeholder and every
/// encoded form of any secret (see [`encoded_forms`]) replaced by `[redacted]`, all in one
/// longest-first pass across secrets so a secret nested in another never splits it. A base64
/// fragment also takes up to two adjacent base64 characters with it, and a base64 run that
/// decodes to text holding a secret is replaced whole (see [`redact_base64_runs`]).
pub fn replace_secrets(text: &str, secrets: &[NamedSecret]) -> String {
    // The secret's own texts carry its placeholder and match exactly; every generator form is
    // redacted, Text folded to ASCII case, so a mixed-case echo of any encoding is caught.
    let mut needles: Vec<Needle> = Vec::new();
    for secret in secrets {
        let escaped = json_escaped(&secret.value);
        let twice = json_escaped(&escaped);
        for own in [&escaped, &secret.value, &twice] {
            needles.push(Needle::exact(own.clone(), &secret.placeholder));
        }
        for form in wire_forms(&secret.value, FormPolicy::new(MIN_CHARS, MIN_FORM_CHARS)) {
            needles.push(match form.kind {
                FormKind::Text => Needle::folded(form.text),
                FormKind::Base64Fragment => Needle::fragment(form.text),
                _ => Needle::exact(form.text, REDACTED),
            });
        }
    }
    // Longest first, a placeholder before `[redacted]` on a tie; one needle per text and flag.
    needles.sort_by(|a, b| {
        (b.text.len(), a.with == REDACTED, &a.text, a.how as u8).cmp(&(
            a.text.len(),
            b.with == REDACTED,
            &b.text,
            b.how as u8,
        ))
    });
    needles.dedup_by(|b, a| a.text == b.text && (a.with == REDACTED) == (b.with == REDACTED));
    // A written placeholder is parked as a private-use stand-in char while later needles run,
    // so no folded form can match inside `{{name}}`; the stand-ins are swapped back at the end.
    let mut spare = ('\u{E000}'..='\u{F8FF}').filter(|c| !text.contains(*c));
    let mut parked: Vec<(char, &str)> = Vec::new();
    let mut out = text.to_owned();
    for n in needles {
        out = match n.how {
            Match::Exact if n.with != REDACTED => {
                let known = parked.iter().find(|(_, p)| *p == n.with).map(|(c, _)| *c);
                match known.or_else(|| spare.next().inspect(|c| parked.push((*c, n.with)))) {
                    Some(c) => out.replace(&n.text, c.encode_utf8(&mut [0u8; 4])),
                    None => out.replace(&n.text, n.with),
                }
            }
            Match::Exact => out.replace(&n.text, n.with),
            Match::Folded => replace_ascii_ci(&out, &n.text, n.with),
            Match::Fragment => replace_base64_fragment(&out, &n.text, n.with),
        };
    }
    let values: Vec<&str> = secrets.iter().map(|s| s.value.as_str()).collect();
    let mut out = redact_base64_runs(&out, &values, REDACTED);
    for (c, placeholder) in parked {
        out = out.replace(c, placeholder);
    }
    out
}

/// How a needle is matched in `replace_secrets`.
#[derive(Clone, Copy)]
enum Match {
    Exact,
    Folded,
    Fragment,
}

struct Needle<'a> {
    text: String,
    with: &'a str,
    how: Match,
}

impl<'a> Needle<'a> {
    fn exact(text: String, with: &'a str) -> Self {
        Needle {
            text,
            with,
            how: Match::Exact,
        }
    }
    fn folded(text: String) -> Self {
        Needle {
            text,
            with: REDACTED,
            how: Match::Folded,
        }
    }
    fn fragment(text: String) -> Self {
        Needle {
            text,
            with: REDACTED,
            how: Match::Fragment,
        }
    }
}

fn is_base64_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'-' | b'_' | b'=')
}

/// `text` with each occurrence of `fragment` (one of [`base64_fragment_forms`]) replaced, and
/// up to two adjacent base64-alphabet characters on each side replaced with it, since the
/// fragment's edges share bits with them. Padding `=` right after is swallowed too.
pub fn replace_base64_fragment(text: &str, fragment: &str, replacement: &str) -> String {
    if fragment.is_empty() {
        return text.to_owned();
    }
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut done = 0;
    for (start, _) in text.match_indices(fragment) {
        if start < done {
            continue;
        }
        let mut from = start;
        while start - from < 2 && from > done && is_base64_char(bytes[from - 1]) {
            from -= 1;
        }
        let mut to = start + fragment.len();
        let after = to;
        while to - after < 2 && to < bytes.len() && is_base64_char(bytes[to]) {
            to += 1;
        }
        while to < bytes.len() && bytes[to] == b'=' {
            to += 1;
        }
        out.push_str(&text[done..from]);
        out.push_str(replacement);
        done = to;
    }
    out.push_str(&text[done..]);
    out
}

/// The base64 forms of `value` that sit inside a longer encoding: the per-alignment runs of
/// characters the value alone decides, at least 6 characters, not the whole-value encodings.
pub fn base64_fragment_forms(value: &str) -> Vec<String> {
    if value.chars().count() < MIN_CHARS {
        return Vec::new();
    }
    let mut forms = Vec::new();
    base64_forms(value.as_bytes(), &mut forms);
    let whole: Vec<String> = [STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD]
        .iter()
        .map(|e| e.encode(value))
        .collect();
    forms.retain(|f| f.chars().count() >= MIN_FORM_CHARS && !whole.contains(f));
    forms.sort();
    forms.dedup();
    forms
}

const LENIENT_STD: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_padding_mode(DecodePaddingMode::Indifferent)
        .with_decode_allow_trailing_bits(true),
);
const LENIENT_URL: GeneralPurpose = GeneralPurpose::new(
    &alphabet::URL_SAFE,
    GeneralPurposeConfig::new()
        .with_decode_padding_mode(DecodePaddingMode::Indifferent)
        .with_decode_allow_trailing_bits(true),
);

// The bytes the longest decodable prefix of `run` gives in one alphabet, from alignment 0.
fn decoded_prefix(engine: &GeneralPurpose, run: &str) -> Vec<u8> {
    let mut end = run.trim_end_matches('=').len();
    while end >= 4 {
        // A single leftover character carries under 6 bits, too few for a byte.
        let usable = end - usize::from(end % 4 == 1);
        match engine.decode(&run[..usable]) {
            Ok(bytes) => return bytes,
            Err(base64::DecodeError::InvalidByte(at, _))
            | Err(base64::DecodeError::InvalidLastSymbol { offset: at, .. }) => end = at,
            Err(_) => return Vec::new(),
        }
    }
    Vec::new()
}

/// `text` with each run of base64-alphabet characters (8 or more, either alphabet) whose
/// decoding holds one of `values` replaced whole by `replacement`. Catches a 4 byte value
/// that only a whole encoding shows. A run decodes at any alignment, and a malformed tail
/// is dropped; values under 4 characters are ignored. A line break (LF or CRLF) between two
/// lines of a wrapped blob does not end a run.
pub fn redact_base64_runs(text: &str, values: &[&str], replacement: &str) -> String {
    let values: Vec<&[u8]> = values
        .iter()
        .filter(|v| v.chars().count() >= MIN_CHARS)
        .map(|v| v.as_bytes())
        .collect();
    if values.is_empty() {
        return text.to_owned();
    }
    let bytes = text.as_bytes();
    // The replacement's own letters are not a run to inspect.
    let own = replacement.trim_matches(['[', ']']);
    let mut out = String::with_capacity(text.len());
    let (mut done, mut i) = (0, 0);
    // The run's characters with line breaks dropped, and where each sits in `text`.
    let (mut run, mut origin) = (String::new(), Vec::new());
    while i < bytes.len() {
        if !is_base64_char(bytes[i]) {
            i += 1;
            continue;
        }
        let start = i;
        run.clear();
        origin.clear();
        let mut seg_start = 0;
        loop {
            while i < bytes.len() && is_base64_char(bytes[i]) {
                run.push(bytes[i] as char);
                origin.push(i);
                i += 1;
            }
            // A line break joins two lines of one wrapped blob only when the line before is
            // long and the line after is long too or the line before is a full MIME line;
            // otherwise a header value or word after the break would be swallowed.
            let eol = if bytes[i..].starts_with(b"\r\n") {
                2
            } else {
                usize::from(bytes[i..].starts_with(b"\n"))
            };
            let seg = run.len() - seg_start;
            let next = bytes[(i + eol).min(bytes.len())..]
                .iter()
                .take_while(|b| is_base64_char(**b))
                .count();
            if eol > 0 && next > 0 && seg >= 16 && (next >= 16 || seg >= 64) {
                i += eol;
                seg_start = run.len();
            } else {
                break;
            }
        }
        if run == own || run.len() < 8 {
            continue;
        }
        if let Some(at) = latest_hit_start(&run, &values) {
            out.push_str(&text[done..origin[at].max(start)]);
            out.push_str(replacement);
            done = i;
        }
    }
    out.push_str(&text[done..]);
    out
}

/// The latest start in `run` from which a decode holds one of `values`, so the redaction
/// removes the least. Per alphabet a run splits into spans at the characters it rejects, and each
/// span is decoded once at each of its four alignments: a decode from a later start is a suffix
/// of the earlier one, so one pass covers every start. A hit's group start is moved back by whole
/// groups to the nearest allowed start (see [`is_start`]), which keeps the alignment.
fn latest_hit_start(run: &str, values: &[&[u8]]) -> Option<usize> {
    let bytes = run.as_bytes();
    let mut best: Option<usize> = None;
    for (engine, rejects) in [(&LENIENT_STD, b"=-_"), (&LENIENT_URL, b"=+/")] {
        let mut begin = 0;
        while begin < bytes.len() {
            let end = bytes[begin..]
                .iter()
                .position(|b| rejects.contains(b))
                .map_or(bytes.len(), |p| begin + p);
            for floor in begin..(begin + 4).min(end) {
                let decoded = decoded_prefix(engine, &run[floor..end]);
                let k = values
                    .iter()
                    .filter_map(|v| decoded.windows(v.len()).rposition(|w| w == *v))
                    .max();
                let Some(k) = k else { continue };
                let mut at = floor + 4 * (k / 3);
                loop {
                    if at + 8 <= bytes.len() && is_start(bytes, at) {
                        best = best.max(Some(at));
                        break;
                    }
                    if at < floor + 4 {
                        break;
                    }
                    at -= 4;
                }
            }
            begin = end + 1;
        }
    }
    best
}

/// Whether a decode of `run` may begin at `at`: within the first four characters, or within
/// four after a separator.
fn is_start(run: &[u8], at: usize) -> bool {
    at < 4
        || (1..=4)
            .filter_map(|back| at.checked_sub(back))
            .any(|j| matches!(run[j], b'=' | b'/' | b'-' | b'_' | b'+'))
}

/// The text of every wire form of `value` (see [`wire_forms`]) other than the value and its
/// JSON escape, each at least 6 characters, longest first and deduplicated; empty for a value
/// under 4 characters.
pub fn encoded_forms(value: &str) -> Vec<String> {
    if value.chars().count() < MIN_CHARS {
        return Vec::new();
    }
    let escaped = json_escaped(value);
    let mut forms: Vec<String> = wire_forms(value, FormPolicy::new(MIN_CHARS, MIN_FORM_CHARS))
        .into_iter()
        .map(|f| f.text)
        .filter(|f| f.chars().count() >= MIN_FORM_CHARS && f != value && *f != escaped)
        .collect();
    forms.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    forms.dedup();
    forms
}

fn unnamed(label: &str, value: &str) -> NamedSecret {
    NamedSecret {
        label: label.to_owned(),
        placeholder: REDACTED.to_owned(),
        value: value.to_owned(),
    }
}

/// `value` expanded over `vars`; `None` when its own placeholders do not resolve, since a
/// value the request never used cannot be among its substitutions.
pub(crate) fn render(value: &serde_json::Value, vars: &Vars) -> Option<String> {
    interpolate(&js::to_text(value), vars).ok()
}

/// The credentials of the effective `auth` a resolved request carries: for basic and bearer the
/// `Authorization` value and the password or token inside it, for an API key its value as placed,
/// for SigV4 the secret key and session token, and the secrets of the digest, WSSE, OAuth 1.0 and EdgeGrid signers.
pub(crate) fn auth_secrets(
    auth: Option<&Mapping>,
    (headers, url, send_auth): (&[(String, String)], &str, Option<&SendAuth>),
    vars: &Vars,
) -> Vec<String> {
    let header = |name: &str| {
        headers
            .iter()
            .rev()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
    };
    let field = |key: &str| auth.and_then(|a| a.get(key)).and_then(js::yaml_to_string);
    let mut out: Vec<String> = match field("type").as_deref() {
        Some(mode @ ("basic" | "bearer")) => {
            let Some(full) = header("authorization") else {
                return Vec::new();
            };
            let inner = if mode == "basic" {
                full.strip_prefix("Basic ")
                    .and_then(|encoded| STANDARD.decode(encoded).ok())
                    .and_then(|bytes| String::from_utf8(bytes).ok())
                    .and_then(|pair| pair.split_once(':').map(|(_, pass)| pass.to_owned()))
            } else {
                full.strip_prefix("Bearer ").map(str::to_owned)
            };
            std::iter::once(full).chain(inner).collect()
        }
        Some("apikey") => {
            let Ok(key) = interpolate(&field("key").unwrap_or_default(), vars) else {
                return Vec::new();
            };
            let value = match field("placement").as_deref() {
                Some("query") => url::Url::parse(url).ok().and_then(|parsed| {
                    parsed
                        .query_pairs()
                        .find(|(name, _)| *name == key)
                        .map(|(_, value)| value.into_owned())
                }),
                _ => header(&key),
            };
            value.into_iter().collect()
        }
        _ => match send_auth {
            Some(SendAuth::AwsV4(sigv4)) => {
                vec![sigv4.secret_access_key.clone(), sigv4.session_token.clone()]
            }
            Some(SendAuth::Digest { password, .. })
            | Some(SendAuth::Wsse { password, .. })
            | Some(SendAuth::Ntlm { password, .. }) => {
                vec![password.clone()]
            }
            Some(SendAuth::OAuth1(signer)) => vec![
                signer.consumer_secret.clone(),
                signer.token.clone(),
                signer.token_secret.clone(),
            ],
            Some(SendAuth::EdgeGrid(signer)) => {
                signer.secrets().into_iter().map(str::to_owned).collect()
            }
            None => Vec::new(),
        },
    };
    out.retain(|value| !value.is_empty());
    out
}

#[cfg(test)]
mod shadowed_secret_tests {
    use super::*;
    use crate::collection::Collection;
    use crate::context::ContextOptions;
    use crate::runtime_vars::RuntimeVars;
    use crate::secrets::{MemoryStore, SecretStore};
    use std::sync::Arc;

    const SECRET: &str = "env-token-value-1";
    const GLOBAL: &str = "global-token-value-2";
    const ROOT: &str = "opencollection: 1.0.0\ninfo:\n  name: c\n";
    const DEV: &str = "name: dev\nvariables:\n  - secret: true\n    name: token\n  - name: gtok\n    value: plain-env-over-global\n";
    const PLAIN: &str = "shadow-plain-value";

    fn folder(vars: &str) -> String {
        format!("info:\n  name: f\n  type: folder\n  seq: 1\nrequest:\n  variables:\n{vars}")
    }

    fn request(extra: &str) -> String {
        format!(
            "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:9/{{{{token}}}}\"\n{extra}"
        )
    }

    // A tree where `token` is an environment secret and `gtok` a global secret the environment
    // overrides with a plain value; `shadow` says who shadows `token`.
    fn setup(
        shadow: &str,
        runtime: Option<RuntimeVars>,
    ) -> (tempfile::TempDir, RequestFile, VarContext) {
        let plain = format!("    - name: token\n      value: {PLAIN}\n");
        let (folder_vars, extra) = match shadow {
            "folder" => (plain.clone(), String::new()),
            "request" => (
                String::new(),
                format!("runtime:\n  variables:\n    - name: token\n      value: {PLAIN}\n"),
            ),
            _ => (String::new(), String::new()),
        };
        let dir = tempfile::TempDir::new().unwrap();
        for (rel, text) in [
            ("opencollection.yml", ROOT.to_owned()),
            ("environments/dev.yml", DEV.to_owned()),
            ("f/folder.yml", folder(&folder_vars)),
            ("f/r.yml", request(&extra)),
        ] {
            let path = dir.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let c = Collection::open(dir.path()).unwrap();
        let store = Arc::new(MemoryStore::new());
        store.set(&c.secret_key("dev", "token"), SECRET).unwrap();
        let mut options = ContextOptions::new()
            .env("dev")
            .secrets(store)
            .sensitive_global_env_var("gtok", GLOBAL);
        if let Some(runtime) = runtime {
            options = options.runtime_vars(runtime);
        }
        let ctx = VarContext::new(&c, options).unwrap();
        let req = RequestFile::load(dir.path().join("f/r.yml")).unwrap();
        (dir, req, ctx)
    }

    fn check(req: &RequestFile, ctx: &VarContext) {
        let known = known_secrets(req, ctx).unwrap();
        let named = |value: &str| {
            known
                .iter()
                .find(|s| s.value() == value)
                .map(|s| s.placeholder().to_owned())
        };
        assert_eq!(named(SECRET).as_deref(), Some("{{token}}"), "{known:?}");
        assert_eq!(named(GLOBAL).as_deref(), Some("{{gtok}}"), "{known:?}");
        assert!(
            named(PLAIN).is_none(),
            "a plain shadowing value is not secret"
        );
        assert!(named("plain-env-over-global").is_none());
        let logged = log_values(req, ctx);
        assert!(logged.iter().any(|v| v == SECRET), "{}", logged.len());
        assert!(logged.iter().any(|v| v == GLOBAL));
        assert!(!logged.iter().any(|v| v == PLAIN));
        let redacted = replace_secrets(&format!("a {SECRET} b {GLOBAL}"), &known);
        assert_eq!(redacted, "a {{token}} b {{gtok}}");
    }

    #[test]
    fn a_folder_variable_shadowing_a_secret_leaves_it_known() {
        let (_dir, req, ctx) = setup("folder", None);
        check(&req, &ctx);
    }

    #[test]
    fn a_request_variable_shadowing_a_secret_leaves_it_known() {
        let (_dir, req, ctx) = setup("request", None);
        check(&req, &ctx);
    }

    #[test]
    fn an_iteration_row_shadowing_a_secret_leaves_it_known() {
        let (_dir, req, ctx) = setup("none", None);
        let ctx = ctx.with_iteration_row(&[("token".to_owned(), PLAIN.to_owned())]);
        check(&req, &ctx);
    }

    #[test]
    fn a_captured_variable_shadowing_a_secret_leaves_it_known() {
        let runtime = RuntimeVars::new();
        runtime.set_text("token", PLAIN);
        let (_dir, req, ctx) = setup("none", Some(runtime));
        check(&req, &ctx);
    }

    #[test]
    fn an_environment_value_shadowing_a_global_secret_leaves_it_known() {
        // `gtok` is a global secret and the environment sets it to a plain value.
        let (_dir, req, ctx) = setup("none", None);
        check(&req, &ctx);
    }

    use super::utf16_bytes as utf16;

    #[test]
    fn redact_wide_finds_copies_at_odd_offsets_in_either_order() {
        for big in [false, true] {
            let body = [b"x".as_slice(), &utf16("k-s\u{e9}cret-1", big), b"y"].concat();
            let out = redact_wide(&body, ["k-s\u{e9}cret-1"]).expect("a copy");
            assert_eq!(out, [b"x".as_slice(), &utf16(REDACTED, big), b"y"].concat());
        }
    }

    #[test]
    fn redact_wide_merges_overlapping_copies_into_one_placeholder() {
        for big in [false, true] {
            let body = [utf16("abab", big), utf16("ab", big), b"!".to_vec()].concat();
            let out = redact_wide(&body, ["abab"]).expect("a copy");
            assert_eq!(out, [utf16(REDACTED, big), b"!".to_vec()].concat());
        }
        assert_eq!(redact_wide(b"plain", ["abab"]), None);
    }

    #[test]
    fn redact_wide_covers_a_short_non_ascii_value_but_not_a_short_ascii_one() {
        for big in [false, true] {
            let body = [b"x".as_slice(), &utf16("\u{e9}t", big), b"y"].concat();
            let out = redact_wide(&body, ["\u{e9}t"]).expect("a copy");
            assert_eq!(out, [b"x".as_slice(), &utf16(REDACTED, big), b"y"].concat());
            let body = utf16("\u{e9}", big);
            assert_eq!(redact_wide(&body, ["\u{e9}"]), Some(utf16(REDACTED, big)));
            assert_eq!(redact_wide(&utf16("ab", big), ["ab"]), None);
        }
    }
}
