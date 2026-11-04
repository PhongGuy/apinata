//! Scrubbing of sensitive values from every text a tool returns.
//!
//! A `Redactor` holds each sensitive value in every encoding a response may echo it in,
//! and replaces any of them with `SECRET` in one left-to-right pass.

use apinata_core::{FormKind, FormPolicy, host_forms, wire_forms};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

/// The marker that replaces a scrubbed value.
pub(crate) const SECRET: &str = "<secret>";
/// The marker that replaces a credential header's whole value.
pub(crate) const REDACTED: &str = "<redacted>";
/// Values shorter than this many chars are not scrubbed: they would mangle ordinary text.
pub(crate) const MIN_LEN: usize = 4;

/// One text to replace. A `fold` pattern stores its folded text and matches ignoring case.
#[derive(Clone, Debug)]
struct Pattern {
    text: String,
    fold: bool,
}

/// `s` with every char lowercased where that keeps its byte length, so byte offsets in the
/// folded copy slice the original and a folded match is a case-insensitive match.
fn fold(s: &str) -> String {
    s.chars()
        .map(|c| {
            let mut lower = c.to_lowercase();
            match (lower.next(), lower.next()) {
                (Some(l), None) if l.len_utf8() == c.len_utf8() => l,
                _ => c,
            }
        })
        .collect()
}

/// The texts to replace, longest first.
#[derive(Clone, Default, Debug)]
pub(crate) struct Redactor {
    patterns: Vec<Pattern>,
    /// Base64 fragments that sit inside a longer encoding, longest first. Each also swallows the
    /// neighbouring characters whose bits it shares, so it cannot be a plain pattern.
    fragments: Vec<String>,
    /// The raw values, for finding them inside a decoded base64 run.
    values: Vec<String>,
    /// A bit per byte value that starts some pattern, so most positions skip the pattern scan.
    first_bytes: [u64; 4],
}

impl Redactor {
    /// Register every encoded form of `value`; a value under `MIN_LEN` chars is ignored.
    pub(crate) fn add_value(&mut self, value: &str) {
        if value.chars().count() < MIN_LEN {
            return;
        }
        // A fragment stays out of the plain patterns, or the pattern pass would match it
        // without its shared edge characters.
        for form in wire_forms(value, FormPolicy::new(MIN_LEN, 1)) {
            match form.kind {
                FormKind::Text => self.insert(form.text, true),
                FormKind::Base64Fragment => self.insert_fragment(form.text),
                _ => self.insert(form.text, false),
            }
        }
        if !self.values.iter().any(|v| v == value) {
            self.values.push(value.to_owned());
        }
    }

    /// Register every value in `values`, as `add_value` does.
    pub(crate) fn add_values<S: AsRef<str>>(&mut self, values: impl IntoIterator<Item = S>) {
        for value in values {
            self.add_value(value.as_ref());
        }
    }

    /// Register the token of each `Basic` Authorization or Proxy-Authorization header whose
    /// decoded `user:pass` contains a registered pattern, so the pair's base64 is scrubbed too.
    pub(crate) fn add_basic_auth(&mut self, headers: &[(String, String)]) {
        for (name, value) in headers {
            if !name.eq_ignore_ascii_case("authorization")
                && !name.eq_ignore_ascii_case("proxy-authorization")
            {
                continue;
            }
            let Some((scheme, token)) = value.trim().split_once(|c: char| c.is_ascii_whitespace())
            else {
                continue;
            };
            let token = token.trim();
            if !scheme.eq_ignore_ascii_case("basic") {
                continue;
            }
            let Ok(decoded) = STANDARD.decode(token) else {
                continue;
            };
            let decoded = String::from_utf8_lossy(&decoded);
            let folded = fold(&decoded);
            let hit = self.patterns.iter().any(|p| {
                if p.fold {
                    folded.contains(p.text.as_str())
                } else {
                    decoded.contains(p.text.as_str())
                }
            });
            if hit {
                self.add_value(token);
            }
        }
    }

    /// Add every pattern `other` holds.
    pub(crate) fn merge(&mut self, other: &Redactor) {
        for pattern in &other.patterns {
            self.insert(pattern.text.clone(), pattern.fold);
        }
        for fragment in &other.fragments {
            self.insert_fragment(fragment.clone());
        }
        for value in &other.values {
            if !self.values.contains(value) {
                self.values.push(value.clone());
            }
        }
    }

    /// Replace every registered pattern in `text` by `SECRET`. One pass marking the longest
    /// match at each position; each run of overlapping or nested matches becomes one marker, and
    /// a marker already written is never matched again. A punycoded label whose decoding holds a
    /// pattern becomes one marker whole first, since its punycode need not contain the pattern's.
    pub(crate) fn scrub(&self, text: &str) -> String {
        if self.patterns.is_empty() && self.fragments.is_empty() {
            return text.to_owned();
        }
        // Whole base64 runs go first: a marker written by the pattern pass would split a run
        // and hide that its decoding holds a value.
        let values: Vec<&str> = self.values.iter().map(String::as_str).collect();
        let text = apinata_core::redact_base64_runs(text, &values, SECRET);
        let scrubbed = if text.to_ascii_lowercase().contains(PUNY) {
            self.scrub_patterns(&self.scrub_labels(&text))
        } else {
            self.scrub_patterns(&text)
        };
        // What is left of a base64 fragment, with the edge characters it shares bits with.
        self.fragments.iter().fold(scrubbed, |text, fragment| {
            apinata_core::replace_base64_fragment(&text, fragment, SECRET)
        })
    }

    /// A redactor over the host forms of each pattern, matched ignoring case.
    fn host_patterns(&self) -> Redactor {
        let mut hosts = Redactor::default();
        for pattern in &self.patterns {
            for form in host_forms(&pattern.text) {
                hosts.insert(form, true);
            }
        }
        hosts
    }

    /// Every `xn--` label in `text` whose Unicode decoding holds a pattern's host form, as one
    /// marker. A label starts at any `xn--`, whatever precedes it (`%2F`, `%40`, a letter).
    fn scrub_labels(&self, text: &str) -> String {
        let folded = self.host_patterns();
        let lower = text.to_ascii_lowercase();
        let (mut out, mut kept, mut from) = (String::with_capacity(text.len()), 0, 0);
        // Every `xn--` start is tried, since an earlier one in the same run may not decode. A
        // hit marks the run from its first `xn--`; a run too long for a host is marked undecoded.
        while let Some(at) = lower[from..].find(PUNY).map(|at| from + at) {
            let end = text[at..]
                .find(|c| !is_label_char(c))
                .map_or(text.len(), |len| at + len);
            let hit = end - at > MAX_HOST || {
                let decoded = url::quirks::domain_to_unicode(&lower[at..end]);
                !decoded.is_empty() && folded.scrub_patterns(&decoded) != decoded
            };
            if hit {
                let run = at
                    - lower[kept..at]
                        .bytes()
                        .rev()
                        .take_while(|&b| is_label_char(char::from(b)))
                        .count();
                let first = lower[run..].find(PUNY).map_or(at, |i| run + i);
                out.push_str(&text[kept..first]);
                out.push_str(SECRET);
                kept = end;
                from = end;
            } else {
                from = at + PUNY.len();
            }
        }
        out.push_str(&text[kept..]);
        out
    }

    /// The pattern pass of `scrub`.
    fn scrub_patterns(&self, text: &str) -> String {
        // The folded copy has the same char boundaries, so one index serves both texts.
        let folded = fold(text);
        let mut out = String::with_capacity(text.len());
        let mut run_end = 0;
        for (i, c) in text.char_indices() {
            let (raw, low) = (text.as_bytes()[i], folded.as_bytes()[i]);
            let bit = |b: u8| self.first_bytes[usize::from(b >> 6)] & (1 << (b & 63)) != 0;
            let len = (bit(raw) || bit(low))
                .then(|| {
                    self.patterns.iter().find_map(|p| {
                        let tail = if p.fold { &folded[i..] } else { &text[i..] };
                        tail.starts_with(p.text.as_str()).then_some(p.text.len())
                    })
                })
                .flatten();
            match len {
                Some(n) if i < run_end => run_end = run_end.max(i + n),
                Some(n) => {
                    out.push_str(SECRET);
                    run_end = i + n;
                }
                None if i < run_end => {}
                None => out.push(c),
            }
        }
        out
    }

    /// Scrub every string in `value` in place, object keys included.
    pub(crate) fn scrub_json(&self, value: &mut serde_json::Value) {
        use serde_json::Value;
        match value {
            Value::String(s) => *s = self.scrub(s),
            Value::Array(items) => items.iter_mut().for_each(|v| self.scrub_json(v)),
            Value::Object(map) => {
                for (key, mut v) in std::mem::take(map) {
                    self.scrub_json(&mut v);
                    map.insert(self.scrub(&key), v);
                }
            }
            _ => {}
        }
    }

    /// The one helper for every shortened text: scrub all of `text`, cut to `max` bytes on a
    /// char boundary, then if cut here or `already_cut` upstream, replace the longest trailing
    /// fragment that some pattern starts with (one char or more) by `SECRET`. Returns
    /// (text, was_cut); the text may exceed `max` by the marker's length.
    pub(crate) fn scrub_cut(&self, text: &str, max: usize, already_cut: bool) -> (String, bool) {
        let mut out = self.scrub(text);
        let cut_here = out.len() > max;
        if cut_here {
            out.truncate(out.floor_char_boundary(max));
        }
        if !(cut_here || already_cut) {
            return (out, false);
        }
        // An upstream cut inside a multibyte char decodes to U+FFFD after the secret's prefix.
        if already_cut {
            let kept = out.trim_end_matches('\u{FFFD}').len();
            out.truncate(kept);
        }
        // A punycoded label cut short cannot be decoded to check it, so none of it is kept,
        // wherever in the trailing run of label chars its `xn--` starts.
        let run = out.trim_end_matches(is_label_char).len();
        if let Some(at) = out[run..].to_ascii_lowercase().find(PUNY) {
            out.truncate(run + at);
            out.push_str(SECRET);
            return (out, true);
        }
        // Only fragments no longer than the longest pattern can start one.
        let longest = self.longest_text();
        let from = out.ceil_char_boundary(out.len().saturating_sub(longest));
        let start = out[from..]
            .char_indices()
            .map(|(i, _)| from + i)
            .find(|&i| {
                let tail = &out[i..];
                let low = fold(tail);
                let plain = self
                    .patterns
                    .iter()
                    .any(|p| p.text.starts_with(if p.fold { low.as_str() } else { tail }));
                plain || self.fragments.iter().any(|f| f.starts_with(tail))
            });
        if let Some(start) = start {
            out.truncate(start);
            out.push_str(SECRET);
        }
        (out, true)
    }

    /// `host`, as core normalised it, with each label holding a registered pattern (matched
    /// ignoring case, in its IDNA forms) scrubbed. A punycoded label that matches in either form
    /// becomes one marker whole, so no piece survives in the other form. Unchanged when none
    /// matches.
    pub(crate) fn scrub_host(&self, host: &str) -> String {
        let folded = self.host_patterns();
        let labels: Vec<String> = host
            .to_lowercase()
            .split('.')
            .map(|label| {
                let unicode = url::quirks::domain_to_unicode(label).to_lowercase();
                if unicode == label || unicode.is_empty() {
                    return folded.scrub(label);
                }
                let hit = folded.scrub(label) != label || folded.scrub(&unicode) != unicode;
                if hit {
                    SECRET.to_owned()
                } else {
                    label.to_owned()
                }
            })
            .collect();
        // A secret holding a dot spans labels, so the whole host is scrubbed too; one found only
        // in the Unicode form of a punycoded host hides the whole host.
        let lower = host.to_lowercase();
        let unicode = url::quirks::domain_to_unicode(&lower).to_lowercase();
        if unicode != lower && !unicode.is_empty() && folded.scrub(&unicode) != unicode {
            return SECRET.to_owned();
        }
        let shown = folded.scrub(&labels.join("."));
        if shown == lower {
            host.to_owned()
        } else {
            shown
        }
    }

    /// The byte length of the longest pattern or fragment.
    fn longest_text(&self) -> usize {
        let first = self.patterns.first().map_or(0, |p| p.text.len());
        first.max(self.fragments.first().map_or(0, String::len))
    }

    /// Insert `fragment` keeping the list sorted by descending byte length, without duplicates.
    fn insert_fragment(&mut self, fragment: String) {
        if fragment.is_empty() || self.fragments.contains(&fragment) {
            return;
        }
        let at = self
            .fragments
            .partition_point(|p| p.len() >= fragment.len());
        self.fragments.insert(at, fragment);
    }

    /// Insert `pattern` (folded first when `fold`) keeping the list sorted by descending byte
    /// length, without duplicates.
    fn insert(&mut self, pattern: String, fold_it: bool) {
        let text = if fold_it { fold(&pattern) } else { pattern };
        if text.is_empty()
            || self
                .patterns
                .iter()
                .any(|p| p.fold == fold_it && p.text == text)
        {
            return;
        }
        let b = text.as_bytes()[0];
        self.first_bytes[usize::from(b >> 6)] |= 1 << (b & 63);
        let at = self
            .patterns
            .partition_point(|p| p.text.len() >= text.len());
        self.patterns.insert(
            at,
            Pattern {
                text,
                fold: fold_it,
            },
        );
    }
}

/// The ACE prefix of a punycoded host label.
const PUNY: &str = "xn--";
/// The longest DNS name; a longer run of label chars cannot be a host.
const MAX_HOST: usize = 253;

fn is_label_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-'
}

/// Request headers with the value of every shared credential header replaced by `REDACTED`.
pub(crate) fn request_headers(h: &[(String, String)]) -> Vec<(String, String)> {
    h.iter()
        .map(|(name, value)| {
            let value = if apinata_core::is_credential_header(name) {
                REDACTED
            } else {
                value
            };
            (name.clone(), value.to_owned())
        })
        .collect()
}

/// Response headers masked by core's rule: a Set-Cookie keeps its cookie name only.
pub(crate) fn response_headers(h: &[(String, String)]) -> Vec<(String, String)> {
    h.iter()
        .map(|(name, value)| {
            (
                name.clone(),
                apinata_core::mask_response_header(name, value),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
    use serde_json::json;
    use url::Url;

    /// Holds a space, a double quote, a slash and a plus, like the end-to-end leak test.
    const AWKWARD: &str = "Ab1 \"q\"/x+y";

    fn redactor(values: &[&str]) -> Redactor {
        let mut r = Redactor::default();
        r.add_values(values.iter().copied());
        r
    }

    /// `host` as core hands it to the policy: lowercased and punycoded.
    fn normalised(host: &str) -> String {
        Url::parse(&format!("http://{host}/"))
            .expect("url")
            .host_str()
            .expect("host")
            .to_owned()
    }

    /// Pins that doubly escaped JSON stays scrubbed; it already was before core's forms joined.
    #[test]
    fn a_doubly_escaped_json_secret_is_scrubbed() {
        let secret = "pa\"ss\\w/rd9";
        let once = serde_json::to_string(secret).unwrap();
        let twice = serde_json::to_string(&once).unwrap();
        let thrice = serde_json::to_string(&twice).unwrap();
        let r = redactor(&[secret]);
        for form in [&once, &twice, &thrice] {
            let shown = r.scrub(&format!("<{form}>"));
            assert!(
                !shown.contains("w/rd9") && !shown.contains("w\\/rd9"),
                "{shown}"
            );
        }
    }

    /// Core's double percent-encoding, which this redactor did not know.
    #[test]
    fn a_double_percent_encoded_secret_is_scrubbed() {
        let r = redactor(&["pa/ss word9"]);
        assert_eq!(r.scrub("x=pa%252Fss%2520word9&y"), "x=<secret>&y");
    }

    /// A fragment inside a longer encoding goes with the edge characters it shares bits with.
    #[test]
    fn a_base64_fragment_swallows_its_shared_edge_characters() {
        let r = redactor(&["Tr0ub4dor"]);
        for (before, after) in [("Q", "Z"), ("QR", "ZY"), ("", "ZYX"), ("QRS", "")] {
            let long = STANDARD_NO_PAD.encode(format!("{before}Tr0ub4dor{after}"));
            let shown = r.scrub(&format!("tok={long};"));
            assert_eq!(shown, "tok=<secret>;", "{before}/{after}");
        }
    }

    /// A 4 byte value has no fragment, so only decoding the whole run finds it.
    #[test]
    fn a_four_byte_secret_inside_a_longer_base64_text_is_scrubbed() {
        let r = redactor(&["Zk3!"]);
        let long = STANDARD.encode("id=7&key=Zk3!&more=1");
        assert_eq!(r.scrub(&format!("blob {long} end")), "blob <secret> end");
    }

    /// A text cut inside a fragment, or inside the edge characters it swallows, keeps no piece.
    #[test]
    fn a_cut_inside_a_base64_fragment_keeps_no_piece_of_it() {
        let r = redactor(&["Tr0ub4dor"]);
        let fragment = apinata_core::base64_fragment_forms("Tr0ub4dor")
            .into_iter()
            .next()
            .expect("a fragment");
        for already_cut in [true, false] {
            for keep in 3..fragment.len() {
                let text = format!("tok=QR{}{}", &fragment[..keep], "x".repeat(40));
                let max = "tok=QR".len() + keep;
                let (shown, cut) = r.scrub_cut(&text, max, already_cut);
                assert!(cut);
                let bare = shown.replace(SECRET, "");
                assert!(!bare.contains(&fragment[..keep]), "{keep}: {shown}");
            }
        }
    }

    #[test]
    fn a_host_shows_no_secret_in_either_form() {
        let one = redactor(&["Geheimnüs9"]);
        let both = redactor(&["LeakyA9", "Geheimnüs9"]);
        for (r, host) in [
            (&one, "Geheimnüs9.Geheimnüs9-api.example"),
            (&both, "LeakyA9.Geheimnüs9-api.example"),
        ] {
            let host = normalised(host);
            let shown = r.scrub_host(&host).to_lowercase();
            assert_ne!(shown, host);
            for leak in ["geheimnüs9", "geheimns9", "leakya9", "xn--"] {
                assert!(!shown.contains(leak), "{host} shown as {shown}");
            }
        }
    }

    #[test]
    fn host_normalised_forms_of_a_value_are_scrubbed_anywhere() {
        let r = redactor(&["LeakyHost9", "Geheimnüs9"]);
        let text = format!(
            "http://leakyhost9.localhost/ and http://{}/",
            normalised("Geheimnüs9.localhost")
        );
        let shown = r.scrub(&text);
        assert_eq!(
            shown,
            "http://<secret>.localhost/ and http://<secret>.localhost/"
        );
    }

    #[test]
    fn a_secret_inside_a_punycoded_label_is_scrubbed_in_text_and_json() {
        let r = redactor(&["Geheimnüs9"]);
        let host = normalised("xGeheimnüs9y.localhost");
        let label = host.split('.').next().expect("label").to_owned();
        assert!(label.starts_with("xn--"), "{label}");
        let text = format!("url http://{host}:80/ and {}", label.to_uppercase());
        assert_eq!(
            r.scrub(&text),
            "url http://<secret>.localhost:80/ and <secret>"
        );
        let mut value = json!({"url": format!("http://{host}/"), host.clone(): [label.clone()]});
        r.scrub_json(&mut value);
        assert_eq!(
            value,
            json!({"url": "http://<secret>.localhost/", "<secret>.localhost": ["<secret>"]})
        );
    }

    #[test]
    fn a_punycoded_label_not_starting_a_run_is_scrubbed() {
        let r = redactor(&["Geheimnüs9"]);
        let host = normalised("xGeheimnüs9y.localhost");
        let label = host.split('.').next().expect("label");
        for before in ["http%3A%2F%2F", "u%40", "9", "a"] {
            let text = format!("{before}{label}.localhost%2Fx");
            assert_eq!(
                r.scrub(&text),
                format!("{before}<secret>.localhost%2Fx"),
                "{text}"
            );
            let cut = format!("{before}{}", &label[..label.len() - 3]);
            let (out, _) = r.scrub_cut(&cut, 1024, true);
            assert_eq!(out, format!("{before}<secret>"), "{cut}");
        }
    }

    #[test]
    fn every_xn_start_inside_a_run_is_tried() {
        let r = redactor(&["Geheimnüs9"]);
        let host = normalised("xGeheimnüs9y.localhost");
        let label = host.split('.').next().expect("label");
        for before in ["xn--", "axn--xn--xn--", "xn--zz", "xn--abc-"] {
            let text = format!("{before}{label}.localhost");
            let shown = r.scrub(&text);
            assert!(
                shown.ends_with("<secret>.localhost"),
                "{text} shown as {shown}"
            );
            assert!(!shown.to_lowercase().contains("geheimn"), "{shown}");
            for max in 1..text.len() {
                let (out, _) = r.scrub_cut(&text, max, false);
                assert!(
                    !out.to_lowercase().contains("geheimn"),
                    "cut at {max}: {out}"
                );
            }
        }
    }

    #[test]
    fn an_xn_run_longer_than_any_host_is_marked_whole() {
        let r = redactor(&["Geheimnüs9"]);
        let text = format!("<{}>", "xn--".repeat(100));
        assert_eq!(r.scrub(&text), "<<secret>>");
    }

    #[test]
    fn a_dotted_secret_spanning_host_labels_is_scrubbed() {
        let r = redactor(&["my.Secret9"]);
        assert_eq!(r.scrub_host("my.secret9.example"), "<secret>.example");
        let r = redactor(&["a.Geheimnüs9"]);
        let host = normalised("xa.Geheimnüs9y.example");
        let shown = r.scrub_host(&host).to_lowercase();
        for leak in ["geheimn", "xn--", "xa."] {
            assert!(!shown.contains(leak), "{host} shown as {shown}");
        }
    }

    #[test]
    fn a_punycoded_label_cut_upstream_leaves_none_of_it() {
        let r = redactor(&["Geheimnüs9"]);
        let host = normalised("xGeheimnüs9y.localhost");
        let label = host.split('.').next().expect("label");
        for end in PUNY.len()..=label.len() {
            let text = format!("http://{}", &label[..end]);
            let (out, cut) = r.scrub_cut(&text, 1024, true);
            assert!(cut);
            assert_eq!(out, "http://<secret>", "cut after {end} bytes");
        }
    }

    #[test]
    fn every_listed_form_is_scrubbed() {
        let r = redactor(&[AWKWARD]);
        let forms = [
            AWKWARD.to_owned(),
            "Ab1%20%22q%22%2Fx%2By".to_owned(),
            "Ab1+%22q%22%2Fx%2By".to_owned(),
            STANDARD.encode(AWKWARD),
            STANDARD_NO_PAD.encode(AWKWARD),
            URL_SAFE.encode(AWKWARD),
            URL_SAFE_NO_PAD.encode(AWKWARD),
            "Ab1 \\\"q\\\"/x+y".to_owned(),
            "Ab1 \\\\\\\"q\\\\\\\"/x+y".to_owned(),
        ];
        for form in forms {
            assert_eq!(r.scrub(&format!("<{form}>")), "<<secret>>", "form {form}");
        }
    }

    #[test]
    fn url_component_forms_match_what_url_serialises() {
        let r = redactor(&[AWKWARD]);
        let mut url = Url::parse("http://h.test/").unwrap();
        url.set_path(&format!("/a/{AWKWARD}"));
        url.set_query(Some(&format!("k={AWKWARD}")));
        url.set_fragment(Some(AWKWARD));
        url.set_username(AWKWARD).unwrap();
        url.set_password(Some(AWKWARD)).unwrap();
        let scrubbed = r.scrub(url.as_str());
        assert_eq!(
            scrubbed,
            "http://<secret>:<secret>@h.test/a/<secret>?k=<secret>#<secret>"
        );
    }

    #[test]
    fn values_under_four_characters_are_left_alone() {
        let r = redactor(&["abc", ""]);
        assert_eq!(r.scrub("abc and abcd"), "abc and abcd");
    }

    #[test]
    fn a_longer_value_is_replaced_before_a_shorter_one_inside_it() {
        let r = redactor(&["token", "token-long-value"]);
        assert_eq!(r.scrub("x token-long-value y"), "x <secret> y");
    }

    #[test]
    fn a_basic_auth_token_is_scrubbed_when_its_pair_holds_a_sensitive_value() {
        let mut r = redactor(&["pa55word"]);
        let token = STANDARD.encode("me:pa55word");
        let plain = STANDARD.encode("me:open");
        r.add_basic_auth(&[
            ("authorization".to_owned(), format!("Basic {token}")),
            ("proxy-authorization".to_owned(), format!("Basic {plain}")),
        ]);
        assert_eq!(r.scrub(&format!("echo {token}")), "echo <secret>");
        assert_eq!(r.scrub(&format!("echo {plain}")), format!("echo {plain}"));
    }

    #[test]
    fn a_marker_is_never_matched_again() {
        let r = redactor(&["secret", "longer-value"]);
        assert_eq!(r.scrub("longer-value secret"), "<secret> <secret>");
    }

    #[test]
    fn json_scrubbing_reaches_nested_strings_and_keys() {
        let r = redactor(&["s3cr3t"]);
        let mut value = json!({"a": ["x s3cr3t", {"s3cr3t": 1}], "n": 5});
        r.scrub_json(&mut value);
        assert_eq!(value, json!({"a": ["x <secret>", {"<secret>": 1}], "n": 5}));
    }

    #[test]
    fn merging_keeps_both_sets() {
        let mut a = redactor(&["alpha-1"]);
        a.merge(&redactor(&["beta-22"]));
        assert_eq!(a.scrub("alpha-1 beta-22"), "<secret> <secret>");
    }

    #[test]
    fn credential_headers_are_redacted_whole_and_cookie_names_survive() {
        let request = request_headers(&[
            ("Authorization".to_owned(), "Bearer t".to_owned()),
            ("proxy-authorization".to_owned(), "Basic x".to_owned()),
            ("cookie".to_owned(), "sid=1".to_owned()),
            ("x-other".to_owned(), "kept".to_owned()),
        ]);
        let values: Vec<&str> = request.iter().map(|(_, v)| v.as_str()).collect();
        assert_eq!(values, [REDACTED, REDACTED, REDACTED, "kept"]);
        let response = response_headers(&[
            ("set-cookie".to_owned(), "sid=abc; Path=/".to_owned()),
            ("content-type".to_owned(), "text/plain".to_owned()),
        ]);
        assert_eq!(response[0].1, "sid=[redacted]");
        assert_eq!(response[1].1, "text/plain");
    }

    #[test]
    fn a_cut_never_leaves_a_piece_of_a_secret() {
        let secret = "Zq7Wk9Px";
        let r = redactor(&[secret]);
        let text = format!("body:{secret}:end");
        for max in 0..=text.len() {
            let (out, cut) = r.scrub_cut(&text, max, false);
            let windows: Vec<&str> = (0..secret.len() - 1).map(|i| &secret[i..i + 2]).collect();
            for w in windows {
                assert!(!out.contains(w), "max {max}: {out:?} holds {w:?}");
            }
            assert!(out.len() <= max.max(5 + SECRET.len()), "max {max}: {out:?}");
            assert_eq!(cut, max < 5 + SECRET.len() + 4, "max {max}");
        }
    }

    #[test]
    fn a_text_cut_upstream_mid_secret_ends_in_the_marker() {
        let r = redactor(&["Zq7Wk9Px"]);
        let (out, cut) = r.scrub_cut("abc Zq7W", 1024, true);
        assert_eq!(out, "abc <secret>");
        assert!(cut);
        // Uncut text is left as scrubbed, even when it ends like a pattern begins.
        assert_eq!(
            r.scrub_cut("abc Zq", 1024, false),
            ("abc Zq".to_owned(), false)
        );
    }

    #[test]
    fn a_secret_cut_upstream_mid_character_leaves_no_prefix() {
        // A lossy decode of bytes cut inside a multibyte char ends in U+FFFD after the prefix.
        let r = redactor(&["päßwört"]);
        let (out, cut) = r.scrub_cut("abc päß\u{FFFD}", 1024, true);
        assert_eq!(out, "abc <secret>");
        assert!(cut);
    }
    /// Holds an ampersand, a slash, a non-ASCII letter, angle brackets, a plus and a space.
    const ESCAPY: &str = "p&ss/wörd<x>+ 1";

    #[test]
    fn json_unicode_slash_and_html_escapes_are_scrubbed() {
        let r = redactor(&[ESCAPY, "k3y\u{1F600}ab"]);
        let forms = [
            "p&ss/w\\u00f6rd<x>+ 1",
            "p&ss/w\\u00F6rd<x>+ 1",
            "p&ss\\/w\\u00f6rd<x>+ 1",
            "p&ss\\/wörd<x>+ 1",
            "p\\u0026ss/wörd\\u003cx\\u003e+ 1",
            "k3y\\ud83d\\ude00ab",
            "k3y\\uD83D\\uDE00ab",
        ];
        for form in forms {
            assert_eq!(r.scrub(&format!("[{form}]")), "[<secret>]", "form {form}");
        }
    }

    #[test]
    fn lowercase_percent_hex_is_scrubbed() {
        let r = redactor(&[ESCAPY]);
        for form in [
            "p%26ss%2fw%c3%b6rd%3cx%3e%2b%201",
            "p%26ss%2fw%c3%b6rd%3cx%3e%2b+1",
        ] {
            assert_eq!(r.scrub(&format!("[{form}]")), "[<secret>]", "form {form}");
        }
    }

    #[test]
    fn base64_of_a_body_holding_the_secret_at_any_offset_is_scrubbed() {
        let secret = "Zq7Wk9Px";
        let r = redactor(&[secret]);
        for prefix in ["", "a", "ab", "{\"t\":\""] {
            let body = format!("{prefix}{secret}\"}}");
            for encoded in [STANDARD.encode(&body), URL_SAFE_NO_PAD.encode(&body)] {
                assert!(
                    r.scrub(&encoded).contains(SECRET),
                    "prefix {prefix:?}: {encoded}"
                );
            }
        }
    }

    #[test]
    fn overlapping_secrets_are_replaced_as_one_run() {
        let r = redactor(&["abcdef", "defghi"]);
        assert_eq!(r.scrub("x abcdefghi y"), "x <secret> y");
    }

    #[test]
    fn a_basic_auth_token_after_any_whitespace_is_registered() {
        let mut r = redactor(&["pa55word"]);
        let token = STANDARD.encode("me:pa55word");
        r.add_basic_auth(&[("Authorization".to_owned(), format!("BASIC\t{token}"))]);
        assert_eq!(r.scrub(&format!("echo {token}")), "echo <secret>");
    }

    #[test]
    fn an_aws_session_token_header_is_redacted_whole() {
        let request = request_headers(&[
            ("X-Amz-Security-Token".to_owned(), "tok".to_owned()),
            ("x-amz-date".to_owned(), "20150830T123600Z".to_owned()),
        ]);
        assert_eq!(request[0].1, REDACTED);
        assert_eq!(request[1].1, "20150830T123600Z");
    }

    use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

    // Baseline helpers: independent of the code under test.
    const PATHSET: &AsciiSet = &UNRESERVED.remove(b'/');
    const UNI: &str = "p&ss/wörd<x>+ 1";
    const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'.')
        .remove(b'_')
        .remove(b'~');

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

    /// Holds `~`, `*`, a space, `/`, a non-ASCII character, a quote and a backslash.
    const W: &str = "a~*b /ö\"\\z";
    /// A host with a non-ASCII label.
    const HOST: &str = "Bücher.Example";

    /// `\uXXXX` for non-ASCII characters only; ASCII stays literal.
    fn unicode_non_ascii(v: &str, upper: bool) -> String {
        let mut out = String::new();
        for ch in v.chars() {
            if ch.is_ascii() {
                out.push(ch);
                continue;
            }
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
    /// serde_json's body with the given extra escapes applied, written independently of `rescape`.
    fn reescaped(v: &str, ascii: Option<bool>, slash: bool, html: bool) -> String {
        let mut body = json(v);
        if slash {
            body = body.replace('/', "\\/");
        }
        if html {
            body = body
                .replace('&', "\\u0026")
                .replace('<', "\\u003c")
                .replace('>', "\\u003e");
        }
        match ascii {
            Some(upper) => unicode_non_ascii(&body, upper),
            None => body,
        }
    }
    /// Each base64 run the bits of `v` alone decide, with a longer encoding that holds it.
    fn fragments(v: &str) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for lead in 0..3usize {
            for tail in 0..3usize {
                let mut bytes = b"Q!z"[..lead].to_vec();
                bytes.extend_from_slice(v.as_bytes());
                bytes.extend_from_slice(&b"~Pw"[..tail]);
                let mut zeros = vec![0u8; lead];
                zeros.extend_from_slice(v.as_bytes());
                let (start, end) = ((8 * lead).div_ceil(6), 8 * (lead + v.len()) / 6);
                for engine in [STANDARD_NO_PAD, URL_SAFE_NO_PAD] {
                    let run = engine.encode(&zeros)[start..end].to_owned();
                    let text = engine.encode(&bytes);
                    assert!(text.contains(&run), "helper: {run} in {text}");
                    out.push((run, text));
                }
            }
        }
        out
    }

    fn mcp_rows(v: &str) -> Vec<(&'static str, String)> {
        let j1 = json(v);
        let ecu: String = v
            .bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect();
        let form: String = url::form_urlencoded::byte_serialize(v.as_bytes()).collect();
        let mut u = Url::parse("http://h.test/").unwrap();
        u.set_query(Some(v));
        let query = u.query().unwrap().to_owned();
        let mut u = Url::parse("http://h.test/").unwrap();
        u.set_password(Some(v)).unwrap();
        let password = u.password().unwrap().to_owned();
        let mut u = Url::parse("http://h.test/").unwrap();
        u.set_path(v);
        let path = u.path().trim_start_matches('/').to_owned();
        let mut u = Url::parse("http://h.test/").unwrap();
        u.set_fragment(Some(v));
        let fragment = u.fragment().unwrap().to_owned();
        let mut u = Url::parse("http://h.test/").unwrap();
        u.set_username(v).unwrap();
        let username = u.username().to_owned();
        let path_set = utf8_percent_encode(v, PATHSET).to_string();
        let plus = form.clone();
        let mut out = vec![
            ("raw", v.to_owned()),
            ("json1", j1.clone()),
            ("json2", json(&j1)),
            ("json3", json(&json(&j1))),
            ("php slash", j1.replace('/', "\\/")),
            (
                "go html",
                j1.replace('&', "\\u0026")
                    .replace('<', "\\u003c")
                    .replace('>', "\\u003e"),
            ),
            ("encodeURIComponent", ecu.clone()),
            ("encodeURIComponent lower", lower_hex(&ecu)),
            ("form", form.clone()),
            ("form lower", lower_hex(&form)),
            ("url query", query),
            ("url password", password),
            ("base64 std", STANDARD.encode(v)),
            ("base64 url nopad", URL_SAFE_NO_PAD.encode(v)),
            ("unicode upper", unicode_all(v, true)),
            ("percent twice", pct(&pct(v))),
        ];
        out.extend([
            ("url path", path),
            ("url fragment", fragment),
            ("url username", username),
            ("lowercase", v.to_ascii_lowercase()),
            ("percent path set", path_set.clone()),
            ("percent path set lower", lower_hex(&path_set)),
            ("percent plus", plus.clone()),
            ("percent plus lower", lower_hex(&plus)),
            ("unicode lower", unicode_all(v, false)),
            ("ascii literal unicode upper", unicode_non_ascii(v, true)),
            ("ascii literal unicode lower", unicode_non_ascii(v, false)),
            ("percent twice path set", pct(&path_set)),
            ("percent twice plus", pct(&plus)),
            ("percent twice lower", pct(&lower_hex(&pct(v)))),
        ]);
        for (ascii, slash, html) in [
            (None, true, true),
            (Some(true), true, false),
            (Some(false), true, false),
            (Some(true), true, true),
            (Some(false), true, true),
            (Some(true), false, true),
            (Some(false), false, true),
            (Some(true), false, false),
            (Some(false), false, false),
        ] {
            out.push(("rescape", reescaped(v, ascii, slash, html)));
        }
        out
    }

    #[test]
    fn every_baseline_echo_is_scrubbed() {
        for v in [AWKWARD, UNI, W] {
            let r = redactor(&[v]);
            for (name, row) in mcp_rows(v) {
                assert_eq!(
                    r.scrub(&format!("<{row}>")),
                    "<<secret>>",
                    "{name} of {v:?}"
                );
            }
        }
    }

    #[test]
    fn every_baseline_host_form_is_scrubbed() {
        let r = redactor(&[HOST]);
        for row in ["bücher.example", "xn--bcher-kva.example", "BÜCHER.EXAMPLE"] {
            assert_eq!(r.scrub(&format!("<{row}>")), "<<secret>>", "{row}");
        }
    }

    #[test]
    fn every_baseline_fragment_is_scrubbed() {
        for v in [AWKWARD, UNI, W] {
            let r = redactor(&[v]);
            for (run, text) in fragments(v) {
                for shown in [run.clone(), text] {
                    let out = r.scrub(&format!("<{shown}>"));
                    assert!(!out.contains(&run), "{run} left in {out} for {v:?}");
                    assert!(out.contains(SECRET), "{out}");
                }
            }
        }
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
    fn a_mixed_case_percent_echo_is_scrubbed() {
        let r = redactor(&[AWKWARD]);
        let mixed = alternate_hex(&pct(AWKWARD));
        assert_eq!(r.scrub(&format!("<{mixed}>")), "<<secret>>");
    }

    #[test]
    fn a_mixed_case_unicode_escape_is_scrubbed() {
        let v = "éöüß key";
        assert_eq!(
            redactor(&[v]).scrub(&format!("<{}>", mixed_unicode(v))),
            "<<secret>>"
        );
    }

    #[test]
    fn the_path_segment_set_is_scrubbed() {
        assert_eq!(
            redactor(&[AWKWARD]).scrub("<Ab1%20%22q%22%2Fx+y>"),
            "<<secret>>"
        );
    }

    #[test]
    fn matching_is_case_insensitive_for_text_but_not_for_base64() {
        let r = redactor(&["Hunter-22x"]);
        assert_eq!(r.scrub("<hUNTER-22X>"), "<<secret>>");
        let b64 = STANDARD.encode("Hunter-22x");
        assert_eq!(r.scrub(&format!("<{b64}>")), "<<secret>>");
        let lower = b64.to_ascii_lowercase();
        assert_eq!(r.scrub(&format!("<{lower}>")), format!("<{lower}>"));
    }

    #[test]
    fn a_value_inside_the_marker_is_not_matched_again() {
        let r = redactor(&["SECRET", "longer-value"]);
        assert_eq!(r.scrub("longer-value secret"), "<secret> <secret>");
    }

    #[test]
    fn a_cut_inside_a_folded_form_keeps_no_prefix() {
        let r = redactor(&["Zq7Wk9Px"]);
        let (out, cut) = r.scrub_cut("abc zq7w", 1024, true);
        assert_eq!(out, "abc <secret>");
        assert!(cut);
    }
}
