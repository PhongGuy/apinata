//! `encoded_forms`: the transformed wire forms of a secret value.

use apinata_core::encoded_forms;
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};

/// Whether any form of `value` occurs in `text`.
fn found(value: &str, text: &str) -> bool {
    encoded_forms(value)
        .iter()
        .any(|f| text.contains(f.as_str()))
}

fn has(forms: &[String], form: &str) -> bool {
    forms.iter().any(|f| f == form)
}

#[test]
fn a_value_under_four_chars_has_no_forms() {
    for value in ["", "abc", "é😀x"] {
        assert!(encoded_forms(value).is_empty(), "{value}");
    }
}

#[test]
fn the_whole_value_is_base64_in_both_alphabets_padded_and_not() {
    let forms = encoded_forms("k>?~~?x");
    for form in ["az4/fn4/eA==", "az4_fn4_eA==", "az4/fn4/eA", "az4_fn4_eA"] {
        assert!(has(&forms, form), "{form} missing from {forms:?}");
    }
}

#[test]
fn each_alignment_keeps_only_the_characters_the_value_decides() {
    // `hunter22` behind 0, 1 and 2 filler bytes.
    let forms = encoded_forms("hunter22");
    for fragment in ["aHVudGVyMj", "h1bnRlcjIy", "odW50ZXIyM"] {
        assert!(has(&forms, fragment), "{fragment} missing from {forms:?}");
    }
    // A 5-byte value keeps exactly 6 characters behind 1 and 2 filler bytes, in both alphabets.
    let five = encoded_forms("s3c>?");
    for fragment in ["MzYz4/", "MzYz4_", "zM2M+P", "zM2M-P"] {
        assert!(has(&five, fragment), "{fragment} missing from {five:?}");
    }
}

#[test]
fn a_value_of_five_bytes_or_more_is_found_inside_a_longer_encoding_at_every_offset() {
    let prefixes = ["", "u", ":", "u:", "us:", "use:", "user:", "users:"];
    let suffixes = ["", "\"}", ".sig"];
    for value in ["s3c>?", "hunter22", "Pé😀ss"] {
        for prefix in prefixes {
            for suffix in suffixes {
                let plain = format!("{prefix}{value}{suffix}");
                for engine in [STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD] {
                    let encoded = engine.encode(&plain);
                    assert!(
                        found(value, &encoded),
                        "{value:?} not found in {encoded} ({plain:?})"
                    );
                }
            }
        }
    }
}

#[test]
fn a_four_byte_value_is_found_only_as_a_whole_encoding() {
    let forms = encoded_forms("ab1Z");
    assert!(has(&forms, "YWIxWg=="), "{forms:?}");
    assert!(has(&forms, "YWIxWg"), "{forms:?}");
    // Ending a 3-byte-aligned encoding, the whole value's encoding still shows.
    assert!(found("ab1Z", &STANDARD.encode("useab1Z")));
    for plain in [
        "uab1Z",
        "u:ab1Z",
        "userab1Z",
        "user:ab1Z",
        "uab1Zxyz",
        "u:ab1Zxyz",
        "userab1Zxyz",
        "user:ab1Zxyz",
        "useab1Zxyz",
    ] {
        for engine in [STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD] {
            let encoded = engine.encode(plain);
            assert!(!found("ab1Z", &encoded), "{plain} matched in {encoded}");
        }
    }
}

#[test]
fn every_form_is_six_chars_or_more_unique_longest_first_and_not_the_raw_text() {
    for value in [
        "ab1Z",
        "AbCdE",
        "hunter22",
        "Pé😀ss",
        "qu\"ote\\d",
        "Key/a b~*é",
    ] {
        let forms = encoded_forms(value);
        assert!(!forms.is_empty(), "{value:?}");
        let escaped = serde_json::to_string(value).unwrap();
        let escaped = &escaped[1..escaped.len() - 1];
        for (i, f) in forms.iter().enumerate() {
            assert!(f.chars().count() >= 6, "{f:?} of {value:?}");
            assert!(
                f.as_str() != value && f.as_str() != escaped,
                "{f:?} repeats {value:?}"
            );
            assert!(!forms[i + 1..].contains(f), "{f:?} twice for {value:?}");
            if let Some(next) = forms.get(i + 1) {
                assert!(f.len() >= next.len(), "not longest first: {forms:?}");
            }
        }
    }
}

#[test]
fn an_uppercase_value_adds_its_ascii_lowercase() {
    assert!(has(&encoded_forms("AbCdEf-9"), "abcdef-9"));
    // A 4- or 5-char value's lowercase falls under the floor: a short host secret is a known limit.
    for value in ["AbCd", "AbCdE"] {
        let lower = value.to_ascii_lowercase();
        assert!(!has(&encoded_forms(value), &lower), "{value}");
    }
}

#[test]
fn percent_encoding_comes_in_three_sets_and_both_hex_cases() {
    let forms = encoded_forms("Key/a b~*é");
    for form in [
        "Key%2Fa%20b~%2A%C3%A9",
        "Key%2fa%20b~%2a%c3%a9",
        "Key/a%20b~%2A%C3%A9",
        "Key/a%20b~%2a%c3%a9",
        "Key%2Fa+b%7E*%C3%A9",
        "Key%2fa+b%7e*%c3%a9",
    ] {
        assert!(has(&forms, form), "{form} missing from {forms:?}");
    }
}

#[test]
fn unicode_escapes_cover_every_char_and_only_non_ascii_in_both_cases() {
    let forms = encoded_forms("Pé😀ss");
    for form in [
        r"\u0050\u00e9\ud83d\ude00\u0073\u0073",
        r"\u0050\u00E9\uD83D\uDE00\u0073\u0073",
        r"P\u00e9\ud83d\ude00ss",
        r"P\u00E9\uD83D\uDE00ss",
    ] {
        assert!(has(&forms, form), "{form} missing from {forms:?}");
    }
}

#[test]
fn the_full_form_lists_are_pinned() {
    assert_eq!(
        encoded_forms("hunter22"),
        [
            r"\\u0068\\u0075\\u006E\\u0074\\u0065\\u0072\\u0032\\u0032",
            r"\\u0068\\u0075\\u006e\\u0074\\u0065\\u0072\\u0032\\u0032",
            r"\u0068\u0075\u006E\u0074\u0065\u0072\u0032\u0032",
            r"\u0068\u0075\u006e\u0074\u0065\u0072\u0032\u0032",
            "aHVudGVyMjI=",
            "aHVudGVyMjI",
            "aHVudGVyMj",
            "h1bnRlcjIy",
            "odW50ZXIyM",
        ]
    );
    assert_eq!(
        encoded_forms("Pé😀ss"),
        [
            r"\\u0050\\u00E9\\uD83D\\uDE00\\u0073\\u0073",
            r"\\u0050\\u00e9\\ud83d\\ude00\\u0073\\u0073",
            r"\u0050\u00E9\uD83D\uDE00\u0073\u0073",
            r"\u0050\u00e9\ud83d\ude00\u0073\u0073",
            "P%25C3%25A9%25F0%259F%2598%2580ss",
            "P%25c3%25a9%25f0%259f%2598%2580ss",
            r"P\\u00E9\\uD83D\\uDE00ss",
            r"P\\u00e9\\ud83d\\ude00ss",
            "P%C3%A9%F0%9F%98%80ss",
            "P%c3%a9%f0%9f%98%80ss",
            r"P\u00E9\uD83D\uDE00ss",
            r"P\u00e9\ud83d\ude00ss",
            "xn--pss-bma90525d",
            "UMOp8J+YgHNz",
            "UMOp8J-YgHNz",
            "DDqfCfmIBzc",
            "Qw6nwn5iAc3",
        ]
    );
}

#[test]
fn json_escaped_and_double_percent_forms_are_included() {
    let forms = encoded_forms("Key/a b~*é");
    for form in [
        r"\\u004B\\u0065\\u0079\\u002F\\u0061\\u0020\\u0062\\u007E\\u002A\\u00E9",
        r"Key/a b~*\\u00e9",
        "Key%252Fa%2520b~%252A%25C3%25A9",
        "Key%252fa%2520b~%252a%25c3%25a9",
    ] {
        assert!(has(&forms, form), "{form} missing from {forms:?}");
    }
    let quoted = encoded_forms("Qu\"ote\\Dd");
    assert!(has(&quoted, r#"qu\"ote\\dd"#), "{quoted:?}");
    assert!(has(&encoded_forms("a/b-secret"), "a%252Fb-secret"));
}

#[test]
fn the_raw_value_json_escaped_twice_is_a_form() {
    let forms = encoded_forms("Qu\"ote\\Dd");
    assert!(has(&forms, r#"Qu\\\"ote\\\\Dd"#), "{forms:?}");
    // The single escape is the app's own list entry, not a form.
    assert!(!has(&forms, r#"Qu\"ote\\Dd"#), "{forms:?}");
}
