//! `Schema::completions` over the fixture schema: one row per scanner context.

use apinata_core::{Completion, CompletionKind, Schema};

const QUERY: [&str; 6] = ["user", "users", "node", "search", "__typename", "oldUsers"];
const ON: [&str; 6] = ["Query", "Mutation", "Node", "User", "Post", "SearchResult"];
const INPUT: [&str; 6] = ["Role", "NewUser", "ID", "String", "Int", "Boolean"];

fn schema() -> Schema {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/graphql/schema.json"
    ))
    .unwrap();
    Schema::from_json(&text).unwrap()
}

/// The completions at the `|` in `marked`, which is removed.
fn at(s: &Schema, marked: &str) -> Vec<Completion> {
    let offset = marked.find('|').expect("a cursor");
    s.completions(&marked.replacen('|', "", 1), offset)
}

fn labels(s: &Schema, marked: &str) -> Vec<String> {
    at(s, marked).iter().map(|c| c.label().to_owned()).collect()
}

#[test]
fn completion_table() {
    let s = schema();
    let cases: &[(&str, &[&str])] = &[
        ("{ |", &QUERY),
        ("query Q { |", &QUERY),
        ("{ u|", &["user", "users"]),
        ("{ US|", &["user", "users"]),
        ("{ o|", &["oldUsers"]),
        ("{ me: us|", &["user", "users"]),
        (
            "{ user(id: 1) { |",
            &["id", "name", "role", "friends", "__typename", "legacyName"],
        ),
        ("{ user { friends { na|", &["name"]),
        ("{ node(id: 1) { |", &["id", "__typename"]),
        ("{ search(term: \"x\") { |", &["__typename"]),
        ("{ user { id } |", &QUERY),
        ("mutation M { |", &["addUser", "__typename"]),
        ("subscription { |", &[]),
        ("fragment F on |", &ON),
        ("fragment F on User { fr|", &["friends"]),
        ("{ search(term: \"x\") { ... on |", &ON),
        ("{ search(term: \"x\") { ... on P|", &["Post"]),
        (
            "{ search(term: \"x\") { ... on Post { |",
            &["title", "__typename"],
        ),
        ("{ user(|", &["id:", "active:"]),
        ("{ user(id: \"a\" ac|", &["active:"]),
        ("{ users(first: 2, |", &["first:", "role:"]),
        ("{ user(id: |", &[]),
        ("{ user(id: $|", &[]),
        ("query Q($id: |", &INPUT),
        ("query Q($id: ID!, $r: R|", &["Role"]),
        ("query Q($ids: [|", &INPUT),
        ("query Q(|", &[]),
        ("query Q($|", &[]),
        ("{ user(id: \"{ (\") { na|", &["name"]),
        ("{ user(id: \"\"\" { ( \"\"\") { na|", &["name"]),
        ("{ # { user\n  us|", &["user", "users"]),
        ("{ # us|", &[]),
        ("{ user(id: \"us|", &[]),
        ("{ nope { |", &[]),
        ("{ user { nope { |", &[]),
        ("fragment F on Nope { |", &[]),
        ("{ user @|", &[]),
        (
            "{ user(id: 1) @include(if: true) { |",
            &["id", "name", "role", "friends", "__typename", "legacyName"],
        ),
        (
            "{ user @skip(if: $x) { |",
            &["id", "name", "role", "friends", "__typename", "legacyName"],
        ),
        ("{ user @include(|", &[]),
        ("{ ... @include(if: $x) { |", &QUERY),
        ("{ ... { |", &QUERY),
        ("{ ...|", &[]),
        ("|", &[]),
        ("quer|", &[]),
        ("{ user { id } }\n|", &[]),
    ];
    for (marked, want) in cases {
        assert_eq!(labels(&s, marked), *want, "{marked:?}");
    }
}

#[test]
fn completions_carry_kind_detail_and_documentation() {
    let s = schema();
    let one = |marked: &str| {
        let mut all = at(&s, marked);
        assert_eq!(all.len(), 1, "{marked:?}");
        all.remove(0)
    };
    let f = one("{ user { fri|");
    assert_eq!(
        (f.label(), f.kind(), f.detail(), f.documentation()),
        (
            "friends",
            CompletionKind::Field,
            "[User!]",
            Some("Friends of the user.")
        )
    );
    let a = one("{ user(i|");
    assert_eq!(
        (a.label(), a.kind(), a.detail(), a.documentation()),
        ("id:", CompletionKind::Argument, "ID!", Some("The user id."))
    );
    let t = one("fragment F on No|");
    assert_eq!(
        (t.label(), t.kind(), t.detail(), t.documentation()),
        (
            "Node",
            CompletionKind::Type,
            "interface",
            Some("Anything with an id.")
        )
    );
    let n = one("{ user { __|");
    assert_eq!(
        (n.label(), n.kind(), n.detail(), n.documentation()),
        ("__typename", CompletionKind::Field, "String!", None)
    );
}

#[test]
fn an_offset_inside_a_character_or_past_the_end_is_clamped() {
    let s = schema();
    let labels = |t: &str, o: usize| {
        s.completions(t, o)
            .iter()
            .map(|c| c.label().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(labels("{ é", 3), QUERY);
    assert_eq!(labels("{ us", 100), ["user", "users"]);
}
