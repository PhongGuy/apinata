//! The filters over apollo's diagnostics, deprecation warnings, ordering, duplicates and the cap.

use std::ops::Range;

use QuerySeverity::{Error, Info, Warning};
use apinata_core::{QuerySeverity, validate_query};

use crate::graphql_compiled_schema::compiled;
use crate::graphql_validate::{SKIPPED, plain};

fn check(query: &str) -> Vec<(Range<usize>, QuerySeverity, String)> {
    plain(&validate_query(query, Some(compiled())))
}

fn e(range: Range<usize>, message: &str) -> (Range<usize>, QuerySeverity, String) {
    (range, Error, message.to_owned())
}

#[test]
fn unsupported_value_type_over_a_mask_is_dropped() {
    for query in [
        "{ user(id: {{id}}) { id } }",
        "{ users(filter: {{f}}) { id } }",
        "{ users(filter: {name: {{n}}}) { id } }",
        "query Q($n: Int = {{n}}) { users(first: $n) { id } }",
        "{ me @include(if: {{f}}) { id } }",
        "{ user(id: {{a}}{{b}}) { id } }",
        "{ users(first: [{{n}}]) { id } }",
    ] {
        assert_eq!(check(query), vec![], "{query}");
    }
}

#[test]
fn undefined_enum_value_over_a_mask_is_dropped() {
    assert_eq!(check("{ users(role: {{r}}) { id } }"), vec![]);
    assert_eq!(check("{ users(filter: {role: {{r}}}) { id } }"), vec![]);
}

#[test]
fn conflicting_field_argument_over_a_mask_is_dropped() {
    assert_eq!(
        check("{ a: user(id: {{x}}) { id } a: user(id: {{yy}}) { id } }"),
        vec![]
    );
}

#[test]
fn real_errors_over_a_mask_are_kept() {
    assert_eq!(
        check("{ user(idd: {{id}}) { id } }"),
        vec![
            e(
                2..26,
                "the required argument `Query.user(id:)` is not provided"
            ),
            e(7..18, "the argument `idd` is not supported by `Query.user`"),
        ]
    );
    assert_eq!(
        check("{ user(id: {{id}}) }"),
        vec![e(
            2..18,
            "interface, union and object types must have a subselection set"
        )]
    );
    assert_eq!(
        check("{ users(filter: {nmae: {{n}}}) { id } }"),
        vec![e(23..28, "field `nmae` does not exist on `Filter`")]
    );
}

#[test]
fn a_placeholder_conflicts_with_a_literal_only_when_the_literal_comes_second() {
    assert_eq!(
        check("{ a: user(id: {{yy}}) { id } a: user(id: 1) { id } }"),
        vec![e(
            29..50,
            "operation must not provide conflicting field arguments for the same name `user`"
        )]
    );
    assert_eq!(
        check("{ a: user(id: 1) { id } a: user(id: {{yy}}) { id } }"),
        vec![]
    );
}

#[test]
fn directive_names_are_not_checked_but_include_arguments_are() {
    // UndefinedDirective.
    assert_eq!(check("{ me @custom(a: 1) { id } }"), vec![]);
    assert_eq!(check("{ me @inclde(if: true) { id } }"), vec![]);
    assert_eq!(
        check("{ me @include(if: 1) { id } }"),
        vec![e(
            18..19,
            "expected value of type Boolean!, found an integer"
        )]
    );
}

#[test]
fn a_typo_inside_a_selection_set_is_only_an_undefined_field() {
    // MissingSubselection.
    assert_eq!(
        check("{ me { nmae } }"),
        vec![e(7..11, "type `User` does not have a field `nmae`")]
    );
    assert_eq!(
        check("{ me { friends { nmae } } }"),
        vec![e(17..21, "type `User` does not have a field `nmae`")]
    );
    assert_eq!(
        check("{ me { name { x } } }"),
        vec![e(
            7..17,
            "field selection of scalar type `String` must not have subselections"
        )]
    );
    assert_eq!(
        check("{ me }"),
        vec![e(
            2..4,
            "interface, union and object types must have a subselection set"
        )]
    );
}

#[test]
fn deprecated_fields_and_enum_values_warn() {
    let w = |r: Range<usize>, m: &str| (r, Warning, m.to_owned());
    assert_eq!(
        check("{ me { old legacy } }"),
        vec![
            w(7..10, "`User.old` is deprecated: use name"),
            w(11..17, "`User.legacy` is deprecated: No longer supported")
        ]
    );
    assert_eq!(
        check("{ me { o: old } }"),
        vec![w(10..13, "`User.old` is deprecated: use name")]
    );
    assert_eq!(
        check("{ users(role: OLD) { id } }"),
        vec![w(14..17, "`Role.OLD` is deprecated: gone")]
    );
    assert_eq!(
        check("{ users(filter: {role: OLD}) { id } }"),
        vec![w(23..26, "`Role.OLD` is deprecated: gone")]
    );
    assert_eq!(
        check("fragment F on User { old } { me { ...F } }"),
        vec![w(21..24, "`User.old` is deprecated: use name")]
    );
}

#[test]
fn syntax_errors_appear_once_and_notes_come_first() {
    assert_eq!(
        check("{ me { id }"),
        vec![e(11..11, "syntax error: expected R_CURLY, got EOF")]
    );
    assert_eq!(
        check("{ me { {{f}} }"),
        vec![
            (0..0, Info, SKIPPED.to_owned()),
            e(14..14, "syntax error: expected R_CURLY, got EOF")
        ]
    );
}

#[test]
fn past_two_hundred_a_summary_comes_last() {
    let got = plain(&validate_query(&"} ".repeat(300), None));
    assert_eq!(got.len(), 201);
    assert_eq!(
        got[0],
        e(
            0..1,
            "syntax error: expected a StringValue, Name or OperationDefinition"
        )
    );
    assert_eq!(got[199].0, 398..399);
    assert_eq!(got[200], (0..0, Info, "and 100 more".to_owned()));
}

#[test]
fn a_note_comes_before_a_syntax_error_at_offset_zero() {
    assert_eq!(
        check("} { me { {{f}} } }"),
        vec![
            (0..0, Info, SKIPPED.to_owned()),
            e(
                0..1,
                "syntax error: expected a StringValue, Name or OperationDefinition"
            ),
        ]
    );
}
