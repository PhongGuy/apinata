//! The Query section of a GraphQL request: a schema row, then a query editor above a variables
//! editor.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use apinata_core::{BodySpec, QuerySyntaxError, VariableSet, check_query, format_query};
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Editor, EditorState, Enter, InputEvent, ShowDocumentHandler};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::*;

use crate::command::Edit;
use crate::graphql_check::{
    CHECK_DELAY, CheckBasis, FORMAT_OFF_THREAD_BYTES, Jobs, editor_diagnostics, syntax_message,
};
use crate::graphql_complete::SchemaCompletions;
use crate::graphql_docs::{DocsPanel, DocsView};
use crate::graphql_hover::{SchemaSymbols, docs_view};
use crate::graphql_schema_store::{self, CheckKey, Lookup, SchemaState, SchemaStore};
use crate::graphql_variables::VariableTools;
use crate::schema_load::load_from_file;
use crate::section::SectionEdit;
use crate::style;
use crate::theme::SelectedStyle;
use crate::ui::ChooseButton as _;
use crate::var_tokens::VarField;

/// The key context on the query editor's wrapper, where F12 jumps to a definition.
pub const GRAPHQL_QUERY_CONTEXT: &str = "GraphqlQuery";

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
    // Live check: job numbers, the typing-pause timer, the running validation, the schema
    // key the last check started against, and what the last finished check used.
    jobs: Jobs,
    pending_check: Option<Task<()>>,
    running_check: Option<Task<()>>,
    checked_key: Option<CheckKey>,
    basis: Option<CheckBasis>,
    // The last refused Beautify, and the off-thread format a second Beautify replaces.
    beautify_error: Option<String>,
    beautify_task: Option<Task<()>>,
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
        let symbols = Rc::new(SchemaSymbols::new(completion_url.clone()));
        let section = cx.weak_entity();
        // The handler runs inside the query editor's update, so it must not read that editor.
        let show_document: ShowDocumentHandler = Rc::new(move |params, _, cx| {
            let Some(view) = docs_view(params.uri.as_str()) else {
                return false;
            };
            section.update(cx, |s, cx| s.show_docs(view, cx)).is_ok()
        });
        query.update(cx, |s, _| {
            let lsp = s.lsp_mut();
            lsp.completion_provider = Some(Rc::new(provider));
            lsp.hover_provider = Some(symbols.clone());
            lsp.definition_provider = Some(symbols);
            lsp.show_document = Some(show_document);
        });
        let variables = cx.new(|cx| EditorState::new(window, cx).language("json"));
        let tools = Rc::new(VariableTools::new(
            query.downgrade(),
            completion_url.clone(),
        ));
        variables.update(cx, |s, _| {
            let lsp = s.lsp_mut();
            lsp.completion_provider = Some(tools.clone());
            lsp.hover_provider = Some(tools);
        });
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
            // A query edit is emitted like a variables edit, then drops a stale format error and
            // schedules a check.
            cx.subscribe(&query, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.emit(cx);
                    if this.beautify_error.take().is_some() {
                        cx.notify();
                    }
                    this.schedule_check(cx);
                }
            }),
            cx.subscribe(&variables, on_change),
            // A schema that settles on a key other than the last check's gets a new check.
            cx.observe_global::<SchemaStore>(|this, cx| {
                let key = graphql_schema_store::compiled(&this.schema_url, cx).key();
                if key.is_some() && key != this.checked_key {
                    this.schedule_check(cx);
                }
                cx.notify();
            }),
        ];
        GraphqlSection {
            query,
            variables,
            vars: None,
            schema_url: String::new(),
            completion_url,
            docs,
            docs_open: false,
            jobs: Jobs::default(),
            pending_check: None,
            running_check: None,
            checked_key: None,
            basis: None,
            beautify_error: None,
            beautify_task: None,
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
        // editor fires no change event for a programmatic set, so nothing is emitted; a changed
        // query is checked anyway and its old format error is cleared here.
        if *self.query.read(cx).value() != *query {
            self.beautify_error = None;
            self.beautify_task = None;
            self.schedule_check(cx);
        }
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
            self.schedule_check(cx);
            cx.notify();
        }
    }

    /// What the last finished query check was measured against.
    pub fn check_basis(&self) -> Option<&CheckBasis> {
        self.basis.as_ref()
    }

    /// Formats the query as one undoable `replace_all`; off the UI thread above FORMAT_OFF_THREAD_BYTES.
    pub fn beautify_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.beautify_task = None;
        let text = self.query.read(cx).value().to_string();
        // Blank text has nothing to lay out; it is neither an error nor an edit.
        if text.trim().is_empty() {
            self.beautify_error = None;
            cx.notify();
            return;
        }
        if text.len() < FORMAT_OFF_THREAD_BYTES {
            let result = format_query(&text);
            self.apply_beautify(&text, result, window, cx);
            return;
        }
        let snapshot = text.clone();
        let run = cx.background_spawn(async move { format_query(&text) });
        self.beautify_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = run.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.apply_beautify(&snapshot, result, window, cx)
            });
        }));
    }

    /// Whether the query has non-blank text to format.
    pub fn can_beautify_query(&self, cx: &App) -> bool {
        !self.query.read(cx).value().trim().is_empty()
    }

    /// The last refused Beautify, cleared by the next query edit.
    pub fn beautify_error(&self) -> Option<&str> {
        self.beautify_error.as_deref()
    }

    // Applies a format result only to the text it was computed from, so an edit made while an
    // off-thread format ran is never overwritten.
    fn apply_beautify(
        &mut self,
        snapshot: &str,
        result: Result<String, QuerySyntaxError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if *self.query.read(cx).value() != *snapshot {
            return;
        }
        match result {
            Ok(laid_out) => {
                self.beautify_error = None;
                if laid_out != snapshot {
                    self.query
                        .update(cx, |q, cx| q.replace_all(laid_out, window, cx));
                }
            }
            Err(err) => self.beautify_error = Some(syntax_message(snapshot, &err)),
        }
        cx.notify();
    }

    // Restart the typing pause; dropping the previous timer task cancels it.
    fn schedule_check(&mut self, cx: &mut Context<Self>) {
        self.pending_check = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(CHECK_DELAY).await;
            let _ = this.update(cx, |this, cx| this.start_check(cx));
        }));
    }

    // Snapshot the text and the schema, validate off the UI thread, apply only the newest
    // result, and only to the text it was computed from.
    fn start_check(&mut self, cx: &mut Context<Self>) {
        let lookup = graphql_schema_store::compiled(&self.schema_url, cx);
        // Pending forgets the last key, so whatever key the store settles on reschedules, and a
        // check still running against the old schema must not apply meanwhile.
        let Some(key) = lookup.key() else {
            self.checked_key = None;
            self.running_check = None;
            return;
        };
        self.checked_key = Some(key);
        let schema = match &lookup {
            Lookup::Ready { schema, .. } => Some(schema.clone()),
            _ => None,
        };
        let source = self.schema_source(cx);
        let text = self.query.read(cx).value().to_string();
        let job = self.jobs.start();
        let run = cx.background_spawn(async move {
            let check = check_query(&text, schema.as_deref());
            (text, check)
        });
        self.running_check = Some(cx.spawn(async move |this, cx| {
            let (text, check) = run.await;
            let _ = this.update(cx, |this, cx| {
                if !this.jobs.is_latest(job) {
                    return;
                }
                this.query.update(cx, |e, cx| {
                    if *e.value() != *text {
                        return;
                    }
                    let rope = e.text().clone();
                    if let Some(set) = e.diagnostics_mut() {
                        set.reset(&rope);
                        set.extend(editor_diagnostics(&rope, check.diagnostics()));
                    }
                    cx.notify();
                });
                this.basis = Some(CheckBasis::of(&lookup, &source, check.schema_use()));
                cx.notify();
            });
        }));
    }

    // The schema file's name while a load from it is in place, else the URL; a failed file
    // load still records its file, so the name only counts once the schema loaded.
    fn schema_source(&self, cx: &App) -> String {
        match graphql_schema_store::state(cx, &self.schema_url) {
            Some(SchemaState::Loaded(_)) => graphql_schema_store::source(cx, &self.schema_url),
            _ => None,
        }
        .unwrap_or_else(|| self.schema_url.clone())
    }

    /// The raw URL the schema row is keyed on.
    pub fn schema_url(&self) -> &str {
        &self.schema_url
    }

    /// The docs explorer beside the editors.
    pub fn docs(&self) -> &Entity<DocsPanel> {
        &self.docs
    }

    /// Opens the Docs panel on `view`. It never reads the query editor, since the jump runs
    /// inside that editor's update.
    pub fn show_docs(&mut self, view: DocsView, cx: &mut Context<Self>) {
        self.docs_open = true;
        self.docs.update(cx, |d, cx| d.open(view, cx));
        cx.notify();
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
        let status = self.basis.as_ref().map(|basis| {
            div()
                .id("graphql-check-status")
                .test_support()
                .text_size(style::TEXT_SMALL)
                .text_color(cx.theme().muted_foreground)
                .child(basis.label())
        });
        h_flex()
            .flex_wrap()
            .gap_2()
            .items_center()
            .child(lead)
            .child(file)
            .child(toggle)
            .children(error)
            .children(status)
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
        let beautify_error = self.beautify_error.clone().map(|message| {
            div()
                .id("graphql-beautify-error")
                .test_support()
                .text_xs()
                .text_color(cx.theme().danger)
                .child(message)
        });
        let editors = v_flex()
            .flex_1()
            .min_w_0()
            .gap_2()
            .child(
                // Accepting a completion lets Enter bubble on, and an unhandled Enter is then
                // typed as a newline; stopping it at each wrapper ends the key at the accepted item.
                self.editor("graphql-query", "Query", &self.query)
                    .key_context(GRAPHQL_QUERY_CONTEXT)
                    .on_action(|_: &Enter, _, cx| cx.stop_propagation()),
            )
            .children(beautify_error)
            .child(
                self.editor("graphql-variables", "Variables", &self.variables)
                    .on_action(|_: &Enter, _, cx| cx.stop_propagation()),
            );
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
