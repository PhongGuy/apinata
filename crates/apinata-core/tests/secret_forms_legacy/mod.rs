//! Frozen copies of the pre-i12 form lists, for the subset test.

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use percent_encoding::{AsciiSet, CONTROLS, NON_ALPHANUMERIC, utf8_percent_encode};
use url::Url;

const MIN_CHARS: usize = 4;
const MIN_LEN: usize = 4;
const MIN_FORM_CHARS: usize = 6;
const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');
const PATH: &AsciiSet = &UNRESERVED.remove(b'/');
const FORM: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'*')
    .remove(b'-')
    .remove(b'.')
    .remove(b'_');
const QUERY: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'<')
    .add(b'>')
    .add(b'\'');
const PATH_SEGMENT: &AsciiSet = &CONTROLS
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

/// The saved-examples list: core's `encoded_forms`.
pub fn core_forms(value: &str) -> Vec<String> {
    if value.chars().count() < MIN_CHARS {
        return Vec::new();
    }
    let mut forms = vec![value.to_ascii_lowercase()];
    for encoded in [
        utf8_percent_encode(value, UNRESERVED).to_string(),
        utf8_percent_encode(value, PATH).to_string(),
        utf8_percent_encode(value, FORM)
            .to_string()
            .replace("%20", "+"),
    ] {
        forms.push(lower_hex(&encoded));
        forms.push(encoded);
    }
    for upper in [false, true] {
        forms.push(unicode_escaped(value, true, upper));
        if !value.is_ascii() {
            forms.push(unicode_escaped(value, false, upper));
        }
    }
    base64_forms(value.as_bytes(), &mut forms);
    // A percent-encoded form typed into a URL is encoded again by the sender: `%2F` -> `%252F`.
    let twice: Vec<String> = forms
        .iter()
        .filter(|f| f.contains('%'))
        .map(|f| utf8_percent_encode(f, UNRESERVED).to_string())
        .collect();
    forms.extend(twice);
    // A form echoed inside a JSON string has its backslashes and quotes escaped once more.
    let echoed: Vec<String> = forms
        .iter()
        .filter(|f| f.contains(['\\', '"']))
        .map(|f| json_escaped(f))
        .collect();
    forms.extend(echoed);
    // The raw value echoed through two JSON layers (its single escape is the caller's raw entry).
    let escaped_value = json_escaped(value);
    if escaped_value.contains(['\\', '"']) {
        forms.push(json_escaped(&escaped_value));
    }
    let escaped = json_escaped(value);
    forms.retain(|f| f.chars().count() >= MIN_FORM_CHARS && f != value && *f != escaped);
    forms.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    forms.dedup();
    forms
}

// The whole value in both alphabets, padded and not, then for each alignment the run of
// characters the value's bits alone decide, so it is found inside a longer encoding.
fn base64_forms(bytes: &[u8], out: &mut Vec<String>) {
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
fn lower_hex(encoded: &str) -> String {
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
fn unicode_escaped(value: &str, all: bool, upper: bool) -> String {
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

fn json_escaped(value: &str) -> String {
    let quoted = serde_json::to_string(value).expect("a string serializes");
    quoted[1..quoted.len() - 1].to_owned()
}

/// The MCP list: its own `encoded_forms` chained with core's.
pub fn mcp_forms(value: &str) -> Vec<String> {
    let mut forms = mcp_encoded_forms(value);
    forms.extend(core_forms(value));
    forms
}

fn mcp_encoded_forms(value: &str) -> Vec<String> {
    let mut forms = vec![
        value.to_owned(),
        encode_uri_component(value),
        url::form_urlencoded::byte_serialize(value.as_bytes()).collect(),
        STANDARD.encode(value),
        STANDARD_NO_PAD.encode(value),
        URL_SAFE.encode(value),
        URL_SAFE_NO_PAD.encode(value),
    ];
    // Each URL component is read back from a fresh URL, as the executor serialises it.
    let fresh = || Url::parse("http://h.test/").expect("a fixed valid URL");
    let mut url = fresh();
    url.set_path(value);
    forms.push(url.path().trim_start_matches('/').to_owned());
    let mut url = fresh();
    url.set_query(Some(value));
    forms.extend(url.query().map(str::to_owned));
    let mut url = fresh();
    url.set_fragment(Some(value));
    forms.extend(url.fragment().map(str::to_owned));
    let mut url = fresh();
    if url.set_username(value).is_ok() {
        forms.push(url.username().to_owned());
    }
    let mut url = fresh();
    if url.set_password(Some(value)).is_ok() {
        forms.extend(url.password().map(str::to_owned));
    }
    let mut escaped = value.to_owned();
    for _ in 0..3 {
        escaped = json_escape(&escaped);
        forms.push(escaped.clone());
    }
    // Other serialisers' JSON escapes: `\uXXXX` for non-ASCII (either hex case), PHP's `\/`,
    // Go's HTML-safe `\u0026`, `\u003c` and `\u003e`, in every combination.
    let plain = json_escape(value);
    for ascii in [None, Some(false), Some(true)] {
        for slash in [false, true] {
            for html in [false, true] {
                forms.push(rescape(&plain, ascii, slash, html));
            }
        }
    }
    forms.extend(base64_inner_forms(value));
    forms.extend(host_forms(value));
    let lowered: Vec<String> = forms.iter().filter_map(|f| lower_percent_hex(f)).collect();
    forms.extend(lowered);
    forms.retain(|f| !f.is_empty());
    forms.sort();
    forms.dedup();
    forms
}

/// `value` as a URL host shows it: lowercased, and its IDNA ASCII (punycode) and mapped
/// Unicode forms when it parses as a domain.
fn host_forms(value: &str) -> Vec<String> {
    use url::quirks::{domain_to_ascii, domain_to_unicode};
    let ascii = domain_to_ascii(value);
    let unicode = domain_to_unicode(&ascii);
    vec![value.to_lowercase(), ascii, unicode]
}

/// `value` as a JSON string body: serde_json's escape without the quotes.
fn json_escape(value: &str) -> String {
    let quoted = serde_json::to_string(value).expect("a string always serialises");
    quoted[1..quoted.len() - 1].to_owned()
}

/// Re-escape a serde_json string body: non-ASCII as `\uXXXX` when `ascii` is set (`Some(true)`
/// for uppercase hex), `/` as `\/` when `slash`, and `&`, `<`, `>` as `\u00XX` when `html`.
fn rescape(body: &str, ascii: Option<bool>, slash: bool, html: bool) -> String {
    let mut out = String::with_capacity(body.len());
    for c in body.chars() {
        match c {
            '/' if slash => out.push_str("\\/"),
            '&' | '<' | '>' if html => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c if !c.is_ascii() && ascii.is_some() => {
                for unit in c.encode_utf16(&mut [0; 2]) {
                    if ascii == Some(true) {
                        out.push_str(&format!("\\u{unit:04X}"));
                    } else {
                        out.push_str(&format!("\\u{unit:04x}"));
                    }
                }
            }
            c => out.push(c),
        }
    }
    out
}

/// The base64 characters of `value` that stay the same whatever bytes surround it, for each
/// of the three alignments and both alphabets; runs shorter than `MIN_LEN` are dropped.
fn base64_inner_forms(value: &str) -> Vec<String> {
    let len = value.len();
    let mut forms = Vec::new();
    for offset in 0..3 {
        let mut bytes = vec![0; offset];
        bytes.extend_from_slice(value.as_bytes());
        // A char is stable when its six bits all come from the value's bytes.
        let start = (8 * offset).div_ceil(6);
        let end = 8 * (offset + len) / 6;
        if end < start + MIN_LEN {
            continue;
        }
        for engine in [STANDARD_NO_PAD, URL_SAFE_NO_PAD] {
            forms.push(engine.encode(&bytes)[start..end].to_owned());
        }
    }
    forms
}

/// `form` with every `%XX` escape in lowercase hex, or `None` when it has no uppercase escape.
fn lower_percent_hex(form: &str) -> Option<String> {
    let mut out = String::with_capacity(form.len());
    let mut rest = form;
    let mut changed = false;
    while let Some(at) = rest.find('%') {
        out.push_str(&rest[..=at]);
        rest = &rest[at + 1..];
        let hex = rest
            .get(..2)
            .filter(|h| h.bytes().all(|b| b.is_ascii_hexdigit()));
        if let Some(hex) = hex {
            let lower = hex.to_ascii_lowercase();
            changed |= lower != hex;
            out.push_str(&lower);
            rest = &rest[2..];
        }
    }
    out.push_str(rest);
    changed.then_some(out)
}

/// JavaScript's `encodeURIComponent`: unreserved marks kept, every other byte as uppercase `%XX`.
fn encode_uri_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for &b in value.as_bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The session-log list: the candidates `Redactor::new` builds for one value.
pub fn session_forms(value: &str) -> Vec<String> {
    let mut forms: Vec<String> = Vec::new();
    for value in [value].into_iter().filter(|v| !v.is_empty()) {
        let escaped = serde_json::to_string(value).unwrap_or_default();
        let escaped = escaped
            .strip_prefix('"')
            .and_then(|e| e.strip_suffix('"'))
            .unwrap_or(value)
            .to_owned();
        let encoded = utf8_percent_encode(value, UNRESERVED).to_string();
        let path = utf8_percent_encode(value, PATH).to_string();
        let form = encoded.replace("%20", "+");
        let urlencoded: String = url::form_urlencoded::byte_serialize(value.as_bytes()).collect();
        let slashed = escaped.replace('/', "\\/");
        let query = utf8_percent_encode(value, QUERY).to_string();
        let segment = utf8_percent_encode(value, PATH_SEGMENT).to_string();
        for candidate in [
            value.to_owned(),
            escaped,
            slashed,
            encoded,
            path,
            form,
            urlencoded,
            query,
            segment,
        ] {
            if !forms.contains(&candidate) {
                forms.push(candidate);
            }
        }
    }
    forms
}

/// The history list: the `base` array of `Forms::new` chained with core's.
pub fn history_forms(value: &str) -> Vec<String> {
    let secret = &value.to_owned();
    let escaped = serde_json::to_string(secret).unwrap_or_default();
    let escaped = escaped
        .strip_prefix('"')
        .and_then(|e| e.strip_suffix('"'))
        .unwrap_or(secret)
        .to_owned();
    let encoded = utf8_percent_encode(secret, UNRESERVED).to_string();
    let path = utf8_percent_encode(secret, PATH).to_string();
    let form = encoded.replace("%20", "+");
    let base = [secret.clone(), escaped, encoded, path, form];
    let mut forms = base.to_vec();
    forms.extend(core_forms(value));
    forms
}
