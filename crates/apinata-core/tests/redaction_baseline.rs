//! Baseline: every wire form the saved-example redactor hides today.

use apinata_core::{derived_secrets, replace_secrets};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

fn hide(text: &str, v: &str) -> String {
    replace_secrets(text, &derived_secrets(&[v.to_owned()]))
}

// Shared helpers: independent of the code under test.
const AWKWARD: &str = "Ab1 \"q\"/x+y";
const UNI: &str = "p&ss/wörd<x>+ 1";
const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');
const PATHSET: &AsciiSet = &UNRESERVED.remove(b'/');

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
fn form_encoded(v: &str) -> String {
    url::form_urlencoded::byte_serialize(v.as_bytes()).collect()
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

/// Every echo of `v` that saved examples and the history scrub hide today.
fn rows(v: &str) -> Vec<(&'static str, String)> {
    let p = pct(v);
    // The FORM set keeps `*` and encodes `~`, with a space as `+`.
    let plus = form_encoded(v);
    let path = utf8_percent_encode(v, PATHSET).to_string();
    let j1 = json(v);
    let mut out = vec![
        ("raw", v.to_owned()),
        ("json", j1.clone()),
        ("json twice", json(&j1)),
        ("lowercase", v.to_ascii_lowercase()),
        ("percent upper", p.clone()),
        ("percent lower", lower_hex(&p)),
        ("percent path", path),
        ("percent plus", plus.clone()),
        ("percent plus lower", lower_hex(&plus)),
        ("unicode upper", unicode_all(v, true)),
        ("unicode lower", unicode_all(v, false)),
        ("base64 std", STANDARD.encode(v)),
        ("base64 nopad", STANDARD_NO_PAD.encode(v)),
        ("base64 url", URL_SAFE.encode(v)),
        ("base64 url nopad", URL_SAFE_NO_PAD.encode(v)),
        ("percent twice", pct(&p)),
    ];
    let f = form_encoded(v);
    out.extend([
        ("form", f.clone()),
        ("form lower", lower_hex(&f)),
        ("ascii literal unicode upper", unicode_non_ascii(v, true)),
        ("ascii literal unicode lower", unicode_non_ascii(v, false)),
        ("percent twice lower", pct(&lower_hex(&p))),
        (
            "percent twice path",
            pct(&utf8_percent_encode(v, PATHSET).to_string()),
        ),
        (
            "percent twice path lower",
            pct(&lower_hex(&utf8_percent_encode(v, PATHSET).to_string())),
        ),
        ("percent twice plus", pct(&plus)),
        ("percent twice plus lower", pct(&lower_hex(&plus))),
        ("percent twice form", pct(&f)),
        ("json of lowercase", json(&v.to_ascii_lowercase())),
        ("json of unicode upper", json(&unicode_all(v, true))),
        ("json of unicode lower", json(&unicode_all(v, false))),
        (
            "json of ascii literal upper",
            json(&unicode_non_ascii(v, true)),
        ),
        (
            "json of ascii literal lower",
            json(&unicode_non_ascii(v, false)),
        ),
    ]);
    out
}

#[test]
fn every_baseline_echo_is_hidden() {
    for v in [AWKWARD, UNI, W] {
        for (name, row) in rows(v) {
            let out = hide(&format!("<{row}>"), v);
            assert_eq!(out, "<[redacted]>", "{name} of {v:?}");
        }
    }
}

#[test]
fn every_baseline_fragment_is_hidden() {
    for v in [AWKWARD, UNI, W] {
        for (run, text) in fragments(v) {
            for shown in [run.clone(), text] {
                let out = hide(&format!("<{shown}>"), v);
                assert!(!out.contains(&run), "{run} left in {out} for {v:?}");
                assert!(out.contains("[redacted]"), "{out}");
            }
        }
    }
}
