//! Hover text and definition links for names in the GraphQL query editor, answered from the
//! loaded schema through core's `symbol_at`.

use std::cell::RefCell;
use std::rc::Rc;

use apinata_core::{Schema, SchemaSymbol, TypeKind};
use gpui_kit::component::input::{DefinitionProvider, HoverProvider};
use gpui_kit::component::{Rope, RopeExt as _};
use gpui_kit::{App, Result, Task, Window};
use lsp_types::{Hover, HoverContents, LocationLink, MarkupContent, MarkupKind, Range, Uri};

use crate::graphql_docs::DocsView;
use crate::graphql_schema_store::{self, SchemaState};

const SCHEME: &str = "apinata-docs:";

/// Markdown hover for the name at byte `offset`, its range in LSP positions; None off a known name.
pub fn hover(schema: &Schema, text: &Rope, offset: usize) -> Option<Hover> {
    let source = text.to_string();
    let found = schema.symbol_at(&source, offset)?;
    let parts = match &found.symbol {
        SchemaSymbol::Type { name } => {
            let ty = schema.type_named(name)?;
            let keyword = match ty.kind() {
                TypeKind::Object => "type",
                TypeKind::Interface => "interface",
                TypeKind::Union => "union",
                TypeKind::Enum => "enum",
                TypeKind::InputObject => "input",
                TypeKind::Scalar => "scalar",
                _ => "type",
            };
            (format!("{keyword} {name}"), ty.description(), None)
        }
        SchemaSymbol::Field { owner, field } => {
            let f = schema
                .type_named(owner)?
                .fields()
                .iter()
                .find(|f| f.name() == field)?;
            let args = if f.args().is_empty() {
                String::new()
            } else {
                let list: Vec<String> = f.args().iter().map(input_signature).collect();
                format!("({})", list.join(", "))
            };
            (
                format!("{owner}.{field}{args}: {}", f.ty().display()),
                f.description(),
                f.deprecation(),
            )
        }
        SchemaSymbol::Argument { owner, field, arg } => {
            let f = schema
                .type_named(owner)?
                .fields()
                .iter()
                .find(|f| f.name() == field)?;
            let a = f.args().iter().find(|a| a.name() == arg)?;
            (input_signature(a), a.description(), None)
        }
        _ => return None,
    };
    let (signature, description, deprecation) = parts;
    let mut sections = vec![format!("`{signature}`")];
    sections.extend(description.filter(|d| !d.is_empty()).map(str::to_owned));
    if let Some(reason) = deprecation {
        sections.push(if reason.is_empty() {
            "Deprecated".to_owned()
        } else {
            format!("Deprecated: {reason}")
        });
    }
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: sections.join("\n\n"),
        }),
        range: Some(lsp_range(text, &found.range)),
    })
}

/// One link to the name's docs page: origin = the name's range, both target ranges zero.
pub fn definition(schema: &Schema, text: &Rope, offset: usize) -> Option<LocationLink> {
    let found = schema.symbol_at(&text.to_string(), offset)?;
    let target = match &found.symbol {
        SchemaSymbol::Type { name } => format!("{SCHEME}type/{name}"),
        SchemaSymbol::Field { owner, field } | SchemaSymbol::Argument { owner, field, .. } => {
            format!("{SCHEME}field/{owner}/{field}")
        }
        _ => return None,
    };
    Some(LocationLink {
        origin_selection_range: Some(lsp_range(text, &found.range)),
        target_uri: target.parse::<Uri>().ok()?,
        target_range: Range::default(),
        target_selection_range: Range::default(),
    })
}

/// The Docs view an `apinata-docs:type/<Name>` or `apinata-docs:field/<Owner>/<field>` URI names.
pub fn docs_view(uri: &str) -> Option<DocsView> {
    let rest = uri.strip_prefix(SCHEME)?;
    let mut parts = rest.split('/');
    let view = match (parts.next()?, parts.next(), parts.next(), parts.next()) {
        ("type", Some(name), None, None) => DocsView::Type(name.to_owned()),
        ("field", Some(owner), Some(field), None) => DocsView::Field {
            owner: owner.to_owned(),
            field: field.to_owned(),
        },
        _ => return None,
    };
    let named = match &view {
        DocsView::Type(n) => !n.is_empty(),
        DocsView::Field { owner, field } => !owner.is_empty() && !field.is_empty(),
        _ => false,
    };
    named.then_some(view)
}

// `name: Type = default`, without the default when there is none.
fn input_signature(v: &apinata_core::InputValue) -> String {
    match v.default_value() {
        Some(d) => format!("{}: {} = {d}", v.name(), v.ty().display()),
        None => format!("{}: {}", v.name(), v.ty().display()),
    }
}

fn lsp_range(text: &Rope, range: &std::ops::Range<usize>) -> Range {
    Range::new(
        text.offset_to_position(range.start),
        text.offset_to_position(range.end),
    )
}

/// Hover and definitions from the store's loaded schema for the URL the cell holds.
pub struct SchemaSymbols {
    url: Rc<RefCell<String>>,
}

impl SchemaSymbols {
    /// A provider for whatever URL `url` holds when asked.
    pub fn new(url: Rc<RefCell<String>>) -> Self {
        SchemaSymbols { url }
    }

    /// `hover` for the URL's loaded schema; None while it is absent, loading or failed.
    pub fn hover_answer(&self, text: &Rope, offset: usize, cx: &App) -> Option<Hover> {
        match graphql_schema_store::state(cx, &self.url.borrow()) {
            Some(SchemaState::Loaded(schema)) => hover(&schema, text, offset),
            _ => None,
        }
    }

    /// `definition` for the URL's loaded schema; empty while it is absent, loading or failed.
    pub fn definition_answer(&self, text: &Rope, offset: usize, cx: &App) -> Vec<LocationLink> {
        match graphql_schema_store::state(cx, &self.url.borrow()) {
            Some(SchemaState::Loaded(schema)) => {
                definition(&schema, text, offset).into_iter().collect()
            }
            _ => Vec::new(),
        }
    }
}

impl HoverProvider for SchemaSymbols {
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

impl DefinitionProvider for SchemaSymbols {
    fn definitions(
        &self,
        text: &Rope,
        offset: usize,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Vec<LocationLink>>> {
        Task::ready(Ok(self.definition_answer(text, offset, cx)))
    }
}
