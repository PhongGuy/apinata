//! Redaction of secret values, in every form they take on the wire, before logging.

use crate::{FormKind, FormPolicy, redact_base64_runs, wire_forms};

/// The placeholder that replaces a redacted value.
pub(crate) const REDACTED: &str = "[redacted]";

/// One string a value may appear as, with the text to match it by.
pub(crate) struct Form {
    pub(crate) text: String,
    /// `text` folded for `Text` forms, else `text` itself.
    needle: String,
    pub(crate) kind: FormKind,
}

/// Every wire form of a list of secret values, built once and applied to many fields.
pub(crate) struct Redactor {
    /// Non-empty values, kept for the whole-run base64 pass.
    values: Vec<String>,
    /// Every generated form, longest first.
    forms: Vec<Form>,
}

impl Form {
    /// The text to find, folded for a `Text` form.
    pub(crate) fn needle(&self) -> &str {
        &self.needle
    }
}

impl Redactor {
    /// The forms of every non-empty value, from `wire_forms` with its length floors.
    pub(crate) fn new<S: AsRef<str>>(values: &[S]) -> Redactor {
        let values: Vec<String> = values
            .iter()
            .map(|v| v.as_ref().to_owned())
            .filter(|v| !v.is_empty())
            .collect();
        let mut forms: Vec<Form> = Vec::new();
        for value in &values {
            for f in wire_forms(value, FormPolicy::new(4, 6)) {
                if forms.iter().any(|g| g.text == f.text && g.kind == f.kind) {
                    continue;
                }
                let needle = match f.kind {
                    FormKind::Text => Redactor::fold(&f.text),
                    _ => f.text.clone(),
                };
                forms.push(Form {
                    text: f.text,
                    needle,
                    kind: f.kind,
                });
            }
        }
        // Longest first over every form, so a short value's form cannot split a longer one.
        forms.sort_by_key(|f| std::cmp::Reverse(f.text.len()));
        Redactor { values, forms }
    }

    /// A redactor with no values, which changes nothing but credential headers.
    pub(crate) fn none() -> Redactor {
        Redactor {
            values: Vec::new(),
            forms: Vec::new(),
        }
    }

    /// The wire forms, longest first.
    #[cfg(test)]
    pub(crate) fn forms(&self) -> &[Form] {
        &self.forms
    }

    /// The forms in the same order, each with its match text.
    pub(crate) fn needles(&self) -> &[Form] {
        &self.forms
    }

    /// The case fold applied to text and forms before matching: ASCII upper case, which keeps
    /// byte offsets and makes percent-escape hex match in either case.
    pub(crate) fn fold(text: &str) -> String {
        text.to_ascii_uppercase()
    }

    /// The byte length of the longest form, or 0 when there are none.
    pub(crate) fn longest_form(&self) -> usize {
        self.forms.first().map_or(0, |f| f.text.len())
    }

    /// `text` with every form replaced by `[redacted]`: text forms match ASCII case-insensitively,
    /// base64 forms exactly, and a fragment also takes the base64 characters beside it.
    pub(crate) fn redact(&self, text: &str) -> String {
        if self.forms.is_empty() {
            return text.to_owned();
        }
        // Collect every occurrence of every form in the original text, overlapping ones
        // included, and replace each merged run once, so overlapping values leave no tail
        // and nothing inserted can match again. The fold keeps byte offsets.
        let haystack = Redactor::fold(text);
        let bytes = text.as_bytes();
        let mut spans: Vec<(usize, usize)> = Vec::new();
        for form in &self.forms {
            let hay = if form.kind == FormKind::Text {
                haystack.as_str()
            } else {
                text
            };
            let mut from = 0;
            while let Some(found) = hay[from..].find(form.needle.as_str()) {
                let start = from + found;
                let mut end = start + form.needle.len();
                let mut lead = start;
                if form.kind == FormKind::Base64Fragment {
                    // The fragment's edges share bits with up to two neighbours, and any
                    // padding after it belongs to the same encoding.
                    while start - lead < 2 && lead > 0 && is_b64(bytes[lead - 1]) {
                        lead -= 1;
                    }
                    let after = end;
                    while end - after < 2 && end < bytes.len() && is_b64(bytes[end]) {
                        end += 1;
                    }
                    while end < bytes.len() && bytes[end] == b'=' {
                        end += 1;
                    }
                }
                spans.push((lead, end));
                from = start + hay[start..].chars().next().map_or(1, char::len_utf8);
            }
        }
        spans.sort_unstable();
        let mut out = String::with_capacity(text.len());
        let mut kept = 0;
        let mut spans = spans.into_iter().peekable();
        while let Some((start, mut end)) = spans.next() {
            // Overlapping spans merge; spans that only touch stay separate placeholders.
            while let Some(&(next_start, next_end)) = spans.peek() {
                if next_start >= end {
                    break;
                }
                end = end.max(next_end);
                spans.next();
            }
            out.push_str(&text[kept..start]);
            out.push_str(REDACTED);
            kept = end;
        }
        out.push_str(&text[kept..]);
        // A short value only shows in a whole base64 run, which no fixed form covers.
        let values: Vec<&str> = self.values.iter().map(String::as_str).collect();
        redact_base64_runs(&out, &values, REDACTED)
    }

    /// A header value, replaced whole for a credential header, masked as a shown response
    /// header is (a url loses its userinfo) and then redacted.
    pub(crate) fn redact_header(&self, name: &str, value: &str) -> String {
        if crate::is_credential_header(name) {
            REDACTED.to_owned()
        } else {
            self.redact(&crate::mask_response_header(name, value))
        }
    }

    /// Whether `bytes` hold any value: a form in their lossy text, or a UTF-16 copy, which no
    /// replacement inside the bytes could hide.
    pub(crate) fn holds(&self, bytes: &[u8]) -> bool {
        let lossy = String::from_utf8_lossy(bytes);
        if self.redact(&lossy) != lossy {
            return true;
        }
        self.values.iter().any(|v| {
            let units: Vec<u16> = v.encode_utf16().collect();
            let le: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
            let be: Vec<u8> = units.iter().flat_map(|u| u.to_be_bytes()).collect();
            [le, be]
                .iter()
                .any(|n| bytes.windows(n.len()).any(|w| w == n.as_slice()))
        })
    }

    /// `bytes` with each value's UTF-16 copies replaced by `[redacted]` in the same encoding,
    /// or `None` when they hold none.
    pub(crate) fn redact_wide(&self, bytes: &[u8]) -> Option<Vec<u8>> {
        crate::secret_names::redact_wide(bytes, self.values.iter().map(String::as_str))
    }

    /// `url` with its whole userinfo dropped, then redacted. A username alone can be the
    /// credential (a token sent as the user), so a record of a send keeps none of it.
    pub(crate) fn redact_url(&self, url: &str) -> String {
        self.redact(&crate::redact_url_userinfo(url))
    }
}

/// Whether `b` belongs to the standard or URL-safe base64 alphabet, padding included.
fn is_b64(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'-' | b'_' | b'=')
}

#[cfg(test)]
mod tests {
    use super::*;
    use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

    /// RFC 3986 unreserved characters stay literal; everything else is percent-encoded.
    const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'.')
        .remove(b'_')
        .remove(b'~');
    /// The unreserved set with `/` also literal.
    const PATH: &AsciiSet = &UNRESERVED.remove(b'/');

    #[test]
    fn query_and_path_segment_encodings_are_redacted() {
        let r = Redactor::new(&["a b+c/d"]);
        let out = r.redact("x?k=a%20b+c/d&p=/a%20b+c%2Fd");
        assert!(!out.contains("b+c") && !out.contains("a%20b"), "{out}");
    }

    #[test]
    fn an_x_wsse_header_is_replaced_whole() {
        assert_eq!(
            Redactor::none().redact_header("X-WSSE", "UsernameToken Username=\"u\""),
            "[redacted]"
        );
    }

    #[test]
    fn replaces_every_wire_form_of_a_value() {
        let r = Redactor::new(&["a b/c\"d"]);
        for form in [
            "a b/c\"d",
            "a b/c\\\"d",
            "a b\\/c\\\"d",
            "a%20b%2Fc%22d",
            "a%20b/c%22d",
            "a+b%2Fc%22d",
        ] {
            assert_eq!(
                r.redact(&format!("x{form}y")),
                "x[redacted]y",
                "form {form}"
            );
        }
        let texts: Vec<&str> = r.forms().iter().map(|f| f.text.as_str()).collect();
        for form in [
            "a b/c\"d",
            "a b/c\\\"d",
            "a b\\/c\\\"d",
            "a%20b%2Fc%22d",
            "a%20b/c%22d",
            "a+b%2Fc%22d",
        ] {
            assert!(texts.contains(&form), "{form}");
        }
    }

    #[test]
    fn longest_form_wins_over_a_shorter_value_inside_it() {
        let r = Redactor::new(&["abc", "abcdef"]);
        assert_eq!(r.redact("1abcdef2abc3"), "1[redacted]2[redacted]3");
        let longest = r.forms().iter().map(|f| f.text.len()).max();
        assert_eq!(Some(r.longest_form()), longest);
        assert!(
            r.forms()
                .windows(2)
                .all(|w| w[0].text.len() >= w[1].text.len())
        );
        let texts: Vec<&str> = r.forms().iter().map(|f| f.text.as_str()).collect();
        for text in ["abcdef", "abc"] {
            assert!(texts.contains(&text), "{text}");
        }
    }

    #[test]
    fn empty_values_are_ignored_and_no_values_is_a_no_op() {
        let r = Redactor::new(&["", "k"]);
        assert_eq!(r.redact("kk"), "[redacted][redacted]");
        let none = Redactor::none();
        assert_eq!(none.redact("anything"), "anything");
        assert_eq!(none.longest_form(), 0);
        assert!(none.forms().is_empty());
    }

    #[test]
    fn a_value_inside_the_placeholder_terminates() {
        let r = Redactor::new(&["red", "acted"]);
        assert_eq!(r.redact("red acted"), "[redacted] [redacted]");
    }

    #[test]
    fn overlapping_values_leave_no_tail() {
        let r = Redactor::new(&["abcd", "cdef"]);
        assert_eq!(r.redact("xabcdefy"), "x[redacted]y");
        assert_eq!(Redactor::new(&["aa"]).redact("aaa"), "[redacted]");
    }

    #[test]
    fn hex_after_a_percent_does_not_hide_a_value() {
        assert_eq!(Redactor::new(&["x%a"]).redact("x%ab"), "[redacted]b");
        let r = Redactor::new(&["deadbeef"]);
        assert_eq!(r.redact("q=%deadbeef"), "q=%[redacted]");
    }

    #[test]
    fn form_urlencoded_values_are_redacted() {
        for (value, wire) in [("a~b", "a%7Eb"), ("x*y z", "x*y+z")] {
            let r = Redactor::new(&[value]);
            assert_eq!(r.redact(&format!("k={wire}")), "k=[redacted]", "{value}");
            assert_eq!(
                r.redact_url(&format!("https://h.test/?k={wire}")),
                "https://h.test/?k=[redacted]",
                "{value}"
            );
        }
    }

    #[test]
    fn lowercase_percent_hex_is_redacted() {
        let r = Redactor::new(&["a/b"]);
        assert_eq!(r.redact("x=a%2fb&y=a%2Fb"), "x=[redacted]&y=[redacted]");
    }

    #[test]
    fn json_escaped_slash_is_redacted() {
        let r = Redactor::new(&["a/b"]);
        assert_eq!(r.redact("{\"k\":\"a\\/b\"}"), "{\"k\":\"[redacted]\"}");
    }

    #[test]
    fn credential_headers_are_replaced_whole_in_any_case() {
        let none = Redactor::none();
        for name in [
            "Authorization",
            "proxy-authorization",
            "COOKIE",
            "Set-Cookie",
        ] {
            assert_eq!(
                none.redact_header(name, "Bearer abc"),
                "[redacted]",
                "{name}"
            );
        }
        assert_eq!(none.redact_header("X-Trace", "Bearer abc"), "Bearer abc");
        let r = Redactor::new(&["abc"]);
        assert_eq!(
            r.redact_header("X-Trace", "Bearer abc"),
            "Bearer [redacted]"
        );
    }

    #[test]
    fn url_userinfo_is_dropped_and_values_redacted() {
        let r = Redactor::new(&["tok en"]);
        assert_eq!(
            r.redact_url("https://user:pw@api.test/a?key=tok%20en&q=tok+en"),
            "https://api.test/a?key=[redacted]&q=[redacted]"
        );
        assert_eq!(r.redact_url("https://api.test/x"), "https://api.test/x");
        assert_eq!(r.redact_url("https://user@api.test/"), "https://api.test/");
        assert_eq!(r.redact_url("not a url @ all"), "not a url @ all");
    }

    // An NTLM Authorization is replaced whole; a challenge header that echoes the password is
    // redacted by value like any other header.
    #[test]
    fn ntlm_headers_hide_the_message_and_the_password() {
        let r = Redactor::new(&["ntlm-pass-1"]);
        assert_eq!(
            r.redact_header("Authorization", "NTLM TlRMTVNTUAAD"),
            "[redacted]"
        );
        let out = r.redact_header("WWW-Authenticate", "NTLM realm=ntlm-pass-1");
        assert!(!out.contains("ntlm-pass-1"), "{out}");
    }

    // Baseline helpers: independent of the code under test.
    const AWKWARD: &str = "Ab1 \"q\"/x+y";
    const UNI: &str = "p&ss/wörd<x>+ 1";

    fn pct(v: &str) -> String {
        utf8_percent_encode(v, UNRESERVED).to_string()
    }
    fn json(v: &str) -> String {
        let q = serde_json::to_string(v).unwrap();
        q[1..q.len() - 1].to_owned()
    }
    /// Lowercases the two hex digits after every `%`.
    fn lower_hex(s: &str) -> String {
        let c: Vec<char> = s.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < c.len() {
            out.push(c[i]);
            if c[i] == '%' && i + 2 < c.len() {
                out.extend(c[i + 1..=i + 2].iter().map(|d| d.to_ascii_lowercase()));
                i += 3;
            } else {
                i += 1;
            }
        }
        out
    }

    fn log_rows(v: &str) -> Vec<(&'static str, String)> {
        let p = pct(v);
        let j1 = json(v);
        let form: String = url::form_urlencoded::byte_serialize(v.as_bytes()).collect();
        let mut u = url::Url::parse("http://h.test/").unwrap();
        u.set_query(Some(v));
        let query = u.query().unwrap().to_owned();
        // A path segment as the url crate writes it: `/` becomes `%2F`, `+` stays literal.
        let mut u = url::Url::parse("http://h.test/").unwrap();
        u.path_segments_mut().unwrap().push(v);
        let segment = u.path().trim_start_matches('/').to_owned();
        vec![
            ("raw", v.to_owned()),
            ("json", j1.clone()),
            ("php slash", j1.replace('/', "\\/")),
            ("percent", p.clone()),
            ("percent lower", lower_hex(&p)),
            ("percent path", utf8_percent_encode(v, PATH).to_string()),
            ("percent plus", p.replace("%20", "+")),
            ("form", form),
            ("query", query),
            ("path segment", segment),
            ("upper case", v.to_ascii_uppercase()),
            ("lower case", v.to_ascii_lowercase()),
        ]
    }

    #[test]
    fn every_baseline_echo_is_redacted() {
        for v in [AWKWARD, UNI] {
            let r = Redactor::new(&[v]);
            for (name, row) in log_rows(v) {
                assert_eq!(
                    r.redact(&format!("<{row}>")),
                    "<[redacted]>",
                    "{name} of {v:?}"
                );
            }
        }
    }

    use base64::Engine as _;
    use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};

    /// `\uXXXX` for every character (UTF-16 units), hex digits in the given case.
    fn unicode_all(v: &str, upper: bool) -> String {
        let mut out = String::new();
        for ch in v.chars() {
            for u in ch.encode_utf16(&mut [0; 2]).iter() {
                out.push_str(&if upper {
                    format!("\\u{u:04X}")
                } else {
                    format!("\\u{u:04x}")
                });
            }
        }
        out
    }

    /// Lowercases the hex of every second `%XX`, so one value mixes both cases.
    fn alternate_hex(s: &str) -> String {
        let c: Vec<char> = s.chars().collect();
        let (mut out, mut i, mut nth) = (String::new(), 0, 0);
        while i < c.len() {
            out.push(c[i]);
            if c[i] == '%' && i + 2 < c.len() {
                for d in &c[i + 1..=i + 2] {
                    out.push(if nth % 2 == 1 {
                        d.to_ascii_lowercase()
                    } else {
                        *d
                    });
                }
                nth += 1;
                i += 3;
            } else {
                i += 1;
            }
        }
        out
    }

    /// `\uXXXX` for every character, upper hex at even positions and lower hex at odd ones.
    fn mixed_unicode(v: &str) -> String {
        let mut out = String::new();
        for (i, ch) in v.chars().enumerate() {
            for u in ch.encode_utf16(&mut [0; 2]).iter() {
                out.push_str(&if i % 2 == 0 {
                    format!("\\u{u:04X}")
                } else {
                    format!("\\u{u:04x}")
                });
            }
        }
        out
    }

    #[test]
    fn base64_unicode_and_percent_twice_forms_are_redacted() {
        let r = Redactor::new(&[AWKWARD]);
        for echo in [
            STANDARD.encode(AWKWARD),
            URL_SAFE_NO_PAD.encode(AWKWARD),
            unicode_all(AWKWARD, true),
            pct(&pct(AWKWARD)),
            alternate_hex(&pct(AWKWARD)),
        ] {
            assert_eq!(r.redact(&format!("<{echo}>")), "<[redacted]>", "{echo}");
        }
        let v = "\u{e9}\u{f6}\u{fc}\u{df} key";
        let r = Redactor::new(&[v]);
        assert_eq!(r.redact(&format!("<{}>", mixed_unicode(v))), "<[redacted]>");
    }

    #[test]
    fn host_forms_are_redacted() {
        let r = Redactor::new(&["B\u{fc}cher.Example"]);
        assert_eq!(
            r.redact("https://xn--bcher-kva.example/"),
            "https://[redacted]/"
        );
    }

    #[test]
    fn a_base64_fragment_takes_its_edge_characters() {
        let r = Redactor::new(&["hunter22"]);
        assert_eq!(r.redact("<QQaHVudGVyMjZZ>"), "<[redacted]>");
        assert_eq!(r.redact("<QQ aHVudGVyMj ZZ>"), "<QQ [redacted] ZZ>");
    }

    #[test]
    fn a_base64_run_decoding_to_the_value_is_redacted() {
        let r = Redactor::new(&["abcd"]);
        assert!(!r.redact("Basic dXNlcjphYmNk").contains("dXNlcjphYmNk"));
    }

    #[test]
    fn a_value_inside_the_placeholder_terminates_for_base64_and_text_forms() {
        let r = Redactor::new(&["redact", "abcdefgh"]);
        assert_eq!(r.redact("redact abcdefgh"), "[redacted] [redacted]");
    }
}
