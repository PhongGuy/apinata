//! A GraphQL response's top-level `errors` array read into a one-line summary.

use apinata_core::graphql_errors;

fn summary(body: &str) -> Option<String> {
    graphql_errors(body.as_bytes()).map(|e| e.summary())
}

#[test]
fn one_error_names_its_message() {
    assert_eq!(
        summary(r#"{"errors":[{"message":"Not authorised"}],"data":null}"#).as_deref(),
        Some("1 GraphQL error: Not authorised")
    );
}

#[test]
fn several_errors_count_and_name_the_first_with_its_path() {
    let body = r#"{"errors":[
        {"message":"Cannot query field \"x\" on type \"Query\"","path":["user","x"]},
        {"message":"second"}
    ]}"#;
    let errors = graphql_errors(body.as_bytes()).unwrap();
    assert_eq!(errors.count, 2);
    assert_eq!(errors.path.as_deref(), Some("user.x"));
    assert!(!errors.partial);
    assert_eq!(
        errors.summary(),
        r#"2 GraphQL errors: Cannot query field "x" on type "Query" (at user.x)"#
    );
}

#[test]
fn a_path_with_list_indices_joins_them_with_dots() {
    assert_eq!(
        summary(r#"{"errors":[{"message":"m","path":["users",0,"name"]}]}"#).as_deref(),
        Some("1 GraphQL error: m (at users.0.name)")
    );
}

#[test]
fn non_null_data_beside_errors_is_partial() {
    assert_eq!(
        summary(r#"{"data":{"user":null},"errors":[{"message":"m"}]}"#).as_deref(),
        Some("1 GraphQL error, partial data: m")
    );
}

#[test]
fn an_error_without_a_message_is_still_counted() {
    assert_eq!(
        summary(r#"{"errors":[{"extensions":{}},{"message":"b"}]}"#).as_deref(),
        Some("2 GraphQL errors")
    );
}

#[test]
fn a_multi_line_message_reads_as_one_line() {
    assert_eq!(
        summary(r#"{"errors":[{"message":"first\n  second"}]}"#).as_deref(),
        Some("1 GraphQL error: first second")
    );
}

#[test]
fn no_summary_without_a_non_empty_top_level_errors_array() {
    for body in [
        r#"{"data":{"a":1}}"#,
        r#"{"errors":[]}"#,
        r#"{"errors":null}"#,
        r#"{"errors":"boom"}"#,
        r#"{"data":{"errors":[{"message":"nested"}]}}"#,
        r#"[{"errors":[{"message":"m"}]}]"#,
        "not json",
        "",
    ] {
        assert_eq!(summary(body), None, "{body}");
    }
    assert!(graphql_errors(&[0xff, 0xfe]).is_none());
}
