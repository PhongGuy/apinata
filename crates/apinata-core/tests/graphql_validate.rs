//! `validate_query`: syntax, apollo's rules, placeholders and limits.

use std::ops::Range;

use QuerySeverity::{Error, Info, Warning};
use apinata_core::{
    CompiledSchema, QUERY_FORMAT_LIMIT, QUERY_VALIDATE_LIMIT, QueryDiagnostic, QuerySeverity,
    Schema, validate_query,
};

use crate::graphql_compiled_schema::compiled;

pub(crate) const SKIPPED: &str =
    "Schema checks skipped: a {{placeholder}} stands for query structure";

pub(crate) fn plain(diags: &[QueryDiagnostic]) -> Vec<(Range<usize>, QuerySeverity, String)> {
    diags
        .iter()
        .map(|d| (d.range(), d.severity(), d.message().to_owned()))
        .collect()
}

pub(crate) fn has(
    diags: &[QueryDiagnostic],
    range: Range<usize>,
    severity: QuerySeverity,
    needle: &str,
) -> bool {
    diags
        .iter()
        .any(|d| d.range() == range && d.severity() == severity && d.message().contains(needle))
}

#[test]
fn a_schema_invalid_by_the_spec_is_used_as_far_as_it_goes() {
    let s = Schema::from_sdl("type Query { u: User }\ntype User implements Node { name: String }\ninterface Node { id: ID! }\n").unwrap();
    let c = CompiledSchema::new(&s).unwrap();
    assert_eq!(validate_query("{ u { name } }", Some(&c)), vec![]);
    assert!(has(
        &validate_query("{ u { nmae } }", Some(&c)),
        6..10,
        Error,
        "does not have a field `nmae`"
    ));
}

#[test]
fn without_a_schema_only_syntax_is_checked() {
    assert_eq!(validate_query("{ me { nmae } }", None), vec![]);
    assert_eq!(
        plain(&validate_query("{ me { id }", None)),
        vec![(
            11..11,
            Error,
            "syntax error: expected R_CURLY, got EOF".to_owned()
        )]
    );
    assert_eq!(
        validate_query("{ me { {{f}} } }", None),
        vec![],
        "no note without a schema"
    );
}

#[test]
fn each_kept_rule_reports_its_range() {
    let cases: &[(&str, Range<usize>, &str)] = &[
        (
            "{ me { nmae } }",
            7..11,
            "type `User` does not have a field `nmae`",
        ),
        (
            "{ user { id } }",
            2..13,
            "the required argument `Query.user(id:)` is not provided",
        ),
        (
            "{ me { friends(frist: 1) { id } } }",
            15..23,
            "the argument `frist` is not supported by `User.friends`",
        ),
        (
            "{ user(id: true) { id } }",
            11..15,
            "expected value of type ID!, found a boolean",
        ),
        (
            "{ users(role: ROOT) { id } }",
            14..18,
            "value `ROOT` does not exist on `Role`",
        ),
        (
            "{ users(filter: {nmae: \"a\"}) { id } }",
            23..26,
            "field `nmae` does not exist on `Filter`",
        ),
        (
            "{ me { name { x } } }",
            7..17,
            "field selection of scalar type `String` must not have subselections",
        ),
        (
            "{ me }",
            2..4,
            "interface, union and object types must have a subselection set",
        ),
        (
            "{ user(id: $x) { id } }",
            11..13,
            "variable `$x` is not defined",
        ),
        (
            "query Q($id: ID!) { me { id } }",
            8..11,
            "unused variable: `$id`",
        ),
        (
            "{ me { ...F } }",
            7..11,
            "cannot find fragment `F` in this document",
        ),
        (
            "fragment F on User { id } { me { id } }",
            0..25,
            "fragment `F` must be used in an operation",
        ),
        (
            "{ a: user(id: 1) { id } a: user(id: 2) { id } }",
            24..45,
            "conflicting field arguments for the same name `user`",
        ),
        (
            "{ me { x: id x: name } }",
            13..20,
            "cannot select different fields into the same alias `x`",
        ),
        (
            "{ users(first: 99999999999) { id } }",
            15..26,
            "int cannot represent non 32-bit signed integer value",
        ),
        (
            "query Q($r: Int) { user(id: $r) { id } }",
            24..30,
            "variable `$r` of type `Int` cannot be used for argument `id` of type `ID!`",
        ),
        (
            "query Q($v: Nope) { me { id } }",
            8..16,
            "cannot find type `Nope` in this document",
        ),
        (
            "{ me { ... on Nope { id } } }",
            14..18,
            "type condition `Nope` of inline fragment is not a type defined in the schema",
        ),
        (
            "{ me @include(if: 1) { id } }",
            18..19,
            "expected value of type Boolean!, found an integer",
        ),
    ];
    for (query, range, needle) in cases {
        let got = validate_query(query, Some(compiled()));
        assert!(
            has(&got, range.clone(), Error, needle),
            "{query}: {:?}",
            plain(&got)
        );
    }
}

#[test]
fn a_structural_placeholder_skips_the_schema_checks() {
    for query in [
        "{ me { {{fields}} nmae } }",
        "{ me { name{{x}} } }",
        "{ me { ...{{frag}} } }",
        "{ me { ... on {{T}} { id } } }",
        "fragment {{F}} on User { id } { me { id } }",
        "query Q($v: {{T}}) { me { id } }",
        "query Q($id: ID!) { user(id: ${{x}}) { id } }",
        "query Q(${{v}}: ID!) { me { id } }",
        "{ user({{arg}}: 1) { id } }",
        "{ users(filter: { {{k}}: 1 }) { id } }",
        "{ me @{{dir}} { id } }",
    ] {
        assert_eq!(
            plain(&validate_query(query, Some(compiled()))),
            vec![(0..0, Info, SKIPPED.to_owned())],
            "{query}"
        );
    }
}

#[test]
fn value_placeholders_keep_the_schema_checks() {
    let got = validate_query("{ user(id: {{id}}) { nmae } }", Some(compiled()));
    assert!(
        has(&got, 21..25, Error, "does not have a field `nmae`"),
        "{:?}",
        plain(&got)
    );
    assert!(!got.iter().any(|d| d.message() == SKIPPED));
    assert_eq!(
        validate_query("{ user(id: \"{{id}}\") { id } }", Some(compiled())),
        vec![]
    );
    assert_eq!(
        validate_query("{ me { id } } # {{c}}", Some(compiled())),
        vec![]
    );
}

#[test]
fn ranges_fall_on_char_boundaries_around_multibyte_placeholders() {
    let text = "{ user(idd: {{ø}}) { id } }";
    let got = validate_query(text, Some(compiled()));
    assert!(
        has(
            &got,
            7..18,
            Error,
            "the argument `idd` is not supported by `Query.user`"
        ),
        "{:?}",
        plain(&got)
    );
    for d in &got {
        assert!(
            text.is_char_boundary(d.range().start) && text.is_char_boundary(d.range().end),
            "{d:?}"
        );
    }
}

#[test]
fn blank_input_has_no_diagnostics() {
    for text in ["", "  \n", ", ,", "# c\n"] {
        assert_eq!(validate_query(text, Some(compiled())), vec![], "{text:?}");
        assert_eq!(validate_query(text, None), vec![], "{text:?}");
    }
}

#[test]
fn size_limits() {
    let big = "{ me { nmae } }".to_owned() + &" ".repeat(QUERY_VALIDATE_LIMIT);
    assert_eq!(
        plain(&validate_query(&big, Some(compiled()))),
        vec![(
            0..0,
            Warning,
            "The query is larger than 256 KiB: only syntax is checked".to_owned()
        )]
    );
    let huge = "{ me { id }".to_owned() + &" ".repeat(QUERY_FORMAT_LIMIT);
    assert_eq!(
        plain(&validate_query(&huge, Some(compiled()))),
        vec![(
            0..0,
            Warning,
            "The query is larger than 4 MiB and was not checked".to_owned()
        )]
    );
}

#[test]
fn deep_nesting_is_a_diagnostic_not_an_overflow() {
    let deep = format!("{}{}", "{me{friends".repeat(1500), "}}".repeat(1500));
    let got = std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(move || validate_query(&deep, Some(compiled())))
        .unwrap()
        .join()
        .unwrap();
    assert!(
        got.iter().any(|d| d.message().contains("recursion")),
        "{} diagnostics",
        got.len()
    );
}

#[test]
fn the_syntax_only_paths_still_report_syntax_errors() {
    let big = "{ me { id } } }".to_owned() + &" ".repeat(QUERY_VALIDATE_LIMIT);
    let got = validate_query(&big, Some(compiled()));
    assert_eq!(got.len(), 2, "{:?}", plain(&got));
    assert!(has(&got, 0..0, Warning, "larger than 256 KiB"));
    assert!(has(&got, 14..15, Error, "syntax error: expected"));

    let got = validate_query("{ me { {{f}} } } }", Some(compiled()));
    assert_eq!(got.len(), 2, "{:?}", plain(&got));
    assert!(has(&got, 0..0, Info, SKIPPED));
    assert!(has(&got, 17..18, Error, "syntax error: expected"));
}

#[test]
fn blank_input_over_the_validate_limit_has_no_diagnostics() {
    let blank = " \n".repeat(QUERY_VALIDATE_LIMIT);
    assert_eq!(validate_query(&blank, Some(compiled())), vec![]);
    assert_eq!(validate_query(&blank, None), vec![]);
}
