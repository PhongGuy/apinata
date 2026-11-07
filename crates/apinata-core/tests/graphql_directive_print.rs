//! Directives in `to_sdl`, and query validation through `CompiledSchema` that now knows them.

use std::ops::Range;

use QuerySeverity::{Error, Info};
use apinata_core::{CompiledSchema, QuerySeverity, Schema, validate_query};
use serde_json::{Value, json};

use crate::graphql_compiled_schema::compiled;
use crate::graphql_directive_model::{intro, names};
use crate::graphql_directive_sdl::TAGGED;
use crate::graphql_validate::{SKIPPED, plain};

fn e(range: Range<usize>, message: &str) -> (Range<usize>, QuerySeverity, String) {
    (range, Error, message.to_owned())
}

#[test]
fn own_directives_print_before_the_types_and_built_ins_do_not() {
    let text = "directive @once on FIELD\n\n\"Tags.\"\ndirective @tag(name: String!, n: Int = 2) repeatable on FIELD | QUERY\n\ntype Query {\n  a: Int\n}\n";
    assert_eq!(
        Schema::from_sdl(text).unwrap().to_sdl().expect("printable"),
        text
    );
    let bare = Schema::from_sdl("type Query { a: Int }")
        .unwrap()
        .to_sdl()
        .unwrap();
    assert!(!bare.contains("directive"), "{bare}");
}

#[test]
fn own_directives_round_trip_with_argument_descriptions() {
    let s = Schema::from_sdl(TAGGED).unwrap();
    let sdl = s.to_sdl().expect("printable");
    let back = Schema::from_sdl(&sdl).unwrap_or_else(|e| panic!("{e}\n{sdl}"));
    assert_eq!(back.directives(), s.directives(), "{sdl}");
    assert!(!sdl.contains("directive @skip"), "{sdl}");
}

#[test]
fn a_redefined_built_in_and_an_introspected_one_of_are_printed() {
    let s = Schema::from_sdl(
        "directive @deprecated(reason: String) on FIELD_DEFINITION\ntype Query { a: Int @deprecated }",
    )
    .unwrap();
    let sdl = s.to_sdl().unwrap();
    assert!(
        sdl.contains("directive @deprecated(reason: String) on FIELD_DEFINITION\n"),
        "{sdl}"
    );
    let one_of = intro(Some(json!([
        {"name": "oneOf", "locations": ["INPUT_OBJECT"], "args": []}
    ])));
    let sdl = one_of.to_sdl().unwrap();
    assert!(sdl.contains("directive @oneOf on INPUT_OBJECT\n"), "{sdl}");
    let compiled = CompiledSchema::new(&one_of).expect("compiles");
    assert_eq!(plain(&validate_query("{ a }", Some(&compiled))), vec![]);
}

#[test]
fn unknown_directives_are_reported_and_known_ones_pass() {
    let check = |q: &str| plain(&validate_query(q, Some(compiled())));
    assert_eq!(
        check("{ me @inclde(if: true) { id } }"),
        vec![e(5..22, "cannot find directive `@inclde` in this document")]
    );
    assert_eq!(check("{ me @include(if: true) { id } }"), vec![]);
    assert_eq!(
        check("{ me @{{d}} { id } }"),
        vec![(0..0, Info, SKIPPED.to_owned())]
    );
    assert_eq!(
        check("fragment F on User @include(if: true) { id } { me { ...F } }"),
        vec![e(
            19..37,
            "include directive is not supported for FRAGMENT_DEFINITION location"
        )]
    );
}

#[test]
fn a_schema_s_own_directives_validate_by_their_definitions() {
    let s = Schema::from_sdl(
        "directive @tag(name: String!) repeatable on FIELD | QUERY\ndirective @once on FIELD\ntype Query { me: Int }",
    )
    .unwrap();
    let c = CompiledSchema::new(&s).expect("compiles");
    let check = |q: &str| plain(&validate_query(q, Some(&c)));
    assert_eq!(check("{ me @tag(name: \"a\") @tag(name: \"b\") }"), vec![]);
    assert_eq!(check("query Q @tag(name: \"x\") { me }"), vec![]);
    assert_eq!(check("{ me @include(if: true) }"), vec![]);
    assert_eq!(
        check("{ me @once @once }"),
        vec![e(
            11..16,
            "non-repeatable directive once can only be used once per location"
        )]
    );
}

/// The four built-ins as a graphql-js 16 server lists them, worded differently from core's
/// table, plus `upper`; `deprecated` replaces the `@deprecated` entry.
fn listed_built_ins(deprecated: Value) -> Schema {
    intro(Some(json!([
        {"name": "include", "description": "Include it.", "locations": ["FIELD", "FRAGMENT_SPREAD", "INLINE_FRAGMENT"],
         "args": [{"name": "if", "description": "When true.", "type": {"kind": "NON_NULL", "ofType": {"kind": "SCALAR", "name": "Boolean"}}}]},
        {"name": "skip", "description": "Skip it.", "locations": ["FIELD", "FRAGMENT_SPREAD", "INLINE_FRAGMENT"],
         "args": [{"name": "if", "description": "When true.", "type": {"kind": "NON_NULL", "ofType": {"kind": "SCALAR", "name": "Boolean"}}}]},
        deprecated,
        {"name": "specifiedBy", "description": "Spec URL.", "locations": ["SCALAR"],
         "args": [{"name": "url", "type": {"kind": "NON_NULL", "ofType": {"kind": "SCALAR", "name": "String"}}}]},
        {"name": "upper", "locations": ["FIELD"], "args": []}
    ])))
}

fn deprecated(locations: Value, extra_args: &[Value]) -> Value {
    let mut args = vec![json!({"name": "reason", "description": "Why.",
        "type": {"kind": "SCALAR", "name": "String"}, "defaultValue": "\"No longer supported\""})];
    args.extend_from_slice(extra_args);
    json!({"name": "deprecated", "description": "Gone.", "locations": locations, "args": args})
}

fn standard_deprecated() -> Value {
    deprecated(
        json!([
            "FIELD_DEFINITION",
            "ARGUMENT_DEFINITION",
            "INPUT_FIELD_DEFINITION",
            "ENUM_VALUE"
        ]),
        &[],
    )
}

/// A graphql-js server lists the built-ins itself, so they arrive as the schema's own.
#[test]
fn introspected_built_ins_and_a_custom_directive_compile_and_validate() {
    let s = listed_built_ins(standard_deprecated());
    let c = CompiledSchema::new(&s).unwrap_or_else(|e| panic!("{e}\n{:?}", s.to_sdl()));
    assert_eq!(
        plain(&validate_query("{ a @include(if: true) @upper }", Some(&c))),
        vec![]
    );
}

#[test]
fn unchanged_listed_built_ins_are_left_out_of_the_sdl() {
    let s = listed_built_ins(standard_deprecated());
    let sdl = s.to_sdl().unwrap();
    assert!(sdl.contains("directive @upper on FIELD\n"), "{sdl}");
    for name in ["skip", "include", "deprecated", "specifiedBy"] {
        assert!(!sdl.contains(&format!("directive @{name}")), "{sdl}");
    }
    assert_eq!(
        names(&s),
        vec!["include", "skip", "deprecated", "specifiedBy", "upper"]
    );
}

#[test]
fn a_listed_built_in_with_changed_arguments_or_locations_is_printed() {
    let extra = json!({"name": "since", "type": {"kind": "SCALAR", "name": "String"}});
    let more_args = deprecated(
        json!([
            "FIELD_DEFINITION",
            "ARGUMENT_DEFINITION",
            "INPUT_FIELD_DEFINITION",
            "ENUM_VALUE"
        ]),
        &[extra],
    );
    let sdl = listed_built_ins(more_args).to_sdl().unwrap();
    assert!(sdl.contains("directive @deprecated("), "{sdl}");
    assert!(sdl.contains("since: String"), "{sdl}");
    assert!(!sdl.contains("directive @skip"), "{sdl}");

    let fewer_locations = deprecated(json!(["FIELD_DEFINITION"]), &[]);
    let sdl = listed_built_ins(fewer_locations).to_sdl().unwrap();
    assert!(sdl.contains(") on FIELD_DEFINITION\n"), "{sdl}");
    assert!(sdl.contains("directive @deprecated("), "{sdl}");
}
