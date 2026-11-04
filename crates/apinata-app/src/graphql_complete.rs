//! Schema completion for the GraphQL query editor: core's completions turned into LSP items
//! for the kit's completion menu, silent inside `{{…}}` placeholders.

use std::cell::RefCell;
use std::rc::Rc;

use apinata_core::{CompletionKind, Schema};
use gpui_kit::component::input::CompletionProvider;
use gpui_kit::component::{Rope, RopeExt as _};
use gpui_kit::{App, Result, Task, Window};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    Documentation, Range, TextEdit,
};

use crate::graphql_schema_store::{self, SchemaState};
use crate::var_complete;

/// Completion items at `offset` (bytes) in `text`, each with a `text_edit` over the typed prefix;
/// empty inside `{{…}}` and after whitespace unless the previous word is `on`.
pub fn items(schema: &Schema, text: &Rope, offset: usize) -> Vec<CompletionItem> {
    let source = text.to_string();
    let mut end = offset.min(source.len());
    while !source.is_char_boundary(end) {
        end -= 1;
    }
    if var_complete::query_at(&source, end).is_some() {
        return Vec::new();
    }
    let start = end - trailing_name(&source[..end]).len();
    // A space triggers the menu, but with nothing typed only a type condition wants names.
    let before = &source[..start];
    if start == end
        && before.ends_with(char::is_whitespace)
        && trailing_name(before.trim_end()) != "on"
    {
        return Vec::new();
    }
    // The edit spans only the typed prefix; the kit's fallback would also eat a typed `{` or `(`.
    let range = Range::new(text.offset_to_position(start), text.offset_to_position(end));
    schema
        .completions(&source, end)
        .into_iter()
        .map(|c| CompletionItem {
            label: c.label().to_owned(),
            kind: match c.kind() {
                CompletionKind::Field => Some(CompletionItemKind::FIELD),
                CompletionKind::Argument => Some(CompletionItemKind::PROPERTY),
                CompletionKind::Type => Some(CompletionItemKind::CLASS),
                _ => None,
            },
            detail: Some(c.detail().to_owned()),
            documentation: c
                .documentation()
                .map(|d| Documentation::String(d.to_owned())),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range,
                new_text: c.label().to_owned(),
            })),
            ..Default::default()
        })
        .collect()
}

// The run of GraphQL name characters that ends `text`.
fn trailing_name(text: &str) -> &str {
    let start = text
        .bytes()
        .rposition(|b| !(b.is_ascii_alphanumeric() || b == b'_'))
        .map_or(0, |i| i + 1);
    &text[start..]
}

/// The query editor's completion provider, answering from the store's schema for a URL the
/// section keeps current.
pub struct SchemaCompletions {
    url: Rc<RefCell<String>>,
}

impl SchemaCompletions {
    /// A provider for whatever URL `url` holds when asked.
    pub fn new(url: Rc<RefCell<String>>) -> Self {
        SchemaCompletions { url }
    }

    /// `items` for the URL's loaded schema; empty when it is absent, loading or failed.
    pub fn answer(&self, text: &Rope, offset: usize, cx: &App) -> Vec<CompletionItem> {
        match graphql_schema_store::state(cx, &self.url.borrow()) {
            Some(SchemaState::Loaded(schema)) => items(&schema, text, offset),
            _ => Vec::new(),
        }
    }
}

impl CompletionProvider for SchemaCompletions {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _: CompletionContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<Result<CompletionResponse>> {
        Task::ready(Ok(CompletionResponse::Array(self.answer(text, offset, cx))))
    }

    fn is_completion_trigger(&self, _: usize, new_text: &str, _: &mut App) -> bool {
        (!new_text.is_empty() && new_text.chars().all(|c| c.is_alphanumeric() || c == '_'))
            || matches!(new_text, "{" | "(" | " ")
    }
}
