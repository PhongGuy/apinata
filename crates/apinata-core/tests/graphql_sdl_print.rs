//! `Schema::to_sdl`: a schema printed as an SDL document that reads back to the same types.

use apinata_core::Schema;

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/graphql/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// Every user type of `schema` survives a print and a read unchanged, roots included.
fn assert_round_trip(schema: &Schema) {
    let sdl = schema.to_sdl();
    let back = Schema::from_sdl(&sdl).unwrap_or_else(|e| panic!("{e}\n{sdl}"));
    assert_eq!(back.query_type(), schema.query_type(), "{sdl}");
    assert_eq!(back.mutation_type(), schema.mutation_type(), "{sdl}");
    assert_eq!(
        back.subscription_type(),
        schema.subscription_type(),
        "{sdl}"
    );
    for ty in schema
        .types()
        .iter()
        .filter(|t| !t.name().starts_with("__"))
    {
        assert_eq!(back.type_named(ty.name()), Some(ty), "{}\n{sdl}", ty.name());
    }
}

#[test]
fn the_introspection_fixture_round_trips() {
    assert_round_trip(&Schema::from_json(&fixture("schema.json")).unwrap());
}

#[test]
fn the_sdl_fixture_round_trips() {
    assert_round_trip(&Schema::from_sdl(&fixture("schema.graphql")).unwrap());
}

#[test]
fn every_kind_prints_in_document_order_without_built_ins() {
    let text = "\"The root.\"\ntype Query {\n  \"One.\"\n  user(id: ID!, n: Int = 3): User\n  old: Int @deprecated\n  gone: Int @deprecated(reason: \"Use user\")\n}\n\ninterface Node {\n  id: ID!\n}\n\ntype User implements Node {\n  id: ID!\n}\n\nunion Any = User | Query\n\nenum Role {\n  A\n  B @deprecated(reason: \"no\")\n}\n\ninput In {\n  r: Role = A\n}\n\nscalar Date\n";
    let schema = Schema::from_sdl(text).unwrap();
    assert_eq!(schema.to_sdl(), text);
}

#[test]
fn unconventional_roots_print_a_schema_block() {
    let text = "schema {\n  query: Root\n  mutation: Change\n}\n\ntype Root {\n  a: Int\n}\n\ntype Change {\n  b: Int\n}\n";
    let schema = Schema::from_sdl(text).unwrap();
    assert_eq!(schema.to_sdl(), text);
    // A type named Query that is not the root still needs the block.
    let odd =
        Schema::from_sdl("schema { query: Root }\ntype Root { a: Int }\ntype Query { b: Int }")
            .unwrap();
    assert_round_trip(&odd);
}

#[test]
fn awkward_descriptions_survive() {
    let schema = Schema::from_json(
        r#"{"__schema":{"queryType":{"name":"Query"},"types":[
            {"kind":"OBJECT","name":"Query","description":"Say \"hi\" \\ then\nnext line\n  indented","fields":[
              {"name":"a","description":"ends in a quote\"","args":[
                {"name":"x","description":"   leading spaces\nmore","type":{"kind":"SCALAR","name":"Int"},"defaultValue":null}
              ],"type":{"kind":"SCALAR","name":"Int"},"isDeprecated":true,"deprecationReason":"why \"not\""},
              {"name":"b","description":"has \"\"\" inside\nand lines","args":[],"type":{"kind":"SCALAR","name":"Int"},"isDeprecated":true,"deprecationReason":null}
            ]},
            {"kind":"SCALAR","name":"Int"}
        ]}}"#,
    )
    .unwrap();
    assert_round_trip(&schema);
}

#[test]
fn an_object_without_fields_and_an_empty_union_still_read_back() {
    let schema = Schema::from_json(
        r#"{"__schema":{"queryType":null,"types":[
            {"kind":"OBJECT","name":"Empty","fields":[]},
            {"kind":"UNION","name":"None","possibleTypes":[]}
        ]}}"#,
    )
    .unwrap();
    let sdl = schema.to_sdl();
    assert_eq!(sdl, "type Empty\n\nunion None\n");
    assert_round_trip(&schema);
}
