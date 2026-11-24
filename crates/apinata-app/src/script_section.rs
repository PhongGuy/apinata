//! The Script and Tests sections: one JavaScript code editor per script phase, the Script
//! section showing one phase at a time under a Pre-request / Post-response switch.

use crate::ui::SolidRing as _;
use std::ops::Range;
use std::rc::Rc;

use apinata_core::{
    RequestResult, RunError, ScriptPhase, Snippet, SnippetPhase, Verdict, own_script_line,
    script_code, script_snippets,
};
use gpui_kit::base::input::{Diagnostic, DiagnosticSeverity};
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Editor, EditorState, Enter, InputEvent, Position, Rope};
use gpui_kit::component::tab::Tab;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Bounds, Entity, EntityInputHandler, EventEmitter, Pixels, SharedString, Subscription,
    TestSupportExt, Window, div, relative,
};

use crate::command::Edit;
use crate::script_complete::ScriptCompletions;
use crate::section::SectionEdit;
use crate::snapshot::ScriptCode;
use crate::style::{self, SPACE_1, SPACE_2, SPACE_3};
use crate::ui::{self, RingRoom as _, RingSides};

/// One phase's editor and the id its wrapper carries.
struct PhaseEditor {
    phase: ScriptPhase,
    id: SharedString,
    code: Entity<EditorState>,
    // Whether the editor holds text the user typed rather than text loaded from the file.
    typed: bool,
    // A text the user's own redo is about to echo back; its load keeps the editor typed.
    redo_text: Option<String>,
    // Whether this editor wraps long lines.
    wrap: bool,
}

/// Edits the scripts of some phases; each change goes out as [`Edit::Script`] for its phase.
pub struct ScriptSection {
    id: &'static str,
    editors: Vec<PhaseEditor>,
    // The index of the editor shown, and whether a load or `show` has set it, after which
    // a load keeps it.
    shown: usize,
    picked: bool,
    // The Preferences default last applied to the editors.
    default_wrap: bool,
    // The last result's script errors for this section's phases, one per phase at most.
    errors: Vec<ScriptErrorNote>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<SectionEdit> for ScriptSection {}

/// The user chose a phase on the switch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SideChosen(pub ScriptPhase);

impl EventEmitter<SideChosen> for ScriptSection {}

/// The user pressed a snippet button of the `phase` editor.
#[derive(Clone, Copy, Debug)]
pub struct SnippetChosen(pub ScriptPhase, pub &'static Snippet);

impl EventEmitter<SnippetChosen> for ScriptSection {}

/// What a snippet button's tooltip shows: the code it inserts.
pub fn snippet_tooltip(snippet: &Snippet) -> String {
    snippet.code.to_string()
}

/// The snippets offered in `phase`, in table order.
fn snippets_for(phase: ScriptPhase) -> impl Iterator<Item = &'static Snippet> {
    let slot = match phase {
        ScriptPhase::PreRequest => SnippetPhase::PreRequest,
        ScriptPhase::PostResponse => SnippetPhase::PostResponse,
        _ => SnippetPhase::Tests,
    };
    script_snippets()
        .iter()
        .filter(move |s| s.phases.contains(&slot))
}

/// The one button every snippet row builds, styled as the app's other secondary buttons; a
/// press emits [`SnippetChosen`], and Enter and Space press it as a tab stop.
fn snippet_button(
    snippet: &'static Snippet,
    phase: ScriptPhase,
    this: Entity<ScriptSection>,
    cx: &App,
) -> Button {
    Button::new(SharedString::from(format!("snippet-{}", snippet.id)))
        .solid_ring(cx)
        .small()
        .label(snippet.label)
        .tooltip(snippet_tooltip(snippet))
        .on_click(move |_, _, cx| {
            this.update(cx, |_, cx| cx.emit(SnippetChosen(phase, snippet)));
        })
}

/// A script error the shown result reports, for the editor of the phase whose script failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptErrorNote {
    /// The phase whose script failed.
    pub phase: ScriptPhase,
    /// The error as the response pane words it, redacted.
    pub text: String,
    /// The 1-based line of this request's own script that failed, when the error names one.
    pub line: Option<u32>,
}

/// The script errors `result` reports, the pre-request one first, then each later phase's.
pub fn script_error_notes(result: &RequestResult) -> Vec<ScriptErrorNote> {
    let pre = match &result.error {
        Some(e @ RunError::Script { phase, message }) => Some(ScriptErrorNote {
            phase: *phase,
            text: result.redact(&ui::plain_quotes(&e.to_string())),
            line: own_script_line(message),
        }),
        _ => None,
    };
    let later = result.tests.iter().filter(|t| t.script_error).map(|t| {
        let detail = match &t.verdict {
            Verdict::Fail { detail, .. } => detail.as_deref().unwrap_or(""),
            _ => "",
        };
        ScriptErrorNote {
            phase: t.phase,
            text: result.redact(&crate::response_pane::test_line(t).1),
            line: own_script_line(detail),
        }
    });
    pre.into_iter().chain(later).collect()
}

// The underline under `line` (1-based) of `text`, carrying the error on hover; `None` past the end.
fn line_diagnostic(text: &Rope, line: u32, message: &str) -> Option<Diagnostic> {
    let row = line.checked_sub(1)?;
    let source = text.to_string();
    let width = source.split('\n').nth(row as usize)?.chars().count() as u32;
    let range = Position::new(row, 0)..Position::new(row, width.max(1));
    Some(Diagnostic::new(range, message.to_string()).with_severity(DiagnosticSeverity::Error))
}

/// Post-response when only the post-response script has content after trimming, else Pre-request.
pub fn default_side(pre: &str, post: &str) -> ScriptPhase {
    if pre.trim().is_empty() && !post.trim().is_empty() {
        ScriptPhase::PostResponse
    } else {
        ScriptPhase::PreRequest
    }
}

/// Where the end of the block holding `at` is: the end of the last non-blank line in the run
/// of non-blank lines that `at` sits in. Snippets span several lines but never a blank one.
fn block_end(text: &str, at: usize) -> usize {
    let mut end = at + text[at..].find('\n').unwrap_or(text.len() - at);
    while let Some(next) = text[end..].strip_prefix('\n') {
        let line = next.split('\n').next().unwrap_or("");
        if line.trim().is_empty() {
            break;
        }
        end += 1 + line.len();
    }
    end
}

/// The text after inserting `snippet` at `sel` of `text`, and the byte range of its
/// placeholder. A snippet never replaces a selection (it may be the previous snippet's
/// placeholder): with one, it goes after the block holding the selection's last character. The snippet gets
/// its own lines: a newline goes before it when text precedes the caret on its line, and after
/// it when text follows.
fn plan_snippet(text: &str, sel: Range<usize>, snippet: &Snippet) -> (String, Range<usize>) {
    let sel = if sel.is_empty() {
        sel
    } else {
        // A selection that ends right after a newline ends on that line, not the next one.
        let last = if text[..sel.end].ends_with('\n') {
            sel.end - 1
        } else {
            sel.end
        };
        let end = block_end(text, last);
        end..end
    };
    let (before, after) = (&text[..sel.start], &text[sel.end..]);
    let line = before.rsplit('\n').next().unwrap_or("");
    let rest = after.split('\n').next().unwrap_or("");
    let lead = if line.trim().is_empty() { "" } else { "\n" };
    let trail = if rest.trim().is_empty() { "" } else { "\n" };
    // The break replaces the blanks it splits, so no line is left with a stray edge space.
    let before = if lead.is_empty() {
        before
    } else {
        before.trim_end_matches([' ', '\t'])
    };
    let after = if trail.is_empty() {
        after
    } else {
        after.trim_start_matches([' ', '\t'])
    };
    let start = before.len() + lead.len();
    let at = snippet.code.find(snippet.select).unwrap_or(0) + start;
    let out = format!("{before}{lead}{}{trail}{after}", snippet.code);
    (out, at..at + snippet.select.len())
}

// Drops the error underline from `code`, if it has one.
fn clear_underline(code: &Entity<EditorState>, cx: &mut App) {
    code.update(cx, |c, cx| {
        if let Some(set) = c.diagnostics_mut()
            && !set.is_empty()
        {
            set.clear();
            cx.notify();
        }
    });
}

// The banner over the editor of a phase whose script failed: the message, cut to one line with
// the whole text on hover, and Go to line when the failing line is in this script.
fn error_banner(note: &ScriptErrorNote, this: Entity<ScriptSection>, cx: &App) -> impl IntoElement {
    let phase = note.phase;
    h_flex()
        .id("script-error")
        .test_support()
        .aria_label(note.text.clone())
        .w_full()
        .gap(SPACE_2)
        .items_center()
        .child(Icon::new(IconName::CircleAlert).text_color(cx.theme().danger))
        .child(
            div()
                .id("script-error-text")
                .min_w_0()
                .flex_1()
                .truncate()
                .text_color(cx.theme().danger)
                .tooltip(ui::text_tooltip("script-error-tooltip", note.text.clone()))
                .child(note.text.clone()),
        )
        .when_some(note.line, |row, line| {
            row.child(
                Button::new("script-error-go-to-line")
                    .solid_ring(cx)
                    .small()
                    .label(format!("Go to line {line}"))
                    .on_click(move |_, window, cx| {
                        this.update(cx, |s, cx| s.go_to_error(phase, window, cx))
                    }),
            )
        })
}

fn label(phase: ScriptPhase) -> &'static str {
    match phase {
        ScriptPhase::PreRequest => "Pre-request",
        ScriptPhase::PostResponse => "Post-response",
        _ => "Tests",
    }
}

// The muted hint an empty editor shows: when its script runs.
fn hint(phase: ScriptPhase) -> &'static str {
    match phase {
        ScriptPhase::PreRequest => "Runs before the request is sent.",
        ScriptPhase::PostResponse => "Runs after the response arrives.",
        _ => "Add tests to check the response.",
    }
}

impl ScriptSection {
    /// One JavaScript editor per phase, completing `bru`, `req` and `res` members, two phases
    /// under a "Pre-request" / "Post-response" switch, one alone, since the section tab names
    /// it; root id `script-section` (phases [PreRequest, PostResponse]) or `tests-section`
    /// (phases [Tests]); editor ids
    /// `script-pre-request-code`, `script-post-response-code`, `script-tests-code`.
    pub fn new(phases: &[ScriptPhase], window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut editors = Vec::new();
        let mut subs = Vec::new();
        let default_wrap = crate::settings::wrap_editors(cx);
        if let Some(app) = cx.try_global::<crate::app::AppState>() {
            let settings = app.settings.clone();
            subs.push(cx.observe_in(&settings, window, |this, _, window, cx| {
                let want = crate::settings::wrap_editors(cx);
                if want != this.default_wrap {
                    this.default_wrap = want;
                    for ix in 0..this.editors.len() {
                        this.set_wrap_at(ix, want, window, cx);
                    }
                }
            }));
        }
        for (ix, &phase) in phases.iter().enumerate() {
            let code = cx.new(|cx| {
                EditorState::new(window, cx)
                    .language("javascript")
                    .placeholder(hint(phase))
                    .soft_wrap(default_wrap)
            });
            code.update(cx, |s, _| {
                s.lsp_mut().completion_provider = Some(Rc::new(ScriptCompletions))
            });
            subs.push(
                cx.subscribe(&code, move |this, state, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        // The underline sits at fixed offsets, so an edit would leave it on the
                        // wrong text; the banner keeps the message.
                        clear_underline(&state, cx);
                        this.editors[ix].typed = true;
                        let text = state.read(cx).value().to_string();
                        cx.emit(SectionEdit::new(Edit::Script(phase, text)));
                        // The switch's "has script" dot reads this editor's text.
                        cx.notify();
                    }
                }),
            );
            let id = format!("script-{}-code", phase.as_str()).into();
            editors.push(PhaseEditor {
                phase,
                id,
                code,
                typed: false,
                redo_text: None,
                wrap: default_wrap,
            });
        }
        let id = if phases.contains(&ScriptPhase::Tests) {
            "tests-section"
        } else {
            "script-section"
        };
        ScriptSection {
            id,
            editors,
            shown: 0,
            picked: false,
            default_wrap,
            errors: Vec::new(),
            _subs: subs,
        }
    }

    /// Marks each of `notes` whose phase this section edits; replaces the earlier marks.
    /// Each shows as a banner over its phase's editor, and a line of this request's own
    /// script is underlined with the message on hover until the user edits that editor.
    pub fn set_errors(&mut self, notes: &[ScriptErrorNote], cx: &mut Context<Self>) {
        self.errors = (self.editors.iter())
            .filter_map(|e| notes.iter().find(|n| n.phase == e.phase).cloned())
            .collect();
        for e in &self.editors {
            let note = self.errors.iter().find(|n| n.phase == e.phase);
            e.code.update(cx, |c, cx| {
                let text = c.text().clone();
                let mark = note.and_then(|n| line_diagnostic(&text, n.line?, &n.text));
                if let Some(set) = c.diagnostics_mut() {
                    set.reset(&text);
                    set.extend(mark);
                }
                cx.notify();
            });
        }
        cx.notify();
    }

    /// The error marked on `phase`, if any.
    pub fn error(&self, phase: ScriptPhase) -> Option<&ScriptErrorNote> {
        self.errors.iter().find(|n| n.phase == phase)
    }

    /// Shows `phase` and puts its caret at the start of the line its error names, focused.
    pub fn go_to_error(&mut self, phase: ScriptPhase, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.error(phase).and_then(|n| n.line?.checked_sub(1)) else {
            return;
        };
        let Some(code) = self.code(phase).cloned() else {
            return;
        };
        self.show(phase, cx);
        code.update(cx, |c, cx| {
            c.set_cursor_position(Position::new(row, 0), window, cx)
        });
    }

    /// Ends the typing run of each phase editor's own undo, as at a save or a reload.
    pub fn end_typing(&self, window: &mut Window, cx: &mut App) {
        for editor in &self.editors {
            crate::ui::end_editor_run(&editor.code, window, cx);
        }
    }

    /// Shows `code` without emitting an edit; an editor whose `script_code` already equals
    /// the snapshot's `script_code` is left alone, so typed edge whitespace survives the echo.
    /// A replaced text is no longer counted as typed. The first load shows [`default_side`]
    /// unless `show` already chose a phase.
    pub fn load(&mut self, code: &ScriptCode, window: &mut Window, cx: &mut Context<Self>) {
        if !self.picked {
            self.picked = true;
            let side = default_side(&code.pre_request, &code.post_response);
            self.shown = self.index(side).unwrap_or(0);
        }
        for e in &mut self.editors {
            let want = code.get(e.phase);
            // The file keeps a script trimmed, so a redo echoes the block without its edge
            // whitespace; the redone text itself is what the editor should show.
            let redo = e
                .redo_text
                .take()
                .filter(|t| script_code(t) == script_code(want));
            let shown = redo.as_deref().unwrap_or(want);
            let replaced = e.code.update(cx, |c, cx| {
                let value = c.value();
                let shown_now = crate::ui::committed_text(&value, c.marked_text_range(window, cx));
                let replace = script_code(&shown_now) != script_code(want);
                if replace {
                    c.set_value(shown.to_string(), window, cx);
                    if let Some(set) = c.diagnostics_mut() {
                        set.clear();
                    }
                }
                replace
            });
            // A redone appended block is the user's own text, so a save still approves it.
            // The approval is one-shot: any load spends it, matched or not.
            let redone = replaced && redo.is_some();
            e.typed = (e.typed && !replaced) || redone;
        }
    }

    /// Replaces the text of `phase` with `text` the user asked for (a generated block added to
    /// what was there). The editor records it in its own undo stack, so Ctrl+Z in the editor
    /// takes it back; its change event marks it typed (a save approves it) and sends one
    /// [`Edit::Script`]. False when the section has no editor for `phase`.
    pub fn append_text(
        &mut self,
        phase: ScriptPhase,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(ix) = self.index(phase) else {
            return false;
        };
        self.editors[ix]
            .code
            .update(cx, |c, cx| c.replace_all(text.to_string(), window, cx));
        true
    }

    /// The whole text `phase` would hold after [`Self::insert_snippet`] of `snippet`, or `None`
    /// when the insert would be refused.
    pub fn snippet_text(&self, phase: ScriptPhase, snippet: &Snippet, cx: &App) -> Option<String> {
        let code = self.code(phase)?.read(cx);
        if !code.is_editable() {
            return None;
        }
        Some(plan_snippet(&code.value(), code.selected_range(), snippet).0)
    }

    /// Inserts `snippet` at the caret of `phase` (after the block holding a selection, which it
    /// leaves intact) and selects its placeholder. The insert is one `replace_all`, so it is one
    /// editor undo step, and its change event marks it typed and sends one [`Edit::Script`].
    /// False when the section has no editor for `phase` or that editor is locked.
    pub fn insert_snippet(
        &mut self,
        phase: ScriptPhase,
        snippet: &Snippet,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(code) = self.code(phase).cloned() else {
            return false;
        };
        let (text, select) = {
            let c = code.read(cx);
            if !c.is_editable() {
                return false;
            }
            plan_snippet(&c.value(), c.selected_range(), snippet)
        };
        code.update(cx, |c, cx| {
            c.replace_all(text, window, cx);
            c.set_selected_range(select, cx);
            c.focus(window, cx);
        });
        true
    }

    /// Notes that the text is about to come back for the phase from a redo of an appended block,
    /// so loading it leaves the phase typed. A phase this section lacks ignores it.
    pub fn expect_redo(&mut self, phase: ScriptPhase, text: &str) {
        if let Some(ix) = self.index(phase) {
            self.editors[ix].redo_text = Some(text.to_string());
        }
    }

    /// The phase on show under the switch; `None` for a one-phase section, which has no switch.
    pub fn shown(&self) -> Option<ScriptPhase> {
        (self.editors.len() > 1).then(|| self.editors[self.shown].phase)
    }

    /// Shows the editor of `phase`, if this section has one; a later load keeps it.
    pub fn show(&mut self, phase: ScriptPhase, cx: &mut Context<Self>) {
        if let Some(ix) = self.index(phase) {
            self.shown = ix;
            self.picked = true;
            cx.notify();
        }
    }

    /// Whether the editor of `phase` wraps long lines; `false` for a phase this section lacks.
    pub fn wrap(&self, phase: ScriptPhase) -> bool {
        self.index(phase).is_some_and(|ix| self.editors[ix].wrap)
    }

    /// Wraps or unwraps long lines in the editor of `phase`, if this section has one.
    pub fn set_wrap(
        &mut self,
        phase: ScriptPhase,
        wrap: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(ix) = self.index(phase) {
            self.set_wrap_at(ix, wrap, window, cx);
        }
    }

    fn set_wrap_at(&mut self, ix: usize, wrap: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.editors[ix].wrap = wrap;
        self.editors[ix]
            .code
            .update(cx, |c, cx| c.set_soft_wrap(wrap, window, cx));
        cx.notify();
    }

    fn index(&self, phase: ScriptPhase) -> Option<usize> {
        self.editors.iter().position(|e| e.phase == phase)
    }

    /// The editor of `phase`, if this section shows it.
    pub fn code(&self, phase: ScriptPhase) -> Option<&Entity<EditorState>> {
        self.editors
            .iter()
            .find(|e| e.phase == phase)
            .map(|e| &e.code)
    }

    /// The caret of the editor on show in window coordinates as drawn, or `None` before it has
    /// been laid out. The editor reports it without its own vertical scroll, added back here.
    pub fn caret(&self, cx: &App) -> Option<Bounds<Pixels>> {
        let state = self.editors.get(self.shown)?.code.read(cx);
        let (mut caret, _) = state.cursor_layout()?;
        caret.origin.y += state.text_bounds()?.origin.y - state.input_bounds().origin.y;
        Some(caret)
    }

    /// The current text of each editor the user typed into since its text was last loaded,
    /// in phase order; a text that came from the file, possibly written by someone else,
    /// is never listed.
    pub fn typed_texts(&self, cx: &App) -> Vec<String> {
        self.editors
            .iter()
            .filter(|e| e.typed)
            .map(|e| e.code.read(cx).value().to_string())
            .collect()
    }

    /// The current text of each editor, in phase order.
    pub fn texts(&self, cx: &App) -> Vec<String> {
        self.editors
            .iter()
            .map(|e| e.code.read(cx).value().to_string())
            .collect()
    }
}

impl ScriptSection {
    // The phase switch: one tab per phase, the content dot after each label whose script has text.
    fn switch(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity();
        let tabs = self.editors.iter().enumerate().map(|(ix, e)| {
            let has_script = !e.code.read(cx).value().trim().is_empty();
            Tab::new().child(
                h_flex()
                    .line_height(relative(style::TAB_LINE_HEIGHT))
                    .gap(SPACE_1)
                    .items_center()
                    .child(
                        div()
                            .id(("script-tab", ix))
                            .test_support()
                            .child(label(e.phase)),
                    )
                    .when(has_script, |d| {
                        d.child(ui::content_dot(("script-dot", ix), "Has a script").test_support())
                    }),
            )
        });
        div().id("script-phase").test_support().w_full().child(
            ui::tab_strip("script-phase-tabs", cx)
                .selected_index(self.shown)
                .children(tabs)
                .on_click(move |ix, _, cx| {
                    let ix = *ix;
                    this.update(cx, |s, cx| {
                        if let Some(phase) = s.editors.get(ix).map(|e| e.phase) {
                            s.show(phase, cx);
                            cx.emit(SideChosen(phase));
                        }
                    })
                }),
        )
    }
}

impl Render for ScriptSection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity();
        let switched = self.editors.len() > 1;
        let switch = switched.then(|| self.switch(cx));
        // Under a switch only the shown phase's editor renders; every editor entity is kept,
        // so each keeps its text, caret and history.
        let shown = self
            .editors
            .iter()
            .enumerate()
            .filter(|(ix, _)| !switched || *ix == self.shown)
            .map(|(_, e)| e);
        v_flex()
            .id(self.id)
            .test_support()
            .w_full()
            .flex_1()
            .gap(SPACE_3)
            .children(switch)
            // Only one editor shows at a time, and it takes the room the controls leave.
            .children(shown.map(|e| {
                v_flex()
                    .w_full()
                    .flex_1()
                    .gap(SPACE_1)
                    // Like Body's mode row: the snippet buttons, then Wrap at the row's end.
                    // The padding leaves room for the kit's ring, which sits outside each
                    // button and would be clipped at the pane edge on the first one.
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap(SPACE_2)
                            .ring_room(RingSides::ALL)
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .flex_wrap()
                                    .gap(SPACE_1)
                                    .children(
                                        snippets_for(e.phase)
                                            .map(|s| snippet_button(s, e.phase, this.clone(), cx)),
                                    ),
                            )
                            .child({
                                let (this, phase) = (this.clone(), e.phase);
                                ui::checkbox("script-wrap-toggle", e.wrap)
                                    .label("Wrap")
                                    .tooltip("Wrap long lines")
                                    .on_click(move |on, window, cx| {
                                        let on = *on;
                                        this.update(cx, |s, cx| s.set_wrap(phase, on, window, cx))
                                    })
                            }),
                    )
                    .when_some(self.error(e.phase), |col, note| {
                        col.child(error_banner(note, this.clone(), cx))
                    })
                    .child(
                        div()
                            .id(e.id.clone())
                            .test_support()
                            .w_full()
                            .flex_1()
                            .min_h(style::SCRIPT_EDITOR_MIN_HEIGHT)
                            .child(ui::ringed_editor(Editor::new(&e.code).h_full(), &e.code))
                            // Accepting a completion lets Enter bubble on, and an unhandled
                            // Enter is then typed as a newline; stop it at the accepted word.
                            .on_action(|_: &Enter, _, cx| cx.stop_propagation()),
                    )
            }))
    }
}
