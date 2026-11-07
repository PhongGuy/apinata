//! Completion and hover for the GraphQL variables editor: core's answers from the query's
//! variable definitions and the schema, turned into LSP items and Markdown hover.

use apinata_core::{Schema, VariableCompletionKind, variable_completions, variable_hover};
use gpui_kit::component::{Rope, RopeExt as _};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionTextEdit, Documentation, Hover, HoverContents,
    MarkupContent, MarkupKind, Range, TextEdit,
};

use crate::graphql_hover::{code_span, plain};
use crate::var_complete;

/// Completion items at `offset` of the variables `text`, each with a `text_edit` over core's
/// range; empty after an unclosed `{{`.
pub fn items(
    query: &str,
    schema: Option<&Schema>,
    text: &Rope,
    offset: usize,
) -> Vec<CompletionItem> {
    let source = text.to_string();
    if source.is_char_boundary(offset) && var_complete::query_at(&source, offset).is_some() {
        return Vec::new();
    }
    variable_completions(query, &source, offset, schema)
        .into_iter()
        .map(|c| CompletionItem {
            kind: match c.kind {
                VariableCompletionKind::Variable => Some(CompletionItemKind::VARIABLE),
                VariableCompletionKind::InputField => Some(CompletionItemKind::FIELD),
                VariableCompletionKind::EnumValue => Some(CompletionItemKind::ENUM_MEMBER),
                VariableCompletionKind::Literal => Some(CompletionItemKind::KEYWORD),
                _ => None,
            },
            detail: c.detail,
            documentation: c.documentation.map(Documentation::String),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range: lsp_range(text, &c.range),
                new_text: c.insert,
            })),
            label: c.label,
            ..Default::default()
        })
        .collect()
}

/// Markdown hover for the key at `offset`: signature, description, default.
pub fn hover(query: &str, schema: Option<&Schema>, text: &Rope, offset: usize) -> Option<Hover> {
    let found = variable_hover(query, &text.to_string(), offset, schema)?;
    let mut sections = vec![code_span(&found.signature)];
    sections.extend(
        found
            .description
            .filter(|d| !d.is_empty())
            .map(|d| plain(&d)),
    );
    sections.extend(found.default.map(|d| format!("Default: {}", code_span(&d))));
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: sections.join("\n\n"),
        }),
        range: Some(lsp_range(text, &found.range)),
    })
}

fn lsp_range(text: &Rope, range: &std::ops::Range<usize>) -> Range {
    Range::new(
        text.offset_to_position(range.start),
        text.offset_to_position(range.end),
    )
}
