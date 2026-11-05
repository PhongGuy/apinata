//! The Query section of a GraphQL request: a schema row, then a query editor above a variables
//! editor.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use apinata_core::{BodySpec, VariableSet};
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Editor, EditorState, Enter, InputEvent};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::*;

use crate::command::Edit;
use crate::graphql_complete::SchemaCompletions;
use crate::graphql_docs::DocsPanel;
use crate::graphql_schema_store::{self, SchemaState, SchemaStore};
use crate::schema_load::load_from_file;
use crate::section::SectionEdit;
use crate::style;
use crate::theme::SelectedStyle;
use crate::ui::ChooseButton as _;
use crate::var_tokens::VarField;

/// The docs explorer's width beside the editors.
const DOCS_WIDTH: Rems = style::ui(320.);

/// Edits a GraphQL body. Any change to either editor emits the whole body, query and
/// variables together, since the file stores them as one `body` mapping.
pub struct GraphqlSection {
    query: Entity<EditorState>,
    variables: Entity<EditorState>,
    vars: Option<Arc<VariableSet>>,
    schema_url: String,
    // The same URL, shared with the query editor's completion provider.
    completion_url: Rc<RefCell<String>>,
    docs: Entity<DocsPanel>,
    docs_open: bool,
    _subs: Vec<Subscription>,
}

/// A request from the section to its editor, which knows the request to load a schema for.
pub enum SchemaAction {
    /// Load, or reload, the schema from the request's server.
    Load,
}

impl EventEmitter<SectionEdit> for GraphqlSection {}
impl EventEmitter<SchemaAction> for GraphqlSection {}

impl GraphqlSection {
    /// Two empty editors: the query as plain text, the variables as JSON.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| EditorState::new(window, cx).language("graphql"));
        let completion_url = Rc::new(RefCell::new(String::new()));
        let provider = SchemaCompletions::new(completion_url.clone());
        query.update(cx, |s, _| {
            s.lsp_mut().completion_provider = Some(Rc::new(provider))
        });
        let variables = cx.new(|cx| EditorState::new(window, cx).language("json"));
        let on_change = |this: &mut Self, _, event: &InputEvent, cx: &mut Context<Self>| {
            if matches!(event, InputEvent::Change) {
                this.emit(cx);
            }
        };
        let docs = cx.new(|cx| DocsPanel::new(window, cx));
        let subs = vec![
            cx.subscribe(&docs, |_, _, _: &SchemaAction, cx| {
                cx.emit(SchemaAction::Load)
            }),
            cx.subscribe(&query, on_change),
            cx.subscribe(&variables, on_change),
            cx.observe_global::<SchemaStore>(|_, cx| cx.notify()),
        ];
        GraphqlSection {
            query,
            variables,
            vars: None,
            schema_url: String::new(),
            completion_url,
            docs,
            docs_open: false,
            _subs: subs,
        }
    }

    /// Show `body` without emitting; a non-GraphQL body shows two empty editors.
    pub fn load(&mut self, body: &BodySpec, window: &mut Window, cx: &mut Context<Self>) {
        let (query, variables) = match body {
            BodySpec::Graphql { query, variables } => (query.as_str(), variables.as_str()),
            _ => ("", ""),
        };
        // Only differing text is set, so an echo of the user's typing keeps the caret. The
        // editor fires no change event for a programmatic set, so nothing is emitted.
        for (editor, text) in [(&self.query, query), (&self.variables, variables)] {
            crate::ui::sync_editor_text(editor, text, window, cx);
        }
        cx.notify();
    }

    /// The query editor.
    pub fn query(&self) -> &Entity<EditorState> {
        &self.query
    }

    /// The focused editor's caret in window coordinates, with its text length and offset to tell
    /// edits and moves apart, or `None` when neither has focus or none has been laid out. The
    /// editor reports a caret without its own vertical scroll, so that is added back to give
    /// where the caret is drawn.
    pub fn caret(&self, window: &Window, cx: &App) -> Option<(Bounds<Pixels>, (usize, usize))> {
        [&self.query, &self.variables].into_iter().find_map(|e| {
            let state = e.read(cx);
            if !state.focus_handle(cx).is_focused(window) {
                return None;
            }
            let (mut caret, _) = state.cursor_layout()?;
            caret.origin.y += state.text_bounds()?.origin.y - state.input_bounds().origin.y;
            Some((caret, (state.value().len(), state.cursor())))
        })
    }

    /// The variables editor.
    pub fn variables(&self) -> &Entity<EditorState> {
        &self.variables
    }

    /// Explain `{{x}}` placeholders on hover in both editors; `None` turns it off.
    pub fn set_variables(&mut self, set: Option<Arc<VariableSet>>, cx: &mut Context<Self>) {
        self.vars = set;
        cx.notify();
    }

    /// Key the schema row on `url`, the request's raw URL.
    pub fn set_schema_url(&mut self, url: String, cx: &mut Context<Self>) {
        if self.schema_url != url {
            self.docs.update(cx, |d, cx| d.set_url(url.clone(), cx));
            self.completion_url.replace(url.clone());
            if !url.is_empty() {
                crate::schema_load::load_from_cache(url.clone(), cx);
            }
            self.schema_url = url;
            cx.notify();
        }
    }

    /// The raw URL the schema row is keyed on.
    pub fn schema_url(&self) -> &str {
        &self.schema_url
    }

    /// The docs explorer beside the editors.
    pub fn docs(&self) -> &Entity<DocsPanel> {
        &self.docs
    }

    /// Whether the docs explorer is shown.
    pub fn docs_open(&self) -> bool {
        self.docs_open
    }

    // Asks for one schema file, introspection JSON or SDL, and loads it for the current URL.
    fn pick_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = reply.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                load_from_file(this.schema_url.clone(), path, cx)
            });
        })
        .detach();
    }

    // Load or Refresh by the store's state for the URL, the file button, and any error inline.
    fn schema_row(&self, cx: &mut Context<Self>) -> Div {
        let state = graphql_schema_store::state(cx, &self.schema_url);
        let load = |id: &'static str, label: &'static str| {
            Button::new(id)
                .label(label)
                .on_click(cx.listener(|_, _, _, cx| cx.emit(SchemaAction::Load)))
        };
        let lead: AnyElement = match &state {
            None | Some(SchemaState::Failed(_)) => {
                load("graphql-schema-load", "Load schema").into_any_element()
            }
            Some(SchemaState::Loading) => div()
                .id("graphql-schema-loading")
                .test_support()
                .text_size(style::TEXT_SMALL)
                .text_color(cx.theme().muted_foreground)
                .child("Loading schema")
                .into_any_element(),
            Some(SchemaState::Loaded(_)) => {
                load("graphql-schema-refresh", "Refresh schema").into_any_element()
            }
        };
        let error = match state {
            Some(SchemaState::Failed(message)) => Some(
                div()
                    .id("graphql-schema-error")
                    .test_support()
                    .text_size(style::TEXT_SMALL)
                    .text_color(cx.theme().danger)
                    .child(message),
            ),
            _ => None,
        };
        let toggle = Button::new("graphql-docs-toggle")
            .label("Docs")
            .choose(self.docs_open, SelectedStyle::Plain, cx)
            .on_click(cx.listener(|this, _, _, cx| {
                this.docs_open = !this.docs_open;
                cx.notify();
            }));
        let file = Button::new("graphql-schema-file")
            .label("Load schema from file")
            .on_click(cx.listener(|this, _, window, cx| this.pick_file(window, cx)));
        h_flex()
            .flex_wrap()
            .gap_2()
            .items_center()
            .child(lead)
            .child(file)
            .child(toggle)
            .children(error)
    }

    fn emit(&mut self, cx: &mut Context<Self>) {
        let body = BodySpec::Graphql {
            query: self.query.read(cx).value().to_string(),
            variables: self.variables.read(cx).value().to_string(),
        };
        cx.emit(SectionEdit::new(Edit::Body(body)));
    }

    // One editor at the Body section's code height, wrapped for placeholder hovers.
    fn editor(&self, id: &'static str, label: &'static str, state: &Entity<EditorState>) -> Div {
        v_flex().gap_1().child(label).child(
            div()
                .id(id)
                .test_support()
                .w_full()
                .min_h(style::EDITOR_MIN_HEIGHT)
                .h(style::EDITOR_HEIGHT)
                .child(
                    VarField::editor(
                        SharedString::from(format!("{id}-vars")),
                        state,
                        crate::ui::ringed_editor(Editor::new(state).h_full(), state),
                        self.vars.clone(),
                    )
                    .size_full(),
                ),
        )
    }
}

impl Render for GraphqlSection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editors = v_flex()
            .flex_1()
            .min_w_0()
            .gap_2()
            .child(
                // Accepting a schema completion lets Enter bubble on, and an unhandled Enter is
                // then typed as a newline; stopping it here ends the key at the accepted word.
                self.editor("graphql-query", "Query", &self.query)
                    .on_action(|_: &Enter, _, cx| cx.stop_propagation()),
            )
            .child(self.editor("graphql-variables", "Variables", &self.variables));
        // The panel carries the `graphql-docs` id itself, so the wrapper only sizes it.
        let docs = self.docs_open.then(|| {
            div()
                .flex_none()
                .w(DOCS_WIDTH)
                .h_full()
                .child(self.docs.clone())
        });
        v_flex()
            .size_full()
            .gap_2()
            .child(self.schema_row(cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .gap_2()
                    .child(editors)
                    .children(docs),
            )
    }
}
