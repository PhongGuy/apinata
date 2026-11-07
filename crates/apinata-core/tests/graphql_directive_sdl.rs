//! Directives read from SDL: definitions, the admission rule and the built-ins.

use apinata_core::{Schema, SchemaError};

use crate::graphql_directive_model::{BUILT_INS, intro, names};

pub(crate) const TAGGED: &str = r#""Tags a field."
directive @tag(
  "The tag."
  name: String! = "x"
  color: Color
) repeatable on FIELD | QUERY
directive @once on FIELD
type Query { a: Int }
enum Color { RED }
"#;

fn sdl(text: &str) -> Schema {
    Schema::from_sdl(text).unwrap_or_else(|e| panic!("{e}\n{text}"))
}

#[test]
fn definitions_keep_description_arguments_repeatable_and_locations() {
    let s = sdl(TAGGED);
    assert_eq!(
        names(&s),
        [
            "tag",
            "once",
            "skip",
            "include",
            "deprecated",
            "specifiedBy"
        ]
    );
    let tag = s.directive_named("tag").unwrap();
    assert_eq!(tag.description(), Some("Tags a field."));
    assert!(tag.is_repeatable());
    assert_eq!(tag.locations(), ["FIELD", "QUERY"]);
    let args: Vec<(&str, &str, Option<&str>, Option<&str>)> = tag
        .args()
        .iter()
        .map(|a| {
            (
                a.name(),
                a.ty().display(),
                a.default_value(),
                a.description(),
            )
        })
        .collect();
    assert_eq!(
        args,
        [
            ("name", "String!", Some("\"x\""), Some("The tag.")),
            ("color", "Color", None, None)
        ]
    );
    let once = s.directive_named("once").unwrap();
    assert!(!once.is_repeatable());
    assert!(once.args().is_empty());
}

#[test]
fn both_readers_end_with_the_same_built_ins() {
    assert_eq!(
        sdl("type Query { a: Int }").directives(),
        intro(None).directives()
    );
    let fixture = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/graphql/schema.graphql"
    ))
    .unwrap();
    assert_eq!(names(&sdl(&fixture)), BUILT_INS);
}

#[test]
fn a_redefined_built_in_keeps_the_schema_s_definition() {
    let s = sdl(
        "directive @deprecated(reason: String) on FIELD_DEFINITION\ntype Query { a: Int @deprecated }",
    );
    assert_eq!(names(&s), ["deprecated", "skip", "include", "specifiedBy"]);
    let deprecated = s.directive_named("deprecated").unwrap();
    assert_eq!(deprecated.args()[0].default_value(), None);
    assert_eq!(deprecated.locations(), ["FIELD_DEFINITION"]);
    assert_eq!(
        s.type_named("Query").unwrap().fields()[0].deprecation(),
        Some("No longer supported")
    );
}

#[test]
fn rejected_definitions_are_skipped_and_the_schema_still_loads() {
    let s = sdl(
        "directive @missing(a: Missing) on FIELD\ndirective @wrapped(a: [Missing!]) on FIELD\ndirective @dup(a: Int) on FIELD\ndirective @dup on QUERY\ntype Query { a: Int }\n",
    );
    assert_eq!(
        names(&s),
        ["dup", "skip", "include", "deprecated", "specifiedBy"]
    );
    assert_eq!(s.directive_named("dup").unwrap().locations(), ["FIELD"]);
    assert_eq!(s.type_named("Query").unwrap().fields().len(), 1);
}

#[test]
fn an_unknown_field_type_still_fails_beside_a_skipped_directive() {
    match Schema::from_sdl("directive @x(a: Missing) on FIELD\ntype Query { a: Gone }") {
        Err(SchemaError::Sdl { detail, .. }) => assert!(detail.contains("Gone"), "{detail}"),
        other => panic!("expected an SDL error, got {other:?}"),
    }
}
