//! Cutting stored fields to a byte cap without leaving any part of a secret behind.

use super::redact::Redactor;
use crate::FormKind;

/// The marker that ends every cut field: `\n[truncated: kept {kept} of {total} bytes]`.
pub(crate) fn marker(kept: usize, total: u64) -> String {
    format!("\n[truncated: kept {kept} of {total} bytes]")
}

/// Where a cut field's kept content ends, and N, the original length its marker reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Cut {
    pub(crate) kept: usize,
    pub(crate) total: u64,
}

/// `bytes` decoded lossily, redacted, and cut to at most `cap` bytes plus the marker,
/// with N = `bytes.len()`; the `Cut` is `Some` when it was cut.
pub(crate) fn cap_bytes(r: &Redactor, bytes: &[u8], cap: usize) -> (String, Option<Cut>) {
    cap_bytes_of(r, bytes, cap, bytes.len() as u64)
}

/// `cap_bytes` with N = `total`, the original length `bytes` were taken from; bytes
/// shorter than `total` are always stored as cut.
pub(crate) fn cap_bytes_of(
    r: &Redactor,
    bytes: &[u8],
    cap: usize,
    total: u64,
) -> (String, Option<Cut>) {
    if (bytes.len() as u64) >= total {
        return cap_window(r, bytes, cap, total);
    }
    // Bytes kept only in part may end inside a value, so their tail is trimmed and marked
    // too; a character split by the cut is dropped first, so the trim sees the prefix.
    let bytes = &bytes[..complete_len(bytes)];
    let (text, cut) = cap_window(r, bytes, cap, total);
    match cut {
        Some(_) => (text, cut),
        None => recut(r, text, cap, total),
    }
}

/// The length of `bytes` without a trailing UTF-8 sequence that the end cuts short.
fn complete_len(bytes: &[u8]) -> usize {
    let tail = bytes.len().saturating_sub(3);
    let Some(start) = (tail..bytes.len()).rev().find(|&i| bytes[i] & 0xC0 != 0x80) else {
        return bytes.len();
    };
    match std::str::from_utf8(&bytes[start..]) {
        Err(e) if e.error_len().is_none() => start,
        _ => bytes.len(),
    }
}

/// `cap_bytes` over a str, returning only the stored text.
#[cfg(test)]
pub(crate) fn cap_text(r: &Redactor, text: &str, cap: usize) -> String {
    cap_bytes(r, text.as_bytes(), cap).0
}

/// A stored field re-redacted with `r` under the truncation invariant. A field recorded
/// as cut has only its content redacted, recut to `cap` and re-marked with its own N. An
/// uncut field is redacted whole and, if that pushes it past `cap`, cut with `total` as N.
pub(crate) fn recap(
    r: &Redactor,
    stored: &str,
    cut: Option<Cut>,
    cap: usize,
    total: u64,
) -> (String, Option<Cut>) {
    match cut {
        // A kept length that is not a boundary of `stored` is a broken record; keep nothing.
        Some(c) => recut(
            r,
            r.redact(stored.get(..c.kept).unwrap_or("")),
            cap,
            c.total,
        ),
        None => cap_window(r, stored.as_bytes(), cap, total),
    }
}

/// The shared core of `cap_bytes` and `recap`: redact, cut at `cap`, trim, mark with `total`.
fn cap_window(r: &Redactor, bytes: &[u8], cap: usize, total: u64) -> (String, Option<Cut>) {
    // Redact a window one longest form past the cap, so a value starting before the cut is
    // either whole in the window or straddles its end as a prefix the trim then removes.
    let mut window = cap.saturating_add(r.longest_form()).min(bytes.len());
    // End the window on a character start, so a value's prefix is never followed by U+FFFD.
    while window < bytes.len() && bytes[window] & 0xC0 == 0x80 {
        window += 1;
    }
    let mut text = r.redact(&String::from_utf8_lossy(&bytes[..window]));
    if window == bytes.len() && text.len() <= cap {
        text.shrink_to_fit();
        return (text, None);
    }
    recut(r, text, cap, total)
}

/// Already-redacted content cut to `cap`, trimmed, and marked with N = `total`.
fn recut(r: &Redactor, mut text: String, cap: usize, total: u64) -> (String, Option<Cut>) {
    truncate_to(&mut text, cap);
    trim_partial(r, &mut text);
    let kept = text.len();
    text.push_str(&marker(kept, total));
    text.shrink_to_fit();
    (text, Some(Cut { kept, total }))
}

/// Cuts `text` to at most `cap` bytes, backing off to a character boundary.
fn truncate_to(text: &mut String, cap: usize) {
    let mut end = cap.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
}

/// Drops any trailing proper prefix of any form, repeatedly, since one trim can expose another.
/// Text forms match ASCII case-insensitively, as in redaction; base64 forms match exactly.
fn trim_partial(r: &Redactor, text: &mut String) {
    if r.needles().is_empty() {
        return;
    }
    let bytes = text.as_bytes();
    let folded = Redactor::fold(text);
    let len = bytes.len();

    // reach[i] is how far a proper form prefix starting at i extends in the text, so a
    // suffix text[i..e] is a trimmable prefix exactly when e <= reach[i].
    let mut reach = vec![0usize; len];
    for (i, slot) in reach.iter_mut().enumerate() {
        if !text.is_char_boundary(i) {
            continue;
        }
        let mut best = 0;
        for f in r.needles() {
            let (n, t) = match f.kind {
                FormKind::Text => (f.needle(), &folded.as_bytes()[i..]),
                _ => (f.text.as_str(), &bytes[i..]),
            };
            if n.as_bytes()[0] != t[0] {
                continue;
            }
            let common = n.bytes().zip(t).take_while(|(a, b)| a == *b).count();
            let mut d = common.min(n.len() - 1);
            while !n.is_char_boundary(d) {
                d -= 1;
            }
            best = best.max(d);
        }
        *slot = i + best;
    }

    // The earliest start covering each end e, assigned in one sweep over increasing starts;
    // `next` skips ends that already have one.
    let mut start_of = vec![usize::MAX; len + 1];
    let mut next: Vec<usize> = (0..=len + 1).collect();
    fn find(next: &mut [usize], mut x: usize) -> usize {
        while next[x] != x {
            next[x] = next[next[x]];
            x = next[x];
        }
        x
    }
    for (i, &r_end) in reach.iter().enumerate() {
        let mut e = find(&mut next, i + 1);
        while e <= r_end && e <= len {
            start_of[e] = i;
            next[e] = e + 1;
            e = find(&mut next, e);
        }
    }

    let mut end = len;
    while start_of[end] != usize::MAX {
        end = start_of[end];
    }
    text.truncate(end);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_bytes_ending_in_a_split_character_trim_the_prefix() {
        let r = Redactor::new(&["abcé-secret"]);
        let (text, cut) = cap_bytes_of(&r, b"x abc\xC3", 100, 900);
        assert!(!text.to_ascii_lowercase().contains("abc"), "{text}");
        assert_eq!(cut.map(|c| c.total), Some(900));
    }

    #[test]
    fn partial_bytes_are_cut_with_their_tail_prefix_trimmed() {
        let r = Redactor::new(&["s3cret-value"]);
        let (text, cut) = cap_bytes_of(&r, b"abc s3cr", 100, 900);
        assert_eq!(text, format!("abc {}", marker(4, 900)));
        assert_eq!(
            cut,
            Some(Cut {
                kept: 4,
                total: 900
            })
        );
    }

    /// The kept text, K and N of a stored field that ends in the marker.
    fn split_marker(stored: &str) -> Option<(&str, usize, u64)> {
        let at = stored.rfind("\n[truncated: kept ")?;
        let (text, tail) = stored.split_at(at);
        let nums = tail
            .strip_prefix("\n[truncated: kept ")?
            .strip_suffix(" bytes]")?;
        let (k, n) = nums.split_once(" of ")?;
        Some((text, k.parse().ok()?, n.parse().ok()?))
    }

    #[test]
    fn short_text_is_redacted_and_not_cut() {
        let r = Redactor::new(&["sec"]);
        assert_eq!(
            cap_bytes(&r, b"a sec b", 100),
            ("a [redacted] b".to_owned(), None)
        );
        assert_eq!(cap_text(&Redactor::none(), "", 0), "");
    }

    #[test]
    fn spec_example_cuts_no_prefix_of_the_secret() {
        let s = "ABCDEFGHIJKLMNOPQRST";
        // The padding keeps the window, which reaches past the longest form, short of the end.
        let text = format!("{}abc{s}{}", s.repeat(5), ".".repeat(400));
        let r = Redactor::new(&[s]);
        let stored = cap_text(&r, &text, 53);
        // `abc` is a case variant of the value's prefix `ABC`, so it is trimmed too.
        assert_eq!(
            stored,
            format!(
                "{}{}",
                "[redacted]".repeat(5),
                marker(50, text.len() as u64)
            )
        );
    }

    #[test]
    fn a_run_of_the_leading_character_is_over_redacted_never_leaked() {
        let r = Redactor::new(&["AAAB"]);
        let text = format!("x{}", "A".repeat(50));
        assert_eq!(cap_text(&r, &text, 10), format!("x{}", marker(1, 51)));
    }

    #[test]
    fn a_cut_inside_a_multibyte_value_leaves_no_prefix() {
        let s = "ééééé";
        let text = format!("ab{s}");
        let r = Redactor::new(&[s]);
        for cap in 0..text.len() {
            let stored = cap_text(&r, &text, cap);
            let (kept, k, n) = split_marker(&stored).expect("cut");
            assert_eq!((k, n), (kept.len(), text.len() as u64));
            assert!(
                !kept.contains('é') && !kept.contains('\u{FFFD}'),
                "cap {cap}: {stored}"
            );
        }
    }

    #[test]
    fn recap_trims_a_late_value_straddling_an_earlier_cut() {
        let text = "hello LATESECRET tail";
        let (stored, cut) = cap_bytes(&Redactor::none(), text.as_bytes(), 9);
        assert_eq!(stored, format!("hello LAT{}", marker(9, 21)));
        assert_eq!(cut, Some(Cut { kept: 9, total: 21 }));
        let late = Redactor::new(&["LATESECRET"]);
        assert_eq!(
            recap(&late, &stored, cut, 9, 21),
            (
                format!("hello {}", marker(6, 21)),
                Some(Cut { kept: 6, total: 21 })
            )
        );
        assert_eq!(
            recap(&late, "xLATESECRETx", None, 100, 12),
            ("x[redacted]x".to_owned(), None)
        );
    }

    #[test]
    fn recap_reads_only_the_recorded_content_of_a_cut_field() {
        let text = "xyzwLATE\u{1D11E}SECRET tail";
        let (stored, cut) = cap_bytes(&Redactor::none(), text.as_bytes(), 10);
        assert_eq!(stored, format!("xyzwLATE{}", marker(8, text.len() as u64)));
        let late = Redactor::new(&["LATE\u{1D11E}SECRET", "1"]);
        assert_eq!(
            recap(&late, &stored, cut, 10, 0).0,
            format!("xyzw{}", marker(4, text.len() as u64))
        );
    }

    #[test]
    fn recap_redacts_an_uncut_marker_shaped_tail_as_text() {
        let fake = "abc\n[truncated: kept 3 of 1234567 bytes]";
        let r = Redactor::new(&["1234567"]);
        assert_eq!(
            recap(&r, fake, None, 1024, fake.len() as u64),
            (fake.replace("1234567", "[redacted]"), None)
        );
    }

    #[test]
    fn recap_cuts_an_uncut_field_a_late_value_grew_past_its_cap() {
        let stored = cap_text(&Redactor::none(), "xbxb", 10);
        assert_eq!(stored, "xbxb");
        let (text, cut) = recap(&Redactor::new(&["b"]), &stored, None, 10, 4);
        assert_eq!(cut, Some(Cut { kept: 10, total: 4 }));
        assert_eq!(text, format!("x[redacted{}", marker(10, 4)));
    }

    #[test]
    fn a_trailing_prefix_in_lowercase_percent_hex_is_trimmed() {
        let r = Redactor::new(&["a/b"]);
        let text = "x a%2fQQQQ";
        assert_eq!(cap_text(&r, text, 6), format!("x {}", marker(2, 10)));
        // A cut inside an escape leaves a lone lowercase digit after the `%`.
        let r = Redactor::new(&["é"]);
        assert_eq!(cap_text(&r, "x %c3QQQ", 4), format!("x {}", marker(2, 8)));
    }

    #[test]
    fn a_value_after_a_percent_is_redacted_before_the_cut() {
        let r = Redactor::new(&["deadbeef"]);
        let text = "q=%deadbeef and some more text";
        assert_eq!(
            cap_text(&r, text, 14),
            format!("q=%[redacted] {}", marker(14, 30))
        );
    }

    #[test]
    fn a_trailing_prefix_after_a_percent_is_trimmed_in_any_case() {
        let r = Redactor::new(&["ab%2fQ"]);
        assert_eq!(
            cap_text(&r, "z%ab%2FQQQQQQQQQQ", 7),
            format!("z%[red{}", marker(6, 17))
        );
        assert_eq!(
            cap_text(&r, "z%AB%2FxQQQQQQQQQ", 7),
            format!("z%{}", marker(2, 17))
        );
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    // Lowercase `b` and `f` mix hex case after a `%`; the placeholder is masked before the
    // case-insensitive leak checks, so its letters cannot pass for a form.
    const ALPHABET: &[char] = &[
        'A', 'B', 'C', 'b', 'f', '0', '1', ' ', '/', '"', 'é', '+', '%', '\\',
    ];

    /// `kept` upper-cased with every placeholder, whole or cut short at the end, as a NUL.
    fn folded_without_placeholders(kept: &str) -> String {
        let mut out = kept.replace("[redacted]", "\0");
        if let Some(at) = out.rfind('[')
            && "[redacted]".starts_with(&out[at..])
        {
            out.replace_range(at.., "\0");
        }
        out.to_ascii_uppercase()
    }

    fn word(rng: &mut Rng, len: usize) -> String {
        (0..len)
            .map(|_| ALPHABET[rng.below(ALPHABET.len())])
            .collect()
    }

    #[test]
    fn randomised_cuts_never_store_a_form_or_end_in_part_of_one() {
        let mut rng = Rng(0x5EED_0FA1);
        for case in 0..3000 {
            let count = 1 + rng.below(3);
            let secrets: Vec<String> = (0..count)
                .map(|_| {
                    let len = 1 + rng.below(8);
                    word(&mut rng, len)
                })
                .collect();
            let r = Redactor::new(&secrets);
            let mut text = String::new();
            for _ in 0..rng.below(12) {
                let filler = rng.below(10);
                text.push_str(&word(&mut rng, filler));
                text.push_str(&secrets[rng.below(secrets.len())]);
            }
            let cap = rng.below(text.len() + 4);
            let (stored, cut) = cap_bytes(&r, text.as_bytes(), cap);
            let kept = if let Some(c) = cut {
                let (kept, k, n) = split_marker(&stored).expect("a cut field ends in the marker");
                assert_eq!((k, n), (kept.len(), text.len() as u64), "case {case}");
                assert_eq!(c, Cut { kept: k, total: n }, "case {case}");
                kept
            } else {
                stored.as_str()
            };
            assert!(kept.len() <= cap, "case {case}: {} > {cap}", kept.len());
            let probe = folded_without_placeholders(kept);
            for f in r.forms() {
                let form = if f.kind == FormKind::Text {
                    f.text.to_ascii_uppercase()
                } else {
                    f.text.clone()
                };
                let probe = if f.kind == FormKind::Text {
                    probe.clone()
                } else {
                    kept.replace("[redacted]", "\0")
                };
                assert!(
                    !probe.contains(&form),
                    "case {case}: {kept:?} holds {form:?}"
                );
                if cut.is_some() {
                    for end in (1..form.len()).filter(|&e| form.is_char_boundary(e)) {
                        assert!(
                            !probe.ends_with(&form[..end]),
                            "case {case}: {kept:?} ends in {form:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_cut_inside_a_base64_fragment_is_trimmed() {
        let r = Redactor::new(&["hunter22"]);
        let (text, cut) = cap_bytes_of(&r, b"xx aHVud", 100, 900);
        assert_eq!(text, format!("xx {}", marker(3, 900)));
        assert_eq!(
            cut,
            Some(Cut {
                kept: 3,
                total: 900
            })
        );
    }

    #[test]
    fn a_cut_inside_a_whole_base64_form_is_trimmed_exactly() {
        let r = Redactor::new(&["hunter22"]);
        let whole = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, "hunter22");
        let (text, _) = cap_bytes_of(&r, format!("x {}", &whole[..5]).as_bytes(), 100, 900);
        assert_eq!(text, format!("x {}", marker(2, 900)));
    }

    #[test]
    fn a_long_run_of_a_forms_first_character_is_trimmed_in_bounded_time() {
        let r = Redactor::new(&[format!("A{}", "BCDEFGHIJK".repeat(10))]);
        let text = "a".repeat(40_000);
        let started = std::time::Instant::now();
        let (stored, cut) = cap_bytes(&r, text.as_bytes(), 39_990);
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
        assert_eq!(cut.map(|c| c.total), Some(40_000));
        assert!(stored.len() < 39_990 + 60);
    }
}
