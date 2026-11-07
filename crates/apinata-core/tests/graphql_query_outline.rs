//! `query_outline`: which field paths a query selects, its operation root, and the locks.

use apinata_core::{FieldPath, Lock, QUERY_VALIDATE_LIMIT, Schema, query_outline};

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

fn schema() -> Schema {
    Schema::from_sdl(SDL).unwrap()
}

fn p(path: &str) -> FieldPath {
    FieldPath::new(path.split('.'))
}

fn last(of: &str) -> Option<Lock> {
    Some(Lock::LastChildOf(of.into()))
}

fn op(of: &str) -> Option<Lock> {
    Some(Lock::LastOfOperation(of.into()))
}

#[test]
fn selected_paths_are_unaliased_fields_outside_fragments() {
    let s = schema();
    let q = "query { me { name best { id } } v: version ...F ... on Query { users { id } } }\n\
             fragment F on Query { user { id } }";
    let o = query_outline(q, &s).unwrap();
    assert_eq!(o.operation_root(), Some("Query"));
    for yes in [
        "Query.me",
        "Query.me.name",
        "Query.me.best",
        "Query.me.best.id",
    ] {
        assert!(o.selects(&p(yes)), "{yes}");
    }
    for no in [
        "Query.version",
        "Query.users",
        "Query.users.id",
        "Query.user",
        "Query.me.id",
        "Mutation.ping",
    ] {
        assert!(!o.selects(&p(no)), "{no}");
    }
}

#[test]
fn placeholder_names_are_never_selected() {
    let s = schema();
    let o = query_outline("{ me { id } {{who}} { name } }", &s).unwrap();
    assert!(o.selects(&p("Query.me.id")));
    assert!(!o.selects(&p("Query.me.name")));
}

#[test]
fn the_first_operation_gives_the_root() {
    let s = schema();
    let root = |q: &str| {
        query_outline(q, &s)
            .unwrap()
            .operation_root()
            .map(str::to_owned)
    };
    assert_eq!(root("{ me { id } }").as_deref(), Some("Query"));
    assert_eq!(root("mutation M { ping }").as_deref(), Some("Mutation"));
    assert_eq!(
        root("subscription { tick }\nquery { version }").as_deref(),
        Some("Subscription")
    );
    assert_eq!(
        root("fragment F on User { id }\n{ version }").as_deref(),
        Some("Query")
    );
    for none in ["", "  # c\n", "fragment F on User { id }"] {
        let o = query_outline(none, &s).unwrap();
        assert_eq!(o.operation_root(), None, "{none:?}");
        for path in ["Query.version", "Mutation.ping", "Subscription.tick.x"] {
            assert_eq!(o.locked(&p(path)), None, "{none:?} {path}");
        }
    }
}

#[test]
fn broken_or_oversized_text_has_no_outline() {
    let s = schema();
    assert!(query_outline("{ me { id }", &s).is_none());
    let big = format!("{{ version }}{}", " ".repeat(QUERY_VALIDATE_LIMIT));
    assert!(query_outline(&big, &s).is_none());
}

#[test]
fn other_roots_unions_and_subscription_fields_are_locked() {
    let s = schema();
    let o = query_outline("{ me { id } }", &s).unwrap();
    assert_eq!(o.locked(&p("Mutation")), Some(Lock::OtherRoot));
    assert_eq!(o.locked(&p("Mutation.ping")), Some(Lock::OtherRoot));
    assert_eq!(o.locked(&p("Query.version")), None);
    assert_eq!(o.locked(&p("Query.me.id")), None);
    assert_eq!(o.locked(&p("Query.search")), Some(Lock::Union));
    let sub = query_outline("subscription { tick }", &s).unwrap();
    assert_eq!(sub.locked(&p("Subscription.tick")), None);
    assert_eq!(
        sub.locked(&p("Subscription.userAdded")),
        Some(Lock::SubscriptionField)
    );
    assert_eq!(
        sub.locked(&p("Subscription.userAdded.id")),
        Some(Lock::SubscriptionField)
    );
    assert_eq!(sub.locked(&p("Query.version")), Some(Lock::OtherRoot));
    let bare = Schema::from_sdl("type Query { a: Int }").unwrap();
    let unset = query_outline("mutation { a }", &bare).unwrap();
    assert_eq!(unset.operation_root(), None);
    assert_eq!(unset.locked(&p("Query.a")), Some(Lock::OtherRoot));
}

#[test]
fn the_last_child_of_a_protected_node_is_locked() {
    let s = schema();
    let lock = |q: &str, path: &str| query_outline(q, &s).unwrap().locked(&p(path));
    assert_eq!(
        lock("{ user(id: 1) { name } }", "Query.user.name"),
        last("user")
    );
    assert_eq!(lock("{ user(id: 1) { name } }", "Query.user"), None);
    assert_eq!(lock("{ user(id: 1) { name } }", "Query.user.id"), None);
    assert_eq!(
        lock("{ user(id: 1) { best { id } } }", "Query.user.best.id"),
        last("user")
    );
    assert_eq!(
        lock("{ user(id: 1) { best { id } } }", "Query.user.best"),
        last("user")
    );
    assert_eq!(
        lock("{ me @include(if: true) { id } }", "Query.me.id"),
        last("me")
    );
    assert_eq!(lock("{ user(id: 1) { id name } }", "Query.user.name"), None);
    assert_eq!(
        lock(
            "{ user(id: 1) { ...F name } }\nfragment F on User { id }",
            "Query.user.name"
        ),
        None
    );
    assert_eq!(lock("query Q { me { id } }", "Query.me"), op("Q"));
    assert_eq!(lock("query Q { me { id } }", "Query.me.id"), op("Q"));
    assert_eq!(
        lock("query ($n: Int) { version }", "Query.version"),
        op("query")
    );
    assert_eq!(
        lock("query user { user(id: 1) { name } }", "Query.user.name"),
        last("user")
    );
    assert_eq!(lock("query user { me { id } }", "Query.me.id"), op("user"));
    assert_eq!(lock("query Q { me version }", "Query.me"), None);
    assert_eq!(lock("{ version }", "Query.version"), None);
}
