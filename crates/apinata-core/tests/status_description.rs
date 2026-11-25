//! `status_description` explains common status codes and stays silent on unknown ones.

use apinata_core::status_description;

#[test]
fn common_codes_have_a_description() {
    for code in [
        100, 101, 200, 201, 204, 301, 302, 304, 307, 308, 400, 401, 403, 404, 405, 409, 410, 415,
        418, 422, 429, 500, 501, 502, 503, 504,
    ] {
        let text = status_description(code).unwrap_or_else(|| panic!("{code} has none"));
        assert!(text.ends_with('.'), "{code}: {text}");
    }
    assert_eq!(
        status_description(404),
        Some("The server cannot find the requested resource.")
    );
}

#[test]
fn unknown_codes_have_none() {
    for code in [0, 99, 299, 499, 599, 600, 999] {
        assert_eq!(status_description(code), None, "{code}");
    }
}
