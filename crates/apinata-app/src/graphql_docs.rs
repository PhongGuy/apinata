//! The GraphQL docs explorer: a pure `page` builder over a loaded schema, and the panel that
//! shows it with a navigation stack, a search on the root page and a show-deprecated toggle.

use std::ops::Range;

use apinata_core::{Directive, Field, InputValue, Schema, SchemaType, TypeKind};
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::*;

use crate::graphql_hover::directive_signature;
use crate::graphql_schema_store::{self, SchemaState, SchemaStore};
use crate::graphql_section::SchemaAction;
use crate::{style, ui};

/// Where the explorer stands: the schema root, one named type, one field of a type, or one
/// directive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocsView {
    /// The root types and the search.
    Root,
    /// One named type.
    Type(String),
    /// One field of an object or interface type.
    Field {
        /// The type that declares the field.
        owner: String,
        /// The field's name.
        field: String,
    },
    /// One directive, named without `@`.
    Directive(String),
}

/// One line of a docs page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocRow {
    /// A section heading such as `Fields`.
    Heading(String),
    /// A line that opens another view when clicked.
    Link {
        /// The text shown.
        label: String,
        /// The view it opens.
        to: DocsView,
    },
    /// A line of text that goes nowhere.
    Plain(String),
}

/// What the panel shows for one view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocsPage {
    /// The page heading.
    pub title: String,
    /// The schema's description of the type or field, if any.
    pub description: Option<String>,
    /// The page's lines in order.
    pub rows: Vec<DocRow>,
}

/// Builds the page for `view`. `search` filters only the root page; deprecated fields and
/// enum values appear, marked, only when `show_deprecated` is set.
pub fn page(schema: &Schema, view: &DocsView, search: &str, show_deprecated: bool) -> DocsPage {
    match view {
        DocsView::Root => root_page(schema, search, show_deprecated),
        DocsView::Type(name) => match schema.type_named(name) {
            Some(ty) => type_page(ty, show_deprecated),
            None => missing(name.clone(), name),
        },
        DocsView::Directive(name) => match schema.directive_named(name) {
            Some(d) => directive_page(d),
            None => DocsPage {
                title: format!("@{name}"),
                description: None,
                rows: vec![DocRow::Plain(format!(
                    "No directive @{name} in this schema"
                ))],
            },
        },
        DocsView::Field { owner, field } => {
            let title = format!("{owner}.{field}");
            let Some(ty) = schema.type_named(owner) else {
                return missing(title, owner);
            };
            match ty.fields().iter().find(|f| f.name() == field) {
                Some(f) => field_page(title, f),
                None => DocsPage {
                    title,
                    description: None,
                    rows: vec![DocRow::Plain(format!("No field {field} on {owner}"))],
                },
            }
        }
    }
}

fn missing(title: String, name: &str) -> DocsPage {
    DocsPage {
        title,
        description: None,
        rows: vec![DocRow::Plain(format!(
            "No type named {name} in this schema"
        ))],
    }
}

fn type_link(label: String, name: &str) -> DocRow {
    DocRow::Link {
        label,
        to: DocsView::Type(name.to_owned()),
    }
}

// Deprecated entries are hidden, or kept with a marker when the toggle is on.
fn shown(label: String, deprecated: bool, show_deprecated: bool) -> Option<String> {
    match (deprecated, show_deprecated) {
        (false, _) => Some(label),
        (true, true) => Some(format!("{label} (deprecated)")),
        (true, false) => None,
    }
}

// A heading followed by its rows, or nothing when the rows are empty.
fn section(rows: &mut Vec<DocRow>, heading: &str, items: Vec<DocRow>) {
    if !items.is_empty() {
        rows.push(DocRow::Heading(heading.to_owned()));
        rows.extend(items);
    }
}

fn root_page(schema: &Schema, search: &str, show_deprecated: bool) -> DocsPage {
    let roots = [
        ("query", schema.query_type()),
        ("mutation", schema.mutation_type()),
        ("subscription", schema.subscription_type()),
    ];
    let mut rows = Vec::new();
    let links = roots
        .into_iter()
        .filter_map(|(op, name)| name.map(|n| type_link(format!("{op}: {n}"), n)))
        .collect();
    section(&mut rows, "Root types", links);
    let directives = schema
        .directives()
        .iter()
        .map(|d| directive_link(d.name()))
        .collect();
    section(&mut rows, "Directives", directives);
    let needle = search.trim().to_lowercase();
    if !needle.is_empty() {
        // Introspection's own `__` types are never results; type names come before fields.
        let user_types = || {
            schema
                .types()
                .iter()
                .filter(|t| !t.name().starts_with("__"))
        };
        let matches = |s: &str| s.to_lowercase().contains(&needle);
        rows.push(DocRow::Heading("Results".into()));
        rows.extend(
            user_types()
                .filter(|t| matches(t.name()))
                .map(|t| type_link(t.name().to_owned(), t.name())),
        );
        rows.extend(
            schema
                .directives()
                .iter()
                .filter(|d| matches(&format!("@{}", d.name())))
                .map(|d| directive_link(d.name())),
        );
        for ty in
            user_types().filter(|t| matches!(t.kind(), TypeKind::Object | TypeKind::Interface))
        {
            for f in ty.fields().iter().filter(|f| matches(f.name())) {
                let label = format!("{}.{}", ty.name(), f.name());
                if let Some(label) = shown(label, f.deprecation().is_some(), show_deprecated) {
                    rows.push(field_link(label, ty.name(), f.name()));
                }
            }
        }
    }
    DocsPage {
        title: "Schema".into(),
        description: None,
        rows,
    }
}

fn directive_link(name: &str) -> DocRow {
    DocRow::Link {
        label: format!("@{name}"),
        to: DocsView::Directive(name.to_owned()),
    }
}

fn directive_page(d: &Directive) -> DocsPage {
    let mut rows = vec![DocRow::Plain(directive_signature(d))];
    if d.is_repeatable() {
        rows.push(DocRow::Plain("Repeatable".into()));
    }
    section(&mut rows, "Arguments", input_links(d.args()));
    let locations = d.locations().iter().cloned().map(DocRow::Plain).collect();
    section(&mut rows, "Locations", locations);
    DocsPage {
        title: format!("@{}", d.name()),
        description: d.description().map(str::to_owned),
        rows,
    }
}

fn field_link(label: String, owner: &str, field: &str) -> DocRow {
    DocRow::Link {
        label,
        to: DocsView::Field {
            owner: owner.to_owned(),
            field: field.to_owned(),
        },
    }
}

// `name: T = default`, the form of an argument or an input field.
fn input_label(value: &InputValue) -> String {
    let base = format!("{}: {}", value.name(), value.ty().display());
    match value.default_value() {
        Some(default) => format!("{base} = {default}"),
        None => base,
    }
}

fn input_links(values: &[InputValue]) -> Vec<DocRow> {
    values
        .iter()
        .map(|v| type_link(input_label(v), v.ty().named()))
        .collect()
}

fn type_page(ty: &SchemaType, show_deprecated: bool) -> DocsPage {
    let mut rows = Vec::new();
    let fields = ty
        .fields()
        .iter()
        .filter_map(|f| {
            let args = f
                .args()
                .iter()
                .map(|a| format!("{}: {}", a.name(), a.ty().display()))
                .collect::<Vec<_>>();
            let label = if args.is_empty() {
                format!("{}: {}", f.name(), f.ty().display())
            } else {
                format!("{}({}): {}", f.name(), args.join(", "), f.ty().display())
            };
            let label = shown(label, f.deprecation().is_some(), show_deprecated)?;
            Some(field_link(label, ty.name(), f.name()))
        })
        .collect();
    section(&mut rows, "Fields", fields);
    section(&mut rows, "Input fields", input_links(ty.input_fields()));
    let values = ty
        .enum_values()
        .iter()
        .filter_map(|v| {
            shown(
                v.name().to_owned(),
                v.deprecation().is_some(),
                show_deprecated,
            )
        })
        .map(DocRow::Plain)
        .collect();
    section(&mut rows, "Values", values);
    let names = |list: &[String]| list.iter().map(|n| type_link(n.clone(), n)).collect();
    section(&mut rows, "Implements", names(ty.interfaces()));
    section(&mut rows, "Possible types", names(ty.possible_types()));
    DocsPage {
        title: ty.name().to_owned(),
        description: ty.description().map(str::to_owned),
        rows,
    }
}

fn field_page(title: String, field: &Field) -> DocsPage {
    let mut rows = Vec::new();
    if let Some(reason) = field.deprecation() {
        rows.push(DocRow::Plain(format!("Deprecated: {reason}")));
    }
    section(&mut rows, "Arguments", input_links(field.args()));
    rows.push(DocRow::Heading("Type".into()));
    rows.push(type_link(
        field.ty().display().to_owned(),
        field.ty().named(),
    ));
    DocsPage {
        title,
        description: field.description().map(str::to_owned),
        rows,
    }
}

/// The docs panel for one request URL. It follows the app's `SchemaStore` and emits
/// `SchemaAction::Load` when the user asks for a schema it does not have.
pub struct DocsPanel {
    url: String,
    stack: Vec<DocsView>,
    search: Entity<InputState>,
    show_deprecated: bool,
    rows: Vec<DocRow>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<SchemaAction> for DocsPanel {}

impl DocsPanel {
    /// An empty panel at the root, keyed on no URL yet.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search the schema"));
        let subs = vec![
            cx.observe_global::<SchemaStore>(|_, cx| cx.notify()),
            cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
        ];
        DocsPanel {
            url: String::new(),
            stack: vec![DocsView::Root],
            search,
            show_deprecated: false,
            rows: Vec::new(),
            _subs: subs,
        }
    }

    /// Key the panel on `url`, the request's raw URL; a different URL returns to the root.
    pub fn set_url(&mut self, url: String, cx: &mut Context<Self>) {
        if self.url != url {
            self.url = url;
            self.stack.truncate(1);
            cx.notify();
        }
    }

    /// The view shown now.
    pub fn view(&self) -> &DocsView {
        self.stack.last().unwrap_or(&DocsView::Root)
    }

    /// Shows `view`, remembering the current one for `back`.
    pub fn open(&mut self, view: DocsView, cx: &mut Context<Self>) {
        self.stack.push(view);
        cx.notify();
    }

    /// Returns to the previous view; the root stays when there is none.
    pub fn back(&mut self, cx: &mut Context<Self>) {
        if self.stack.len() > 1 {
            self.stack.pop();
            cx.notify();
        }
    }

    /// Whether deprecated fields and values are listed.
    pub fn show_deprecated(&self) -> bool {
        self.show_deprecated
    }

    fn status(&self, id: &'static str, text: String, cx: &App) -> Stateful<Div> {
        div()
            .id(id)
            .text_size(style::TEXT_SMALL)
            .text_color(cx.theme().muted_foreground)
            .child(text)
    }

    // The loaded page: header, root-only search, toggle, back, then the rows as a virtual list.
    fn loaded(&mut self, schema: &Schema, cx: &mut Context<Self>) -> Div {
        let view = self.view().clone();
        let search = self.search.read(cx).value().to_string();
        let page = page(schema, &view, &search, self.show_deprecated);
        self.rows = page.rows;
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let back = (self.stack.len() > 1).then(|| {
            Button::new("graphql-docs-back")
                .label("Back")
                .on_click(cx.listener(|this, _, _, cx| this.back(cx)))
        });
        let search_box = (view == DocsView::Root).then(|| {
            div()
                .id("graphql-docs-search")
                .test_support()
                .child(ui::padded_input(
                    Input::new(&self.search).bg(ui::field_fill(false, cx)),
                    &self.search,
                ))
        });
        let toggle = ui::checkbox("graphql-docs-deprecated", self.show_deprecated)
            .label("Show deprecated")
            .on_click(cx.listener(|this, on: &bool, _, cx| {
                this.show_deprecated = *on;
                cx.notify();
            }));
        let list = uniform_list(
            "graphql-docs-rows",
            self.rows.len(),
            cx.processor(|this: &mut Self, range: Range<usize>, _, cx| {
                range.map(|ix| this.row(ix, cx)).collect::<Vec<_>>()
            }),
        )
        .size_full();
        v_flex()
            .size_full()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .children(back)
                    .child(page.title),
            )
            .children(page.description.map(|d| {
                div()
                    .text_size(style::TEXT_SMALL)
                    .text_color(muted)
                    .child(d)
            }))
            .children(search_box)
            .child(toggle)
            .child(div().flex_1().min_h_0().child(list))
    }

    fn row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let base = div()
            .id(("graphql-docs-row", ix))
            .whitespace_nowrap()
            .text_size(style::TEXT_SMALL);
        match &self.rows[ix] {
            DocRow::Heading(text) => base
                .text_color(theme.muted_foreground)
                .child(text.clone())
                .test_support()
                .into_any_element(),
            DocRow::Plain(text) => base.child(text.clone()).test_support().into_any_element(),
            DocRow::Link { label, to } => {
                let to = to.clone();
                base.text_color(theme.link)
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.open(to.clone(), cx)))
                    .child(label.clone())
                    .test_support()
                    .into_any_element()
            }
        }
    }
}

impl Render for DocsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body: AnyElement = match graphql_schema_store::state(cx, &self.url) {
            None => v_flex()
                .gap_2()
                .child(self.status(
                    "graphql-docs-hint",
                    "Load the schema to browse its types.".into(),
                    cx,
                ))
                .child(
                    Button::new("graphql-docs-load")
                        .label("Load schema")
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(SchemaAction::Load))),
                )
                .into_any_element(),
            Some(SchemaState::Loading) => self
                .status("graphql-docs-loading", "Loading schema".into(), cx)
                .test_support()
                .into_any_element(),
            Some(SchemaState::Failed(message)) => self
                .status("graphql-docs-error", message, cx)
                .text_color(cx.theme().danger)
                .test_support()
                .into_any_element(),
            Some(SchemaState::Loaded(schema)) => self.loaded(&schema, cx).into_any_element(),
        };
        v_flex()
            .id("graphql-docs")
            .test_support()
            .size_full()
            .gap_2()
            .child(body)
    }
}
