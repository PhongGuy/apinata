use apinata_app::scrub::{scrub_snapshot, scrub_text, scrub_url};
use apinata_core::DraftSnapshot;

const S: &str = "Url-S3cret-9";

fn secrets(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).to_owned()).collect()
}

#[test]
fn text_is_scrubbed_longest_first() {
    let s = secrets(&["abcd", "abcdefgh"]);
    assert_eq!(
        scrub_text("x abcdefgh y abcd", &s),
        "x [redacted] y [redacted]"
    );
}

#[test]
fn an_empty_secret_changes_nothing() {
    assert_eq!(scrub_text("keep me", &secrets(&[""])), "keep me");
}

#[test]
fn a_query_value_is_redacted_and_other_pairs_kept() {
    let url = format!("http://h/p?token={S}&x=1");
    assert_eq!(
        scrub_url(&url, &secrets(&[S])),
        "http://h/p?token=[redacted]&x=1"
    );
}

#[test]
fn a_percent_encoded_secret_is_redacted() {
    let s = secrets(&["a+b/c=Zz9"]);
    assert_eq!(
        scrub_url("http://h/p?k=a%2Bb%2Fc%3DZz9", &s),
        "http://h/p?k=[redacted]"
    );
}

#[test]
fn an_encoded_secret_with_a_suffix_is_redacted() {
    let url = format!("http://h/p?oauth_consumer_key=ck&oauth_signature={S}%26");
    assert_eq!(
        scrub_url(&url, &secrets(&[S])),
        "http://h/p?oauth_consumer_key=ck&oauth_signature=[redacted]"
    );
}

#[test]
fn a_userinfo_password_is_always_dropped() {
    assert_eq!(
        scrub_url("http://user:pw@h:8080/p", &[]),
        "http://user@h:8080/p"
    );
    let url = format!("http://user:{S}@h/p");
    assert_eq!(scrub_url(&url, &secrets(&[S])), "http://user@h/p");
}

#[test]
fn a_secret_username_is_redacted() {
    let url = format!("http://{S}@h/p");
    assert_eq!(scrub_url(&url, &secrets(&[S])), "http://[redacted]@h/p");
}

#[test]
fn a_path_segment_is_redacted() {
    let url = format!("http://h/users/{S}/x");
    assert_eq!(
        scrub_url(&url, &secrets(&[S])),
        "http://h/users/[redacted]/x"
    );
}

#[test]
fn a_valueless_query_part_and_a_fragment_are_redacted() {
    let url = format!("http://h/p?{S}#{S}");
    assert_eq!(
        scrub_url(&url, &secrets(&[S])),
        "http://h/p?[redacted]#[redacted]"
    );
}

#[test]
fn a_url_without_secrets_is_unchanged() {
    let url = "https://h.test:8443/a%20b/c?x=1&y=&z#frag";
    assert_eq!(scrub_url(url, &secrets(&[S])), url);
}

#[test]
fn text_that_is_not_a_url_falls_back_to_plain_scrubbing() {
    let text = format!("not a url {S}");
    assert_eq!(scrub_url(&text, &secrets(&[S])), "not a url [redacted]");
}

#[test]
fn a_form_encoded_secret_is_redacted() {
    let s = secrets(&["my api key"]);
    assert_eq!(
        scrub_url("http://h/p?k=my+api+key&x=1", &s),
        "http://h/p?k=[redacted]&x=1"
    );
}

#[test]
fn a_scheme_inside_a_query_is_not_taken_for_the_url_scheme() {
    let s = secrets(&["a b c d"]);
    assert_eq!(
        scrub_url("h/p?next=http://x&k=a%20b%20c%20d", &s),
        "h/p?next=http://x&k=[redacted]"
    );
}

#[test]
fn text_is_scrubbed_in_its_encoded_forms() {
    let s = secrets(&["a b/c-S3cret"]);
    assert_eq!(scrub_text("q=a%20b%2Fc-S3cret&x=1", &s), "q=[redacted]&x=1");
    assert_eq!(scrub_text("q=a+b%2Fc-S3cret", &s), "q=[redacted]");
    let quoted = secrets(&["a\"b\\c-S3cret"]);
    assert_eq!(
        scrub_text(r#"{"t":"a\"b\\c-S3cret"}"#, &quoted),
        r#"{"t":"[redacted]"}"#
    );
}

#[test]
fn a_path_secret_spanning_segments_and_needing_encoding_is_scrubbed() {
    let s = secrets(&["a b/c-S3cret"]);
    assert_eq!(
        scrub_url("http://h/x/a%20b/c-S3cret/y?k=1", &s),
        "http://h/x/[redacted]/[redacted]/y?k=1"
    );
}

#[test]
fn a_spanning_path_secret_covering_part_of_a_segment_redacts_that_segment() {
    let s = secrets(&["a b/c"]);
    assert_eq!(
        scrub_url("http://h/a/xa%20b/c-y/z", &s),
        "http://h/a/[redacted]/[redacted]/z"
    );
}

#[test]
fn a_schemeless_text_keeps_its_path_text() {
    let s = secrets(&["a b/c"]);
    assert_eq!(scrub_url("note xa%20b/c-y", &s), "note x[redacted]-y");
}

fn b64(text: &str) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(text)
}

#[test]
fn a_secret_nested_in_another_leaves_no_remainder_in_any_form() {
    let s = secrets(&["Prod-key-abcdef", "key-abcdef"]);
    let forms = [
        "Prod-key-abcdef",
        "prod-key-abcdef",
        "Prod%2Dkey%2Dabcdef",
        "Prod-key-abcdef\\u0021",
    ];
    for form in forms {
        let out = scrub_text(&format!("a {form} b key-abcdef c"), &s);
        assert!(!out.contains("prod-") && !out.contains("Prod-"), "{out}");
        assert!(!out.contains("key-abcdef"), "{out}");
    }
    let json = serde_json::to_string(&serde_json::json!({"k": "Prod-key-abcdef"})).unwrap();
    assert_eq!(scrub_text(&json, &s), r#"{"k":"[redacted]"}"#);
    let url = scrub_url("http://h/prod-key-abcdef/key-abcdef?k=Prod-key-abcdef", &s);
    assert_eq!(url, "http://h/[redacted]/[redacted]?k=[redacted]");
}

#[test]
fn a_short_mixed_case_secret_in_a_lowercased_host_is_redacted() {
    let s = secrets(&["Ab1Cd"]);
    assert_eq!(
        scrub_url("https://ab1cd.example.test/x", &s),
        "https://[redacted].example.test/x"
    );
    assert_eq!(
        scrub_url("https://u@ab1cd.example.test:8080/x", &s),
        "https://u@[redacted].example.test:8080/x"
    );
    // The whole URL is matched without regard to case, so a lowercase path is redacted too.
    assert_eq!(
        scrub_url("https://h.test/ab1cd", &s),
        "https://h.test/[redacted]"
    );
}

#[test]
fn double_escaped_and_double_percent_forms_are_scrubbed() {
    let s = secrets(&["Pa/ss\"word"]);
    let once = serde_json::to_string("Pa/ss\"word").unwrap();
    let inner = once.trim_matches('"').to_owned();
    let twice = serde_json::to_string(&inner).unwrap();
    let twice = twice.trim_matches('"');
    assert_eq!(scrub_text(&format!("<{twice}>"), &s), "<[redacted]>");
    let unicode = secrets(&["Pa/ss-word"]);
    assert_eq!(
        scrub_text(
            r"<\\u0050\\u0061\\u002F\\u0073\\u0073\\u002D\\u0077\\u006F\\u0072\\u0064>",
            &unicode
        ),
        "<[redacted]>"
    );
    let slash = secrets(&["a/b-secret"]);
    assert_eq!(scrub_text("x=a%252Fb-secret", &slash), "x=[redacted]");
    assert_eq!(
        scrub_url("http://h/a%252Fb-secret/p?q=a%252Fb-secret", &slash),
        "http://h/[redacted]/p?q=[redacted]"
    );
}

#[test]
fn a_four_byte_basic_credential_is_redacted_whole() {
    let s = secrets(&["abcd"]);
    let header = format!("Basic {}", b64("user:abcd"));
    assert_eq!(scrub_text(&header, &s), "Basic [redacted]");
}

#[test]
fn a_scrubbed_base64_fragment_takes_its_boundary_characters() {
    let secret = "hunter2-Passw0rd";
    let s = secrets(&[secret]);
    // Each alignment of the secret inside `<prefix><secret>-tail`: the characters beside the
    // fragment share bits with the secret and go with it, leaving only the prefix and tail.
    let cases = [
        ("", "Basic [redacted]0YWls end"),
        ("u:", "Basic d[redacted]RhaWw= end"),
        ("us:", "Basic dX[redacted]0YWls end"),
        ("use:", "Basic dXNl[redacted]dGFpbA== end"),
    ];
    for (prefix, expected) in cases {
        let token = b64(&format!("{prefix}{secret}-tail"));
        assert_eq!(
            scrub_text(&format!("Basic {token} end"), &s),
            expected,
            "{prefix}"
        );
    }
}

// Baseline helpers: independent of the code under test.
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

fn hide(text: &str, v: &str) -> String {
    scrub_text(text, &[v.to_owned()])
}

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

/// `\uXXXX` for every character, upper hex for even positions and lower for odd ones.
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
    let mixed = alternate_hex(&pct(AWKWARD));
    assert_eq!(hide(&format!("<{mixed}>"), AWKWARD), "<[redacted]>");
}

#[test]
fn a_mixed_case_unicode_escape_is_scrubbed() {
    let v = "éöüß key";
    assert_eq!(hide(&format!("<{}>", mixed_unicode(v)), v), "<[redacted]>");
}

#[test]
fn query_segment_and_url_crate_sets_are_scrubbed() {
    let mut u = url::Url::parse("http://h.test/").unwrap();
    u.set_username(AWKWARD).unwrap();
    for echo in ["Ab1%20%22q%22/x+y", "Ab1%20%22q%22%2Fx+y", u.username()] {
        assert_eq!(
            hide(&format!("<{echo}>"), AWKWARD),
            "<[redacted]>",
            "{echo}"
        );
    }
}

#[test]
fn a_four_character_secret_matches_in_any_case_but_its_base64_is_left() {
    assert_eq!(hide("<ABcD>", "abCd"), "<[redacted]>");
    // A 2-character secret keeps its Basic forms only: raw is replaced, base64 is not.
    assert_eq!(hide("<ab>", "ab"), "<[redacted]>");
    assert_eq!(hide("<YWI=>", "ab"), "<YWI=>");
}

#[test]
fn history_hides_everything_saved_examples_hide() {
    for v in [AWKWARD, UNI] {
        for (name, row) in rows(v) {
            assert_eq!(
                hide(&format!("<{row}>"), v),
                "<[redacted]>",
                "{name} of {v:?}"
            );
            let core = apinata_core::replace_secrets(
                &format!("<{row}>"),
                &apinata_core::derived_secrets(&[v.to_owned()]),
            );
            assert_eq!(core, "<[redacted]>", "{name} of {v:?} in saved examples");
        }
    }
}

#[test]
fn a_secret_inside_the_placeholder_still_terminates() {
    for v in ["red", "acted", "[redacted]", "ACTED"] {
        let out = hide("a [redacted] b", v);
        assert!(out.contains("a ") && out.ends_with(" b"), "{v}: {out}");
        assert_eq!(hide(&format!("x{v}y"), v), "x[redacted]y");
    }
}

#[test]
fn a_short_secret_never_splits_a_placeholder_written_earlier() {
    let s = secrets(&["tok-longsecret-1234", "redact", "acted", "ACTED"]);
    assert_eq!(
        scrub_text("a tok-longsecret-1234 b redact c acted d ACTED e", &s),
        "a [redacted] b [redacted] c [redacted] d [redacted] e"
    );
    // Text left over after a replacement still reads as the placeholder alone.
    assert_eq!(scrub_text("x tok-longsecret-1234", &s), "x [redacted]");
}

#[test]
fn scrub_still_hides_secrets_when_no_stand_in_char_is_free() {
    // Every private-use char is already in the text, so placeholders cannot be parked.
    let all: String = ('\u{E000}'..='\u{F8FF}').collect();
    let s = secrets(&["tok-longsecret-1234", "redact", "acted", "ACTED"]);
    // Delimiters make a surviving secret recognisable even next to a split placeholder.
    let text = format!("{all}|tok-longsecret-1234|redact|acted|ACTED|{all}");
    let out = scrub_text(&text, &s);
    for left in ["tok-longsecret-1234", "|redact|", "|acted|", "|ACTED|"] {
        assert!(!out.contains(left), "{left} survived in {out:?}");
    }
    assert!(out.starts_with(&all) && out.ends_with(&all));
}

// The kept url of a snapshot holding `url`, after scrubbing with `secrets`.
fn snapshot_url(url: &str, secrets: &[String]) -> String {
    let json = serde_json::json!({"http": {"method": "GET", "url": url}}).to_string();
    let mut snap = DraftSnapshot::from_json(&json).unwrap();
    scrub_snapshot(&mut snap, secrets);
    let kept: serde_json::Value = serde_json::from_str(&snap.to_json()).unwrap();
    kept["http"]["url"].as_str().unwrap().to_owned()
}

#[test]
fn a_snapshot_url_password_is_dropped_whatever_its_encoding() {
    // Each typed password beside the decoded form core derives from it, which differs in case.
    for (typed, derived) in [
        ("p%4a%4Bss", "pJKss"),
        ("a%2fb%2F", "a/b/"),
        ("pJ%4bss", "pJKss"),
    ] {
        let url = format!("http://joe:{typed}@127.0.0.1:1/x");
        let kept = snapshot_url(&url, &secrets(&[derived]));
        assert_eq!(kept, "http://joe@127.0.0.1:1/x", "{typed}");
    }
}

#[test]
fn a_snapshot_url_password_is_dropped_with_no_secret_known() {
    for typed in ["p%4a%4Bss", "%2f%2F"] {
        let kept = snapshot_url(&format!("https://joe:{typed}@h/x?a=1"), &[]);
        assert_eq!(kept, "https://joe@h/x?a=1", "{typed}");
    }
}

#[test]
fn a_placeholder_snapshot_url_password_stays() {
    let url = "http://joe:{{pw}}@127.0.0.1:1/x";
    assert_eq!(snapshot_url(url, &secrets(&["Pw-S3cret"])), url);
}

// The parser reads this as a url with a password, so a send would send it: it is dropped.
#[test]
fn the_parser_drops_a_password_even_in_text_that_reads_as_free_text() {
    let kept = apinata_app::scrub::without_password("http://h is down, ask joe:pw@h");
    assert!(!kept.contains("pw"), "{kept:?}");
}

#[test]
fn free_text_the_parser_reads_without_a_password_stays_byte_for_byte() {
    for text in [
        "ask joe:pw at http://h/x",
        "see http://h for details",
        r#"{"next": "http://h/x?a=1", "user": "joe:pw"}"#,
    ] {
        assert_eq!(apinata_app::scrub::without_password(text), text);
    }
}

#[test]
fn whitespace_in_a_url_password_never_keeps_it() {
    for url in ["http://u:my secret@h/x", "http://u:my\tsecret@h/x"] {
        assert_eq!(
            apinata_app::scrub::without_password(url),
            "http://u@h/x",
            "{url:?}"
        );
    }
}

#[test]
fn a_url_after_leading_space_or_control_loses_its_password() {
    for lead in [" ", "\t", "\u{1}"] {
        let url = format!("{lead}http://u:pwsecret@h/x");
        let kept = apinata_app::scrub::without_password(&url);
        assert_eq!(kept, format!("{lead}http://u@h/x"), "{url:?}");
    }
}

#[test]
fn a_url_the_parser_reads_with_odd_slashes_loses_its_password() {
    for url in ["http:/u:pwsecret@h/x", "http:\\\\u:pwsecret@h/x"] {
        let kept = apinata_app::scrub::without_password(url);
        assert!(!kept.contains("pwsecret"), "{url:?} kept {kept:?}");
        assert!(kept.contains("u@h"), "{url:?} kept {kept:?}");
    }
}

#[test]
fn whitespace_the_parser_skips_in_the_user_or_host_keeps_no_password() {
    for url in [
        "http://u:pwsecret@h\n",
        "http://u:pwsecret@h\n/x",
        "http://u\t:pwsecret@h/x",
        "http://u:pwsecret@h\r/x",
    ] {
        let kept = apinata_app::scrub::without_password(url);
        assert!(!kept.contains("pwsecret"), "{url:?} kept {kept:?}");
        assert!(kept.contains("u@h"), "{url:?} kept {kept:?}");
    }
}

#[test]
fn a_space_in_the_username_keeps_no_password() {
    let kept = apinata_app::scrub::without_password("http://my user:pwsecret@h");
    assert!(!kept.contains("pwsecret"), "{kept:?}");
    assert!(kept.contains("user@h"), "{kept:?}");
}

#[test]
fn an_empty_user_with_a_url_password_drops_the_whole_userinfo() {
    assert_eq!(
        apinata_app::scrub::without_password("http://:pwsecret@h/x"),
        "http://h/x"
    );
    assert_eq!(snapshot_url("http://:pwsecret@h/x", &[]), "http://h/x");
}

#[test]
fn an_encoded_colon_in_the_userinfo_drops_what_follows_it() {
    for url in ["http://u%3Apwsecret@h/x", "http://u%3apwsecret:more@h/x"] {
        assert_eq!(apinata_app::scrub::without_password(url), "http://u@h/x");
        assert_eq!(snapshot_url(url, &[]), "http://u@h/x");
    }
}

#[test]
fn a_schemeless_snapshot_url_is_read_as_the_send_reads_it() {
    assert_eq!(snapshot_url("u%3Apwsecret@h/x", &[]), "u@h/x");
    assert_eq!(snapshot_url("u:pwsecret@h/x", &[]), "u@h/x");
}

#[test]
fn a_url_password_after_a_backslash_or_a_placeholder_is_hidden() {
    for text in [
        "http://h\\u:pwsecret@x",
        "postgres://u:{{p}}@h\\x:pwsecret@k",
    ] {
        let kept = apinata_app::scrub::without_password(text);
        assert!(!kept.contains("pwsecret"), "{text:?} kept {kept:?}");
    }
    assert!(!snapshot_url("http://h\\u:pwsecret@x", &[]).contains("pwsecret"));
}
