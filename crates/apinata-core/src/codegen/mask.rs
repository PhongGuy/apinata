//! Masked generation. Secrets are swapped for alphanumeric sentinels in the request before any
//! target escapes it, since no escaper alters letters and digits; the snippet is generated from
//! that copy and each sentinel then becomes its secret's placeholder.

use crate::resolve::ResolvedRequest;
use crate::secret_names::{MIN_CHARS, exact_forms, masking, redact_base64_runs, replace_secrets};

use super::{Target, generate};

/// The request as a `target` snippet with every secret replaced. Each mask is a secret value
/// and the text shown in its place; a base64 run that decodes to text holding a secret, such as
/// a Basic credential, shows as `redacted`. Values under 4 characters are not masked, as every
/// redactor leaves them.
pub fn generate_masked(
    req: &ResolvedRequest,
    target: Target,
    masks: &[(&str, &str)],
    redacted: &str,
) -> String {
    let masks: Vec<(&str, &str)> = masks
        .iter()
        .copied()
        .filter(|(value, _)| value.chars().count() >= MIN_CHARS)
        .collect();
    if masks.is_empty() {
        return generate(req, target);
    }
    let values: Vec<&str> = masks.iter().map(|(value, _)| *value).collect();
    let mut fresh = Sentinels::new(&generate(req, target), &values);
    let sentinels: Vec<String> = masks.iter().map(|_| fresh.next()).collect();
    let hidden = fresh.next();

    // Every exact form of every secret, longest first, so a secret nested in another never
    // splits it; base64 runs that decode to a secret go after, while the sentinels are inert.
    let mut needles: Vec<(String, &str)> = Vec::new();
    for ((value, _), sentinel) in masks.iter().zip(&sentinels) {
        needles.extend(
            exact_forms(value)
                .into_iter()
                .map(|f| (f, sentinel.as_str())),
        );
    }
    needles.sort_by_key(|n| std::cmp::Reverse(n.0.len()));
    let masked = req.map_text(|text| {
        redact_base64_runs(&replace_leftmost_longest(text, &needles), &values, &hidden)
    });

    // Core's redactor runs as a backstop while the sentinels stand, so it can never match
    // inside a placeholder; the sentinels become the placeholders last.
    let named: Vec<_> = masks
        .iter()
        .zip(&sentinels)
        .map(|((value, _), sentinel)| masking(value, sentinel))
        .collect();
    let mut out = replace_secrets(&generate(&masked, target), &named);
    // A target that encodes a credential itself, as raw HTTP does URL userinfo, encodes the
    // sentinel; that run is hidden too, so the snippet never carries a stand-in credential.
    let marks: Vec<&str> = sentinels.iter().map(String::as_str).collect();
    out = redact_base64_runs(&out, &marks, &hidden);
    for ((_, placeholder), sentinel) in masks.iter().zip(&sentinels) {
        out = out.replace(sentinel.as_str(), placeholder);
    }
    out.replace(&hidden, redacted)
}

/// Sentinels: distinct runs of 16 lowercase letters, each absent from the snippet and holding
/// no secret, so none collides with snippet text and no secret matches inside one. Lowercase,
/// since URL parsing folds a host to lowercase.
struct Sentinels {
    seen: String,
    values: Vec<String>,
    made: Vec<String>,
    state: u64,
}

impl Sentinels {
    fn new(seen: &str, values: &[&str]) -> Sentinels {
        Sentinels {
            seen: seen.to_ascii_lowercase(),
            values: values.iter().map(|v| v.to_ascii_lowercase()).collect(),
            made: Vec::new(),
            state: 0x9e37_79b9_7f4a_7c15,
        }
    }

    fn next(&mut self) -> String {
        loop {
            let word: String = (0..16)
                .map(|_| {
                    // xorshift64: a fixed sequence, so generation stays deterministic.
                    self.state ^= self.state << 13;
                    self.state ^= self.state >> 7;
                    self.state ^= self.state << 17;
                    char::from(b'a' + u8::try_from(self.state % 26).unwrap_or(0))
                })
                .collect();
            let clash = self.seen.contains(&word)
                || self.made.contains(&word)
                || self.values.iter().any(|v| word.contains(v.as_str()));
            if !clash {
                self.made.push(word.clone());
                return word;
            }
        }
    }
}

/// `text` with each needle replaced in one left-to-right pass, the longest needle winning at
/// each position; `needles` is sorted longest first.
fn replace_leftmost_longest(text: &str, needles: &[(String, &str)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        match needles.iter().find(|(n, _)| rest.starts_with(n.as_str())) {
            Some((n, with)) => {
                out.push_str(with);
                rest = &rest[n.len()..];
            }
            None => {
                out.push(c);
                rest = &rest[c.len_utf8()..];
            }
        }
    }
    out
}
