//! The builder's rows from a schema, the query outline and the expanded paths; lock texts
//! and where the caret goes after an edit.

use std::collections::HashSet;

use apinata_app::graphql_builder::{BROKEN_NOTE, BuilderRow, RowKind, caret_after, rows};
use apinata_core::{FieldPath, Schema, query_outline};

fn schema() -> Schema {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../apinata-core/tests/fixtures/graphql/schema.json"
    ))
    .unwrap();
    Schema::from_json(&text).unwrap()
}

fn p(path: &str) -> FieldPath {
    FieldPath::new(path.split('.'))
}

type Seen = (usize, String, String, RowKind, bool, Option<String>);

fn seen(rows: &[BuilderRow]) -> Vec<Seen> {
    rows.iter()
        .map(|r| {
            (
                r.depth,
                r.label.clone(),
                r.ty.clone(),
                r.kind,
                r.checked,
                r.lock.clone(),
            )
        })
        .collect()
}

fn row(
    depth: usize,
    label: &str,
    ty: &str,
    kind: RowKind,
    checked: bool,
    lock: Option<&str>,
) -> Seen {
    (
        depth,
        label.into(),
        ty.into(),
        kind,
        checked,
        lock.map(str::to_owned),
    )
}

fn open(paths: &[&str]) -> HashSet<FieldPath> {
    paths.iter().map(|x| p(x)).collect()
}

const OTHER: &str = "One operation per request";

#[test]
fn collapsed_roots_only() {
    let s = schema();
    let o = query_outline("{ users { name } }", &s);
    assert_eq!(
        seen(&rows(&s, o.as_ref(), &HashSet::new(), false)),
        [
            row(0, "query: Query", "", RowKind::Root, false, None),
            row(
                0,
                "mutation: Mutation",
                "",
                RowKind::Root,
                false,
                Some(OTHER)
            ),
        ]
    );
    let blank = query_outline("", &s);
    assert!(
        rows(&s, blank.as_ref(), &HashSet::new(), false)
            .iter()
            .all(|r| r.lock.is_none())
    );
}

#[test]
fn expanded_paths_list_fields_with_checked_states() {
    use RowKind::*;
    let s = schema();
    let o = query_outline("{ users { name } }", &s);
    let all = rows(&s, o.as_ref(), &open(&["Query", "Query.users"]), false);
    assert_eq!(
        seen(&all),
        [
            row(0, "query: Query", "", Root, false, None),
            row(1, "user", "User", Composite, false, None),
            row(1, "users", "[User!]!", Composite, true, None),
            row(2, "id", "ID!", Leaf, false, None),
            row(2, "name", "String", Leaf, true, None),
            row(2, "role", "Role", Leaf, false, None),
            row(2, "friends", "[User!]", Composite, false, None),
            row(1, "node", "Node", Composite, false, None),
            row(
                1,
                "search",
                "[SearchResult!]!",
                Union,
                false,
                Some("Union fields need inline fragments")
            ),
            row(0, "mutation: Mutation", "", Root, false, Some(OTHER)),
        ]
    );
    assert!(all[0].expanded && all[2].expanded && !all[1].expanded);
    assert_eq!(all[4].path, p("Query.users.name"));
    let with = rows(&s, o.as_ref(), &open(&["Query", "Query.users"]), true);
    let old: Vec<_> = with
        .iter()
        .filter(|r| r.deprecated)
        .map(|r| r.label.as_str())
        .collect();
    assert_eq!(old, ["legacyName", "oldUsers"]);
}

#[test]
fn protected_parents_and_broken_text_lock_rows() {
    let s = schema();
    let o = query_outline("{ user(id: 1) { name } }", &s);
    let r = rows(&s, o.as_ref(), &open(&["Query", "Query.user"]), false);
    let name = r.iter().find(|r| r.path == p("Query.user.name")).unwrap();
    assert_eq!(
        name.lock.as_deref(),
        Some("Last field of `user`; untick `user` to remove it")
    );
    let o = query_outline("query Q { users { name } }", &s);
    let r = rows(&s, o.as_ref(), &open(&["Query", "Query.users"]), false);
    let name = r.iter().find(|r| r.path == p("Query.users.name")).unwrap();
    assert_eq!(
        name.lock.as_deref(),
        Some("Last field of operation `Q`; edit the query to remove it")
    );
    let o = query_outline("query user { user(id: 1) { name } }", &s);
    let r = rows(&s, o.as_ref(), &open(&["Query", "Query.user"]), false);
    let name = r.iter().find(|r| r.path == p("Query.user.name")).unwrap();
    assert_eq!(
        name.lock.as_deref(),
        Some("Last field of `user`; untick `user` to remove it")
    );
    let r = rows(&s, None, &open(&["Query", "Query.users"]), false);
    assert!(
        r.iter()
            .all(|r| r.lock.as_deref() == Some(BROKEN_NOTE) && !r.checked)
    );
}

#[test]
fn the_caret_stays_shifts_or_moves_to_the_start() {
    assert_eq!(caret_after(3, 5..5, 4), 3);
    assert_eq!(caret_after(5, 5..5, 4), 5);
    assert_eq!(caret_after(9, 5..5, 4), 13);
    assert_eq!(caret_after(12, 5..10, 0), 7);
    assert_eq!(caret_after(10, 5..10, 0), 5);
    assert_eq!(caret_after(7, 5..10, 0), 5);
    assert_eq!(caret_after(2, 5..10, 0), 2);
}
