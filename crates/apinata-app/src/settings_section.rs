//! The Settings and Docs sections: Bruno's per-request settings pane and the markdown docs.

use apinata_core::{RequestSettings, Timeout};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::IndexPath;
use gpui_kit::component::input::{Editor, EditorState, Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::text::TextView;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Entity, EventEmitter, SharedString, Subscription, TestSupportExt, Window, div,
};
use std::time::Duration;

use crate::app::AppState;
use crate::command::Edit;
use crate::kv_table::set_text;
use crate::scroll_follow::{FocusFollow, Followed};
use crate::section::SectionEdit;
use crate::style::{self, FORM_INPUT_WIDTH, SPACE_2};
use crate::tag_input::{TagField, TagsChanged};
use crate::ui;

/// The hint beside a custom timeout: its unit, as the file stores it, and that 0 lifts the limit.
pub const TIMEOUT_HINT: &str = "Milliseconds, 0 means no limit";

/// Whether a request takes the Preferences default timeout or its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum TimeoutMode {
    Inherit,
    Custom,
}

impl TimeoutMode {
    const ALL: [TimeoutMode; 2] = [TimeoutMode::Inherit, TimeoutMode::Custom];

    /// The text the timeout select shows for this mode; Inherit names `default`, the
    /// Preferences timeout, in the whole seconds Preferences takes.
    pub fn label(self, default: Option<Duration>) -> SharedString {
        match (self, default) {
            (TimeoutMode::Inherit, None) => "Default (no limit)".into(),
            (TimeoutMode::Inherit, Some(d)) => format!("Default ({} s)", d.as_secs()).into(),
            (TimeoutMode::Custom, _) => "Custom".into(),
        }
    }

    /// The stable key the mode's select row is found by; the Inherit label names the
    /// default, so it cannot serve.
    pub fn key(self) -> &'static str {
        match self {
            TimeoutMode::Inherit => "inherit",
            TimeoutMode::Custom => "custom",
        }
    }

    fn rows(default: Option<Duration>) -> Vec<ui::IdItem> {
        TimeoutMode::ALL
            .iter()
            .map(|m| ui::IdItem::new("set-timeout-mode", m.key(), m.label(default)))
            .collect()
    }

    fn of(timeout: Timeout) -> TimeoutMode {
        match timeout {
            Timeout::Inherit => TimeoutMode::Inherit,
            _ => TimeoutMode::Custom,
        }
    }
}

/// Bruno's settings pane: three toggles, max redirects and the timeout.
///
/// Each change sends the last sent or loaded settings with one field changed, so changes
/// compose before their echo arrives. Number inputs take digits only; empty means 0.
pub struct SettingsSection {
    // The settings last sent or loaded; the base each change builds on.
    base: RequestSettings,
    max_redirects: Entity<InputState>,
    timeout_select: Entity<ui::IdSelect>,
    // The request's own timeout last loaded or typed, which Custom brings back after Default.
    custom: Timeout,
    // The Preferences timeout the Inherit label names.
    default_timeout: Option<Duration>,
    timeout: Entity<InputState>,
    tags: Entity<TagField>,
    invalid: bool,
    // Where the form rows report their bounds to the scroll area that holds the section.
    follow: FocusFollow,
    _subs: Vec<Subscription>,
}

impl EventEmitter<SectionEdit> for SettingsSection {}

// Bruno's number rule: digits only, empty is 0; a non-digit or an overflow is refused.
fn digits<T: std::str::FromStr + Default>(text: &str) -> Option<T> {
    if text.is_empty() {
        return Some(T::default());
    }
    if !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

impl SettingsSection {
    /// Reports each row's bounds to `follow`, the watch of the scroll area holding the section.
    pub fn set_follow(&mut self, follow: FocusFollow) {
        self.follow = follow;
    }

    /// A section showing Bruno's default settings.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let base = RequestSettings::new();
        let max_redirects =
            cx.new(|cx| InputState::new(window, cx).default_value(base.max_redirects.to_string()));
        let timeout = cx.new(|cx| InputState::new(window, cx).default_value("0"));
        let default_timeout = crate::settings::request_timeout(cx);
        let labels = TimeoutMode::rows(default_timeout);
        let timeout_select =
            cx.new(|cx| SelectState::new(labels, Some(IndexPath::new(1)), window, cx));
        let tags = cx.new(|cx| TagField::new(window, cx));
        let mut subs = vec![
            cx.subscribe(&tags, |_, _, e: &TagsChanged, cx| {
                cx.emit(SectionEdit::new(Edit::Tags(e.0.clone())))
            }),
            cx.subscribe(&max_redirects, |this, state, event: &InputEvent, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let text = state.read(cx).value().to_string();
                this.number(digits::<u32>(&text), |s, n| s.max_redirects = n, cx);
            }),
            cx.subscribe(&timeout, |this, state, event: &InputEvent, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let text = state.read(cx).value().to_string();
                let timeout = digits::<u64>(&text).map(|n| match n {
                    0 => Timeout::Unlimited,
                    n => Timeout::Millis(n),
                });
                if let Some(t) = timeout {
                    this.custom = t;
                }
                this.number(timeout, |s, t| s.timeout = t, cx);
            }),
            cx.subscribe_in(
                &timeout_select,
                window,
                |this, _, event: &SelectEvent<Vec<ui::IdItem>>, window, cx| {
                    if let SelectEvent::Confirm(Some(label)) = event
                        && let Some(mode) = TimeoutMode::ALL
                            .into_iter()
                            .find(|m| m.label(this.default_timeout) == *label)
                    {
                        this.set_timeout_mode(mode, window, cx);
                    }
                },
            ),
        ];
        // The settings exist only under `app::start`; a bare section in a test has none.
        if let Some(settings) = cx.try_global::<AppState>().map(|s| s.settings.clone()) {
            subs.push(cx.observe_in(&settings, window, |this, _, window, cx| {
                let default = crate::settings::request_timeout(cx);
                if default != this.default_timeout {
                    this.default_timeout = default;
                    let labels = TimeoutMode::rows(default);
                    (this.timeout_select).update(cx, |s, cx| s.set_items(labels, window, cx));
                    this.show_timeout(window, cx);
                }
            }));
        }
        SettingsSection {
            follow: FocusFollow::default(),
            base,
            max_redirects,
            timeout_select,
            custom: Timeout::Unlimited,
            default_timeout,
            timeout,
            tags,
            invalid: false,
            _subs: subs,
        }
    }

    /// Shows `settings` from a snapshot without emitting an edit.
    pub fn load(
        &mut self,
        settings: &RequestSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.base = *settings;
        if settings.timeout != Timeout::Inherit {
            self.custom = settings.timeout;
        }
        self.invalid = false;
        set_text(
            &self.max_redirects,
            &settings.max_redirects.to_string(),
            window,
            cx,
        );
        self.show_timeout(window, cx);
        cx.notify();
    }

    /// Shows the request's `tags` from a snapshot without emitting an edit.
    pub fn load_tags(&mut self, tags: &[String], cx: &mut Context<Self>) {
        self.tags.update(cx, |t, cx| t.load(tags, cx));
    }

    /// Ends the typing run of each number input's own undo, as at a save or a reload.
    pub fn end_typing(&self, window: &mut Window, cx: &mut App) {
        crate::ui::end_input_run(&self.max_redirects, window, cx);
        crate::ui::end_input_run(&self.timeout, window, cx);
    }

    /// The Max redirects field.
    pub fn max_redirects_input(&self) -> &Entity<InputState> {
        &self.max_redirects
    }

    /// The tag chip input.
    pub fn tags(&self) -> &Entity<TagField> {
        &self.tags
    }

    /// The text the timeout select shows.
    pub fn timeout_shown(&self, cx: &App) -> Option<SharedString> {
        self.timeout_select.read(cx).selected_value().cloned()
    }

    /// Whether the timeout is inherited or the request's own.
    pub fn timeout_mode(&self) -> TimeoutMode {
        TimeoutMode::of(self.base.timeout)
    }

    /// Switches the timeout mode and sends it; Custom brings back the request's own timeout,
    /// or no limit if it never had one.
    pub fn set_timeout_mode(
        &mut self,
        mode: TimeoutMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.timeout_mode() == mode {
            return;
        }
        self.base.timeout = match mode {
            TimeoutMode::Inherit => Timeout::Inherit,
            TimeoutMode::Custom => self.custom,
        };
        self.show_timeout(window, cx);
        self.emit(cx);
    }

    fn show_timeout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let label = self.timeout_mode().label(self.default_timeout);
        self.timeout_select
            .update(cx, |s, cx| s.set_selected_value(&label, window, cx));
        let millis = match self.base.timeout {
            Timeout::Millis(n) => Some(n),
            Timeout::Unlimited => Some(0),
            _ => None,
        };
        if let Some(n) = millis {
            set_text(&self.timeout, &n.to_string(), window, cx);
        }
        cx.notify();
    }

    // A typed number either updates one field and is sent, or is refused and only flagged.
    fn number<T>(
        &mut self,
        value: Option<T>,
        apply: impl FnOnce(&mut RequestSettings, T),
        cx: &mut Context<Self>,
    ) {
        match value {
            Some(v) => {
                self.invalid = false;
                apply(&mut self.base, v);
                self.emit(cx);
            }
            None => self.invalid = true,
        }
        cx.notify();
    }

    fn toggle(&mut self, apply: impl FnOnce(&mut RequestSettings), cx: &mut Context<Self>) {
        apply(&mut self.base);
        self.emit(cx);
        cx.notify();
    }

    fn emit(&mut self, cx: &mut Context<Self>) {
        cx.emit(SectionEdit::new(Edit::Settings(self.base)));
    }

    /// Whether `edit`, which this section sent, was built from values a load has since
    /// replaced: the settings or tags it would send now differ.
    pub fn lags(&self, edit: &Edit, cx: &App) -> bool {
        match edit {
            Edit::Settings(sent) => *sent != self.base,
            Edit::Tags(sent) => sent.as_slice() != self.tags.read(cx).tags(),
            _ => false,
        }
    }
}

impl Render for SettingsSection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let s = self.base;
        let follow = self.follow.clone();
        let followed = move |row: ui::FormRow| Followed::new(row.wrapper_id(), &follow, row);
        let custom = self.timeout_mode() == TimeoutMode::Custom;
        // Checkboxes carry no label of their own: the form row's label names them, so every
        // label sits in one column at one size.
        // Wraps so the input drops under the select in a narrow side-by-side column.
        let timeout = h_flex()
            .w_full()
            .flex_wrap()
            .gap(SPACE_2)
            .child(
                div()
                    .w(FORM_INPUT_WIDTH)
                    .child(Select::new(&self.timeout_select).id("set-timeout-mode")),
            )
            .when(custom, |r| {
                r.child(ui::padded_input(
                    Input::new(&self.timeout)
                        .id("set-timeout")
                        .w(FORM_INPUT_WIDTH)
                        .bg(ui::field_fill(false, cx)),
                    &self.timeout,
                ))
            });
        v_flex()
            .gap(SPACE_2)
            .child(followed(ui::form_row(
                "set-row-encode-url",
                "URL encoding",
                ui::checkbox("set-encode-url", s.encode_url).on_click(
                    cx.listener(|t, on: &bool, _, cx| t.toggle(|s| s.encode_url = *on, cx)),
                ),
            )))
            .child(followed(ui::form_row(
                "set-row-follow-redirects",
                "Follow redirects",
                ui::checkbox("set-follow-redirects", s.follow_redirects).on_click(
                    cx.listener(|t, on: &bool, _, cx| t.toggle(|s| s.follow_redirects = *on, cx)),
                ),
            )))
            .child(followed(ui::form_row(
                "set-row-forward-auth",
                "Keep Authorization header on redirect",
                ui::checkbox("set-forward-auth", s.forward_authorization_header).on_click(
                    cx.listener(|t, on: &bool, _, cx| {
                        t.toggle(|s| s.forward_authorization_header = *on, cx)
                    }),
                ),
            )))
            .child(followed(ui::form_row(
                "set-row-max-redirects",
                "Max redirects",
                // Unfollowed redirects make the limit moot, so the field is disabled but keeps its number.
                div()
                    .id("set-max-redirects-wrap")
                    .when(!s.follow_redirects, |d| {
                        d.tooltip(ui::text_tooltip(
                            "set-max-redirects-tooltip",
                            MAX_REDIRECTS_OFF_TIP,
                        ))
                    })
                    .child(ui::padded_input(
                        Input::new(&self.max_redirects)
                            .id("set-max-redirects")
                            .w(FORM_INPUT_WIDTH)
                            .disabled(!s.follow_redirects)
                            .bg(ui::field_fill(!s.follow_redirects, cx)),
                        &self.max_redirects,
                    )),
            )))
            .child(followed(
                ui::form_row("set-row-timeout", "Timeout", timeout)
                    .when(custom, |r| r.hint("set-timeout-hint", TIMEOUT_HINT)),
            ))
            .child(followed(ui::form_row(
                "set-row-tags",
                "Tags",
                self.tags.clone(),
            )))
            .when(self.invalid, |v| {
                v.child(ui::form_note(
                    "set-invalid",
                    "Only digits are allowed; the last valid number is kept.",
                ))
            })
    }
}

/// Why Max redirects is disabled while redirects are not followed.
pub const MAX_REDIRECTS_OFF_TIP: &str = "Max redirects only applies while Follow redirects is on.";

/// Whether the Docs section shows its markdown rendered or in the editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocsMode {
    /// The markdown rendered.
    Preview,
    /// The markdown in the code editor.
    Edit,
}

/// The message in place of a blank preview.
const DOCS_EMPTY: &str = "No docs yet. Choose Edit to write some.";

/// The Docs section: the request's markdown documentation, rendered or in a code editor.
pub struct DocsSection {
    code: Entity<EditorState>,
    mode: DocsMode,
    /// Set by the first `load` or any `set_mode`; after that only the toggle moves the mode.
    mode_chosen: bool,
    _sub: Subscription,
}

impl EventEmitter<SectionEdit> for DocsSection {}

impl DocsSection {
    /// An empty docs section, in Edit.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let code = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("markdown")
                .placeholder("Write notes about this request. Markdown works.")
        });
        let sub = cx.subscribe(&code, |_, state, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let text = state.read(cx).value().to_string();
                cx.emit(SectionEdit::new(Edit::Docs(text)));
            }
        });
        DocsSection {
            code,
            mode: DocsMode::Edit,
            mode_chosen: false,
            _sub: sub,
        }
    }

    /// Shows `docs` from a snapshot without emitting an edit. The text is replaced only when
    /// it differs, since a replacement resets the caret and an echo of typing must not. The
    /// first load opens written docs in Preview; later loads keep the mode.
    pub fn load(&mut self, docs: &str, window: &mut Window, cx: &mut Context<Self>) {
        crate::ui::sync_editor_text(&self.code, docs, window, cx);
        if !self.mode_chosen {
            self.mode_chosen = true;
            if !docs.trim().is_empty() {
                self.mode = DocsMode::Preview;
            }
            cx.notify();
        }
    }

    /// Which view the section shows.
    pub fn mode(&self) -> DocsMode {
        self.mode
    }

    /// Shows the rendered preview or the editor; switching to Edit focuses the editor.
    pub fn set_mode(&mut self, mode: DocsMode, window: &mut Window, cx: &mut Context<Self>) {
        self.mode_chosen = true;
        self.mode = mode;
        if mode == DocsMode::Edit {
            self.code.update(cx, |c, cx| c.focus(window, cx));
        }
        cx.notify();
    }

    /// The markdown the preview renders: the editor's current text.
    pub fn preview_text(&self, cx: &App) -> SharedString {
        self.code.read(cx).value()
    }

    /// The markdown editor.
    pub fn code(&self) -> &Entity<EditorState> {
        &self.code
    }
}

impl Render for DocsSection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let toggle = ui::segmented(
            "docs-mode",
            ["Preview", "Edit"],
            match self.mode {
                DocsMode::Preview => 0,
                DocsMode::Edit => 1,
            },
            move |ix, window, cx| {
                let mode = if *ix == 0 {
                    DocsMode::Preview
                } else {
                    DocsMode::Edit
                };
                let _ = this.update(cx, |d, cx| d.set_mode(mode, window, cx));
            },
        );
        // The preview takes the editor's slot so switching views never moves the layout.
        let body = match self.mode {
            DocsMode::Edit => div().id("docs-code").child(ui::ringed_editor(
                Editor::new(&self.code).h_full(),
                &self.code,
            )),
            DocsMode::Preview => {
                let text = self.preview_text(cx);
                let content = if text.trim().is_empty() {
                    div()
                        .id("docs-empty")
                        .test_support()
                        .text_color(cx.theme().muted_foreground)
                        .child(DOCS_EMPTY)
                        .into_any_element()
                } else {
                    TextView::markdown("docs-markdown", text)
                        .selectable(true)
                        .scrollable(true)
                        .size_full()
                        .into_any_element()
                };
                div().id("docs-preview").child(content)
            }
        };
        v_flex().w_full().gap(SPACE_2).child(toggle).child(
            body.test_support()
                .w_full()
                .min_h(style::EDITOR_MIN_HEIGHT)
                .h(style::EDITOR_HEIGHT),
        )
    }
}
