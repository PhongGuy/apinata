//! Variables-editor completion items and hover text from the query and the schema.

use apinata_app::graphql_variables::{hover, items};
use apinata_core::Schema;
use gpui_kit::component::Rope;
use lsp_types::{
    CompletionItemKind, CompletionTextEdit, Documentation, HoverContents, MarkupContent,
    MarkupKind, Position, Range, TextEdit,
};

const Q: &str = "query Q($id: ID!, $role: Role, $input: NewUser) { user(id: $id) { name } }";

fn fixture() -> Schema {
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

fn labels(schema: Option<&Schema>, text: &str, offset: usize) -> Vec<String> {
    items(Q, schema, &Rope::from(text), offset)
        .into_iter()
        .map(|i| i.label)
        .collect()
}

fn markdown(
    query: &str,
    schema: Option<&Schema>,
    text: &str,
    offset: usize,
) -> Option<(String, Range)> {
    hover(query, schema, &Rope::from(text), offset).map(|h| {
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

#[test]
fn key_items_edit_the_whole_key_token() {
    let all = items(Q, None, &Rope::from("{\"\"}"), 2);
    assert_eq!(
        all.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(),
        ["id", "role", "input"]
    );
    assert_eq!(
        all[0].text_edit,
        Some(CompletionTextEdit::Edit(TextEdit {
            range: r(0, 1, 0, 3),
            new_text: "\"id\": ".into()
        }))
    );
    assert_eq!(all[0].kind, Some(CompletionItemKind::VARIABLE));
    assert_eq!(all[0].detail.as_deref(), Some("ID!"));
}

#[test]
fn value_items_carry_kinds_and_documentation() {
    let s = fixture();
    let all = items(Q, Some(&s), &Rope::from("{\"role\": }"), 9);
    assert_eq!(
        all.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(),
        ["ADMIN", "MEMBER", "GUEST", "null"]
    );
    assert_eq!(all[0].kind, Some(CompletionItemKind::ENUM_MEMBER));
    assert_eq!(
        all[0].documentation,
        Some(Documentation::String("Full access.".into()))
    );
    assert_eq!(
        all[0].text_edit,
        Some(CompletionTextEdit::Edit(TextEdit {
            range: r(0, 9, 0, 9),
            new_text: "\"ADMIN\"".into()
        }))
    );
    assert_eq!(all[2].detail.as_deref(), Some("deprecated"));
    assert_eq!(all[3].kind, Some(CompletionItemKind::KEYWORD));
    let fields = items(Q, Some(&s), &Rope::from("{\"input\": {\"\"}}"), 12);
    assert_eq!(
        fields
            .iter()
            .map(|i| (i.label.as_str(), i.kind))
            .collect::<Vec<_>>(),
        [
            ("name", Some(CompletionItemKind::FIELD)),
            ("role", Some(CompletionItemKind::FIELD))
        ]
    );
}

#[test]
fn ranges_count_chars_on_later_lines() {
    let text = "{\n  \"\u{f8}\": 1,\n  \"\"\n}";
    let all = items(Q, None, &Rope::from(text), 16);
    assert_eq!(all.len(), 3);
    let Some(CompletionTextEdit::Edit(edit)) = &all[0].text_edit else {
        panic!("no edit")
    };
    assert_eq!(edit.range, r(2, 2, 2, 4));
}

#[test]
fn an_open_placeholder_silences_the_items() {
    assert!(!apinata_core::variable_completions(Q, "{{i", 3, None).is_empty());
    assert!(labels(None, "{{i", 3).is_empty());
    assert!(labels(None, "{\"id\": {{", 9).is_empty());
}

#[test]
fn hover_shows_signature_description_and_default() {
    let s = fixture();
    assert_eq!(
        markdown(Q, Some(&s), "{\"input\": {\"role\": \"ADMIN\"}}", 12),
        Some((
            "`NewUser.role: Role`\n\nDefault: `MEMBER`".into(),
            r(0, 11, 0, 17)
        ))
    );
    assert_eq!(
        markdown(Q, None, "{\"id\": 1}", 2),
        Some(("`$id: ID!`".into(), r(0, 1, 0, 5)))
    );
    let described =
        Schema::from_sdl("type Query { a: Int }\ninput F {\n  \"Use *bold* <b>\"\n  x: Int = 1\n}")
            .unwrap();
    assert_eq!(
        markdown(
            "query Q($f: F) { a }",
            Some(&described),
            "{\"f\": {\"x\": 1}}",
            8
        )
        .map(|(m, _)| m),
        Some("`F.x: Int`\n\nUse \\*bold\\* \\<b\\>\n\nDefault: `1`".into())
    );
    assert_eq!(markdown(Q, None, "{\"id\": 1}", 7), None);
}
