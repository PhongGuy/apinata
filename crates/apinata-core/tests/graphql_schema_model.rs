//! `Schema::from_json`: the introspection model, with and without the `data` wrapper.

use apinata_core::{Schema, SchemaError, TypeKind};

fn fixture() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/graphql/schema.json"
    ))
    .unwrap()
}

fn schema() -> Schema {
    Schema::from_json(&fixture()).expect("fixture parses")
}

#[test]
fn the_wrapped_fixture_keeps_roots_and_introspection_order() {
    use TypeKind::*;
    let s = schema();
    assert_eq!(s.query_type(), Some("Query"));
    assert_eq!(s.mutation_type(), Some("Mutation"));
    assert_eq!(s.subscription_type(), None);
    let names: Vec<&str> = s.types().iter().map(|t| t.name()).collect();
    assert_eq!(
        names,
        [
            "Query",
            "Mutation",
            "Node",
            "User",
            "Post",
            "SearchResult",
            "Role",
            "NewUser",
            "ID",
            "String",
            "Int",
            "Boolean",
            "__Type"
        ]
    );
    let kinds: Vec<TypeKind> = s.types().iter().map(|t| t.kind()).collect();
    assert_eq!(
        kinds,
        [
            Object,
            Object,
            Interface,
            Object,
            Object,
            Union,
            Enum,
            InputObject,
            Scalar,
            Scalar,
            Scalar,
            Scalar,
            Object
        ]
    );
}

#[test]
fn fields_args_and_type_refs() {
    let s = schema();
    let query = s.type_named("Query").expect("Query");
    assert_eq!(query.description(), Some("The root query."));
    let user = &query.fields()[0];
    assert_eq!(
        (user.name(), user.description()),
        ("user", Some("One user by id."))
    );
    assert_eq!((user.ty().named(), user.ty().display()), ("User", "User"));
    let args: Vec<(&str, &str, Option<&str>, Option<&str>)> = user
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
            ("id", "ID!", None, Some("The user id.")),
            ("active", "Boolean", Some("true"), None)
        ]
    );
    let users = &query.fields()[1];
    assert_eq!(
        (users.ty().named(), users.ty().display()),
        ("User", "[User!]!")
    );
    let user_type = s.type_named("User").unwrap();
    let friends = user_type
        .fields()
        .iter()
        .find(|f| f.name() == "friends")
        .unwrap();
    assert_eq!(friends.ty().display(), "[User!]");
    assert_eq!(s.type_named("Post").unwrap().description(), None);
    assert!(s.type_named("Nope").is_none());
}

#[test]
fn deprecation_carries_the_reason_or_an_empty_one() {
    let s = schema();
    let field = |t: &str, f: &str| {
        let ty = s.type_named(t).unwrap();
        ty.fields()
            .iter()
            .find(|x| x.name() == f)
            .unwrap()
            .deprecation()
            .map(str::to_owned)
    };
    assert_eq!(field("Query", "oldUsers"), Some("Use users".to_owned()));
    assert_eq!(field("User", "legacyName"), Some(String::new()));
    assert_eq!(field("User", "name"), None);
    let role = s.type_named("Role").unwrap();
    let values: Vec<(&str, Option<&str>, Option<&str>)> = role
        .enum_values()
        .iter()
        .map(|v| (v.name(), v.description(), v.deprecation()))
        .collect();
    assert_eq!(
        values,
        [
            ("ADMIN", Some("Full access."), None),
            ("MEMBER", None, None),
            ("GUEST", None, Some("Gone"))
        ]
    );
}

#[test]
fn input_fields_interfaces_and_possible_types() {
    let s = schema();
    let new_user = s.type_named("NewUser").unwrap();
    let input: Vec<(&str, &str, Option<&str>)> = new_user
        .input_fields()
        .iter()
        .map(|f| (f.name(), f.ty().display(), f.default_value()))
        .collect();
    assert_eq!(
        input,
        [("name", "String!", None), ("role", "Role", Some("MEMBER"))]
    );
    assert_eq!(
        s.type_named("User").unwrap().interfaces(),
        ["Node".to_owned()]
    );
    assert_eq!(
        s.type_named("SearchResult").unwrap().possible_types(),
        ["User".to_owned(), "Post".to_owned()]
    );
    assert_eq!(
        s.type_named("Node").unwrap().possible_types(),
        ["User".to_owned()]
    );
    assert!(s.type_named("ID").unwrap().fields().is_empty());
}

#[test]
fn the_bare_form_parses_to_the_same_schema() {
    let v: serde_json::Value = serde_json::from_str(&fixture()).unwrap();
    let bare = serde_json::to_string(&v["data"]).unwrap();
    assert_eq!(Schema::from_json(&bare).unwrap(), schema());
}

#[test]
fn answers_without_a_schema_are_no_data_or_not_json() {
    for text in [
        "{\"data\": null}",
        "{\"data\": {}}",
        "{\"errors\": [{\"message\": \"no\"}]}",
        "[]",
        "{\"data\": {\"__schema\": null}}",
    ] {
        assert!(
            matches!(Schema::from_json(text), Err(SchemaError::NoData)),
            "{text}"
        );
    }
    assert!(matches!(
        Schema::from_json("<html></html>"),
        Err(SchemaError::NotJson { .. })
    ));
}

#[test]
fn a_malformed_schema_is_invalid() {
    for text in [
        "{\"__schema\": {\"types\": 5}}",
        "{\"__schema\": {}}",
        "{\"__schema\": {\"types\": [{\"kind\": \"OBJECT\"}]}}",
        "{\"__schema\": {\"types\": [{\"kind\": \"WIDGET\", \"name\": \"W\"}]}}",
        "{\"__schema\": {\"types\": [{\"kind\": \"OBJECT\", \"name\": \"Q\", \"fields\": [{\"name\": \"f\", \"args\": []}]}]}}",
    ] {
        assert!(
            matches!(Schema::from_json(text), Err(SchemaError::Invalid { .. })),
            "{text}"
        );
    }
    let empty = Schema::from_json("{\"__schema\": {\"queryType\": null, \"types\": []}}").unwrap();
    assert_eq!(empty.query_type(), None);
    assert!(empty.types().is_empty());
}

#[test]
fn error_texts_name_no_other_product() {
    let cases = [
        (
            SchemaError::NotGraphql,
            "only a GraphQL request has a schema",
        ),
        (
            SchemaError::Status {
                status: 500,
                text: "Internal Server Error".into(),
            },
            "the server answered 500 Internal Server Error",
        ),
        (
            SchemaError::TooLarge {
                limit: 32 * 1024 * 1024,
            },
            "the response is larger than the 32 MiB schema limit",
        ),
        (
            SchemaError::NotJson {
                detail: "expected value".into(),
            },
            "the text is not JSON: expected value",
        ),
        (SchemaError::NoData, "no schema in the response"),
        (
            SchemaError::Invalid {
                detail: "types is not a list".into(),
            },
            "the schema is malformed: types is not a list",
        ),
    ];
    for (error, text) in cases {
        assert_eq!(error.to_string(), text);
        assert!(!text.to_lowercase().contains("bruno"));
    }
}
