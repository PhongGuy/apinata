//! Percent-encoding and decoding a piece of text, keeping `{{var}}` placeholders whole.

use apinata_core::{percent_decode, percent_encode, widen_to_placeholders};

#[test]
fn encoding_then_decoding_gives_back_the_text() {
    let text = "a b/c?d=é&e=100% {{host}} ✓";
    let encoded = percent_encode(text);
    assert_ne!(encoded, text);
    assert_eq!(percent_decode(&encoded).expect("decodes"), text);
}

#[test]
fn placeholders_are_kept_whole() {
    assert_eq!(
        percent_encode("{{base Url}}/a b{{id}}"),
        "{{base Url}}%2Fa%20b{{id}}"
    );
    assert_eq!(
        percent_decode("{{a%20b}}%2F{{c}}").expect("decodes"),
        "{{a%20b}}/{{c}}"
    );
}

#[test]
fn a_stray_percent_is_left_alone_on_decode() {
    assert_eq!(
        percent_decode("100% sure %zz %2 50%25").expect("decodes"),
        "100% sure %zz %2 50%"
    );
}

#[test]
fn escapes_that_are_not_utf8_are_refused() {
    assert!(percent_decode("%FF").is_err());
    assert!(percent_decode("ok %C3 then").is_err());
    let message = percent_decode("%E2%82").expect_err("refused").to_string();
    assert!(message.contains("UTF-8"), "{message}");
}

#[test]
fn reserved_characters_are_encoded_as_in_a_query_value() {
    assert_eq!(
        percent_encode(":/?#[]@!$&'()*+,;= ~-_.%"),
        "%3A%2F%3F%23%5B%5D%40!%24%26'()*%2B%2C%3B%3D%20~-_.%25"
    );
    assert_eq!(percent_encode("é"), "%C3%A9");
}

#[test]
fn a_nested_placeholder_is_kept_as_the_resolver_finds_it() {
    // The resolver reads `{{a{{b}}` as one placeholder, leaving the last `}}` as text.
    assert_eq!(percent_encode("{{a{{b}}}}"), "{{a{{b}}%7D%7D");
    assert_eq!(
        percent_decode("{{a{{b}}%7D%7D").expect("decodes"),
        "{{a{{b}}}}"
    );
}

#[test]
fn an_unterminated_placeholder_is_plain_text() {
    assert_eq!(percent_encode("{{x"), "%7B%7Bx");
    assert_eq!(percent_encode("{{x }"), "%7B%7Bx%20%7D");
    assert_eq!(percent_decode("{{x%20").expect("decodes"), "{{x ");
}

#[test]
fn a_range_that_cuts_a_placeholder_grows_to_cover_it() {
    let text = "a {{host}}/{{path}} b";
    // The end falls inside `{{host}}`.
    assert_eq!(widen_to_placeholders(text, 0..6), 0..10);
    // The start falls inside `{{path}}`.
    assert_eq!(widen_to_placeholders(text, 14..21), 11..21);
    // Both ends inside, in different placeholders.
    assert_eq!(widen_to_placeholders(text, 5..13), 2..19);
    // A range on placeholder edges, or clear of them, stays as it is.
    assert_eq!(widen_to_placeholders(text, 2..10), 2..10);
    assert_eq!(widen_to_placeholders(text, 0..2), 0..2);
    assert_eq!(widen_to_placeholders(text, 6..6), 6..6);
}
