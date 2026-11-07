//! Directive completion by location class, directive arguments, and `symbol_at` on directives.

use apinata_core::{Completion, CompletionKind, Schema, SchemaSymbol};

const SDL: &str = r#"
"Tags."
directive @tag(name: String!, "How loud." level: Int) repeatable on FIELD | QUERY
directive @op on QUERY | MUTATION
directive @frag on FRAGMENT_DEFINITION | FRAGMENT_SPREAD
directive @var on VARIABLE_DEFINITION
directive @typeOnly on OBJECT | FIELD_DEFINITION
type Query { user(id: ID): User }
type User { id: ID name: String }
"#;

const IN_SELECTION: [&str; 4] = ["tag", "frag", "skip", "include"];
const IN_HEADER: [&str; 3] = ["tag", "op", "frag"];

fn schema() -> Schema {
    Schema::from_sdl(SDL).unwrap()
}

fn at(s: &Schema, marked: &str) -> Vec<Completion> {
    let offset = marked.find('|').expect("a cursor");
    s.completions(&marked.replacen('|', "", 1), offset)
}

fn labels(s: &Schema, marked: &str) -> Vec<String> {
    at(s, marked).iter().map(|c| c.label().to_owned()).collect()
}

fn symbol(s: &Schema, marked: &str) -> Option<(SchemaSymbol, String)> {
    let offset = marked.find('|').expect("a cursor");
    let query = marked.replacen('|', "", 1);
    s.symbol_at(&query, offset)
        .map(|found| (found.symbol, query[found.range].to_owned()))
}

fn directive(name: &str) -> SchemaSymbol {
    SchemaSymbol::Directive { name: name.into() }
}

fn directive_arg(d: &str, arg: &str) -> SchemaSymbol {
    SchemaSymbol::DirectiveArgument {
        directive: d.into(),
        arg: arg.into(),
    }
}

#[test]
fn directives_after_at_follow_the_location_class() {
    let s = schema();
    let cases: &[(&str, &[&str])] = &[
        ("{ user @|", &IN_SELECTION),
        ("{ user(id: 1) { id @|", &IN_SELECTION),
        ("{ ... @|", &IN_SELECTION),
        ("{ ...F @|", &IN_SELECTION),
        ("{ user { ... on User @|", &IN_SELECTION),
        ("{ user @tag(name: \"a\") @|", &IN_SELECTION),
        ("{ user @t|", &["tag"]),
        ("{ user @IN|", &["include"]),
        ("query Q @|", &IN_HEADER),
        ("mutation @|", &IN_HEADER),
        ("fragment F on User @|", &IN_HEADER),
        ("query Q($id: ID @|", &["var"]),
        ("query Q($id: ID = 1 @|", &["var"]),
        ("# @|", &[]),
        ("{ user(id: \"@|", &[]),
        ("{ user {{a@|", &[]),
        ("{ user {{a}}\n  {{b@|", &[]),
    ];
    for (marked, want) in cases {
        assert_eq!(labels(&s, marked), *want, "{marked:?}");
    }
}

#[test]
fn directive_arguments_complete_like_field_arguments() {
    let s = schema();
    let cases: &[(&str, &[&str])] = &[
        ("{ user @tag(|", &["name:", "level:"]),
        ("{ user @tag(name: \"a\" |", &["name:", "level:"]),
        ("{ user @tag(l|", &["level:"]),
        ("{ user @tag(name: |", &[]),
        ("{ user @tag(name: $|", &[]),
        ("{ user @nope(|", &[]),
        ("{ user @op(|", &[]),
        ("query Q @tag(|", &["name:", "level:"]),
        ("query Q($id: ID @tag(|", &["name:", "level:"]),
        (
            "{ user @tag(name: \"a\") { |",
            &["id", "name", "__typename"],
        ),
        ("query Q @op { |", &["user", "__typename"]),
    ];
    for (marked, want) in cases {
        assert_eq!(labels(&s, marked), *want, "{marked:?}");
    }
}

#[test]
fn directive_items_carry_kind_signature_and_description() {
    let s = schema();
    let tag = at(&s, "{ user @ta|").remove(0);
    assert_eq!(
        (tag.label(), tag.kind(), tag.detail(), tag.documentation()),
        (
            "tag",
            CompletionKind::Directive,
            "@tag(name: String!, level: Int)",
            Some("Tags.")
        )
    );
    let op = at(&s, "query @o|").remove(0);
    assert_eq!(
        (op.label(), op.detail(), op.documentation()),
        ("op", "@op", None)
    );
    let level = at(&s, "{ user @tag(le|").remove(0);
    assert_eq!(
        (
            level.label(),
            level.kind(),
            level.detail(),
            level.documentation()
        ),
        ("level:", CompletionKind::Argument, "Int", Some("How loud."))
    );
}

#[test]
fn symbol_at_names_directives_and_their_arguments() {
    let s = schema();
    let cases: Vec<(&str, SchemaSymbol, &str)> = vec![
        (
            "{ user @ta|g(name: \"a\") { id } }",
            directive("tag"),
            "tag",
        ),
        ("{ user @|tag { id } }", directive("tag"), "tag"),
        (
            "{ user @tag(na|me: \"a\") { id } }",
            directive_arg("tag", "name"),
            "name",
        ),
        (
            "{ user @tag(name: \"a\", lev|el: 1) { id } }",
            directive_arg("tag", "level"),
            "level",
        ),
        ("query Q @o|p { user { id } }", directive("op"), "op"),
        (
            "query Q($id: ID @va|r) { user(id: $id) { id } }",
            directive("var"),
            "var",
        ),
        (
            "fragment F on User @fr|ag { id }",
            directive("frag"),
            "frag",
        ),
        (
            "{ user @typeOn|ly { id } }",
            directive("typeOnly"),
            "typeOnly",
        ),
        (
            "{ user @sk|ip(if: true) { id } }",
            directive("skip"),
            "skip",
        ),
        (
            "{ user @tag(name: \"a\") { na|me } }",
            SchemaSymbol::Field {
                owner: "User".into(),
                field: "name".into(),
            },
            "name",
        ),
    ];
    for (marked, want, text) in cases {
        assert_eq!(
            symbol(&s, marked),
            Some((want, text.to_owned())),
            "{marked}"
        );
    }
    for marked in [
        "{ user @nop|e { id } }",
        "{ user @nope(a|b: 1) { id } }",
        "{ user @tag(nam|ex: 1) { id } }",
        "{ user @tag(name: $na|me) { id } }",
        "{ user @{{d|ir}} { id } }",
    ] {
        assert_eq!(symbol(&s, marked), None, "{marked}");
    }
}
