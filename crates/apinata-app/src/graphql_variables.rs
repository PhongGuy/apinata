//! Completion and hover for the GraphQL variables editor: core's answers from the query's
//! variable definitions and the schema, turned into LSP items and Markdown hover.

use std::cell::RefCell;
use std::rc::Rc;

use apinata_core::{Schema, VariableCompletionKind, variable_completions, variable_hover};
use gpui_kit::component::input::{CompletionProvider, EditorState, HoverProvider};
use gpui_kit::component::{Rope, RopeExt as _};
use gpui_kit::{App, Result, Task, WeakEntity, Window};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    Documentation, Hover, HoverContents, MarkupContent, MarkupKind, Range, TextEdit,
};

use crate::graphql_hover::{code_span, plain};
use crate::graphql_schema_store::{self, SchemaState};
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

/// The variables editor's completion and hover provider: the query editor's current text, and
/// the store's schema for the URL the cell holds (loading or failed counts as no schema).
pub struct VariableTools {
    query: WeakEntity<EditorState>,
    url: Rc<RefCell<String>>,
}

impl VariableTools {
    /// A provider reading `query` and whatever URL `url` holds when asked.
    pub fn new(query: WeakEntity<EditorState>, url: Rc<RefCell<String>>) -> Self {
        VariableTools { query, url }
    }

    /// `items` for the query's current text; empty once the query editor is gone.
    pub fn completion_answer(&self, text: &Rope, offset: usize, cx: &App) -> Vec<CompletionItem> {
        self.with_inputs(cx, |query, schema| items(query, schema, text, offset))
            .unwrap_or_default()
    }

    /// `hover` for the query's current text; none once the query editor is gone.
    pub fn hover_answer(&self, text: &Rope, offset: usize, cx: &App) -> Option<Hover> {
        self.with_inputs(cx, |query, schema| hover(query, schema, text, offset))
            .flatten()
    }

    // Asked inside the variables editor's update, so this reads only the query editor.
    fn with_inputs<T>(&self, cx: &App, f: impl FnOnce(&str, Option<&Schema>) -> T) -> Option<T> {
        let query = self.query.upgrade()?.read(cx).value().to_string();
        let schema = match graphql_schema_store::state(cx, &self.url.borrow()) {
            Some(SchemaState::Loaded(schema)) => Some(schema),
            _ => None,
        };
        Some(f(&query, schema.as_deref()))
    }
}

impl CompletionProvider for VariableTools {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _: CompletionContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<Result<CompletionResponse>> {
        Task::ready(Ok(CompletionResponse::Array(
            self.completion_answer(text, offset, cx),
        )))
    }

    fn is_completion_trigger(&self, _: usize, new_text: &str, _: &mut App) -> bool {
        (!new_text.is_empty() && new_text.chars().all(|c| c.is_alphanumeric() || c == '_'))
            || matches!(new_text, "\"" | "{" | "[" | "," | ":" | " ")
    }
}

impl HoverProvider for VariableTools {
    fn hover(
        &self,
        text: &Rope,
        offset: usize,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Option<Hover>>> {
        Task::ready(Ok(self.hover_answer(text, offset, cx)))
    }
}
