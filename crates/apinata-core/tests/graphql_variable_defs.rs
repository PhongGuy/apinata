//! `variable_definitions`: the `$name: Type = default` list of a query, broken or not.

use apinata_core::{QUERY_VALIDATE_LIMIT, VariableDefinition, variable_definitions};

fn defs(query: &str) -> Vec<(String, String, Option<String>)> {
    let all: Vec<VariableDefinition> = variable_definitions(query);
    all.into_iter().map(|d| (d.name, d.ty, d.default)).collect()
}

fn d(name: &str, ty: &str, default: Option<&str>) -> (String, String, Option<String>) {
    (name.into(), ty.into(), default.map(str::to_owned))
}

#[test]
fn a_valid_query_lists_names_types_and_defaults() {
    assert_eq!(
        defs(
            "query Q($id: ID!, $f: [ Int ! ] = [1, 2], $u: NewUser = {name: \"x\"}) { user(id: $id) { name } }"
        ),
        vec![
            d("id", "ID!", None),
            d("f", "[Int!]", Some("[1, 2]")),
            d("u", "NewUser", Some("{name: \"x\"}")),
        ]
    );
}

#[test]
fn a_half_written_query_keeps_its_definitions() {
    assert_eq!(
        defs("query Q($id: ID!, $role: Role) { user("),
        vec![d("id", "ID!", None), d("role", "Role", None)]
    );
    assert_eq!(defs("query Q($id: ID!, $f: "), vec![d("id", "ID!", None)]);
    assert_eq!(defs("query Q($id: ID!, $f"), vec![d("id", "ID!", None)]);
}

#[test]
fn every_operation_counts_and_the_first_definition_wins() {
    assert_eq!(
        defs("query A($a: Int) { a } mutation B($a: String, $b: Boolean = true) { b }"),
        vec![d("a", "Int", None), d("b", "Boolean", Some("true"))]
    );
}

#[test]
fn placeholders_survive_in_defaults_and_hide_names_and_types() {
    assert_eq!(
        defs("query Q($id: ID = {{id}}) { a }"),
        vec![d("id", "ID", Some("{{id}}"))]
    );
    assert_eq!(
        defs("query Q(${{v}}: ID!, $t: {{T}}, $ok: Int) { a }"),
        vec![d("ok", "Int", None)]
    );
}

#[test]
fn nothing_without_definitions_or_over_the_limit() {
    assert!(defs("{ a }").is_empty());
    assert!(defs("").is_empty());
    let big = format!(
        "query Q($a: Int) {{ a }}{}",
        " ".repeat(QUERY_VALIDATE_LIMIT)
    );
    assert!(defs(&big).is_empty());
}
