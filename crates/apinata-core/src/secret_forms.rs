//! The one generator of the wire forms a secret value can take, shared by every redactor.
//!
//! A redactor asks [`wire_forms`] for the strings to hide and matches them in its own way,
//! guided by each form's [`FormKind`]. Floors on value and form length are the caller's
//! [`FormPolicy`], so a looser or stricter shell changes the policy, not the list.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use url::Url;

pub(crate) const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');
pub(crate) const PATH: &AsciiSet = &UNRESERVED.remove(b'/');
/// WHATWG form encoding keeps `*-._` literal; space then becomes `+`.
pub(crate) const FORM: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'*')
    .remove(b'-')
    .remove(b'.')
    .remove(b'_');
/// What JavaScript's `encodeURIComponent` leaves literal.
const URI_COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');
/// The WHATWG URL query percent-encode set.
const QUERY: &AsciiSet = &percent_encoding::CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'<')
    .add(b'>')
    .add(b'\'');
/// The WHATWG URL path-segment percent-encode set.
const SEGMENT: &AsciiSet = &percent_encoding::CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'<')
    .add(b'>')
    .add(b'`')
    .add(b'?')
    .add(b'{')
    .add(b'}')
    .add(b'/')
    .add(b'%');

/// The shortest inner base64 run the generator emits; a policy may raise it.
const MIN_FRAGMENT_CHARS: usize = 4;

/// How a redactor must match a form.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormKind {
    /// Plain text, matched as a substring.
    Text,
    /// A whole-value base64 encoding, matched exactly.
    Base64,
    /// A run of base64 characters that sits inside a longer encoding.
    Base64Fragment,
}

/// One string a secret may appear as, and how to match it.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WireForm {
    /// The text to look for.
    pub text: String,
    /// How to look for it.
    pub kind: FormKind,
}

/// The length floors a redactor applies to the generated list.
#[non_exhaustive]
#[derive(Clone, Copy, Debug)]
pub struct FormPolicy {
    min_value_chars: usize,
    min_extended_chars: usize,
}

impl FormPolicy {
    /// A value under `min_value_chars` characters gets only the basic forms; any other form
    /// under `min_extended_chars` characters is dropped.
    pub fn new(min_value_chars: usize, min_extended_chars: usize) -> FormPolicy {
        FormPolicy {
            min_value_chars,
            min_extended_chars,
        }
    }
}

/// Every form `value` may take on the wire, deduplicated and longest first (ties by text).
/// The basic forms (raw, JSON, the common percent encodings) are always present; the rest
/// obey `policy`. Empty for an empty value.
pub fn wire_forms(value: &str, policy: FormPolicy) -> Vec<WireForm> {
    if value.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<WireForm> = Vec::new();
    let mut push = |text: String, kind: FormKind| {
        if !text.is_empty() {
            out.push(WireForm { text, kind });
        }
    };

    for text in basic_texts(value) {
        push(text, FormKind::Text);
    }
    if value.chars().count() >= policy.min_value_chars {
        let long_enough = |t: &String| t.chars().count() >= policy.min_extended_chars;
        for text in extended_texts(value).into_iter().filter(long_enough) {
            push(text, FormKind::Text);
        }
        let mut forms = Vec::new();
        base64_forms(value.as_bytes(), &mut forms);
        let wholes: Vec<String> = [STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD]
            .iter()
            .map(|e| e.encode(value))
            .collect();
        for text in forms.into_iter().filter(long_enough) {
            let is_whole = wholes.contains(&text);
            if is_whole {
                push(text, FormKind::Base64);
            } else if text.chars().count() >= MIN_FRAGMENT_CHARS {
                push(text, FormKind::Base64Fragment);
            }
        }
    }
    out.sort_by(|a, b| {
        (b.text.len().cmp(&a.text.len()))
            .then_with(|| a.text.cmp(&b.text))
            .then_with(|| (a.kind as u8).cmp(&(b.kind as u8)))
    });
    out.dedup();
    out
}

/// `value` as a URL host shows it: lowercase, IDNA ASCII (punycode) and IDNA Unicode.
/// A form that does not convert is left out.
pub fn host_forms(value: &str) -> Vec<String> {
    use url::quirks::{domain_to_ascii, domain_to_unicode};
    let ascii = domain_to_ascii(value);
    let unicode = domain_to_unicode(&ascii);
    let mut forms = vec![value.to_lowercase(), ascii, unicode];
    forms.retain(|f| !f.is_empty());
    forms.sort();
    forms.dedup();
    forms
}

/// `text` with each ASCII-case-insensitive occurrence of `needle` replaced by `with`, left to
/// right and without overlap. Non-ASCII text is compared exactly, so offsets never shift.
pub fn replace_ascii_ci(text: &str, needle: &str, with: &str) -> String {
    if needle.is_empty() {
        return text.to_owned();
    }
    // ASCII upper-casing keeps byte offsets, so matches in the folded copy slice the original.
    let folded = text.to_ascii_uppercase();
    let mut out = String::with_capacity(text.len());
    let mut done = 0;
    for (start, found) in folded.match_indices(&needle.to_ascii_uppercase()) {
        out.push_str(&text[done..start]);
        out.push_str(with);
        done = start + found.len();
    }
    out.push_str(&text[done..]);
    out
}

fn pct(value: &str, set: &'static AsciiSet) -> String {
    utf8_percent_encode(value, set).to_string()
}

// The forms any value has, whatever its length: they are the common ways a client writes it
// into a URL, a form or a JSON body.
fn basic_texts(value: &str) -> Vec<String> {
    let json = json_escaped(value);
    vec![
        value.to_owned(),
        json.replace('/', "\\/"),
        json,
        pct(value, UNRESERVED),
        pct(value, PATH),
        pct(value, UNRESERVED).replace("%20", "+"),
        pct(value, FORM).replace("%20", "+"),
        pct(value, QUERY),
        pct(value, SEGMENT),
    ]
}

// Every other text form: case, deeper JSON, rescapes, the remaining percent sets in both hex
// cases, each percent form encoded again, and JSON-escaped echoes of all of them.
fn extended_texts(value: &str) -> Vec<String> {
    let mut forms = vec![value.to_ascii_lowercase()];

    let mut json = json_escaped(value);
    for _ in 0..2 {
        json = json_escaped(&json);
        forms.push(json.clone());
    }
    for bits in 0..32u8 {
        forms.push(json_rescaped(
            value,
            Rescape {
                slash: bits & 1 != 0,
                html: bits & 2 != 0,
                non_ascii: bits & 4 != 0,
                upper: bits & 8 != 0,
                html_upper: bits & 16 != 0,
            },
        ));
    }
    for upper in [false, true] {
        forms.push(unicode_escaped(value, true, upper));
        forms.push(unicode_escaped(value, false, upper));
    }

    let mut percent = vec![
        pct(value, URI_COMPONENT),
        pct(value, FORM),
        pct(value, QUERY),
        pct(value, SEGMENT),
        pct(value, UNRESERVED),
        pct(value, PATH),
        pct(value, UNRESERVED).replace("%20", "+"),
        pct(value, FORM).replace("%20", "+"),
    ];
    percent.extend(url_crate_forms(value));
    let lower: Vec<String> = percent.iter().map(|f| lower_hex(f)).collect();
    percent.extend(lower);
    percent.retain(|f| f.contains('%'));
    let twice: Vec<String> = percent.iter().map(|f| pct(f, UNRESERVED)).collect();
    forms.extend(percent);
    forms.extend(twice);

    forms.extend(host_forms(value));

    let echoed: Vec<String> = forms
        .iter()
        .filter(|f| f.contains(['\\', '"']))
        .map(|f| json_escaped(f))
        .collect();
    forms.extend(echoed);
    forms
}

// The value as the `url` crate writes it into a path, query, fragment, username or password.
fn url_crate_forms(value: &str) -> Vec<String> {
    // A fixed base URL, so parsing it cannot fail.
    #[allow(clippy::expect_used)]
    let base = || Url::parse("http://h.test/").expect("a literal URL parses");
    let mut out = Vec::new();
    let mut u = base();
    u.set_path(value);
    out.push(u.path().strip_prefix('/').unwrap_or(u.path()).to_owned());
    let mut u = base();
    u.set_query(Some(value));
    out.extend(u.query().map(str::to_owned));
    let mut u = base();
    u.set_fragment(Some(value));
    out.extend(u.fragment().map(str::to_owned));
    let mut u = base();
    if u.set_username(value).is_ok() {
        out.push(u.username().to_owned());
    }
    let mut u = base();
    if u.set_password(Some(value)).is_ok() {
        out.extend(u.password().map(str::to_owned));
    }
    out
}

// Which rescapes `json_rescaped` applies; the hex case of the HTML escapes is its own flag
// because encoders mix it with the case of the non-ASCII escapes.
struct Rescape {
    slash: bool,
    html: bool,
    non_ascii: bool,
    upper: bool,
    html_upper: bool,
}

// The JSON text of `value` as one encoder family writes it: `/` as `\/` (PHP), `&<>` as
// `\u00XX` (Go HTML safe), non-ASCII as `\uXXXX` (many), with hex digits in either case.
fn json_rescaped(value: &str, r: Rescape) -> String {
    let mut out = String::new();
    for c in value.chars() {
        let escape_unit = |out: &mut String, c: char, upper: bool| {
            let mut units = [0u16; 2];
            for unit in c.encode_utf16(&mut units) {
                out.push_str(&if upper {
                    format!("\\u{unit:04X}")
                } else {
                    format!("\\u{unit:04x}")
                });
            }
        };
        match c {
            '/' if r.slash => out.push_str("\\/"),
            '&' | '<' | '>' if r.html => escape_unit(&mut out, c, r.html_upper),
            c if !c.is_ascii() && r.non_ascii => escape_unit(&mut out, c, r.upper),
            c => out.push_str(&json_escaped(c.encode_utf8(&mut [0u8; 4]))),
        }
    }
    out
}

// The whole value in both alphabets, padded and not, then for each alignment the run of
// characters the value's bits alone decide, so it is found inside a longer encoding.
pub(crate) fn base64_forms(bytes: &[u8], out: &mut Vec<String>) {
    for engine in [STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD] {
        out.push(engine.encode(bytes));
    }
    // Behind `lead` filler bytes the value spans bits [8*lead, 8*(lead+n)); output char i
    // spans [6i, 6i+6), so chars ceil(8*lead/6)..floor(8*(lead+n)/6) depend on the value only.
    for lead in 0..3usize {
        let mut shifted = vec![0u8; lead];
        shifted.extend_from_slice(bytes);
        let start = (8 * lead).div_ceil(6);
        let end = 8 * (lead + bytes.len()) / 6;
        for engine in [STANDARD_NO_PAD, URL_SAFE_NO_PAD] {
            let encoded = engine.encode(&shifted);
            if start < end {
                out.push(encoded[start..end].to_owned());
            }
        }
    }
}

// Lowercases only the two hex digits after each `%`; literal letters keep their case.
pub(crate) fn lower_hex(encoded: &str) -> String {
    let mut out = String::with_capacity(encoded.len());
    let mut hex_left = 0;
    for c in encoded.chars() {
        if hex_left > 0 {
            out.push(c.to_ascii_lowercase());
            hex_left -= 1;
        } else {
            hex_left = if c == '%' { 2 } else { 0 };
            out.push(c);
        }
    }
    out
}

// `\uXXXX` per UTF-16 unit (a surrogate pair outside the BMP); `all` false keeps ASCII literal.
pub(crate) fn unicode_escaped(value: &str, all: bool, upper: bool) -> String {
    let mut out = String::new();
    for c in value.chars() {
        if !all && c.is_ascii() {
            out.push(c);
            continue;
        }
        let mut units = [0u16; 2];
        for unit in c.encode_utf16(&mut units) {
            out.push_str(&if upper {
                format!("\\u{unit:04X}")
            } else {
                format!("\\u{unit:04x}")
            });
        }
    }
    out
}

pub(crate) fn json_escaped(value: &str) -> String {
    // Serializing a plain string to JSON cannot fail.
    #[allow(clippy::expect_used)]
    let quoted = serde_json::to_string(value).expect("a string serializes");
    quoted[1..quoted.len() - 1].to_owned()
}
