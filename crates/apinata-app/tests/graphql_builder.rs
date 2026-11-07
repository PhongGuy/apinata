//! The builder panel in the GraphQL section: the side-slot toggles, the schema states,
//! and expansion that survives text edits.

use crate::support;

use apinata_app::graphql_builder::BuilderPanel;
use apinata_app::graphql_schema_store::SchemaStore;
use apinata_app::graphql_section::{GraphqlSection, SidePanel};
use apinata_core::{BodySpec, FieldPath, Schema};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use support::*;

const URL: &str = "http://builder.test/graphql";

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

fn load(cx: &mut TestAppContext, win: AnyWindowHandle, s: &Entity<GraphqlSection>, query: &str) {
    let spec = BodySpec::Graphql {
        query: query.into(),
        variables: String::new(),
    };
    cx.update_window(win, |_, w, cx| s.update(cx, |s, cx| s.load(&spec, w, cx)))
        .unwrap();
    frame(cx, win);
}

// A section on URL showing `query`; `loaded` puts the fixture schema in the store first.
fn section(
    cx: &mut TestAppContext,
    query: &str,
    loaded: bool,
) -> (AnyWindowHandle, Entity<GraphqlSection>) {
    let (win, host) = host_section(cx, GraphqlSection::new);
    let s = host.read_with(cx, |h, _| h.view.clone());
    if loaded {
        cx.update(|cx| {
            let store = cx.default_global::<SchemaStore>();
            let g = store.begin(URL);
            store.finish(URL, g, Ok(schema()));
        });
    }
    s.update(cx, |s, cx| s.set_schema_url(URL.into(), cx));
    load(cx, win, &s, query);
    (win, s)
}

fn click(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<gpui_kit::ElementId>) {
    let id = id.into();
    cx.update_window(win, |_, w, cx| w.click(id, cx)).unwrap();
    frame(cx, win);
}

fn builder(cx: &mut TestAppContext, s: &Entity<GraphqlSection>) -> Entity<BuilderPanel> {
    s.read_with(cx, |s, _| s.builder().clone())
}

fn labels(cx: &mut TestAppContext, b: &Entity<BuilderPanel>) -> Vec<String> {
    b.read_with(cx, |b, _| {
        b.rows().iter().map(|r| r.label.clone()).collect()
    })
}

#[gpui_kit::test]
fn builder_and_docs_toggles_close_each_other(cx: &mut TestAppContext) {
    let (win, s) = section(cx, "{ users { id } }", true);
    let side = |cx: &mut TestAppContext| s.read_with(cx, |s, _| s.side());
    assert_eq!(side(cx), SidePanel::Off);
    click(cx, win, "graphql-builder-toggle");
    assert_eq!(side(cx), SidePanel::Builder);
    assert!(present(cx, win, "graphql-builder") && !present(cx, win, "graphql-docs"));
    click(cx, win, "graphql-docs-toggle");
    assert_eq!(side(cx), SidePanel::Docs);
    assert!(present(cx, win, "graphql-docs") && !present(cx, win, "graphql-builder"));
    click(cx, win, "graphql-docs-toggle");
    assert_eq!(side(cx), SidePanel::Off);
    click(cx, win, "graphql-builder-toggle");
    click(cx, win, "graphql-builder-toggle");
    assert_eq!(side(cx), SidePanel::Off);
    assert!(!present(cx, win, "graphql-builder"));
}

#[gpui_kit::test]
fn the_builder_shows_the_schema_states(cx: &mut TestAppContext) {
    let (win, _s) = section(cx, "", false);
    click(cx, win, "graphql-builder-toggle");
    assert!(present(cx, win, "graphql-builder-load"));
    cx.update(|cx| {
        cx.default_global::<SchemaStore>().begin(URL);
    });
    frame(cx, win);
    assert!(present(cx, win, "graphql-builder-loading"));
}

#[gpui_kit::test]
fn roots_expand_on_demand_and_stay_expanded_across_edits(cx: &mut TestAppContext) {
    let (win, s) = section(cx, "{ users { id } }", true);
    click(cx, win, "graphql-builder-toggle");
    let b = builder(cx, &s);
    assert_eq!(labels(cx, &b), ["query: Query", "mutation: Mutation"]);
    click(cx, win, ("graphql-builder-chevron", 0usize));
    assert!(b.read_with(cx, |b, _| b.is_expanded(&p("Query"))));
    assert_eq!(
        labels(cx, &b),
        [
            "query: Query",
            "user",
            "users",
            "node",
            "search",
            "mutation: Mutation"
        ]
    );
    load(cx, win, &s, "{ node(id: 1) { id } }");
    assert_eq!(labels(cx, &b).len(), 6);
    let checked: Vec<bool> = b.read_with(cx, |b, _| b.rows().iter().map(|r| r.checked).collect());
    assert_eq!(checked, [false, false, false, true, false, false]);
}

#[gpui_kit::test]
fn broken_text_shows_the_note(cx: &mut TestAppContext) {
    let (win, _s) = section(cx, "{ users {", true);
    click(cx, win, "graphql-builder-toggle");
    assert!(present(cx, win, "graphql-builder-note"));
}

#[gpui_kit::test]
fn each_field_row_shows_its_type(cx: &mut TestAppContext) {
    let (win, s) = section(cx, "{ users { id } }", true);
    click(cx, win, "graphql-builder-toggle");
    click(cx, win, ("graphql-builder-chevron", 0usize));
    let b = builder(cx, &s);
    let (ix, ty) = b.read_with(cx, |b, _| {
        let ix = b
            .rows()
            .iter()
            .position(|r| r.path == p("Query.users"))
            .unwrap();
        (ix, b.rows()[ix].ty.clone())
    });
    assert!(!ty.is_empty());
    let shown = cx
        .update_window(win, |_, w, _| {
            w.find(("graphql-builder-type", ix))
                .label()
                .map(str::to_owned)
        })
        .unwrap();
    assert_eq!(shown.as_deref(), Some(ty.as_str()));
}
