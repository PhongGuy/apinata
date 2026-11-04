//! `response_view`: how a response prints for a person or a pipe.

use apinata_core::{Response, response_view};

fn sample() -> Response {
    Response::new(404)
        .with_status_text("Not Found")
        .with_header("Content-Type", "application/json")
        .with_header("X-Token", "abc")
}

#[test]
fn status_line_names_version_status_and_reason() {
    assert_eq!(
        response_view::status_line(&sample()),
        "HTTP/1.1 404 Not Found"
    );
}

#[test]
fn status_line_without_a_reason_has_no_trailing_space() {
    assert_eq!(
        response_view::status_line(&Response::new(204)),
        "HTTP/1.1 204"
    );
}

#[test]
fn head_lists_every_header_through_the_redactor() {
    let head = response_view::head(&sample(), &|t| t.replace("abc", "[redacted]"));
    assert_eq!(
        head,
        "HTTP/1.1 404 Not Found\ncontent-type: application/json\nx-token: [redacted]\n"
    );
}

#[test]
fn pretty_json_keeps_key_order() {
    let out = response_view::pretty_json(br#"{"b":1,"a":[true,null]}"#).expect("json");
    assert_eq!(
        out,
        "{\n  \"b\": 1,\n  \"a\": [\n    true,\n    null\n  ]\n}"
    );
}

#[test]
fn pretty_json_declines_text_that_is_not_json() {
    assert!(response_view::pretty_json(b"hello").is_none());
    assert!(response_view::pretty_json(&[0xff, 0xfe]).is_none());
}

#[test]
fn only_invalid_utf8_counts_as_binary() {
    assert!(response_view::is_binary(&[0x89, 0x50, 0xff]));
    assert!(!response_view::is_binary("caf\u{e9}".as_bytes()));
    assert!(!response_view::is_binary(b""));
}

#[test]
fn binary_placeholder_names_the_size_and_the_escape_hatch() {
    assert_eq!(
        response_view::binary_placeholder(7),
        "[binary body, 7 bytes; use -o FILE]"
    );
}

#[test]
fn pretty_json_keeps_number_text_and_key_order() {
    let src = br#"{"id":12345678901234567890123,"a":1.50,"b":1e3,"z":-0,"c":1.0}"#;
    let out = response_view::pretty_json(src).expect("json");
    assert_eq!(
        out,
        "{\n  \"id\": 12345678901234567890123,\n  \"a\": 1.50,\n  \"b\": 1e3,\n  \"z\": -0,\n  \"c\": 1.0\n}"
    );
}

#[test]
fn pretty_json_keeps_duplicate_keys_and_structural_text_in_strings() {
    let out = response_view::pretty_json(br#"{"k":"a,{[\"","k":2}"#).expect("json");
    assert_eq!(out, "{\n  \"k\": \"a,{[\\\"\",\n  \"k\": 2\n}");
}

#[test]
fn pretty_json_declines_invalid_json() {
    assert!(response_view::pretty_json(b"{\"a\":1,}").is_none());
}

#[test]
fn pretty_json_scalars_and_empty_containers() {
    let pj = |b: &[u8]| response_view::pretty_json(b);
    assert_eq!(pj(b"42").as_deref(), Some("42"));
    assert_eq!(pj(b"\"x\"").as_deref(), Some("\"x\""));
    assert_eq!(pj(b"null").as_deref(), Some("null"));
    assert_eq!(pj(b"{}").as_deref(), Some("{}"));
    assert_eq!(pj(b"[]").as_deref(), Some("[]"));
    assert_eq!(pj(b"[ ]").as_deref(), Some("[]"));
    assert_eq!(pj(b"{ }").as_deref(), Some("{}"));
    assert_eq!(pj(b"[\n ]").as_deref(), Some("[]"));
}

#[test]
fn pretty_json_edge_inputs() {
    let pj = |b: &[u8]| response_view::pretty_json(b);
    assert_eq!(pj(b"\xEF\xBB\xBF{\"a\":1}"), None);
    assert_eq!(pj(b"{\"a\":1}\n").as_deref(), Some("{\n  \"a\": 1\n}"));
    assert_eq!(pj(b"   "), None);
    assert_eq!(pj(b""), None);
}
