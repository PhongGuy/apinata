//! Directives in the query editor: hover, definition links, docs URIs and completion items.

use apinata_app::graphql_complete::items;
use apinata_app::graphql_docs::DocsView;
use apinata_app::graphql_hover::{definition, docs_view, hover};
use apinata_core::Schema;
use gpui_kit::component::Rope;
use lsp_types::{
    CompletionItemKind, CompletionTextEdit, HoverContents, MarkupContent, MarkupKind, Position,
    Range, TextEdit,
};

// `@` 14, `include` 15..22, `if` 23..25.
const Q: &str = "{ user(id: 1) @include(if: true) { name } }";

fn schema() -> Schema {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../apinata-core/tests/fixtures/graphql/schema.json"
    ))
    .unwrap();
    Schema::from_json(&text).unwrap()
}

fn r(l1: u32, c1: u32, l2: u32, c2: u32) -> Range {
    Range::new(Position::new(l1, c1), Position::new(l2, c2))
}

fn hover_in(s: &Schema, text: &str, offset: usize) -> Option<(String, Range)> {
    hover(s, &Rope::from(text), offset).map(|h| {
        let HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }) = h.contents
        else {
            panic!("not Markdown: {:?}", h.contents)
        };
        (value, h.range.expect("a range"))
    })
}

fn link_at(text: &str, offset: usize) -> Option<(String, Range)> {
    definition(&schema(), &Rope::from(text), offset).map(|l| {
        (
            l.target_uri.as_str().to_owned(),
            l.origin_selection_range.expect("an origin"),
        )
    })
}

#[test]
fn hover_on_a_directive_shows_its_signature_and_description() {
    let s = schema();
    assert_eq!(
        hover_in(&s, Q, 16),
        Some((
            "`@include(if: Boolean!) on FIELD | FRAGMENT_SPREAD | INLINE_FRAGMENT`\n\nDirects the executor to include this field or fragment only when the \\`if\\` argument is true\\.".into(),
            r(0, 15, 0, 22)
        ))
    );
    assert_eq!(
        hover_in(&s, Q, 24),
        Some((
            "`@include(if: Boolean!)`\n\nIncluded when true\\.".into(),
            r(0, 23, 0, 25)
        ))
    );
    assert_eq!(hover_in(&s, "{ user(id: 1) @nope { name } }", 16), None);
    let own =
        Schema::from_sdl("directive @once repeatable on FIELD\ntype Query { a: Int }").unwrap();
    assert_eq!(
        hover_in(&own, "{ a @once }", 6),
        Some(("`@once repeatable on FIELD`".into(), r(0, 5, 0, 9)))
    );
}

#[test]
fn a_directive_and_its_arguments_link_to_the_directive_page() {
    assert_eq!(
        link_at(Q, 16),
        Some(("apinata-docs:directive/include".into(), r(0, 15, 0, 22)))
    );
    assert_eq!(
        link_at(Q, 24),
        Some(("apinata-docs:directive/include".into(), r(0, 23, 0, 25)))
    );
    assert_eq!(
        docs_view("apinata-docs:directive/include"),
        Some(DocsView::Directive("include".into()))
    );
    for bad in ["apinata-docs:directive/", "apinata-docs:directive/a/b"] {
        assert_eq!(docs_view(bad), None, "{bad}");
    }
}

#[test]
fn after_at_the_items_are_directives_with_the_function_kind() {
    let all = items(&schema(), &Rope::from("{ user @"), 8);
    assert_eq!(
        all.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(),
        ["skip", "include"]
    );
    assert_eq!(all[0].kind, Some(CompletionItemKind::FUNCTION));
    assert_eq!(all[0].detail.as_deref(), Some("@skip(if: Boolean!)"));
    assert_eq!(
        all[0].text_edit,
        Some(CompletionTextEdit::Edit(TextEdit {
            range: r(0, 8, 0, 8),
            new_text: "skip".into()
        }))
    );
    let typed = items(&schema(), &Rope::from("{ user @in"), 10);
    assert_eq!(
        typed
            .iter()
            .map(|i| i.text_edit.clone())
            .collect::<Vec<_>>(),
        [Some(CompletionTextEdit::Edit(TextEdit {
            range: r(0, 8, 0, 10),
            new_text: "include".into()
        }))]
    );
    assert!(items(&schema(), &Rope::from("{ user {{a@"), 11).is_empty());
}
