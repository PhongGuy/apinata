//! `Schema::from_sdl` and `Schema::from_text`: SDL schema files read into the introspection model.

use apinata_core::{Schema, SchemaError, TypeKind};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/graphql/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn sdl(text: &str) -> Schema {
    Schema::from_sdl(text).unwrap_or_else(|e| panic!("{e}\n{text}"))
}

fn sdl_error(text: &str) -> (usize, usize, String) {
    match Schema::from_sdl(text) {
        Err(SchemaError::Sdl {
            line,
            column,
            detail,
        }) => (line, column, detail),
        other => panic!("expected an SDL error for {text:?}, got {other:?}"),
    }
}

#[test]
fn the_sdl_fixture_matches_the_introspection_fixture() {
    let json = Schema::from_json(&fixture("schema.json")).unwrap();
    let from_sdl = sdl(&fixture("schema.graphql"));
    assert_eq!(from_sdl.query_type(), json.query_type());
    assert_eq!(from_sdl.mutation_type(), json.mutation_type());
    assert_eq!(from_sdl.subscription_type(), None);
    for ty in json.types().iter().filter(|t| !t.name().starts_with("__")) {
        assert_eq!(from_sdl.type_named(ty.name()), Some(ty), "{}", ty.name());
    }
}

#[test]
fn types_keep_document_order_then_the_built_in_scalars() {
    let s = sdl("type Query { a: Int }\nscalar Date\nscalar ID\nenum E { X }");
    let names: Vec<&str> = s.types().iter().map(|t| t.name()).collect();
    assert_eq!(
        names,
        [
            "Query", "Date", "ID", "E", "String", "Int", "Float", "Boolean"
        ]
    );
    use TypeKind::*;
    let kinds: Vec<TypeKind> = s.types().iter().map(|t| t.kind()).collect();
    assert_eq!(
        kinds,
        [Object, Scalar, Scalar, Enum, Scalar, Scalar, Scalar, Scalar]
    );
}

#[test]
fn every_type_kind_reads_with_its_members() {
    let s = sdl(r#"
        type Query { pet(kind: Kind = DOG, filter: Filter = {name: "x", tags: ["a"]}): Pet }
        interface Named { name: String }
        interface Pet implements Named { name: String }
        type Dog implements Pet & Named { name: String, barks: Boolean! }
        type Cat implements Pet & Named { name: String }
        union Animal = | Dog | Cat
        enum Kind { DOG CAT }
        input Filter { name: String, tags: [String!] = [] }
        scalar Url
    "#);
    let pet = s.type_named("Pet").unwrap();
    assert_eq!(pet.kind(), TypeKind::Interface);
    assert_eq!(pet.interfaces(), ["Named".to_owned()]);
    assert_eq!(pet.possible_types(), ["Dog".to_owned(), "Cat".to_owned()]);
    let named = s.type_named("Named").unwrap();
    assert_eq!(named.possible_types(), ["Dog".to_owned(), "Cat".to_owned()]);
    let dog = s.type_named("Dog").unwrap();
    assert_eq!(dog.interfaces(), ["Pet".to_owned(), "Named".to_owned()]);
    let barks = &dog.fields()[1];
    assert_eq!(
        (barks.ty().named(), barks.ty().display()),
        ("Boolean", "Boolean!")
    );
    let animal = s.type_named("Animal").unwrap();
    assert_eq!(animal.kind(), TypeKind::Union);
    assert_eq!(
        animal.possible_types(),
        ["Dog".to_owned(), "Cat".to_owned()]
    );
    let kind: Vec<&str> = s
        .type_named("Kind")
        .unwrap()
        .enum_values()
        .iter()
        .map(|v| v.name())
        .collect();
    assert_eq!(kind, ["DOG", "CAT"]);
    let filter = s.type_named("Filter").unwrap();
    assert_eq!(filter.kind(), TypeKind::InputObject);
    let fields: Vec<(&str, &str, Option<&str>)> = filter
        .input_fields()
        .iter()
        .map(|f| (f.name(), f.ty().display(), f.default_value()))
        .collect();
    assert_eq!(
        fields,
        [("name", "String", None), ("tags", "[String!]", Some("[]"))]
    );
    let args: Vec<(&str, Option<&str>)> = s.type_named("Query").unwrap().fields()[0]
        .args()
        .iter()
        .map(|a| (a.name(), a.default_value()))
        .collect();
    assert_eq!(
        args,
        [
            ("kind", Some("DOG")),
            ("filter", Some("{name: \"x\", tags: [\"a\"]}"))
        ]
    );
    assert_eq!(s.type_named("Url").unwrap().kind(), TypeKind::Scalar);
}

#[test]
fn descriptions_keep_strings_and_dedent_block_strings() {
    let s = sdl(concat!(
        "\"\"\"\n",
        "    The root.\n",
        "      Indented line.\n",
        "\"\"\"\n",
        "type Query {\n",
        "  \"Line one\\nquoted \\\"x\\\"\" a: Int\n",
        "  b(\"\"\"arg\"\"\" x: Int): Int\n",
        "}\n",
        "enum E { \"value\" V }\n",
        "\"a scalar\" scalar S\n",
    ));
    let q = s.type_named("Query").unwrap();
    assert_eq!(q.description(), Some("The root.\n  Indented line."));
    assert_eq!(q.fields()[0].description(), Some("Line one\nquoted \"x\""));
    assert_eq!(q.fields()[1].args()[0].description(), Some("arg"));
    assert_eq!(
        s.type_named("E").unwrap().enum_values()[0].description(),
        Some("value")
    );
    assert_eq!(s.type_named("S").unwrap().description(), Some("a scalar"));
    assert_eq!(s.type_named("String").unwrap().description(), None);
}

#[test]
fn deprecated_without_a_reason_uses_the_default_reason() {
    let s = sdl(
        "type Query { a: Int @deprecated, b: Int @deprecated(reason: \"Use a\"), c: Int @other }\nenum E { X @deprecated }",
    );
    let q = s.type_named("Query").unwrap();
    let reasons: Vec<Option<&str>> = q.fields().iter().map(|f| f.deprecation()).collect();
    assert_eq!(reasons, [Some("No longer supported"), Some("Use a"), None]);
    assert_eq!(
        s.type_named("E").unwrap().enum_values()[0].deprecation(),
        Some("No longer supported")
    );
}

#[test]
fn root_types_default_to_their_names_or_follow_the_schema_block() {
    let plain = sdl("type Query { a: Int } type Mutation { b: Int } type Subscription { c: Int }");
    assert_eq!(
        (
            plain.query_type(),
            plain.mutation_type(),
            plain.subscription_type()
        ),
        (Some("Query"), Some("Mutation"), Some("Subscription"))
    );
    let none = sdl("type Root { a: Int }");
    assert_eq!(none.query_type(), None);
    let renamed = sdl(
        "schema { query: Root subscription: Events }\ntype Root { a: Int }\ntype Events { e: Int }\ntype Query { unused: Int }\ntype Mutation { m: Int }",
    );
    assert_eq!(
        (
            renamed.query_type(),
            renamed.mutation_type(),
            renamed.subscription_type()
        ),
        (Some("Root"), None, Some("Events"))
    );
    let extended = sdl(
        "schema { query: Root }\nextend schema { mutation: Change }\ntype Root { a: Int }\ntype Change { b: Int }",
    );
    assert_eq!(
        (extended.query_type(), extended.mutation_type()),
        (Some("Root"), Some("Change"))
    );
}

#[test]
fn extensions_add_members_to_their_type() {
    let s = sdl(r#"
        extend type Query { late: String }
        type Query { a: Int }
        interface I { id: ID }
        extend type Query implements I { id: ID }
        extend interface I { more: Int }
        enum E { A }
        extend enum E { B }
        union U = Query
        type Other { x: Int }
        extend union U = Other
        input In { a: Int }
        extend input In { b: Int }
        scalar S
        extend scalar S @specifiedBy(url: "https://example.test")
    "#);
    let q = s.type_named("Query").unwrap();
    let fields: Vec<&str> = q.fields().iter().map(|f| f.name()).collect();
    assert_eq!(fields, ["a", "late", "id"]);
    assert_eq!(q.interfaces(), ["I".to_owned()]);
    let i = s.type_named("I").unwrap();
    assert_eq!(i.fields().len(), 2);
    assert_eq!(i.possible_types(), ["Query".to_owned()]);
    let e: Vec<&str> = s
        .type_named("E")
        .unwrap()
        .enum_values()
        .iter()
        .map(|v| v.name())
        .collect();
    assert_eq!(e, ["A", "B"]);
    assert_eq!(
        s.type_named("U").unwrap().possible_types(),
        ["Query".to_owned(), "Other".to_owned()]
    );
    assert_eq!(s.type_named("In").unwrap().input_fields().len(), 2);
}

#[test]
fn directive_definitions_are_ignored() {
    let s = sdl(
        "directive @auth(role: String) on FIELD_DEFINITION | OBJECT\ntype Query { a: Int @auth(role: \"x\") }",
    );
    assert_eq!(
        s.type_named("Query").unwrap().fields()[0].deprecation(),
        None
    );
    assert!(s.type_named("auth").is_none());
}

#[test]
fn malformed_sdl_reports_the_line_and_column() {
    let (line, column, detail) = sdl_error("type Query {\n  a: Int\n  b Int\n}\n");
    assert_eq!((line, column), (3, 5), "{detail}");
    let (line, column, _) = sdl_error("<html></html>");
    assert_eq!((line, column), (1, 1));
    let (line, column, detail) = sdl_error("type Query {\n  a: Missing\n}");
    assert_eq!((line, column), (2, 6));
    assert!(detail.contains("Missing"), "{detail}");
    let (line, column, detail) = sdl_error("type Query { a: Int }\ntype Query { b: Int }");
    assert_eq!((line, column), (2, 6));
    assert!(detail.contains("Query"), "{detail}");
    let (line, column, detail) = sdl_error("type Query { a: Int }\n\nextend type Nope { b: Int }");
    assert_eq!((line, column), (3, 13));
    assert!(detail.contains("Nope"), "{detail}");
    let (line, column, detail) = sdl_error("type Query { a: Int }\nquery Q { a }");
    assert_eq!((line, column), (2, 1));
    assert!(detail.contains("query"), "{detail}");
    // Columns count characters, not bytes.
    let (line, column, _) = sdl_error("\"\u{e9}\u{e9}\" type Query { a: \u{e9} }");
    assert_eq!((line, column), (1, 22));
}

#[test]
fn hostile_input_is_an_error_not_a_panic() {
    let deep = format!(
        "type Query {{ a: {}Int{} }}",
        "[".repeat(5000),
        "]".repeat(5000)
    );
    assert!(matches!(
        Schema::from_sdl(&deep),
        Err(SchemaError::Sdl { .. })
    ));
    for text in ["\"\\uD800\" type Query { a: Int }", "\"\"\"", "type", ""] {
        let _ = Schema::from_sdl(text);
    }
}

#[test]
fn the_error_text_names_the_position() {
    let error = SchemaError::Sdl {
        line: 3,
        column: 5,
        detail: "expected a Name".into(),
    };
    assert_eq!(
        error.to_string(),
        "the schema is not valid SDL at line 3, column 5: expected a Name"
    );
}

#[test]
fn from_text_picks_json_or_sdl_by_content() {
    let json = Schema::from_json(&fixture("schema.json")).unwrap();
    assert_eq!(Schema::from_text(&fixture("schema.json")).unwrap(), json);
    let v: serde_json::Value = serde_json::from_str(&fixture("schema.json")).unwrap();
    let bare = serde_json::to_string_pretty(&v["data"]).unwrap();
    assert_eq!(
        Schema::from_text(&format!("\u{feff}\n  {bare}")).unwrap(),
        json
    );
    assert_eq!(
        Schema::from_text(&fixture("schema.graphql")).unwrap(),
        sdl(&fixture("schema.graphql"))
    );
    assert!(matches!(
        Schema::from_text("{\"data\": "),
        Err(SchemaError::NotJson { .. })
    ));
    assert!(matches!(
        Schema::from_text("type Query {"),
        Err(SchemaError::Sdl { .. })
    ));
}

#[test]
fn a_schema_extension_alone_keeps_the_default_roots_it_does_not_set() {
    let s = sdl(
        "extend schema { mutation: Change }\ntype Query { a: Int }\ntype Change { b: Int }\ntype Subscription { c: Int }",
    );
    assert_eq!(
        (s.query_type(), s.mutation_type(), s.subscription_type()),
        (Some("Query"), Some("Change"), Some("Subscription"))
    );
}

#[test]
fn error_positions_skip_comments_before_the_culprit() {
    let (line, column, _) = sdl_error("type Query { a: Int }\n# an operation\nquery Q { a }");
    assert_eq!((line, column), (3, 1));
    let (line, column, _) = sdl_error("type Query { a: Int }\n# again\ntype Query { b: Int }");
    assert_eq!((line, column), (3, 6));
    let (line, column, _) = sdl_error("type Query {\n  a: # the type\n    Missing\n}");
    assert_eq!((line, column), (3, 5));
}

#[test]
fn every_line_break_style_starts_a_new_line() {
    for text in [
        "type Query {\r  a: Missing\r}",
        "type Query {\r\n  a: Missing\r\n}",
        "type Query {\n  a: Missing\n}",
    ] {
        let (line, column, _) = sdl_error(text);
        assert_eq!((line, column), (2, 6), "{text:?}");
    }
    let (line, column, _) = sdl_error("type Query {\r\r\n\n  a: Missing }");
    assert_eq!((line, column), (4, 6));
}
