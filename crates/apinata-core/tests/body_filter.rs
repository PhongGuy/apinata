//! Filtering a response body by its content type: the engine choice, singular unwrapping and
//! the expression check that runs before a send.

use apinata_core::{BodyFilterError, filter_body, validate_filter};

const JSON: &str = r#"[{"name":"a","tags":[1,2]},{"name":"b","tags":[]}]"#;
const XML: &str = "<r><i id=\"1\">x</i><i id=\"2\">y</i></r>";

#[test]
fn a_singular_json_path_prints_its_one_value() {
    let out = filter_body(Some("application/json"), JSON, "$[0].name").unwrap();
    assert_eq!(out, "\"a\"");
    let out = filter_body(Some("application/json"), JSON, "$[1]['name']").unwrap();
    assert_eq!(out, "\"b\"");
    let out = filter_body(Some("application/json"), JSON, "$[0]").unwrap();
    assert_eq!(
        out,
        "{\n  \"name\": \"a\",\n  \"tags\": [\n    1,\n    2\n  ]\n}"
    );
}

#[test]
fn a_path_that_can_match_many_stays_an_array() {
    let json = Some("application/json");
    assert_eq!(
        filter_body(json, JSON, "$[*].name").unwrap(),
        "[\n  \"a\",\n  \"b\"\n]"
    );
    assert_eq!(
        filter_body(json, JSON, "$..name").unwrap(),
        "[\n  \"a\",\n  \"b\"\n]"
    );
    assert_eq!(
        filter_body(json, JSON, "$[0:1].name").unwrap(),
        "[\n  \"a\"\n]"
    );
    assert_eq!(
        filter_body(json, JSON, "$[?@.name=='a'].name").unwrap(),
        "[\n  \"a\"\n]"
    );
}

#[test]
fn a_singular_path_with_no_match_is_an_empty_array() {
    assert_eq!(
        filter_body(Some("application/json"), JSON, "$[5].name").unwrap(),
        "[]"
    );
}

#[test]
fn xml_types_use_xpath() {
    for ct in [
        "application/xml",
        "text/xml; charset=utf-8",
        "application/atom+xml",
    ] {
        assert_eq!(
            filter_body(Some(ct), XML, "//i/@id").unwrap(),
            "1\n2",
            "{ct}"
        );
    }
    assert_eq!(
        filter_body(Some("application/xml"), XML, "//nope").unwrap(),
        ""
    );
}

#[test]
fn other_types_cannot_be_filtered() {
    for ct in [Some("text/html"), Some("text/plain"), None] {
        let e = filter_body(ct, "<p>x</p>", "$.a").unwrap_err();
        assert!(matches!(e, BodyFilterError::Unfilterable { .. }), "{e:?}");
        assert!(e.to_string().contains("not JSON or XML"), "{e}");
    }
    let e = filter_body(Some("text/plain"), "x", "$.a").unwrap_err();
    assert!(e.to_string().contains("text/plain"), "{e}");
}

#[test]
fn a_dialect_mismatch_is_a_filter_error() {
    let e = filter_body(Some("application/json"), JSON, "//i").unwrap_err();
    assert!(matches!(e, BodyFilterError::Filter(_)), "{e:?}");
}

#[test]
fn validate_accepts_either_dialect_and_names_the_expression_otherwise() {
    assert!(validate_filter("$[0].name").is_ok());
    assert!(validate_filter("//i[@id='1']").is_ok());
    let e = validate_filter("$[?(").unwrap_err();
    let text = e.to_string();
    assert!(text.contains("$[?("), "{text}");
    assert!(validate_filter("   ").is_err());
}

#[test]
fn a_name_union_is_not_singular_even_when_one_name_exists() {
    let json = Some("application/json");
    let body = r#"{"a":1,"a,b":5,"a]":7}"#;
    assert_eq!(
        filter_body(json, body, "$['a','zzz']").unwrap(),
        "[\n  1\n]"
    );
    assert_eq!(
        filter_body(json, body, "$['a','a,b']").unwrap(),
        "[\n  1,\n  5\n]"
    );
    assert_eq!(
        filter_body(json, "[1,2,3]", "$[0,1]").unwrap(),
        "[\n  1,\n  2\n]"
    );
}

#[test]
fn a_quoted_name_holding_a_comma_or_bracket_is_still_singular() {
    let json = Some("application/json");
    let body = r#"{"a,b":5,"a]":7}"#;
    assert_eq!(filter_body(json, body, "$['a,b']").unwrap(), "5");
    assert_eq!(filter_body(json, body, "$['a]']").unwrap(), "7");
    assert_eq!(filter_body(json, body, "$[\"a,b\"]").unwrap(), "5");
}
