//! The Body section: a mode picker over a code editor or a key-value table.

use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

pub use apinata_core::collection_relative;
use apinata_core::{BodySpec, JsonSyntaxError, VariableSet, json_syntax_error};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{
    Editor, EditorState, InputEvent, RangeDecoration, RangeDecorationCollection,
    RangeDecorationStyle, TextDecoration, TextDecorationCollection,
};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::v_flex;
use gpui_kit::component::{ActiveTheme as _, IndexPath, Sizable as _, ThemeMode, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Entity, EventEmitter, HighlightStyle, PathPromptOptions, Result, SharedString,
    Subscription, TestSupportExt, Window, div,
};

use crate::body_text::{beautify_json, beautify_xml, minify_json, minify_xml};
use crate::command::Edit;
use crate::kv_table::{
    KvColumns, KvEvent, KvRow, KvTable, PickFiles, file_rows, files_from, header_rows,
    headers_from, lags, multipart_from, multipart_rows,
};
use crate::section::SectionEdit;
use crate::style;
use crate::theme::{VarKind, var_tint};
use crate::ui::{IdItem, IdSelect};
use crate::var_tokens::{VarField, tokens};

/// JSON bodies larger than this are neither parsed nor marked, so typing in a huge body stays cheap.
pub const JSON_CHECK_MAX_BYTES: usize = 2 * 1024 * 1024;

/// A body mode the editor models, in Bruno's picker order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(missing_docs)]
pub enum BodyMode {
    None,
    Json,
    Xml,
    Text,
    Sparql,
    Form,
    Multipart,
    File,
}

impl BodyMode {
    /// Every mode, in picker order.
    pub const ALL: [BodyMode; 8] = [
        BodyMode::None,
        BodyMode::Json,
        BodyMode::Xml,
        BodyMode::Text,
        BodyMode::Sparql,
        BodyMode::Form,
        BodyMode::Multipart,
        BodyMode::File,
    ];

    /// The picker's label.
    pub fn label(self) -> &'static str {
        match self {
            BodyMode::None => "No Body",
            BodyMode::Json => "JSON",
            BodyMode::Xml => "XML",
            BodyMode::Text => "Text",
            BodyMode::Sparql => "SPARQL",
            BodyMode::Form => "Form URL Encoded",
            BodyMode::Multipart => "Multipart Form",
            BodyMode::File => "File",
        }
    }

    /// The stable key the mode's select row is found by, as the file names the mode.
    pub fn key(self) -> &'static str {
        match self {
            BodyMode::None => "none",
            BodyMode::Json => "json",
            BodyMode::Xml => "xml",
            BodyMode::Text => "text",
            BodyMode::Sparql => "sparql",
            BodyMode::Form => "form-urlencoded",
            BodyMode::Multipart => "multipart-form",
            BodyMode::File => "file",
        }
    }

    /// The mode of `body`; `None` for a mode the editor does not model.
    pub fn of(body: &BodySpec) -> Option<BodyMode> {
        Some(match body {
            BodySpec::None => BodyMode::None,
            BodySpec::Json(_) => BodyMode::Json,
            BodySpec::Xml(_) => BodyMode::Xml,
            BodySpec::Text(_) => BodyMode::Text,
            BodySpec::Sparql(_) => BodyMode::Sparql,
            BodySpec::FormUrlEncoded(_) => BodyMode::Form,
            BodySpec::Multipart(_) => BodyMode::Multipart,
            BodySpec::File(_) => BodyMode::File,
            _ => return None,
        })
    }

    /// This mode with no content.
    pub fn empty(self) -> BodySpec {
        match self {
            BodyMode::None => BodySpec::None,
            BodyMode::Json => BodySpec::Json(String::new()),
            BodyMode::Xml => BodySpec::Xml(String::new()),
            BodyMode::Text => BodySpec::Text(String::new()),
            BodyMode::Sparql => BodySpec::Sparql(String::new()),
            BodyMode::Form => BodySpec::FormUrlEncoded(Vec::new()),
            BodyMode::Multipart => BodySpec::Multipart(Vec::new()),
            BodyMode::File => BodySpec::File(Vec::new()),
        }
    }

    // The code mode's spec holding `text`; `None` for modes without a code editor.
    fn with_text(self, text: String) -> Option<BodySpec> {
        match self {
            BodyMode::Json => Some(BodySpec::Json(text)),
            BodyMode::Xml => Some(BodySpec::Xml(text)),
            BodyMode::Text => Some(BodySpec::Text(text)),
            BodyMode::Sparql => Some(BodySpec::Sparql(text)),
            _ => None,
        }
    }

    // Whether Beautify applies: only the two structured text modes.
    fn beautifiable(self) -> bool {
        matches!(self, BodyMode::Json | BodyMode::Xml)
    }

    fn is_table(self) -> bool {
        matches!(self, BodyMode::Form | BodyMode::Multipart | BodyMode::File)
    }
}

// The table columns and rows a table body shows; `None` for the other modes.
fn table_of(body: &BodySpec) -> Option<(KvColumns, Vec<KvRow>)> {
    match body {
        BodySpec::FormUrlEncoded(kv) => Some((KvColumns::DESCRIBED, header_rows(kv))),
        BodySpec::Multipart(fields) => Some((KvColumns::MULTIPART, multipart_rows(fields))),
        BodySpec::File(files) => Some((KvColumns::FILE_BODY, file_rows(files))),
        _ => None,
    }
}

// The table body `rows` describe, editing clones of the entries in `base` they came from.
fn table_body(base: &BodySpec, rows: &[KvRow]) -> Option<BodySpec> {
    match base {
        BodySpec::FormUrlEncoded(kv) => Some(BodySpec::FormUrlEncoded(headers_from(kv, rows))),
        BodySpec::Multipart(fields) => Some(BodySpec::Multipart(multipart_from(fields, rows))),
        BodySpec::File(files) => Some(BodySpec::File(files_from(files, rows))),
        _ => None,
    }
}

// The text a code body shows and the grammar it is highlighted with. No XML grammar is
// bundled, so XML borrows the HTML one, which colours its tags and attributes; SPARQL has no
// grammar either and stays plain.
fn code_of(body: &BodySpec) -> Option<(&str, &'static str)> {
    match body {
        BodySpec::Json(t) => Some((t, "json")),
        BodySpec::Xml(t) => Some((t, "html")),
        BodySpec::Text(t) | BodySpec::Sparql(t) => Some((t, "text")),
        _ => None,
    }
}

// The muted hint the empty code editor shows under each code mode.
fn hint(mode: BodyMode) -> &'static str {
    match mode {
        BodyMode::Json => "Write the JSON body here.",
        BodyMode::Xml => "Write the XML body here.",
        BodyMode::Sparql => "Write the SPARQL query here.",
        _ => "Write the body here.",
    }
}

/// Edits a request body. Each mode remembers its last value for the life of the view, as
/// Bruno's draft does, so switching back restores it.
pub struct BodySection {
    root: PathBuf,
    mode: Option<BodyMode>,
    // The table body last put on screen; table rows point into it by index.
    base: BodySpec,
    unsupported: Option<String>,
    // Why the last Beautify refused the text; cleared by the next edit or mode change.
    beautify_error: Option<String>,
    // The JSON syntax error of the current text, set on edit and load, never while rendering.
    json_error: Option<JsonSyntaxError>,
    select: Entity<IdSelect>,
    code: Entity<EditorState>,
    table: Entity<KvTable>,
    memory: HashMap<BodyMode, BodySpec>,
    vars: Option<Arc<VariableSet>>,
    // The editor's placeholder tints and plain text colour, the ranges behind them, and the
    // theme mode and runtime revision they were coloured and classified under.
    decorations: RangeDecorationCollection,
    text_colour: TextDecorationCollection,
    fills: Vec<(Range<usize>, VarKind)>,
    decorated: Option<(ThemeMode, u64)>,
    // Whether the code editor wraps long lines, and the Preferences default last applied.
    wrap: bool,
    default_wrap: bool,
    _subs: Vec<Subscription>,
}

impl EventEmitter<SectionEdit> for BodySection {}

impl BodySection {
    /// An empty body section for the collection at `root`.
    pub fn new(root: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let labels: Vec<IdItem> = BodyMode::ALL
            .iter()
            .map(|m| IdItem::new("body-mode", m.key(), m.label()))
            .collect();
        let select = cx.new(|cx| SelectState::new(labels, Some(IndexPath::new(0)), window, cx));
        let default_wrap = crate::settings::wrap_editors(cx);
        let code = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("text")
                .soft_wrap(default_wrap)
        });
        let (decorations, text_colour) = code.update(cx, |s, cx| {
            (
                s.create_range_decorations_collection(Vec::new(), cx),
                s.create_decorations_collection(Vec::new(), cx),
            )
        });
        let table = cx.new(|cx| KvTable::new(window, cx));
        let mut subs = Vec::new();
        if let Some(app) = cx.try_global::<crate::app::AppState>() {
            let settings = app.settings.clone();
            subs.push(cx.observe_in(&settings, window, |this, _, window, cx| {
                let want = crate::settings::wrap_editors(cx);
                if want != this.default_wrap {
                    this.default_wrap = want;
                    this.set_wrap(want, window, cx);
                }
            }));
        }
        subs.extend([
            cx.subscribe_in(
                &select,
                window,
                |this, _, event: &SelectEvent<Vec<IdItem>>, window, cx| {
                    let SelectEvent::Confirm(Some(label)) = event else {
                        return;
                    };
                    if let Some(mode) = BodyMode::ALL
                        .into_iter()
                        .find(|m| m.label() == label.as_ref())
                    {
                        this.set_mode(mode, window, cx);
                    }
                },
            ),
            cx.subscribe(&code, |this, state, event: &InputEvent, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                this.decorate(cx);
                if this.beautify_error.take().is_some() {
                    cx.notify();
                }
                let text = state.read(cx).value().to_string();
                this.check_json(&text);
                if let Some(body) = this.mode.and_then(|m| m.with_text(text)) {
                    this.emit(body, None, false, cx);
                }
            }),
            cx.subscribe(&table, |this, table, event: &KvEvent, cx| {
                let KvEvent::Changed(rows) = event;
                if let Some(body) = table_body(&this.base, rows) {
                    let typed = this.table.read(cx).last_typed();
                    let lagging = lags(&table, rows, cx);
                    this.emit(body, typed, lagging, cx);
                }
            }),
            cx.subscribe_in(&table, window, |this, _, PickFiles(ix), window, cx| {
                this.pick(*ix, window, cx);
            }),
        ]);
        BodySection {
            root,
            mode: Some(BodyMode::None),
            base: BodySpec::None,
            unsupported: None,
            beautify_error: None,
            json_error: None,
            select,
            code,
            table,
            memory: HashMap::new(),
            vars: None,
            decorations,
            text_colour,
            fills: Vec::new(),
            decorated: None,
            wrap: default_wrap,
            default_wrap,
            _subs: subs,
        }
    }

    /// Shows `body` from a snapshot without emitting an edit.
    pub fn load(&mut self, body: &BodySpec, window: &mut Window, cx: &mut Context<Self>) {
        match BodyMode::of(body) {
            Some(mode) => {
                self.memory.insert(mode, body.clone());
                self.show(mode, body, window, cx);
            }
            None => {
                self.mode = None;
                self.unsupported = Some(match body {
                    BodySpec::Unsupported { kind } => kind.clone(),
                    _ => "unknown".into(),
                });
                self.select
                    .update(cx, |s, cx| s.set_selected_index(None, window, cx));
                cx.notify();
            }
        }
    }

    /// Whether the code editor wraps long lines.
    pub fn wrap(&self) -> bool {
        self.wrap
    }

    /// Wraps or unwraps long lines in the code editor.
    pub fn set_wrap(&mut self, wrap: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.wrap = wrap;
        self.code
            .update(cx, |c, cx| c.set_soft_wrap(wrap, window, cx));
        cx.notify();
    }

    /// The mode shown; `None` while the body's mode is not modelled.
    pub fn mode(&self) -> Option<BodyMode> {
        self.mode
    }

    /// Switches to `mode`, sending its remembered value or else its empty value.
    pub fn set_mode(&mut self, mode: BodyMode, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == Some(mode) {
            return;
        }
        let body = self
            .memory
            .get(&mode)
            .cloned()
            .unwrap_or_else(|| mode.empty());
        self.show(mode, &body, window, cx);
        self.emit(body, None, false, cx);
    }

    /// Re-lays out the JSON or XML body by whitespace alone, as an undoable edit. Text that
    /// does not parse stays as written and `beautify_error` says why; other modes do nothing.
    pub fn beautify(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.rewrite(beautify_json, beautify_xml, window, cx);
    }

    /// Whether Beautify and Minify have a body to lay out: JSON or XML with some text.
    pub fn can_reformat(&self, cx: &App) -> bool {
        self.mode.is_some_and(BodyMode::beautifiable)
            && !self.code.read(cx).value().trim().is_empty()
    }

    /// Like [`Self::beautify`], but strips the whitespace between tokens instead.
    pub fn minify(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.rewrite(minify_json, minify_xml, window, cx);
    }

    // Applies a whitespace-only rewrite of the JSON or XML body as one replace_all edit.
    fn rewrite(
        &mut self,
        json: fn(&str) -> Result<String, String>,
        xml: fn(&str) -> Result<String, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let layout = match self.mode {
            Some(BodyMode::Json) => json,
            Some(BodyMode::Xml) => xml,
            _ => return,
        };
        let text = self.code.read(cx).value().to_string();
        // An empty body has nothing to lay out; it is neither an error nor an edit.
        if text.trim().is_empty() {
            self.beautify_error = None;
            cx.notify();
            return;
        }
        match layout(&text) {
            Ok(laid_out) => {
                self.beautify_error = None;
                if laid_out != text {
                    self.code
                        .update(cx, |c, cx| c.replace_all(laid_out, window, cx));
                }
            }
            // The live marker already reports invalid JSON, so a second message would repeat it.
            Err(message) => {
                let marker_shows = self.mode == Some(BodyMode::Json) && self.json_error.is_some();
                self.beautify_error = (!marker_shows).then_some(message);
            }
        }
        cx.notify();
    }

    /// The JSON marker text (`Line 2, column 6: expected value`) while a JSON body is invalid.
    pub fn json_error(&self) -> Option<String> {
        let e = self.json_error.as_ref()?;
        Some(format!(
            "Line {}, column {}: {}",
            e.line, e.column, e.message
        ))
    }

    // Re-parses `text` for the marker; only a JSON body within the size bound is checked.
    fn check_json(&mut self, text: &str) {
        let checked = self.mode == Some(BodyMode::Json) && text.len() <= JSON_CHECK_MAX_BYTES;
        self.json_error = checked.then(|| json_syntax_error(text)).flatten();
    }

    /// The message of the last refused Beautify, while it still applies.
    pub fn beautify_error(&self) -> Option<&str> {
        self.beautify_error.as_deref()
    }

    /// The code editor of the JSON, XML and Text modes.
    pub fn code(&self) -> &Entity<EditorState> {
        &self.code
    }

    /// The table of the form, multipart and file modes.
    pub fn table(&self) -> &Entity<KvTable> {
        &self.table
    }

    /// Tint and explain placeholders in the code editor and the form table; `None` turns it off.
    pub fn set_variables(&mut self, vars: Option<Arc<VariableSet>>, cx: &mut Context<Self>) {
        self.table
            .update(cx, |t, cx| t.set_variables(vars.clone(), cx));
        self.vars = vars;
        self.decorate(cx);
        cx.notify();
    }

    /// The code editor's tinted placeholders, by byte range, as last computed.
    pub fn token_fills(&self) -> &[(Range<usize>, VarKind)] {
        &self.fills
    }

    /// The byte ranges the code editor draws in its plain text colour, over any syntax colour.
    pub fn token_text_ranges(&self, cx: &App) -> Vec<Range<usize>> {
        self.text_colour.get_ranges(cx)
    }

    // Re-classify the editor's placeholders and hand their tints to the editor, which
    // paints them under the text. Without variables every tint is cleared.
    fn decorate(&mut self, cx: &mut Context<Self>) {
        self.fills = match &self.vars {
            Some(vars) => tokens(&self.code.read(cx).value(), vars)
                .into_iter()
                .map(|t| (t.range, t.kind))
                .collect(),
            None => Vec::new(),
        };
        self.decorated = self.vars.as_ref().map(|v| (cx.theme().mode, v.revision()));
        let decorations = self
            .fills
            .iter()
            .map(|(range, kind)| {
                RangeDecoration::new(range.clone())
                    .with_style(RangeDecorationStyle::Fill)
                    .with_color(var_tint(*kind, cx))
            })
            .collect();
        self.decorations.set(decorations, cx);
        // Over a tint the syntax colour (a JSON string's, say) loses contrast, so a
        // placeholder's text takes the editor's plain foreground instead.
        let plain = HighlightStyle {
            color: Some(cx.theme().foreground),
            ..Default::default()
        };
        let text = self
            .fills
            .iter()
            .map(|(range, _)| TextDecoration::new(range.clone(), plain))
            .collect();
        self.text_colour.set(text, cx);
    }

    // Asks for files for table row `ix` and gives the row the answer; a multipart field
    // takes several files, a file body one. A cancelled or failed prompt does nothing.
    fn pick(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let multiple = self.mode == Some(BodyMode::Multipart);
        let reply = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = reply.await else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                let take = if multiple { paths.len() } else { 1 };
                let files: Vec<String> = paths
                    .iter()
                    .take(take)
                    .map(|p| collection_relative(&this.root, p))
                    .collect();
                // A file body sends its content type as written, so a blank one would go out
                // as an empty header; the picked file's kind fills it unless the user typed one.
                // A multipart file part with no type gets one from its name when sent.
                let guess = (paths.first().filter(|_| !multiple))
                    .map(|path| apinata_core::file_content_type(path));
                if !files.is_empty() {
                    this.table.update(cx, |t, cx| {
                        if let Some(guess) = guess {
                            t.fill_blank_content_type(ix, guess, window, cx);
                        }
                        t.set_files(ix, Some(files), window, cx)
                    });
                }
            });
        })
        .detach();
    }

    // Puts `body` on screen under `mode`. The editor text is replaced only when it differs,
    // since a replacement resets the caret and an echo of the user's own typing must not.
    fn show(
        &mut self,
        mode: BodyMode,
        body: &BodySpec,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mode = Some(mode);
        self.unsupported = None;
        self.beautify_error = None;
        let label: SharedString = mode.label().into();
        self.select
            .update(cx, |s, cx| s.set_selected_value(&label, window, cx));
        if let Some((text, language)) = code_of(body) {
            self.code.update(cx, |c, cx| {
                if c.language_name() != language {
                    c.set_highlighter(language, cx);
                }
                c.set_placeholder(hint(mode), window, cx);
            });
            crate::ui::sync_editor_text(&self.code, text, window, cx);
            self.decorate(cx);
            self.check_json(text);
        } else {
            self.json_error = None;
        }
        // Columns change before rows load, so rows never show under another mode's layout.
        if let Some((columns, rows)) = table_of(body) {
            self.base = body.clone();
            let type_hint = match mode {
                BodyMode::File => "Blank sends an empty header",
                _ => "Content type",
            };
            self.table.update(cx, |t, cx| {
                t.set_columns(columns, cx);
                t.set_content_type_placeholder(type_hint, window, cx);
                t.load(rows, window, cx);
            });
        }
        cx.notify();
    }

    /// Whether `edit`, which this section sent, was built from a body a load has since
    /// replaced: the mode shown or the body remembered for it is no longer the one sent.
    pub fn lags(&self, edit: &Edit) -> bool {
        let Edit::Body(sent) = edit else {
            return false;
        };
        let Some(mode) = BodyMode::of(sent) else {
            return false;
        };
        self.mode != Some(mode) || self.memory.get(&mode) != Some(sent)
    }

    fn emit(
        &mut self,
        body: BodySpec,
        typed: Option<(usize, u8)>,
        lagging: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(mode) = BodyMode::of(&body) {
            self.memory.insert(mode, body.clone());
        }
        cx.emit(SectionEdit {
            edit: Edit::Body(body),
            typed,
            lagging,
            loads: 0,
        });
    }
}

impl Render for BodySection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A theme switch recolours the tints and a runtime capture can reclassify them.
        let key = self.vars.as_ref().map(|v| (cx.theme().mode, v.revision()));
        if key != self.decorated {
            self.decorate(cx);
        }
        let content = match (self.mode, &self.unsupported) {
            (None, kind) => div()
                .id("body-unsupported")
                .test_support()
                .child(format!(
                    "This body mode ({}) is not editable here; it is kept as written.",
                    kind.as_deref().unwrap_or("unknown")
                ))
                .into_any_element(),
            (Some(BodyMode::None), _) => div()
                .id("body-none")
                .test_support()
                .child("This request has no body.")
                .into_any_element(),
            (Some(mode), _) if mode.is_table() => div()
                .id("body-table")
                .test_support()
                .w_full()
                .child(self.table.clone())
                .into_any_element(),
            (Some(_), _) => div()
                .id("body-code")
                .test_support()
                .w_full()
                .min_h(style::EDITOR_MIN_HEIGHT)
                .h(style::EDITOR_HEIGHT)
                .child(
                    VarField::editor(
                        "body-code-vars",
                        &self.code,
                        crate::ui::ringed_editor(Editor::new(&self.code).h_full(), &self.code),
                        self.vars.clone(),
                    )
                    .size_full(),
                )
                .into_any_element(),
        };
        let beautify = self.mode.filter(|m| m.beautifiable()).map(|_| {
            Button::new("body-beautify")
                .ghost()
                .small()
                .label("Beautify")
                .tooltip("Re-indent the body")
                .on_click(cx.listener(|this, _, window, cx| this.beautify(window, cx)))
        });
        let minify = self.mode.filter(|m| m.beautifiable()).map(|_| {
            Button::new("body-minify")
                .ghost()
                .small()
                .label("Minify")
                .tooltip("Remove whitespace from the body")
                .on_click(cx.listener(|this, _, window, cx| this.minify(window, cx)))
        });
        let wrap = self
            .mode
            .filter(|m| *m != BodyMode::None && !m.is_table())
            .map(|_| {
                crate::ui::checkbox("body-wrap-toggle", self.wrap)
                    .label("Wrap")
                    .tooltip("Wrap long lines")
                    .on_click(
                        cx.listener(|this, on: &bool, window, cx| this.set_wrap(*on, window, cx)),
                    )
            });
        let error = self.beautify_error.clone().map(|message| {
            div()
                .id("body-beautify-error")
                .test_support()
                .text_xs()
                .text_color(cx.theme().danger)
                .child(message)
        });
        let json_error = self.json_error().map(|message| {
            div()
                .id("body-json-error")
                .test_support()
                .text_xs()
                .text_color(cx.theme().danger)
                .child(message)
        });
        v_flex()
            .size_full()
            .gap_2()
            .child(
                h_flex()
                    .gap(style::SPACE_2)
                    .items_center()
                    .child(
                        div()
                            .w(style::MODE_SELECT_WIDTH)
                            .max_w_full()
                            .child(Select::new(&self.select).id("body-mode")),
                    )
                    .children(beautify)
                    .children(minify)
                    .children(wrap),
            )
            .child(content)
            .children(json_error)
            .children(error)
    }
}
