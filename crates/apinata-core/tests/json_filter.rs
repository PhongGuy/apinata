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

mod parsed_once {
    use std::cell::Cell;
    use std::time::Instant;

    use apinata_core::{FilterError, JsonFilter, ParsedJson, filter_json};

    use super::BODY;

    fn never() -> bool {
        false
    }

    #[test]
    fn a_parsed_body_filters_the_same_as_the_one_shot_call() {
        let parsed = ParsedJson::parse(BODY).unwrap();
        for expression in ["$.data[0]", "$..name", "data[*].id", "$.nope", "$.n"] {
            let compiled = JsonFilter::compile(expression).unwrap();
            assert_eq!(
                compiled.run(&parsed, never),
                filter_json(BODY, expression),
                "{expression}"
            );
        }
    }

    #[test]
    fn compile_and_parse_report_the_same_errors_as_the_one_shot_call() {
        let expression = JsonFilter::compile("$.data[1").unwrap_err();
        assert_eq!(Err(expression), filter_json(BODY, "$.data[1"));
        let body = ParsedJson::parse("{nope").unwrap_err();
        assert_eq!(Err(body), filter_json("{nope", "$.a"));
        assert!(ParsedJson::parse("\u{feff}[1]").is_ok());
    }

    #[test]
    fn a_cancelled_run_stops_at_the_next_element() {
        let body = format!("[{}]", vec!["{\"a\":[1,2,3]}"; 10_000].join(","));
        let parsed = ParsedJson::parse(&body).unwrap();
        let compiled = JsonFilter::compile("$").unwrap();
        // The run asks before each element; it is told to stop on the sixth question.
        let asked = Cell::new(0);
        let cancelled = || {
            asked.set(asked.get() + 1);
            asked.get() > 5
        };
        assert_eq!(
            compiled.run(&parsed, cancelled),
            Err(FilterError::Cancelled)
        );
        assert_eq!(asked.get(), 6);
    }

    // A rough timing of keystrokes over a large body, with and without the parsed copy kept:
    // cargo test --release -p apinata-core --test integration parsed_once -- --ignored --nocapture
    #[test]
    #[ignore = "a timing, not a check"]
    fn time_filters_with_and_without_the_parsed_copy() {
        let row = r#"{"id":1,"name":"widget","tags":["a","b","c"],"price":12.5,"nested":{"x":true,"y":null}}"#;
        let body = format!("[{}]", vec![row; 110_000].join(","));
        let keystrokes = ["$[0", "$[0]", "$[0].", "$[0].n", "$[0].na", "$[0].name"];
        let start = Instant::now();
        for e in keystrokes {
            let _ = std::hint::black_box(filter_json(&body, e));
        }
        let uncached = start.elapsed();
        let start = Instant::now();
        let parsed = ParsedJson::parse(&body).unwrap();
        for e in keystrokes {
            if let Ok(compiled) = JsonFilter::compile(e) {
                let _ = std::hint::black_box(compiled.run(&parsed, never));
            }
        }
        let cached = start.elapsed();
        println!(
            "{} MB, {} keystrokes: reparse each {uncached:?}, parse once {cached:?}",
            body.len() / 1_000_000,
            keystrokes.len()
        );
    }
}
