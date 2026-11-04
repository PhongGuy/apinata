use crate::support;

use apinata_app::command::{Command, Target};
use apinata_app::request_editor::Section;
use apinata_app::save_example::offered;
use apinata_core::RequestKind;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, TestAppContext};
use std::path::Path;
use support::*;

fn corpus(rel: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../apinata-core/tests/fixtures/reference/2026-10-13-graphql-parity")
            .join(rel),
    )
    .unwrap()
}

#[test]
fn the_body_tab_is_called_query_on_graphql() {
    assert_eq!(Section::Body.label_for(RequestKind::Graphql), "Query");
    assert_eq!(Section::Body.label_for(RequestKind::Http), "Body");
    assert_eq!(
        Section::Headers.label_for(RequestKind::Graphql),
        Section::Headers.label()
    );
}

#[gpui_kit::test]
fn a_graphql_request_edits_its_variables_in_the_query_tab(cx: &mut TestAppContext) {
    let text = corpus("normalized/synthetic/graphql.yml");
    let col = write_tree(&[("opencollection.yml", ROOT), ("graphql.yml", text.as_str())]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "graphql.yml", false);
    cx.update_window(win, |_, w, cx| {
        w.click(("section", Section::Body as usize), cx);
        w.render_frame(cx)
    })
    .unwrap();
    assert!(present(cx, win, "graphql-query"));
    assert!(present(cx, win, "graphql-variables"));
    assert!(!present(cx, win, "body-mode"));
    replace_code(cx, win, Some("body"), "graphql-variables", "{\"limit\": 10");
    cx.run_until_parked();
    ws.update(cx, |w, cx| {
        w.send(&root, Command::Save(Target::Request(ip("graphql.yml"))), cx)
    });
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(col.path().join("graphql.yml")).unwrap(),
        corpus("expected/gql-variables-single.yml")
    );
}

#[gpui_kit::test]
fn an_empty_graphql_body_leaves_the_query_tab_unmarked(cx: &mut TestAppContext) {
    let bare = "info:\n  name: g\n  type: graphql\n  seq: 1\ngraphql:\n  url: u\n";
    let col = write_tree(&[("opencollection.yml", ROOT), ("g.yml", bare)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (_win, ed) = editor_for(cx, &ws, &root, "g.yml", false);
    assert!(!ed.read_with(cx, |e, cx| e.section_marked(Section::Body, cx)));
}

#[gpui_kit::test]
fn save_as_example_is_offered_for_http_only(cx: &mut TestAppContext) {
    let gql = "info:\n  name: g\n  type: graphql\n  seq: 2\ngraphql:\n  url: u\n";
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request("r", "https://x.test/r")),
        ("g.yml", gql),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    assert!(offered(&view_of(cx, &ws, &root, "r.yml")));
    let g = view_of(cx, &ws, &root, "g.yml");
    assert_eq!(g.kind, RequestKind::Graphql);
    assert!(!offered(&g));
}
