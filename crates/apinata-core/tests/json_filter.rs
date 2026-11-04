//! JSONPath filtering of response bodies.

use apinata_core::{FilterError, filter_json};

const BODY: &str = r#"{"data":[{"id":7,"name":"a","price":12.5},{"id":8,"name":"b","price":3}],"alpha":{"name":"c"},"n":1e2}"#;

fn ok(expression: &str) -> String {
    filter_json(BODY, expression).unwrap_or_else(|e| panic!("{expression}: {e}"))
}

#[test]
fn a_path_returns_its_matches_as_a_pretty_array() {
    assert_eq!(ok("$.data[0].id"), "[\n  7\n]");
    assert_eq!(ok("$.data[-1].id"), "[\n  8\n]");
    assert_eq!(ok("$.data[*].id"), "[\n  7,\n  8\n]");
    assert_eq!(ok("$..name"), "[\n  \"a\",\n  \"b\",\n  \"c\"\n]");
    assert_eq!(ok("$['alpha'].name"), "[\n  \"c\"\n]");
    assert_eq!(ok("$.nope"), "[]");
}

#[test]
fn objects_keep_their_key_order_and_numbers_print_as_javascript_does() {
    assert_eq!(
        ok("$.data[0]"),
        "[\n  {\n    \"id\": 7,\n    \"name\": \"a\",\n    \"price\": 12.5\n  }\n]"
    );
    assert_eq!(ok("$.n"), "[\n  100\n]");
}

#[test]
fn filters_select_by_comparison_with_or_without_parentheses() {
    assert_eq!(ok("$.data[?(@.price > 10)].name"), "[\n  \"a\"\n]");
    assert_eq!(ok("$.data[?@.price < 10].id"), "[\n  8\n]");
    assert_eq!(ok("$.data[?match(@.name, 'b')].id"), "[\n  8\n]");
}

#[test]
fn the_root_is_optional_and_whitespace_is_ignored() {
    assert_eq!(ok("data[0].id"), ok("$.data[0].id"));
    assert_eq!(ok("..name"), ok("$..name"));
    assert_eq!(ok("['alpha'].name"), ok("$['alpha'].name"));
    assert_eq!(ok("  $.data[0].id  "), "[\n  7\n]");
}

#[test]
fn an_invalid_expression_names_where_it_broke() {
    let e = filter_json(BODY, "$.data[0").unwrap_err();
    assert!(
        matches!(e, FilterError::Expression { position: 8, .. }),
        "{e:?}"
    );
    let e = filter_json(BODY, "data[0").unwrap_err();
    assert!(
        matches!(e, FilterError::Expression { position: 6, .. }),
        "{e:?}"
    );
    assert_eq!(
        e.to_string(),
        "Invalid filter at character 7: expected closing ']'"
    );
    assert!(matches!(
        filter_json(BODY, "   ").unwrap_err(),
        FilterError::Expression { position: 0, .. }
    ));
}

#[test]
fn a_generic_parser_error_names_the_character_it_stopped_at() {
    let message = |expression| filter_json(BODY, expression).unwrap_err().to_string();
    assert_eq!(
        message("$[?@.a && ]"),
        "Invalid filter at character 11: unexpected \"]\""
    );
    assert_eq!(
        message("$[?@.a &&"),
        "Invalid filter at character 10: unexpected end of filter"
    );
    assert_eq!(
        message("$.a."),
        "Invalid filter at character 5: must start with lowercase alpha or '_'"
    );
}

#[test]
fn a_body_that_is_not_json_is_its_own_error_after_the_expression() {
    assert!(matches!(
        filter_json("nope", "$").unwrap_err(),
        FilterError::Body { .. }
    ));
    assert!(matches!(
        filter_json("nope", "$[").unwrap_err(),
        FilterError::Expression { .. }
    ));
}

#[test]
fn error_positions_count_characters_not_bytes() {
    let e = filter_json(BODY, "€€[").unwrap_err();
    assert!(
        matches!(e, FilterError::Expression { position: 2, .. }),
        "{e:?}"
    );
    assert!(
        e.to_string().starts_with("Invalid filter at character 3:"),
        "{e}"
    );
}

#[test]
fn a_leading_byte_order_mark_is_ignored() {
    assert_eq!(
        filter_json("\u{feff}{\"a\":1}", "$.a").unwrap(),
        "[\n  1\n]"
    );
}
