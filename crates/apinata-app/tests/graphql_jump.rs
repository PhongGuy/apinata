//! Jumping from the query editor to the Docs panel: the providers, the handler, `show_docs`.

use crate::support;

use apinata_app::graphql_docs::DocsView;
use apinata_app::graphql_schema_store::SchemaStore;
use apinata_app::graphql_section::GraphqlSection;
use apinata_core::{BodySpec, Schema};
use gpui_kit::component::input::Position;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use lsp_types::ShowDocumentParams;
use support::*;

const URL: &str = "http://jump.test/graphql";
// `user` 2..6, `id` 7..9, `{` at 14, `name` 16..20.
const Q: &str = "{ user(id: 1) { name } }";

fn schema() -> Schema {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../apinata-core/tests/fixtures/graphql/schema.json"
    ))
    .unwrap();
    Schema::from_json(&text).unwrap()
}

// A section whose URL has the fixture schema loaded, showing `Q`.
fn section(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<GraphqlSection>) {
    let (win, host) = host_section(cx, GraphqlSection::new);
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update(|cx| {
        let store = cx.default_global::<SchemaStore>();
        let g = store.begin(URL);
        store.finish(URL, g, Ok(schema()));
    });
    let spec = BodySpec::Graphql {
        query: Q.into(),
        variables: String::new(),
    };
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| {
            s.set_schema_url(URL.into(), cx);
            s.load(&spec, w, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    (win, s)
}

fn docs(cx: &mut TestAppContext, s: &Entity<GraphqlSection>) -> (bool, DocsView) {
    s.read_with(cx, |s, cx| {
        (s.docs_open(), s.docs().read(cx).view().clone())
    })
}

fn params(uri: &str) -> ShowDocumentParams {
    ShowDocumentParams {
        uri: uri.parse().unwrap(),
        external: Some(false),
        take_focus: Some(true),
        selection: None,
    }
}

#[gpui_kit::test]
fn only_the_query_editor_gets_the_providers(cx: &mut TestAppContext) {
    let (_win, s) = section(cx);
    let (query, variables) = s.read_with(cx, |s, _| (s.query().clone(), s.variables().clone()));
    assert!(query.read_with(cx, |q, _| {
        let l = q.lsp();
        l.hover_provider.is_some() && l.definition_provider.is_some() && l.show_document.is_some()
    }));
    assert!(variables.read_with(cx, |v, _| {
        let l = v.lsp();
        l.hover_provider.is_none() && l.definition_provider.is_none() && l.show_document.is_none()
    }));
}

#[gpui_kit::test]
fn show_docs_opens_the_panel_on_the_view(cx: &mut TestAppContext) {
    let (win, s) = section(cx);
    assert_eq!(docs(cx, &s), (false, DocsView::Root));
    assert!(!present(cx, win, "graphql-docs"));
    let view = DocsView::Field {
        owner: "User".into(),
        field: "name".into(),
    };
    s.update(cx, |s, cx| s.show_docs(view.clone(), cx));
    assert_eq!(docs(cx, &s), (true, view));
    assert!(present(cx, win, "graphql-docs"));
}

#[gpui_kit::test]
fn the_show_document_handler_takes_only_docs_uris(cx: &mut TestAppContext) {
    let (win, s) = section(cx);
    let query = s.read_with(cx, |s, _| s.query().clone());
    let handler = query.read_with(cx, |q, _| q.lsp().show_document.clone().expect("a handler"));
    let shown = cx
        .update_window(win, |_, w, cx| {
            handler(&params("https://example.com/x"), w, cx)
        })
        .unwrap();
    assert!(!shown);
    assert_eq!(docs(cx, &s), (false, DocsView::Root));
    let shown = cx
        .update_window(win, |_, w, cx| {
            handler(&params("apinata-docs:type/User"), w, cx)
        })
        .unwrap();
    assert!(shown);
    assert_eq!(docs(cx, &s), (true, DocsView::Type("User".into())));
}

// Puts the caret at `column` of the query's only line, then presses F12 there.
fn f12_at(cx: &mut TestAppContext, win: AnyWindowHandle, s: &Entity<GraphqlSection>, column: u32) {
    use gpui_kit::test::TestWindowExt as _;
    let query = s.read_with(cx, |s, _| s.query().clone());
    cx.update_window(win, |_, w, cx| {
        query.update(cx, |q, cx| {
            q.set_cursor_position(Position::new(0, column), w, cx)
        });
        w.render_frame(cx);
        w.press("f12", cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn f12_on_a_name_opens_its_docs_page(cx: &mut TestAppContext) {
    let (win, s) = section(cx);
    cx.update(apinata_app::keymap::init);
    f12_at(cx, win, &s, 14);
    assert_eq!(docs(cx, &s), (false, DocsView::Root), "no name before `{{`");
    f12_at(cx, win, &s, 18);
    assert_eq!(
        docs(cx, &s),
        (
            true,
            DocsView::Field {
                owner: "User".into(),
                field: "name".into()
            }
        )
    );
    f12_at(cx, win, &s, 8);
    assert_eq!(
        docs(cx, &s),
        (
            true,
            DocsView::Field {
                owner: "Query".into(),
                field: "user".into()
            }
        ),
        "an argument jumps to its field"
    );
}

#[gpui_kit::test]
fn f12_in_the_variables_editor_opens_no_docs(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let (win, s) = section(cx);
    cx.update(apinata_app::keymap::init);
    let variables = s.read_with(cx, |s, _| s.variables().clone());
    cx.update_window(win, |_, w, cx| {
        variables.update(cx, |v, cx| v.focus(w, cx));
        w.render_frame(cx);
        w.press("f12", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(docs(cx, &s), (false, DocsView::Root));
}
