//! Directives in `to_sdl`, and query validation through `CompiledSchema` that now knows them.

use std::ops::Range;

use QuerySeverity::{Error, Info};
use apinata_core::{CompiledSchema, QuerySeverity, Schema, validate_query};
use serde_json::json;

use crate::graphql_compiled_schema::compiled;
use crate::graphql_directive_model::intro;
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

/// A graphql-js server lists the built-ins itself, so they arrive as the schema's own.
#[test]
fn introspected_built_ins_and_a_custom_directive_compile_and_validate() {
    let s = intro(Some(json!([
        {"name": "include", "locations": ["FIELD", "FRAGMENT_SPREAD", "INLINE_FRAGMENT"],
         "args": [{"name": "if", "type": {"kind": "NON_NULL", "ofType": {"kind": "SCALAR", "name": "Boolean"}}}]},
        {"name": "skip", "locations": ["FIELD", "FRAGMENT_SPREAD", "INLINE_FRAGMENT"],
         "args": [{"name": "if", "type": {"kind": "NON_NULL", "ofType": {"kind": "SCALAR", "name": "Boolean"}}}]},
        {"name": "deprecated", "locations": ["FIELD_DEFINITION", "ARGUMENT_DEFINITION", "INPUT_FIELD_DEFINITION", "ENUM_VALUE"],
         "args": [{"name": "reason", "type": {"kind": "SCALAR", "name": "String"}, "defaultValue": "\"No longer supported\""}]},
        {"name": "specifiedBy", "locations": ["SCALAR"],
         "args": [{"name": "url", "type": {"kind": "NON_NULL", "ofType": {"kind": "SCALAR", "name": "String"}}}]},
        {"name": "upper", "locations": ["FIELD"], "args": []}
    ])));
    let c = CompiledSchema::new(&s).unwrap_or_else(|e| panic!("{e}\n{:?}", s.to_sdl()));
    assert_eq!(
        plain(&validate_query("{ a @include(if: true) @upper }", Some(&c))),
        vec![]
    );
}
