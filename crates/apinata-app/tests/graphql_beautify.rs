//! Beautify query: an undoable canonical layout, an inline syntax error, and large queries
//! formatted off the UI thread.

use crate::support;

use apinata_app::command::Edit;
use apinata_app::graphql_section::GraphqlSection;
use apinata_core::BodySpec;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use support::*;

type Host = Entity<SectionHost<GraphqlSection>>;

fn section(
    cx: &mut TestAppContext,
    query: &str,
) -> (AnyWindowHandle, Host, Entity<GraphqlSection>) {
    let (win, host) = host_section(cx, GraphqlSection::new);
    let s = host.read_with(cx, |h, _| h.view.clone());
    let spec = BodySpec::Graphql {
        query: query.into(),
        variables: String::new(),
    };
    cx.update_window(win, |_, w, cx| s.update(cx, |s, cx| s.load(&spec, w, cx)))
        .unwrap();
    cx.run_until_parked();
    (win, host, s)
}

fn query(cx: &mut TestAppContext, s: &Entity<GraphqlSection>) -> String {
    let q = s.read_with(cx, |s, _| s.query().clone());
    q.read_with(cx, |e, _| e.value().to_string())
}

fn start(cx: &mut TestAppContext, win: AnyWindowHandle, s: &Entity<GraphqlSection>) {
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.beautify_query(w, cx))
    })
    .unwrap();
}

fn beautify(cx: &mut TestAppContext, win: AnyWindowHandle, s: &Entity<GraphqlSection>) {
    start(cx, win, s);
    cx.run_until_parked();
}

fn last_query(cx: &mut TestAppContext, host: &Host) -> Option<String> {
    host.read_with(cx, |h, _| match h.edits.last() {
        Some(Edit::Body(BodySpec::Graphql { query, .. })) => Some(query.clone()),
        _ => None,
    })
}

const PRETTY: &str = "{\n  me {\n    id\n  }\n}\n";

#[gpui_kit::test]
fn beautify_formats_the_query_and_edits_the_draft(cx: &mut TestAppContext) {
    let (win, host, s) = section(cx, "{ me { id } }");
    beautify(cx, win, &s);
    assert_eq!(query(cx, &s), PRETTY);
    assert_eq!(last_query(cx, &host).as_deref(), Some(PRETTY));
    assert!(s.read_with(cx, |s, _| s.beautify_error().is_none()));
}

#[gpui_kit::test]
fn undo_restores_the_query_before_beautify(cx: &mut TestAppContext) {
    let (win, host, s) = section(cx, "{ me { id } }");
    beautify(cx, win, &s);
    cx.update_window(win, |_, w, cx| {
        use gpui_kit::test::TestWindowExt as _;
        w.click_at("graphql-query", code_at(), cx);
        w.press(&key("z"), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(query(cx, &s), "{ me { id } }");
    assert_eq!(last_query(cx, &host).as_deref(), Some("{ me { id } }"));
}

#[gpui_kit::test]
fn a_syntax_error_keeps_the_text_and_shows_until_the_next_edit(cx: &mut TestAppContext) {
    let (win, host, s) = section(cx, "{ me { id }");
    beautify(cx, win, &s);
    assert_eq!(query(cx, &s), "{ me { id }");
    assert!(host.read_with(cx, |h, _| h.edits.is_empty()));
    assert_eq!(
        s.read_with(cx, |s, _| s.beautify_error().map(str::to_owned))
            .as_deref(),
        Some("Line 1, column 12: expected R_CURLY, got EOF")
    );
    assert!(present(cx, win, "graphql-beautify-error"));
    replace_code(cx, win, None, "graphql-query", "query Q");
    assert!(!present(cx, win, "graphql-beautify-error"));
}

#[gpui_kit::test]
fn placeholders_survive_and_blank_text_is_left_alone(cx: &mut TestAppContext) {
    let (win, _host, s) = section(cx, "{ user(id: {{id}}) { {{fields}} } }");
    beautify(cx, win, &s);
    assert_eq!(
        query(cx, &s),
        "{\n  user(id: {{id}}) {\n    {{fields}}\n  }\n}\n"
    );
    let (win, host, s) = section(cx, "  ");
    assert!(!s.read_with(cx, |s, cx| s.can_beautify_query(cx)));
    beautify(cx, win, &s);
    assert_eq!(query(cx, &s), "  ");
    assert!(host.read_with(cx, |h, _| h.edits.is_empty()));
}

#[gpui_kit::test]
fn a_large_query_is_formatted_off_the_ui_thread(cx: &mut TestAppContext) {
    let big = format!("{{ {}}}", "a ".repeat(40_000));
    let (win, _host, s) = section(cx, &big);
    start(cx, win, &s);
    assert_eq!(
        query(cx, &s),
        big,
        "nothing applied before the background job ran"
    );
    cx.run_until_parked();
    assert!(query(cx, &s).starts_with("{\n  a\n  a\n"));
}

#[gpui_kit::test]
fn an_edit_during_an_off_thread_format_wins(cx: &mut TestAppContext) {
    let big = format!("{{ {}}}", "a ".repeat(40_000));
    let (win, _host, s) = section(cx, &big);
    start(cx, win, &s);
    let q = s.read_with(cx, |s, _| s.query().clone());
    cx.update_window(win, |_, w, cx| {
        q.update(cx, |e, cx| e.set_value("{ b }", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(query(cx, &s), "{ b }");
}
