//! The schema store: generations, shared state across tabs, file loads and inline errors.

use crate::support;

use apinata_app::graphql_schema_store::{
    CheckKey, Lookup, SchemaState, SchemaStore, compiled, state,
};
use apinata_app::request_editor::Section;
use apinata_app::schema_load::load_from_file;
use apinata_core::{CompiledSchema, Schema};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext};
use std::path::Path;
use std::sync::Arc;
use support::*;
use wiremock::ResponseTemplate;

fn fixture() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../apinata-core/tests/fixtures/graphql/schema.json"
    ))
    .unwrap()
}

fn gql(name: &str, url: &str) -> String {
    format!("info:\n  name: {name}\n  type: graphql\n  seq: 1\ngraphql:\n  url: \"{url}\"\n")
}

fn query_tab(cx: &mut TestAppContext, win: AnyWindowHandle) {
    cx.update_window(win, |_, w, cx| {
        w.click(("section", Section::Body as usize), cx);
        w.render_frame(cx)
    })
    .unwrap();
}

fn loaded(cx: &mut TestAppContext, url: &str) -> bool {
    matches!(cx.update(|cx| state(cx, url)), Some(SchemaState::Loaded(_)))
}

fn failed(cx: &mut TestAppContext, url: &str) -> Option<String> {
    match cx.update(|cx| state(cx, url)) {
        Some(SchemaState::Failed(message)) => Some(message),
        _ => None,
    }
}

#[test]
fn an_older_load_never_overwrites_a_newer_one() {
    let schema = Schema::from_json(&fixture()).unwrap();
    let mut s = SchemaStore::default();
    let first = s.begin("u");
    let second = s.begin("u");
    assert!(second > first);
    assert!(!s.finish("u", first, Err("old".into())));
    assert!(matches!(s.state("u"), Some(SchemaState::Loading)));
    assert!(s.finish("u", second, Ok(schema)));
    assert!(matches!(s.state("u"), Some(SchemaState::Loaded(_))));
    assert!(s.state("v").is_none());
    let third = s.begin("u");
    assert!(s.finish("u", third, Err("boom".into())));
    assert!(matches!(s.state("u"), Some(SchemaState::Failed(m)) if m == "boom"));
}

#[gpui_kit::test]
fn a_loaded_schema_is_shared_by_two_tabs_on_the_same_url(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200).set_body_string(fixture()));
    let url = format!("{}/graphql", server.uri());
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &gql("a", &url)),
        ("b.yml", &gql("b", &url)),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (wa, _) = editor_for(cx, &ws, &root, "a.yml", false);
    let (wb, _) = editor_for(cx, &ws, &root, "b.yml", false);
    query_tab(cx, wa);
    query_tab(cx, wb);
    assert!(present(cx, wa, "graphql-schema-load"));
    assert!(present(cx, wb, "graphql-schema-load"));
    cx.update_window(wa, |_, w, cx| w.click("graphql-schema-load", cx))
        .unwrap();
    wait_real(cx, |cx| loaded(cx, &url));
    assert!(present(cx, wa, "graphql-schema-refresh"));
    assert!(present(cx, wb, "graphql-schema-refresh"));
    assert_eq!(served(cx, &server), ["/graphql"]);
}

#[gpui_kit::test]
fn a_failed_load_shows_the_error_inline(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(500));
    let url = format!("{}/graphql", server.uri());
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &gql("a", &url))]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _) = editor_for(cx, &ws, &root, "a.yml", false);
    query_tab(cx, win);
    cx.update_window(win, |_, w, cx| w.click("graphql-schema-load", cx))
        .unwrap();
    wait_real(cx, |cx| failed(cx, &url).is_some());
    assert_eq!(
        failed(cx, &url).unwrap(),
        "the server answered 500 Internal Server Error"
    );
    assert!(present(cx, win, "graphql-schema-error"));
}

#[gpui_kit::test]
fn schema_files_load_with_or_without_the_wrapper(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let v: serde_json::Value = serde_json::from_str(&fixture()).unwrap();
    let files = [
        ("w.json", fixture()),
        ("b.json", serde_json::to_string(&v["data"]).unwrap()),
        ("h.json", "<html></html>".to_owned()),
        ("s.graphql", "type Query { me: String }".to_owned()),
    ];
    for (name, text) in &files {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    for (url, name) in [
        ("u1", "w.json"),
        ("u2", "b.json"),
        ("u3", "h.json"),
        ("u4", "missing.json"),
        ("u5", "s.graphql"),
    ] {
        let path = dir.path().join(name);
        cx.update(|cx| load_from_file(url.into(), path, cx));
    }
    wait_for(cx, "file loads", |cx| {
        ["u1", "u2", "u3", "u4", "u5"].iter().all(|u| {
            !matches!(
                cx.update(|cx| state(cx, u)),
                None | Some(SchemaState::Loading)
            )
        })
    });
    assert!(loaded(cx, "u1"));
    assert!(loaded(cx, "u2"));
    // Text that does not start like JSON is read as SDL, so markup fails as SDL at its start.
    assert!(
        failed(cx, "u3")
            .unwrap()
            .starts_with("the schema is not valid SDL at line 1, column 1")
    );
    assert!(loaded(cx, "u5"));
    assert!(failed(cx, "u4").unwrap().starts_with("could not read"));
}

// An SDL file picked through the section's file button fills the docs explorer and completion.
#[gpui_kit::test]
fn an_sdl_file_picked_in_the_section_loads_docs_and_completion(cx: &mut TestAppContext) {
    use apinata_app::graphql_complete::SchemaCompletions;
    use apinata_app::graphql_docs::{DocRow, DocsView, page};
    use gpui_kit::component::Rope;
    use std::cell::RefCell;
    use std::rc::Rc;

    let url = "http://sdl.test/graphql";
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &gql("a", url))]);
    let schema_file = col.path().join("schema.graphqls");
    std::fs::write(
        &schema_file,
        "schema { query: Root }\n\"The entry point.\"\ntype Root { viewer: Person }\ntype Person { name: String }\n",
    )
    .unwrap();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _) = editor_for(cx, &ws, &root, "a.yml", false);
    query_tab(cx, win);
    cx.update_window(win, |_, w, cx| w.click("graphql-schema-file", cx))
        .unwrap();
    wait_for(cx, "the schema file prompt", |cx| cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(move |o| {
        assert!(o.files && !o.directories && !o.multiple);
        Some(vec![schema_file])
    });
    wait_for(cx, "the SDL schema", |cx| loaded(cx, url));
    let Some(SchemaState::Loaded(schema)) = cx.update(|cx| state(cx, url)) else {
        panic!("no schema")
    };
    let root_page = page(&schema, &DocsView::Root, "", false);
    assert!(root_page.rows.contains(&DocRow::Link {
        label: "query: Root".into(),
        to: DocsView::Type("Root".into()),
    }));
    cx.update_window(win, |_, w, cx| {
        w.click("graphql-docs-toggle", cx);
        w.render_frame(cx)
    })
    .unwrap();
    assert!(present(cx, win, "graphql-docs-search"));
    assert!(present(cx, win, ("graphql-docs-row", 1usize)));
    let provider = SchemaCompletions::new(Rc::new(RefCell::new(url.to_owned())));
    let labels: Vec<String> = cx
        .update(|cx| provider.answer(&Rope::from("{ vi"), 4, cx))
        .into_iter()
        .map(|i| i.label)
        .collect();
    assert_eq!(labels, ["viewer"]);
}

const SDL_A: &str = "type Query { me: User }\ntype User { name: String }\n";

fn sdl(text: &str) -> Schema {
    Schema::from_sdl(text).unwrap()
}

#[test]
fn compiled_schemas_are_keyed_by_the_loaded_generation() {
    let mut s = SchemaStore::default();
    assert!(matches!(s.lookup("u"), Lookup::NoSchema));
    assert!(s.claim_build("u").is_none());
    let g = s.begin("u");
    assert!(matches!(s.lookup("u"), Lookup::Pending));
    assert!(
        s.claim_build("u").is_none(),
        "nothing to build while loading"
    );
    assert!(s.finish("u", g, Ok(sdl(SDL_A))));
    assert!(matches!(s.lookup("u"), Lookup::Pending));
    let (claimed, schema) = s.claim_build("u").expect("a loaded schema to build");
    assert_eq!(claimed, g);
    assert!(s.claim_build("u").is_none(), "one build per generation");
    assert!(s.store_compiled("u", g, Ok(CompiledSchema::new(&schema).unwrap())));
    let Lookup::Ready {
        generation,
        schema: first,
    } = s.lookup("u")
    else {
        panic!("not ready")
    };
    assert_eq!(generation, g);
    let Lookup::Ready { schema: again, .. } = s.lookup("u") else {
        panic!("not ready")
    };
    assert!(Arc::ptr_eq(&first, &again));
    assert_eq!(s.lookup("u").key(), Some(CheckKey::Ready(g)));
    // A reload replaces the build, and the old generation can no longer be stored.
    let g2 = s.begin("u");
    assert!(s.lookup("u").key().is_none());
    assert!(!s.store_compiled("u", g, Err("late".into())));
    assert!(s.finish("u", g2, Ok(sdl(SDL_A))));
    assert_eq!(s.claim_build("u").unwrap().0, g2);
    assert!(s.store_compiled("u", g2, Err("cannot print".into())));
    assert!(matches!(
        s.lookup("u"),
        Lookup::Unusable { generation, ref reason } if generation == g2 && reason == "cannot print"
    ));
    assert_eq!(s.lookup("u").key(), Some(CheckKey::Unusable(g2)));
    // A failed load checks syntax only.
    let g3 = s.begin("u");
    assert!(s.finish("u", g3, Err("boom".into())));
    assert_eq!(s.lookup("u").key(), Some(CheckKey::NoSchema));
}

#[test]
fn a_restored_schema_builds_under_its_own_generation() {
    let mut s = SchemaStore::default();
    assert!(s.restore("u", sdl(SDL_A)));
    let (g, _) = s.claim_build("u").expect("restored schemas build too");
    assert!(s.store_compiled("u", g, Err("x".into())));
    assert!(matches!(s.lookup("u"), Lookup::Unusable { .. }));
}

#[test]
fn the_source_names_a_schema_file_until_the_next_load() {
    let mut s = SchemaStore::default();
    let g = s.begin_file("u", Path::new("/tmp/s.graphqls"));
    assert!(s.finish("u", g, Ok(sdl(SDL_A))));
    assert_eq!(s.source("u"), Some("/tmp/s.graphqls"));
    s.begin("u");
    assert_eq!(s.source("u"), None);
}

#[test]
fn the_shared_types_cross_threads() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<Schema>();
    send_sync::<CompiledSchema>();
}

#[gpui_kit::test]
fn compiled_builds_once_off_the_ui_thread_and_is_shared(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let s = cx.default_global::<SchemaStore>();
        let g = s.begin("u");
        s.finish("u", g, Ok(sdl(SDL_A)));
    });
    assert!(matches!(cx.update(|cx| compiled("u", cx)), Lookup::Pending));
    assert!(matches!(cx.update(|cx| compiled("u", cx)), Lookup::Pending));
    cx.run_until_parked();
    let Lookup::Ready { schema: a, .. } = cx.update(|cx| compiled("u", cx)) else {
        panic!("not built")
    };
    let Lookup::Ready { schema: b, .. } = cx.update(|cx| compiled("u", cx)) else {
        panic!("not built")
    };
    assert!(Arc::ptr_eq(&a, &b));
    assert!(matches!(
        cx.update(|cx| compiled("none", cx)),
        Lookup::NoSchema
    ));
}
