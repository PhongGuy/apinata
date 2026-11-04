//! Display redaction of userinfo-like text nested in a url's path, query or fragment.

use apinata_core::{mask_response_header, redact_url_userinfo};

#[test]
fn a_nested_userinfo_after_a_double_slash_or_backslash_is_dropped() {
    for (value, shown) in [
        ("http://u:pw@h//v:pw@k", "http://h//k"),
        ("http://h\\u:pw@x", "http://h\\x"),
        ("http://h/p?q=//a\\x:pw@b", "http://h/p?q=//a\\b"),
        ("http://h/p#//x:pw@b/c", "http://h/p#//b/c"),
    ] {
        assert_eq!(redact_url_userinfo(value), shown, "{value}");
        assert_eq!(mask_response_header("Location", value), shown, "{value}");
    }
}

#[test]
fn a_scheme_relative_header_loses_every_nested_userinfo() {
    assert_eq!(
        mask_response_header("Location", "//u:pw@h//v:pw@k"),
        "//h//k"
    );
    assert_eq!(
        mask_response_header("Refresh", "0; url=//u:pw@h//v:pw@k"),
        "0; url=//h//k"
    );
}

#[test]
fn a_colonless_nested_at_sign_and_the_real_host_stay() {
    for value in [
        "http://h//a@b",
        "http://h/p?next=//a@b",
        "http://127.0.0.1:8080/r/joe:pw@127.0.0.1:8080/next",
        "http://127.0.0.1:8080/a//b@c",
        "http://h/a:b@c",
    ] {
        assert_eq!(redact_url_userinfo(value), value, "{value}");
        assert_eq!(mask_response_header("Location", value), value, "{value}");
    }
}
