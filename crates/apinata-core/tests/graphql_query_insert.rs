//! `toggle_field` ticking: where a field and its missing ancestors are written.

use apinata_core::{FieldPath, QUERY_VALIDATE_LIMIT, Schema, toggle_field};

const SDL: &str = "
type Query { me: User user(id: ID): User users: [User!]! version: String search: [SearchResult!]! node: Node }
type Mutation { ping: Boolean addUser(name: String): User }
type Subscription { tick: Int userAdded: User }
interface Node { id: ID! }
type User implements Node { id: ID! name: String role: Role best: User friends: [User!] }
type Post { title: String }
union SearchResult = User | Post
enum Role { ADMIN MEMBER }
";

fn p(path: &str) -> FieldPath {
    FieldPath::new(path.split('.'))
}

fn tick(q: &str, path: &str) -> Option<String> {
    let s = Schema::from_sdl(SDL).unwrap();
    let edit = toggle_field(q, &s, &p(path), true)?;
    let mut out = q.to_owned();
    out.replace_range(edit.range.clone(), &edit.text);
    Some(out)
}

#[test]
fn a_document_without_an_operation_gets_one() {
    let rows = [
        ("", "Query.version", "query {\n  version\n}"),
        (
            "",
            "Query.me.best.id",
            "query {\n  me {\n    best {\n      id\n    }\n  }\n}",
        ),
        ("  \n", "Subscription.tick", "subscription {\n  tick\n}"),
        (
            "# notes\n",
            "Query.version",
            "# notes\n\nquery {\n  version\n}",
        ),
        (
            "fragment F on User { id }",
            "Mutation.ping",
            "fragment F on User { id }\n\nmutation {\n  ping\n}",
        ),
    ];
    for (q, path, want) in rows {
        assert_eq!(tick(q, path).as_deref(), Some(want), "{q:?} {path}");
    }
}

#[test]
fn multi_line_sets_get_a_new_line_with_the_sibling_indent() {
    let rows = [
        (
            "{\n  me {\n    id\n  }\n}",
            "Query.me.name",
            "{\n  me {\n    id\n    name\n  }\n}",
        ),
        (
            "query Q {\n  version\n}",
            "Query.me.name",
            "query Q {\n  version\n  me {\n    name\n  }\n}",
        ),
        (
            "{\n  me {\n    id\n  }\n}",
            "Query.version",
            "{\n  me {\n    id\n  }\n  version\n}",
        ),
        (
            "{\n\tme {\n\t\tid\n\t}\n}",
            "Query.me.name",
            "{\n\tme {\n\t\tid\n\t\tname\n\t}\n}",
        ),
        (
            "{\n\tversion\n}",
            "Query.me.id",
            "{\n\tversion\n\tme {\n\t  id\n\t}\n}",
        ),
        (
            "{\n  version # v\n}",
            "Query.users.id",
            "{\n  version # v\n  users {\n    id\n  }\n}",
        ),
        (
            "{\n  me { id }\n  version }",
            "Query.users.id",
            "{\n  me { id }\n  version\n  users {\n    id\n  } }",
        ),
    ];
    for (q, path, want) in rows {
        assert_eq!(tick(q, path).as_deref(), Some(want), "{q:?} {path}");
    }
}

#[test]
fn one_line_sets_stay_on_one_line() {
    assert_eq!(
        tick("{ me { id } }", "Query.me.name").as_deref(),
        Some("{ me { id name } }")
    );
    assert_eq!(
        tick("{ version }", "Query.me.best.id").as_deref(),
        Some("{ version me { best { id } } }")
    );
}

#[test]
fn a_newline_inside_a_placeholder_does_not_count_as_a_line() {
    assert_eq!(
        tick("{ me(x: {{ a\nb }}) { id } }", "Query.version").as_deref(),
        Some("{ me(x: {{ a\nb }}) { id } version }")
    );
    assert_eq!(
        tick("{\n  version(x: {{ a\nb }}) me { id }\n}", "Query.users.id").as_deref(),
        Some("{\n  version(x: {{ a\nb }}) me { id }\n  users {\n    id\n  }\n}")
    );
}

#[test]
fn comments_placeholders_aliases_and_fragments_are_left_alone() {
    let q = "# top {{x}}\nquery Q($id: ID) {\n  user(id: {{id}}) {\n    id # c\n  }\n}";
    assert_eq!(
        tick(q, "Query.user.name").as_deref(),
        Some("# top {{x}}\nquery Q($id: ID) {\n  user(id: {{id}}) {\n    id # c\n    name\n  }\n}")
    );
    assert_eq!(
        tick("{ v: version }", "Query.version").as_deref(),
        Some("{ v: version version }")
    );
    assert_eq!(
        tick("{ ...F }\nfragment F on Query { version }", "Query.version").as_deref(),
        Some("{ ...F version }\nfragment F on Query { version }")
    );
}

#[test]
fn nothing_to_write_gives_none() {
    let big = format!("{{ version }}{}", " ".repeat(QUERY_VALIDATE_LIMIT));
    for (q, path) in [
        ("{ version }", "Query.version"),
        ("", "Query.me"),
        ("", "Query"),
        ("", "Query.nope"),
        ("", "Query.version.x"),
        ("", "Query.search"),
        ("", "Query.search.id"),
        ("{ version }", "Mutation.ping"),
        ("subscription { tick }", "Subscription.userAdded.id"),
        ("{ version", "Query.me.id"),
        (big.as_str(), "Query.me.id"),
    ] {
        assert_eq!(tick(q, path), None, "{path}");
    }
    let bare = Schema::from_sdl("type Query { a: Int }").unwrap();
    assert_eq!(toggle_field("", &bare, &p("Mutation.a"), true), None);
}
