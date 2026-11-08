//! `toggle_field` unticking: removal ranges, one copy per tick, the empty-parent cascade
//! and the protected nodes it stops at.

use apinata_core::{FieldPath, Schema, query_outline, toggle_field};

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

fn untick(q: &str, path: &str) -> Option<String> {
    let s = Schema::from_sdl(SDL).unwrap();
    // Toggling through a built outline must give the same edit as from the text.
    let via_outline = query_outline(q, &s).and_then(|o| o.toggle(&p(path), false));
    let edit = toggle_field(q, &s, &p(path), false);
    assert_eq!(via_outline, edit, "{q:?} {path}");
    let edit = edit?;
    assert!(edit.text.is_empty(), "a removal writes nothing");
    let mut out = q.to_owned();
    out.replace_range(edit.range.clone(), "");
    Some(out)
}

fn check(rows: &[(&str, &str, &str)]) {
    for (q, path, want) in rows {
        assert_eq!(untick(q, path).as_deref(), Some(*want), "{q:?} {path}");
    }
}

#[test]
fn a_field_alone_on_its_lines_takes_the_whole_lines() {
    check(&[
        (
            "{\n  me {\n    id\n    name\n  }\n}",
            "Query.me.name",
            "{\n  me {\n    id\n  }\n}",
        ),
        (
            "{\n  me {\n    id, # the id\n    name\n  }\n}",
            "Query.me.id",
            "{\n  me {\n    name\n  }\n}",
        ),
        (
            "{\n  me {\n    id\n  }\n  version\n}",
            "Query.me",
            "{\n  version\n}",
        ),
        (
            "{\n  user(id: 1) {\n    name\n  }\n  version\n}",
            "Query.user",
            "{\n  version\n}",
        ),
        (
            "{\r\n  me {\r\n    id\r\n    name\r\n  }\r\n}",
            "Query.me.name",
            "{\r\n  me {\r\n    id\r\n  }\r\n}",
        ),
    ]);
}

#[test]
fn a_field_sharing_its_line_takes_one_separator() {
    check(&[
        ("{ me { id name } }", "Query.me.id", "{ me { name } }"),
        ("{ me { id name } }", "Query.me.name", "{ me { id } }"),
        ("{ me { id, name } }", "Query.me.id", "{ me { name } }"),
        ("{ me { id, name } }", "Query.me.name", "{ me { id } }"),
        (
            "{ me {\n    id name # c\n  } }",
            "Query.me.name",
            "{ me {\n    id # c\n  } }",
        ),
        (
            "{ me {\r\n    id name\r\n  } }",
            "Query.me.name",
            "{ me {\r\n    id\r\n  } }",
        ),
        (
            "query Q { me { id } version }",
            "Query.me",
            "query Q { version }",
        ),
        (
            "{ me { ...F name } }\nfragment F on User { id }",
            "Query.me.name",
            "{ me { ...F } }\nfragment F on User { id }",
        ),
    ]);
}

#[test]
fn emptied_parents_and_operations_go_too() {
    check(&[
        (
            "{\n  version\n  me {\n    best {\n      id\n    }\n  }\n}",
            "Query.me.best.id",
            "{\n  version\n}",
        ),
        (
            "{ me { best { id } } version }",
            "Query.me.best.id",
            "{ version }",
        ),
        ("{ version }", "Query.version", ""),
        ("# c\nquery {\n  version\n}\n", "Query.version", "# c\n"),
        ("subscription {\n  tick\n}", "Subscription.tick", ""),
        (
            "fragment F on User { id }\n\n{ version }",
            "Query.version",
            "fragment F on User { id }\n\n",
        ),
    ]);
}

#[test]
fn one_copy_goes_per_tick_and_aliases_stay() {
    let once = untick("{ me { id } version version }", "Query.version").unwrap();
    assert_eq!(once, "{ me { id } version }");
    assert_eq!(
        untick(&once, "Query.version").as_deref(),
        Some("{ me { id } }")
    );
    assert_eq!(
        untick("{ v: version version }", "Query.version").as_deref(),
        Some("{ v: version }")
    );
    assert_eq!(untick("{ v: version }", "Query.version"), None);
}

#[test]
fn comments_and_placeholders_around_the_edit_are_kept() {
    check(&[(
        "# {{a}}\n{\n  user(id: {{id}}) { # keep\n    id\n    name\n  }\n}",
        "Query.user.name",
        "# {{a}}\n{\n  user(id: {{id}}) { # keep\n    id\n  }\n}",
    )]);
}

#[test]
fn protected_and_unselected_paths_give_none() {
    for (q, path) in [
        ("{ user(id: 1) { name } }", "Query.user.name"),
        ("{ user(id: 1) { best { id } } }", "Query.user.best.id"),
        ("query Q { version }", "Query.version"),
        ("{ version }", "Query.me.id"),
        ("{ version", "Query.version"),
    ] {
        assert_eq!(untick(q, path), None, "{q:?} {path}");
    }
}

#[test]
fn a_selected_union_can_be_unticked_but_never_ticked() {
    check(&[(
        "{ search { ... on User { id } } version }",
        "Query.search",
        "{ version }",
    )]);
    let s = Schema::from_sdl(SDL).unwrap();
    assert_eq!(
        toggle_field("{ version }", &s, &p("Query.search"), true),
        None
    );
}

#[test]
fn a_last_field_before_an_inline_closer_takes_its_line_break() {
    check(&[
        (
            "{\n  version\n  me {\n    id\n  } }",
            "Query.me",
            "{\n  version }",
        ),
        (
            "{\r\n  version\r\n  me {\r\n    id\r\n  } }",
            "Query.me",
            "{\r\n  version }",
        ),
        (
            "{\n  version # c\n  me {\n    id\n  } }",
            "Query.me",
            "{\n  version # c\n }",
        ),
    ]);
}
