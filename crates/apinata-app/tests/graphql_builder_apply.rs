//! Builder ticks write to the query editor: one undoable edit, caret kept, stale edits dropped.

use crate::support;

use apinata_app::command::Edit;
use apinata_app::graphql_builder::BuilderPanel;
use apinata_app::graphql_schema_store::SchemaStore;
use apinata_app::graphql_section::GraphqlSection;
use apinata_core::{BodySpec, FieldPath, Schema, toggle_field};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use support::*;

const URL: &str = "http://apply.test/graphql";

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

struct Open {
    win: AnyWindowHandle,
    host: Entity<SectionHost<GraphqlSection>>,
    s: Entity<GraphqlSection>,
    b: Entity<BuilderPanel>,
}

// A section with the fixture schema showing `query`, the builder open with `expanded` paths.
fn open(cx: &mut TestAppContext, query: &str, expanded: &[&str]) -> Open {
    let (win, host) = host_section(cx, GraphqlSection::new);
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update(|cx| {
        let store = cx.default_global::<SchemaStore>();
        let g = store.begin(URL);
        store.finish(URL, g, Ok(schema()));
    });
    let spec = BodySpec::Graphql {
        query: query.into(),
        variables: String::new(),
    };
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| {
            s.set_schema_url(URL.into(), cx);
            s.load(&spec, w, cx);
        })
    })
    .unwrap();
    cx.update_window(win, |_, w, cx| w.click("graphql-builder-toggle", cx))
        .unwrap();
    let b = s.read_with(cx, |s, _| s.builder().clone());
    for path in expanded {
        b.update(cx, |b, cx| b.toggle_expanded(&p(path), cx));
    }
    frame(cx, win);
    Open { win, host, s, b }
}

fn query(cx: &mut TestAppContext, o: &Open) -> String {
    let q = o.s.read_with(cx, |s, _| s.query().clone());
    q.read_with(cx, |e, _| e.value().to_string())
}

fn caret(cx: &mut TestAppContext, o: &Open) -> usize {
    let q = o.s.read_with(cx, |s, _| s.query().clone());
    q.read_with(cx, |e, _| e.cursor())
}

fn set_caret(cx: &mut TestAppContext, o: &Open, at: usize) {
    let q = o.s.read_with(cx, |s, _| s.query().clone());
    q.update(cx, |e, cx| e.set_selected_range(at..at, cx));
}

fn tick(cx: &mut TestAppContext, o: &Open, path: &str) {
    let ix =
        o.b.read_with(cx, |b, _| b.rows().iter().position(|r| r.path == p(path)));
    let ix = ix.unwrap_or_else(|| panic!("no row for {path}"));
    cx.update_window(o.win, |_, w, cx| w.click(("graphql-builder-check", ix), cx))
        .unwrap();
    frame(cx, o.win);
}

#[gpui_kit::test]
fn a_tick_writes_one_undoable_edit(cx: &mut TestAppContext) {
    let o = open(cx, "{ users { id } }", &["Query", "Query.users"]);
    tick(cx, &o, "Query.users.name");
    assert_eq!(query(cx, &o), "{ users { id name } }");
    let last = o.host.read_with(cx, |h, _| match h.edits.last() {
        Some(Edit::Body(BodySpec::Graphql { query, .. })) => Some(query.clone()),
        _ => None,
    });
    assert_eq!(last.as_deref(), Some("{ users { id name } }"));
    assert!(o.b.read_with(cx, |b, _| {
        b.rows()
            .iter()
            .any(|r| r.path == p("Query.users.name") && r.checked)
    }));
    cx.update_window(o.win, |_, w, cx| {
        w.click_at("graphql-query", code_at(), cx);
        w.press(&key("z"), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(query(cx, &o), "{ users { id } }");
}

#[gpui_kit::test]
fn unticking_a_composite_removes_it_and_an_unchecked_one_only_expands(cx: &mut TestAppContext) {
    let o = open(cx, "{ users { id } node(id: 1) { id } }", &["Query"]);
    tick(cx, &o, "Query.users");
    assert_eq!(query(cx, &o), "{ node(id: 1) { id } }");
    tick(cx, &o, "Query.user");
    assert_eq!(query(cx, &o), "{ node(id: 1) { id } }");
    assert!(o.b.read_with(cx, |b, _| b.is_expanded(&p("Query.user"))));
}

#[gpui_kit::test]
fn the_caret_is_kept(cx: &mut TestAppContext) {
    let o = open(
        cx,
        "{\n  users {\n    id\n  }\n}",
        &["Query", "Query.users"],
    );
    set_caret(cx, &o, 1);
    tick(cx, &o, "Query.users.name");
    assert_eq!(query(cx, &o), "{\n  users {\n    id\n    name\n  }\n}");
    assert_eq!(caret(cx, &o), 1);
    let end = query(cx, &o).len();
    set_caret(cx, &o, end);
    tick(cx, &o, "Query.users.role");
    assert_eq!(caret(cx, &o), query(cx, &o).len());
}

#[gpui_kit::test]
fn a_caret_inside_the_removed_range_moves_to_its_start(cx: &mut TestAppContext) {
    let o = open(cx, "{ users { id name } }", &["Query", "Query.users"]);
    set_caret(cx, &o, 15);
    tick(cx, &o, "Query.users.name");
    assert_eq!(query(cx, &o), "{ users { id } }");
    assert_eq!(caret(cx, &o), 12);
}

#[gpui_kit::test]
fn a_stale_edit_is_dropped(cx: &mut TestAppContext) {
    let o = open(cx, "{ users { id } }", &["Query"]);
    let edit = toggle_field("{ users { id } }", &schema(), &p("Query.users.name"), true).unwrap();
    let b = o.b.clone();
    let applied = cx
        .update_window(o.win, |_, w, cx| {
            b.update(cx, |b, cx| b.apply("{ users { role } }", edit, w, cx))
        })
        .unwrap();
    assert!(!applied);
    assert_eq!(query(cx, &o), "{ users { id } }");
}

#[gpui_kit::test]
fn broken_text_ignores_ticks(cx: &mut TestAppContext) {
    let o = open(cx, "{ users {", &["Query"]);
    tick(cx, &o, "Query.users");
    assert_eq!(query(cx, &o), "{ users {");
}
