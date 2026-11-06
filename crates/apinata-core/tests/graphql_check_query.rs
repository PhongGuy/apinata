//! `check_query`: which schema checks ran, beside the same findings `validate_query` gives.

use apinata_core::{
    QUERY_FORMAT_LIMIT, QUERY_VALIDATE_LIMIT, SCHEMA_SKIP_NOTE, SchemaUse, check_query,
    validate_query,
};

use crate::graphql_compiled_schema::compiled;

#[test]
fn a_plain_query_with_a_schema_is_checked() {
    let check = check_query("{ me { nmae } }", Some(compiled()));
    assert_eq!(check.schema_use(), SchemaUse::Checked);
    assert!(check.schema_checked());
    assert_eq!(check.diagnostics().len(), 1, "{:?}", check.diagnostics());
}

#[test]
fn a_blank_query_with_a_schema_counts_as_checked() {
    let check = check_query("  \n", Some(compiled()));
    assert_eq!(check.schema_use(), SchemaUse::Checked);
    assert!(check.diagnostics().is_empty());
}

#[test]
fn no_schema_is_not_given_whatever_the_size() {
    for text in [
        "{ me { id } }".to_owned(),
        "".to_owned(),
        "{ me { id } }".to_owned() + &" ".repeat(QUERY_VALIDATE_LIMIT),
        "{ me { id } }".to_owned() + &" ".repeat(QUERY_FORMAT_LIMIT),
    ] {
        let check = check_query(&text, None);
        assert_eq!(check.schema_use(), SchemaUse::NotGiven, "{}", text.len());
        assert!(!check.schema_checked());
    }
}

#[test]
fn a_query_over_either_limit_is_too_large() {
    for pad in [QUERY_VALIDATE_LIMIT, QUERY_FORMAT_LIMIT] {
        let text = "{ me { id } }".to_owned() + &" ".repeat(pad);
        let check = check_query(&text, Some(compiled()));
        assert_eq!(check.schema_use(), SchemaUse::TooLarge, "{pad}");
        assert!(!check.schema_checked());
    }
}

#[test]
fn a_structural_placeholder_is_reported_with_the_skip_note() {
    let check = check_query("{ me { {{f}} } }", Some(compiled()));
    assert_eq!(check.schema_use(), SchemaUse::Placeholder);
    assert!(!check.schema_checked());
    assert!(
        check
            .diagnostics()
            .iter()
            .any(|d| d.message() == SCHEMA_SKIP_NOTE)
    );
}

#[test]
fn validate_query_is_check_query_without_the_basis() {
    let big = "{ me { nmae } } }".to_owned() + &" ".repeat(QUERY_VALIDATE_LIMIT);
    let huge = "{ me { id }".to_owned() + &" ".repeat(QUERY_FORMAT_LIMIT);
    let texts = [
        "{ me { nmae old } }",
        "{ me { id } } }",
        "{ me { {{f}} } } }",
        "{ user(id: {{id}}) { nmae } }",
        "",
        big.as_str(),
        huge.as_str(),
    ];
    for text in texts {
        for schema in [Some(compiled()), None] {
            assert_eq!(
                validate_query(text, schema),
                check_query(text, schema).into_diagnostics(),
                "{} bytes",
                text.len()
            );
        }
    }
}
