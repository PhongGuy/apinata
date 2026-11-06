//! `CompiledSchema`: built leniently from the schema model and shareable across threads; also
//! the test schema the validation tests share.

use std::sync::OnceLock;

use apinata_core::{CompiledSchema, Schema, SchemaError};

pub(crate) const SDL: &str = r#"
type Query { me: User user(id: ID!): User users(filter: Filter, role: Role, first: Int): [User] }
type User { id: ID! name: String old: String @deprecated(reason: "use name") legacy: String @deprecated role: Role friends(first: Int): [User] }
enum Role { ADMIN USER OLD @deprecated(reason: "gone") }
input Filter { name: String role: Role }
"#;

pub(crate) fn compiled() -> &'static CompiledSchema {
    static SCHEMA: OnceLock<CompiledSchema> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        CompiledSchema::new(&Schema::from_sdl(SDL).expect("sdl")).expect("compiles")
    })
}

#[test]
fn a_compiled_schema_is_shareable() {
    fn check<T: Send + Sync + std::fmt::Debug>() {}
    check::<CompiledSchema>();
    assert_eq!(format!("{:?}", compiled()), "CompiledSchema { .. }");
}

#[test]
fn a_schema_that_cannot_be_printed_is_refused() {
    let s = Schema::from_json(r#"{"__schema":{"queryType":null,"types":[{"kind":"OBJECT","name":"Bad Name","fields":[{"name":"a","args":[],"type":{"kind":"SCALAR","name":"Int"}}]},{"kind":"SCALAR","name":"Int"}]}}"#).unwrap();
    assert!(matches!(
        CompiledSchema::new(&s),
        Err(SchemaError::NotPrintable { .. })
    ));
}

#[test]
fn a_schema_invalid_by_the_spec_still_compiles() {
    let s = Schema::from_sdl("type Query { u: User }\ntype User implements Node { name: String }\ninterface Node { id: ID! }\n").unwrap();
    assert!(CompiledSchema::new(&s).is_ok());
}
