//! The JSON validity check behind the body editor's error marker.

use apinata_core::json_syntax_error;

fn at(text: &str) -> (usize, usize, String) {
    let e = json_syntax_error(text)
        .unwrap_or_else(|| panic!("expected an error for {:?}", &text[..text.len().min(40)]));
    (e.line, e.column, e.message)
}

#[test]
fn valid_and_blank_bodies_have_no_error() {
    assert_eq!(json_syntax_error(""), None);
    assert_eq!(json_syntax_error("  \n\t "), None);
    assert_eq!(json_syntax_error("{\"a\": [1, 2, {\"b\": null}]}"), None);
}

#[test]
fn a_missing_closing_brace_points_past_the_end() {
    let (line, column, message) = at("{\"a\": 1");
    assert_eq!((line, column), (1, 8));
    assert!(message.contains("EOF"), "{message}");
    assert!(
        !message.contains("line"),
        "position leaked into message: {message}"
    );
}

#[test]
fn a_missing_value_on_the_second_line() {
    let (line, column, message) = at("{\n\"b\": }");
    assert_eq!((line, column), (2, 6));
    assert_eq!(message, "expected value");
}

#[test]
fn a_trailing_comma_is_reported() {
    let (line, column, message) = at("[1,2,]");
    assert_eq!((line, column), (1, 6));
    assert_eq!(message, "expected value");
    let (line, column, _) = at("{\"a\":1,}");
    assert_eq!((line, column), (1, 8));
}

#[test]
fn the_column_counts_characters_not_bytes() {
    let (line, column, _) = at("{\"é\": 1, \"ü\": }");
    assert_eq!((line, column), (1, 15));
}

#[test]
fn a_very_long_single_line_keeps_an_exact_column() {
    let text = format!("[{}]", "1,".repeat(50_000));
    let (line, column, _) = at(&text);
    assert_eq!((line, column), (1, 100_002));
}

#[test]
fn a_placeholder_as_a_value_is_valid() {
    assert_eq!(json_syntax_error("{\"id\": {{id}}}"), None);
}

#[test]
fn a_placeholder_inside_a_string_is_valid() {
    assert_eq!(
        json_syntax_error("{\"s\": \"{{name}} and {{other}}\"}"),
        None
    );
}

#[test]
fn an_error_after_a_placeholder_keeps_its_column() {
    let (line, column, _) = at("{\"id\": {{id}}, ");
    assert_eq!((line, column), (1, 16));
}

#[test]
fn a_placeholder_as_a_key_is_flagged() {
    assert!(json_syntax_error("{{{k}}: 1}").is_some());
    assert!(json_syntax_error("{{{k}}}: 1}").is_some());
}

#[test]
fn an_unclosed_placeholder_is_left_alone_and_flagged() {
    let (line, column, _) = at("{\"a\": {{x}");
    assert_eq!((line, column), (1, 8));
}

#[test]
fn a_placeholder_spanning_lines_is_not_one() {
    assert!(json_syntax_error("{\"a\": {{x\n}}}").is_some());
}

// Each CRLF case must report the same line and column as the LF twin of the same body.
fn same_for_lf_and_crlf(lf: &str) -> (usize, usize) {
    let crlf = lf.replace('\n', "\r\n");
    let (l, c, _) = at(lf);
    let (l2, c2, _) = at(&crlf);
    assert_eq!((l, c), (l2, c2), "CRLF differs from LF for {lf:?}");
    (l, c)
}

#[test]
fn crlf_mid_line_error_matches_lf() {
    assert_eq!(
        same_for_lf_and_crlf("{\n  \"a\": 1,\n  \"b\": }\n}"),
        (3, 8)
    );
}

#[test]
fn crlf_end_of_input_error_matches_lf() {
    assert_eq!(same_for_lf_and_crlf("{\n  \"a\": 1"), (2, 9));
    assert_eq!(same_for_lf_and_crlf("{\n  \"a\": 1\n"), (3, 1));
}

#[test]
fn crlf_error_on_the_last_line_matches_lf() {
    assert_eq!(
        same_for_lf_and_crlf("{\n  \"a\": 1,\n  \"b\": 2,\n}"),
        (4, 1)
    );
    assert_eq!(same_for_lf_and_crlf("[1,\n2,\n3 4]"), (3, 3));
}

#[test]
fn multibyte_and_placeholder_before_the_error_count_characters() {
    // "é" is two bytes and "{{name}}" is masked to filler of equal length; both precede the error.
    let (line, column, _) = at("{\"é\": \"{{name}}\", \"b\": }");
    assert_eq!((line, column), (1, 24));
    let (line, column, _) = at("{\"é\": \"{{name}}\",\r\n\"ü\": {{id}}, \"b\": }");
    assert_eq!((line, column), (2, 19));
}
