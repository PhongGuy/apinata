//! `replace_secrets` second pass: each encoded form of a secret becomes `[redacted]`.

use crate::common;

use apinata_core::{
    Collection, ContextOptions, MemoryStore, NamedSecret, SecretStore, VarContext, encoded_forms,
    named_secrets, replace_secrets,
};
use common::{request, write_tree};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use std::sync::Arc;

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

/// The named secrets of a request whose body uses each `(name, value)` secret of `dev`.
fn secrets(vars: &[(&str, &str)]) -> Vec<NamedSecret> {
    let mut dev = "name: dev\nvariables:\n".to_owned();
    for (name, _) in vars {
        dev.push_str(&format!("  - name: {name}\n    secret: true\n"));
    }
    let body: Vec<String> = vars
        .iter()
        .map(|(name, _)| format!("{{{{{name}}}}}"))
        .collect();
    let req = format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: POST\n  url: \"https://x.test/\"\n  body:\n    type: text\n    data: \"{}\"\n",
        body.join(" ")
    );
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", &dev),
        ("r.yml", &req),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let store = MemoryStore::new();
    for (name, value) in vars {
        store.set(&c.secret_key("dev", name), value).unwrap();
    }
    let options = ContextOptions::new().env("dev").secrets(Arc::new(store));
    let ctx = VarContext::new(&c, options).expect("context");
    named_secrets(request(&c, "r.yml"), &ctx).expect("resolves")
}

/// Like [`secrets`], plus `bearer` as the request's bearer token: an unnamed secret.
fn secrets_with_bearer(vars: &[(&str, &str)], bearer: &str) -> Vec<NamedSecret> {
    let mut dev = "name: dev\nvariables:\n".to_owned();
    for (name, _) in vars {
        dev.push_str(&format!("  - name: {name}\n    secret: true\n"));
    }
    let body: Vec<String> = vars
        .iter()
        .map(|(name, _)| format!("{{{{{name}}}}}"))
        .collect();
    let req = format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: POST\n  url: \"https://x.test/\"\n  body:\n    type: text\n    data: \"{}\"\n  auth:\n    type: bearer\n    token: {bearer}\n",
        body.join(" ")
    );
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", &dev),
        ("r.yml", &req),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let store = MemoryStore::new();
    for (name, value) in vars {
        store.set(&c.secret_key("dev", name), value).unwrap();
    }
    let options = ContextOptions::new().env("dev").secrets(Arc::new(store));
    let ctx = VarContext::new(&c, options).expect("context");
    named_secrets(request(&c, "r.yml"), &ctx).expect("resolves")
}

const KEY: &str = "Key/a b~*é";

/// Every encoded form of `KEY`, written out by hand.
const KEY_FORMS: [&str; 27] = [
    r"\\u004B\\u0065\\u0079\\u002F\\u0061\\u0020\\u0062\\u007E\\u002A\\u00E9",
    r"\\u004b\\u0065\\u0079\\u002f\\u0061\\u0020\\u0062\\u007e\\u002a\\u00e9",
    r"Key/a b~*\\u00E9",
    r"Key/a b~*\\u00e9",
    "Key%252Fa%2520b~%252A%25C3%25A9",
    "Key%252fa%2520b~%252a%25c3%25a9",
    "Key%252Fa%2Bb%257E%2A%25C3%25A9",
    "Key%252fa%2Bb%257e%2A%25c3%25a9",
    "Key%2Fa%2520b~%252A%25C3%25A9",
    "Key%2Fa%2520b~%252a%25c3%25a9",
    r"\u004B\u0065\u0079\u002F\u0061\u0020\u0062\u007E\u002A\u00E9",
    r"\u004b\u0065\u0079\u002f\u0061\u0020\u0062\u007e\u002a\u00e9",
    "Key%2Fa%20b~%2A%C3%A9",
    "Key%2fa%20b~%2a%c3%a9",
    "Key%2Fa+b%7E*%C3%A9",
    "Key%2fa+b%7e*%c3%a9",
    "Key/a%20b~%2A%C3%A9",
    "Key/a%20b~%2a%c3%a9",
    "S2V5L2EgYn4qw6k=",
    r"Key/a b~*\u00E9",
    r"Key/a b~*\u00e9",
    "S2V5L2EgYn4qw6k",
    "LZXkvYSBifirDq",
    "S2V5L2EgYn4qw6",
    "tleS9hIGJ+KsOp",
    "tleS9hIGJ-KsOp",
    "key/a b~*é",
];

#[test]
fn each_encoded_form_becomes_redacted_not_a_placeholder() {
    let s = secrets(&[("key", KEY)]);
    for form in KEY_FORMS {
        assert_eq!(
            replace_secrets(&format!("<{form}>"), &s),
            "<[redacted]>",
            "{form}"
        );
    }
    assert_eq!(replace_secrets(&format!("raw {KEY}"), &s), "raw {{key}}");
}

#[test]
fn a_secret_inside_a_user_pass_encoding_is_redacted_at_each_offset() {
    let s = secrets(&[("key", KEY)]);
    // Behind `u:`, `us:`, `use:` and `user:`: offsets 2, 0, 1 and 2.
    assert_eq!(replace_secrets("dTpLZXkvYSBifirDqQ==", &s), "d[redacted]");
    assert_eq!(
        replace_secrets("dXNlOktleS9hIGJ+KsOp", &s),
        "dXNl[redacted]"
    );
    assert_eq!(
        replace_secrets("dXM6S2V5L2EgYn4qw6k=", &s),
        "dXM6[redacted]"
    );
    assert_eq!(
        replace_secrets("dXNlcjpLZXkvYSBifirDqQ==", &s),
        "dXNlc[redacted]"
    );
}

#[test]
fn encoded_forms_go_longest_first_across_secrets() {
    let s = secrets(&[("a", "Xalpha-secret"), ("b", "ALPHA-SEC")]);
    // `b`'s lowercase sits inside `a`'s; `a`'s goes first, so nothing is split.
    assert_eq!(replace_secrets("xalpha-secret", &s), "[redacted]");
    assert_eq!(
        replace_secrets("ALPHA-SEC and alpha-sec", &s),
        "{{b}} and [redacted]"
    );
}

#[test]
fn a_named_secret_nested_in_an_unnamed_one_does_not_split_it() {
    let s = secrets_with_bearer(&[("short", "key-abcdef")], "Prod/key-abcdef");
    // Raw in a JSON string, lowercase, and percent-encoded in both hex cases.
    let text =
        r#"{"a":"Prod/key-abcdef","b":"x"} prod/key-abcdef Prod%2Fkey-abcdef Prod%2fkey-abcdef"#;
    let out = replace_secrets(text, &s);
    assert!(!out.to_lowercase().contains("prod"), "{out}");
    assert!(!out.contains("key-abcdef"), "{out}");
    assert_eq!(
        replace_secrets(r#"{"k":"Prod/key-abcdef"} key-abcdef"#, &s),
        r#"{"k":"[redacted]"} {{short}}"#
    );
}

#[test]
fn a_json_body_echoed_inside_a_json_string_is_redacted_with_doubled_backslashes() {
    let s = secrets(&[("key", KEY)]);
    let echoed =
        r#"{\"body\":\"\\u004B\\u0065\\u0079\\u002F\\u0061\\u0020\\u0062\\u007E\\u002A\\u00E9\"}"#;
    let out = replace_secrets(echoed, &s);
    assert_eq!(out, r#"{\"body\":\"[redacted]\"}"#);
    assert!(serde_json::from_str::<serde_json::Value>(&format!("\"{out}\"")).is_ok());
}

#[test]
fn a_percent_encoded_secret_encoded_again_is_redacted() {
    let s = secrets(&[("key", "a/b-secret")]);
    for form in ["a%252Fb-secret", "a%252fb-secret"] {
        assert_eq!(
            replace_secrets(&format!("/x?q={form}&z=1"), &s),
            "/x?q=[redacted]&z=1",
            "{form}"
        );
    }
}

#[test]
fn a_base64_fragment_takes_its_boundary_characters_with_it() {
    let s = secrets(&[("pw", "hunter22")]);
    // "user:hunter22": the fragment leaves `p` before and `g==` after.
    assert_eq!(
        replace_secrets("Basic dXNlcjpodW50ZXIyMg==", &s),
        "Basic dXNlc[redacted]"
    );
    assert_eq!(
        replace_secrets("\"odW50ZXIyM\" and q.odW50ZXIyMzz-", &s),
        "\"[redacted]\" and q.[redacted]-"
    );
    // Plain forms never swallow their neighbours.
    let s = secrets(&[("pw", "HUNTER22x")]);
    assert_eq!(replace_secrets("abchunter22xdef", &s), "abc[redacted]def");
}

#[test]
fn a_four_byte_secret_is_found_in_a_base64_run() {
    let s = secrets(&[("pw", "pass")]);
    for (text, want) in [
        ("Basic dXNlcjpwYXNz", "Basic [redacted]"),
        ("{\"a\":\"dXNlcjpwYXNzMQ==\"}", "{\"a\":\"[redacted]\"}"),
        ("dXNlcjpwYXNzXX!", "[redacted]!"),
        ("dXNlcjpwYXNz.", "[redacted]."),
    ] {
        assert_eq!(replace_secrets(text, &s), want);
    }
}

#[test]
fn ordinary_base64_looking_text_is_left_alone() {
    let s = secrets(&[("pw", "pass")]);
    let text = "internationalization documentation 550e8400e29b41d4 a1b2c3d4e5f6 /api/v1/customers";
    assert_eq!(replace_secrets(text, &s), text);
}

#[test]
fn a_raw_secret_with_quote_and_backslash_is_redacted_when_json_echoed_inside_json() {
    let s = secrets(&[("q", r#"Qu"ote\Dd"#)]);
    // Once escaped, twice escaped, and three times for a body echoed through two layers.
    let once = r#"Qu\"ote\\Dd"#;
    let twice = r#"Qu\\\"ote\\\\Dd"#;
    assert_eq!(replace_secrets(once, &s), "{{q}}");
    assert_eq!(replace_secrets(twice, &s), "{{q}}");
    assert_eq!(replace_secrets(r#"{"e":"x"}"#, &s), r#"{"e":"x"}"#);
}

#[test]
fn a_four_byte_secret_is_found_after_a_boundary_inside_a_base64_looking_run() {
    let s = secrets(&[("pw", "pass")]);
    for (text, want) in [
        ("/x?token=dXNlcjpwYXNz", "/x?token=[redacted]"),
        ("/api/dXNlcjpwYXNz", "/api/[redacted]"),
        ("a-dXNlcjpwYXNz", "a-[redacted]"),
        ("k_dXNlcjpwYXNz&z=1", "k_[redacted]&z=1"),
    ] {
        assert_eq!(replace_secrets(text, &s), want);
    }
}

const AWK: &str = "Ab1 \"q\"/x+y";

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

#[test]
fn a_mixed_case_percent_echo_is_redacted() {
    let s = secrets(&[("token", AWK)]);
    let enc = utf8_percent_encode(AWK, NON_ALPHANUMERIC).to_string();
    let mixed = alternate_hex(&enc);
    assert_ne!(mixed, enc);
    assert_ne!(mixed, enc.to_ascii_lowercase());
    assert_eq!(replace_secrets(&format!("<{mixed}>"), &s), "<[redacted]>");
}

#[test]
fn query_and_path_segment_sets_are_redacted() {
    let s = secrets(&[("token", AWK)]);
    for echo in ["Ab1%20%22q%22/x+y", "Ab1%20%22q%22%2Fx+y"] {
        assert_eq!(
            replace_secrets(&format!("<{echo}>"), &s),
            "<[redacted]>",
            "{echo}"
        );
    }
}

#[test]
fn the_raw_value_keeps_its_placeholder_and_other_cases_are_redacted() {
    let s = secrets(&[("token", "lowercase-secret-9")]);
    assert_eq!(
        replace_secrets("a lowercase-secret-9 b", &s),
        "a {{token}} b"
    );
    assert_eq!(
        replace_secrets("a LOWERCASE-SECRET-9 b", &s),
        "a [redacted] b"
    );
    assert_eq!(
        replace_secrets("a Lowercase-Secret-9 b", &s),
        "a [redacted] b"
    );
}

#[test]
fn encoded_forms_keeps_its_contract_over_the_generator() {
    let forms = encoded_forms(AWK);
    assert!(forms.iter().all(|f| f.chars().count() >= 6 && f != AWK));
    assert!(forms.windows(2).all(|w| w[0].len() >= w[1].len()));
    assert!(forms.contains(&"Ab1%20%22q%22/x+y".to_owned()), "query set");
    assert!(encoded_forms("abc").is_empty());
}

#[test]
fn a_written_placeholder_is_not_redacted_again() {
    let s = secrets(&[("token", "token")]);
    assert_eq!(
        replace_secrets("a token b TOKEN c", &s),
        "a {{token}} b [redacted] c"
    );
    let s = secrets(&[("mypassword", "pass")]);
    assert_eq!(
        replace_secrets("x pass y Pass z", &s),
        "x {{mypassword}} y [redacted] z"
    );
}

/// `pass` inside the encoding of `user:pass`.
const ENCODED: &str = "dXNlcjpwYXNz";

#[test]
fn a_secret_at_an_unaligned_offset_in_a_run_is_redacted() {
    for lead in ["z", "zz", "zzz"] {
        let text = format!("{lead}{ENCODED}");
        let out = apinata_core::redact_base64_runs(&text, &["pass"], "[redacted]");
        assert!(out.ends_with("[redacted]"), "{lead}: {out}");
        assert!(!out.contains("YXNz"), "{lead}: {out}");
    }
}

#[test]
fn a_start_past_sixty_four_separators_is_tried() {
    // Every start before the encoding is misaligned with it, so only an offset after the
    // 64th separator finds it.
    let text = format!("{}-z{ENCODED}", "-a".repeat(100));
    let out = apinata_core::redact_base64_runs(&text, &["pass"], "[redacted]");
    assert!(out.ends_with("[redacted]"), "{out}");
    assert!(!out.contains("YXNz"), "{out}");
}

/// Redaction of `text` for the value `pass`, with how long it took.
fn timed(text: &str) -> (String, std::time::Duration) {
    let started = std::time::Instant::now();
    let out = apinata_core::redact_base64_runs(text, &["pass"], "[redacted]");
    (out, started.elapsed())
}

fn slashes(len: usize) -> String {
    (0..len)
        .map(|i| if i % 2 == 0 { '/' } else { 'a' })
        .collect()
}

/// Pseudo-random bytes as standard base64, the shape of an embedded image.
fn image(bytes: usize) -> String {
    use base64::Engine as _;
    let raw: Vec<u8> = (0..bytes as u64)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 7) as u8)
        .collect();
    base64::engine::general_purpose::STANDARD.encode(raw)
}

#[test]
fn a_long_benign_run_is_untouched_and_the_cost_grows_linearly() {
    let (small, big) = (slashes(50_000), slashes(400_000));
    let (out, t_small) = timed(&small);
    assert_eq!(out, small);
    let (out, t_big) = timed(&big);
    assert_eq!(out, big);
    // Eight times the text: linear work is about 8x, quadratic about 64x.
    let floor = std::time::Duration::from_millis(5);
    assert!(
        t_big < t_small.max(floor) * 24,
        "{t_small:?} then {t_big:?}"
    );
}

#[test]
fn an_embedded_image_is_left_alone_by_a_secret_it_does_not_hold() {
    let body = format!("{{\"img\":\"data:image/png;base64,{}\"}}", image(100_000));
    let out = apinata_core::redact_base64_runs(&body, &["s3cr3tvl"], "[redacted]");
    assert_eq!(out, body);
}

#[test]
fn a_word_after_a_long_benign_run_is_untouched() {
    let text = format!("{} description", slashes(112_000));
    let (out, _) = timed(&text);
    assert!(out.ends_with(" description"), "{}", &out[out.len() - 20..]);
}

#[test]
fn a_secret_at_a_late_offset_of_a_long_run_is_redacted_from_its_group() {
    for lead in ["", "z", "zz", "zzz"] {
        let text = format!("{}{lead}{ENCODED}", slashes(112_000));
        let (out, _) = timed(&text);
        assert!(out.ends_with("[redacted]"), "{lead}");
        assert!(!out.contains("YXNz"), "{lead}");
        assert!(out.len() > 100_000, "{lead}: the benign head stays");
    }
}

/// `blob` broken into lines of 76 characters joined by `eol`, the MIME layout.
fn wrapped(blob: &str, eol: &str) -> String {
    blob.as_bytes()
        .chunks(76)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect::<Vec<_>>()
        .join(eol)
}

/// A blob whose encoded `pass` crosses the first 76 column line break.
fn blob_with_secret_across_break() -> String {
    format!("{}{ENCODED}{}", "A".repeat(70), "B".repeat(100))
}

#[test]
fn a_secret_across_a_line_break_of_a_wrapped_blob_is_redacted() {
    for eol in ["\n", "\r\n"] {
        let text = wrapped(&blob_with_secret_across_break(), eol);
        assert!(text.contains(eol), "the blob must wrap");
        let out = apinata_core::redact_base64_runs(&text, &["pass"], "[redacted]");
        assert!(out.contains("[redacted]"), "{eol:?}: {out}");
        assert!(!out.contains("pwYXNz"), "{eol:?}: {out}");
        assert!(!out.contains("dXNlcj"), "{eol:?}: {out}");
    }
}

#[test]
fn a_benign_wrapped_blob_is_byte_identical() {
    for eol in ["\n", "\r\n"] {
        let text = format!("head\n{}\ntail", wrapped(&image(5_000), eol));
        let out = apinata_core::redact_base64_runs(&text, &["pass"], "[redacted]");
        assert_eq!(out, text, "{eol:?}");
    }
}

#[test]
fn text_after_a_redacted_wrapped_blob_is_kept() {
    let text = format!(
        "{}\n\ntail words",
        wrapped(&blob_with_secret_across_break(), "\n")
    );
    let out = apinata_core::redact_base64_runs(&text, &["pass"], "[redacted]");
    assert!(out.ends_with("[redacted]\n\ntail words"), "{out}");
}

#[test]
fn a_wrapped_blob_costs_linear_time() {
    let time = |len: usize| {
        let text = wrapped(&slashes(len), "\n");
        let started = std::time::Instant::now();
        let out = apinata_core::redact_base64_runs(&text, &["pass"], "[redacted]");
        assert_eq!(out, text);
        started.elapsed()
    };
    let (small, big) = (time(50_000), time(400_000));
    let floor = std::time::Duration::from_millis(5);
    assert!(big < small.max(floor) * 24, "{small:?} then {big:?}");
}

#[test]
fn a_header_line_after_an_encoded_value_is_kept() {
    for (tail, eol) in [
        ("Host: example.com", "\r\n"),
        ("next line here", "\n"),
        ("Truncated.", "\n"),
        ("word", "\n"),
    ] {
        let text = format!("Authorization: Basic {ENCODED}{eol}{tail}");
        let out = apinata_core::redact_base64_runs(&text, &["pass"], "[redacted]");
        assert_eq!(
            out,
            format!("Authorization: Basic [redacted]{eol}{tail}"),
            "{tail}"
        );
    }
}
