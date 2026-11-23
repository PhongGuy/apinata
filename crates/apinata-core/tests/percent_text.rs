//! Percent-encoding and decoding a piece of text, keeping `{{var}}` placeholders whole.

use apinata_core::{percent_decode, percent_encode};

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
