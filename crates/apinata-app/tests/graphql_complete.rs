//! The query editor's schema completion: prefix edits, kinds, silence in `{{` and while loading.

use crate::support;

use std::cell::RefCell;
use std::rc::Rc;

use apinata_app::graphql_complete::{SchemaCompletions, items};
use apinata_app::graphql_schema_store::SchemaStore;
use apinata_app::graphql_section::GraphqlSection;
use apinata_core::Schema;
use gpui_kit::component::Rope;
use gpui_kit::{AppContext as _, TestAppContext};
use lsp_types::{CompletionItemKind, CompletionTextEdit, Documentation, Position, Range, TextEdit};
use support::*;

fn schema() -> Schema {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../apinata-core/tests/fixtures/graphql/schema.json"
    ))
    .unwrap();
    Schema::from_json(&text).unwrap()
}
fn labels(text: &str, offset: usize) -> Vec<String> {
    items(&schema(), &Rope::from(text), offset)
        .into_iter()
        .map(|i| i.label)
        .collect()
}

#[test]
fn items_replace_only_the_typed_prefix() {
    let all = items(&schema(), &Rope::from("{\n  us"), 6);
    assert_eq!(
        all.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(),
        ["user", "users"]
    );
    let user = &all[0];
    assert_eq!(
        user.text_edit,
        Some(CompletionTextEdit::Edit(TextEdit {
            range: Range::new(Position::new(1, 2), Position::new(1, 4)),
            new_text: "user".into()
        }))
    );
    assert_eq!(user.kind, Some(CompletionItemKind::FIELD));
    assert_eq!(user.detail.as_deref(), Some("User"));
    assert_eq!(
        user.documentation,
        Some(Documentation::String("One user by id.".into()))
    );
    let arg = &items(&schema(), &Rope::from("{ user("), 7)[0];
    assert_eq!(
        (arg.label.as_str(), arg.kind),
        ("id:", Some(CompletionItemKind::PROPERTY))
    );
    let open = &items(&schema(), &Rope::from("{"), 1)[0];
    assert_eq!(
        open.text_edit,
        Some(CompletionTextEdit::Edit(TextEdit {
            range: Range::new(Position::new(0, 1), Position::new(0, 1)),
            new_text: "user".into()
        }))
    );
}

#[test]
fn a_space_offers_only_after_on_and_braces_stay_with_variables() {
    assert_eq!(
        labels("{ search(term: \"x\") { ... on ", 29),
        ["Query", "Mutation", "Node", "User", "Post", "SearchResult"]
    );
    assert_eq!(
        items(
            &schema(),
            &Rope::from("{ search(term: \"x\") { ... on N"),
            30
        )[0]
        .kind,
        Some(CompletionItemKind::CLASS)
    );
    assert!(labels("{ ", 2).is_empty());
    assert!(labels("{ user ", 7).is_empty());
    assert!(labels("{ user(id: \"{{us", 16).is_empty());
    assert!(labels("{ {{", 4).is_empty());
}

#[gpui_kit::test]
fn the_provider_answers_only_while_the_schema_is_loaded(cx: &mut TestAppContext) {
    init(cx);
    let url = "http://x.test/g";
    let provider = SchemaCompletions::new(Rc::new(RefCell::new(url.to_owned())));
    let rope = Rope::from("{ us");
    assert!(cx.update(|cx| provider.answer(&rope, 4, cx)).is_empty());
    let generation = cx.update(|cx| cx.default_global::<SchemaStore>().begin(url));
    assert!(cx.update(|cx| provider.answer(&rope, 4, cx)).is_empty());
    cx.update(|cx| {
        cx.default_global::<SchemaStore>()
            .finish(url, generation, Ok(schema()))
    });
    assert_eq!(cx.update(|cx| provider.answer(&rope, 4, cx)).len(), 2);
    cx.update(|cx| {
        cx.default_global::<SchemaStore>().begin(url);
    });
    assert!(cx.update(|cx| provider.answer(&rope, 4, cx)).is_empty());
}

#[gpui_kit::test]
fn the_query_editor_has_the_provider(cx: &mut TestAppContext) {
    let (_win, host) = host_section(cx, GraphqlSection::new);
    let s = host.read_with(cx, |h, _| h.view.clone());
    let query = s.read_with(cx, |s, _| s.query().clone());
    assert!(query.read_with(cx, |q, _| q.lsp().completion_provider.is_some()));
}

// The query editor with the fixture schema loaded for its URL, focused and empty.
fn loaded_query_editor(
    cx: &mut TestAppContext,
) -> (
    gpui_kit::AnyWindowHandle,
    gpui_kit::Entity<gpui_kit::component::input::EditorState>,
) {
    use gpui_kit::test::TestWindowExt as _;
    let url = "http://x.test/g";
    let (win, host) = host_section(cx, GraphqlSection::new);
    let s = host.read_with(cx, |h, _| h.view.clone());
    s.update(cx, |s, cx| s.set_schema_url(url.to_owned(), cx));
    cx.update(|cx| {
        let store = cx.default_global::<SchemaStore>();
        let generation = store.begin(url);
        store.finish(url, generation, Ok(schema()));
    });
    cx.update_window(win, |_, w, cx| {
        w.click_at("graphql-query", code_at(), cx);
        w.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    let query = s.read_with(cx, |s, _| s.query().clone());
    (win, query)
}

// Type `text`, then press each of `keys` as a platform keystroke, which also carries the
// key's character (a newline for Enter) whenever the bound action lets the key propagate.
fn keys(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle, text: &str, keys: &[&str]) {
    use gpui_kit::test::TestWindowExt as _;
    for ch in text.chars() {
        cx.update_window(win, |_, w, cx| {
            w.input(&ch.to_string(), cx);
            w.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
    for k in keys {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.dispatch_keystroke(gpui_kit::Keystroke::parse(k).unwrap(), cx);
            w.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn enter_accepts_a_completion_on_one_line_without_a_newline(cx: &mut TestAppContext) {
    let (win, query) = loaded_query_editor(cx);
    keys(cx, win, "{ us", &["enter"]);
    assert_eq!(query.read_with(cx, |q, _| q.value().to_string()), "{ user}");
}

#[gpui_kit::test]
fn enter_accepts_a_completion_in_a_selection_without_a_newline(cx: &mut TestAppContext) {
    let (win, query) = loaded_query_editor(cx);
    keys(cx, win, "{", &["escape", "enter"]);
    keys(cx, win, "us", &["enter"]);
    assert_eq!(
        query.read_with(cx, |q, _| q.value().to_string()),
        "{\n  user\n}"
    );
}

#[gpui_kit::test]
fn typing_at_opens_the_directive_list(cx: &mut TestAppContext) {
    let (win, query) = loaded_query_editor(cx);
    keys(cx, win, "{ user @", &["enter"]);
    assert_eq!(
        query.read_with(cx, |q, _| q.value().to_string()),
        "{ user @skip}"
    );
}
