//! The Settings and Docs sections: Bruno's per-request settings pane and the markdown docs.

use apinata_core::{HttpVersionPolicy, RequestSettings, Timeout, TlsVerify};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::IndexPath;
use gpui_kit::component::input::{Editor, EditorState, Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::text::TextView;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Div, Entity, EventEmitter, SharedString, Subscription, TestSupportExt, Window, div,
};
use std::time::Duration;

use crate::app::AppState;
use crate::command::Edit;
use crate::kv_table::set_text;
use crate::scroll_follow::{FocusFollow, Followed};
use crate::section::SectionEdit;
use crate::settings::HttpVersionPref;
use crate::style::{self, FORM_INPUT_WIDTH, SPACE_1, SPACE_2};
use crate::tag_input::{TagField, TagsChanged};
use crate::ui;
use crate::vim::Vim;

/// Why a typed custom timeout was refused: the limit is lifted by No limit, never by 0.
pub const TIMEOUT_ZERO_NOTE: &str = "Enter a number above 0, or choose No limit.";

/// Why a typed custom timeout was refused for not being seconds to the millisecond.
pub const SECONDS_NOTE: &str =
    "Enter seconds, such as 2 or 2.5, to at most three decimals; the last valid number is kept.";

/// Why a typed number was refused for not being digits.
pub const DIGITS_NOTE: &str = "Only digits are allowed; the last valid number is kept.";

// The two number inputs, each refused and marked on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NumberField {
    MaxRedirects,
    Timeout,
}

impl NumberField {
    fn id(self) -> &'static str {
        match self {
            NumberField::MaxRedirects => "set-max-redirects",
            NumberField::Timeout => "set-timeout",
        }
    }
}

/// Whether a request takes the Preferences default timeout, no limit, or a limit of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum TimeoutMode {
    Inherit,
    Unlimited,
    Custom,
}

impl TimeoutMode {
    const ALL: [TimeoutMode; 3] = [
        TimeoutMode::Inherit,
        TimeoutMode::Unlimited,
        TimeoutMode::Custom,
    ];

    /// The text the timeout select shows for this mode; Inherit names `default`, the
    /// Preferences timeout, in the whole seconds Preferences takes.
    pub fn label(self, default: Option<Duration>) -> SharedString {
        match (self, default) {
            (TimeoutMode::Inherit, None) => "Default (No limit)".into(),
            (TimeoutMode::Inherit, Some(d)) => format!("Default ({} s)", d.as_secs()).into(),
            (TimeoutMode::Unlimited, _) => "No limit".into(),
            (TimeoutMode::Custom, _) => "Custom".into(),
        }
    }

    /// The line under the timeout row saying where this mode's limit comes from.
    pub fn hint(self) -> &'static str {
        match self {
            TimeoutMode::Inherit => "Uses the request timeout set in Preferences.",
            TimeoutMode::Unlimited => "Ignores the request timeout set in Preferences.",
            TimeoutMode::Custom => "Replaces the request timeout set in Preferences.",
        }
    }

    /// The stable key the mode's select row is found by; the Inherit label names the
    /// default, so it cannot serve.
    pub fn key(self) -> &'static str {
        match self {
            TimeoutMode::Inherit => "inherit",
            TimeoutMode::Unlimited => "unlimited",
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
            Timeout::Unlimited => TimeoutMode::Unlimited,
            _ => TimeoutMode::Custom,
        }
    }
}

/// Whether a request checks certificates as Preferences says, always, or never.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsMode {
    /// Follow Preferences.
    Inherit,
    /// Always verify.
    On,
    /// Accept any certificate.
    Off,
}

impl TlsMode {
    const ALL: [TlsMode; 3] = [TlsMode::Inherit, TlsMode::On, TlsMode::Off];

    /// The text the select shows; Inherit names what Preferences does, `insecure` meaning
    /// certificates go unchecked there.
    pub fn label(self, insecure: bool) -> SharedString {
        match (self, insecure) {
            (TlsMode::Inherit, false) => "Default (Verify)".into(),
            (TlsMode::Inherit, true) => "Default (Don't verify)".into(),
            (TlsMode::On, _) => "Verify".into(),
            (TlsMode::Off, _) => "Don't verify".into(),
        }
    }

    fn key(self) -> &'static str {
        match self {
            TlsMode::Inherit => "inherit",
            TlsMode::On => "on",
            TlsMode::Off => "off",
        }
    }

    fn of(verify: TlsVerify) -> TlsMode {
        match verify {
            TlsVerify::On => TlsMode::On,
            TlsVerify::Off => TlsMode::Off,
            _ => TlsMode::Inherit,
        }
    }

    fn verify(self) -> TlsVerify {
        match self {
            TlsMode::Inherit => TlsVerify::Inherit,
            TlsMode::On => TlsVerify::On,
            TlsMode::Off => TlsVerify::Off,
        }
    }
}

/// Whether a request speaks the Preferences HTTP version or one of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionMode {
    /// Follow Preferences.
    Inherit,
    /// HTTP/1.1 only.
    Http1Only,
    /// HTTP/2 over https when the server offers it.
    Auto,
    /// HTTP/2 from the first byte.
    Http2PriorKnowledge,
}

impl VersionMode {
    const ALL: [VersionMode; 4] = [
        VersionMode::Inherit,
        VersionMode::Http1Only,
        VersionMode::Auto,
        VersionMode::Http2PriorKnowledge,
    ];

    /// The text the select shows; Inherit names the Preferences choice `pref`.
    pub fn label(self, pref: HttpVersionPref) -> SharedString {
        match self {
            VersionMode::Inherit => {
                let own = match pref {
                    HttpVersionPref::Http1Only => VersionMode::Http1Only,
                    HttpVersionPref::Auto => VersionMode::Auto,
                    HttpVersionPref::Http2PriorKnowledge => VersionMode::Http2PriorKnowledge,
                };
                format!("Default ({})", own.label(pref)).into()
            }
            VersionMode::Http1Only => "HTTP/1.1".into(),
            VersionMode::Auto => "Auto".into(),
            VersionMode::Http2PriorKnowledge => "HTTP/2 prior knowledge".into(),
        }
    }

    fn key(self) -> &'static str {
        match self {
            VersionMode::Inherit => "inherit",
            VersionMode::Http1Only => "1.1",
            VersionMode::Auto => "auto",
            VersionMode::Http2PriorKnowledge => "2-prior-knowledge",
        }
    }

    fn of(version: Option<HttpVersionPolicy>) -> VersionMode {
        match version {
            None => VersionMode::Inherit,
            Some(HttpVersionPolicy::Auto) => VersionMode::Auto,
            Some(HttpVersionPolicy::Http2PriorKnowledge) => VersionMode::Http2PriorKnowledge,
            Some(_) => VersionMode::Http1Only,
        }
    }

    fn policy(self) -> Option<HttpVersionPolicy> {
        match self {
            VersionMode::Inherit => None,
            VersionMode::Http1Only => Some(HttpVersionPolicy::Http1Only),
            VersionMode::Auto => Some(HttpVersionPolicy::Auto),
            VersionMode::Http2PriorKnowledge => Some(HttpVersionPolicy::Http2PriorKnowledge),
        }
    }
}

/// The Preferences certificate and version choices a request inherits; the app defaults
/// when the app state is absent (tests).
fn transport_defaults(cx: &App) -> (bool, HttpVersionPref) {
    cx.try_global::<AppState>().map_or_else(
        || (false, HttpVersionPref::default()),
        |state| {
            let settings = state.settings.read(cx).get();
            (settings.insecure, settings.http_version)
        },
    )
}

fn tls_rows(insecure: bool) -> Vec<ui::IdItem> {
    (TlsMode::ALL.iter())
        .map(|m| ui::IdItem::new("set-verify-tls", m.key(), m.label(insecure)))
        .collect()
}

fn version_rows(pref: HttpVersionPref) -> Vec<ui::IdItem> {
    (VersionMode::ALL.iter())
        .map(|m| ui::IdItem::new("set-http-version", m.key(), m.label(pref)))
        .collect()
}

/// Bruno's settings pane: three toggles, max redirects and the timeout, plus the request's
/// own certificate check and HTTP version.
///
/// Each change sends the last sent or loaded settings with one field changed, so changes
/// compose before their echo arrives. Max redirects takes digits, empty meaning 0; a custom
/// timeout takes seconds to the millisecond, stored as milliseconds, and must be above 0.
pub struct SettingsSection {
    // The settings last sent or loaded; the base each change builds on.
    base: RequestSettings,
    max_redirects: Entity<InputState>,
    timeout_select: Entity<ui::IdSelect>,
    // The request's own limit in ms, last loaded or typed, which Custom brings back.
    custom: Option<u64>,
    // Custom is picked but holds no limit yet; the file keeps its timeout until one is typed.
    custom_pending: bool,
    // The Preferences timeout the Inherit label names.
    default_timeout: Option<Duration>,
    timeout: Entity<InputState>,
    tls_select: Entity<ui::IdSelect>,
    version_select: Entity<ui::IdSelect>,
    // The Preferences choices the Default entries name.
    transport_defaults: (bool, HttpVersionPref),
    tags: Entity<TagField>,
    // The field whose last typed number was refused, and why; shown under that field.
    invalid: Option<(NumberField, &'static str)>,
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

// The custom field shows seconds, as Preferences does, while the file keeps milliseconds: a
// whole number with up to three decimals; empty is 0, anything else is refused.
fn seconds_to_millis(text: &str) -> Option<u64> {
    let (whole, frac) = text.split_once('.').unwrap_or((text, ""));
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if !digits(whole)
        || !digits(frac)
        || frac.len() > 3
        || (text.contains('.') && whole.is_empty() && frac.is_empty())
    {
        return None;
    }
    let whole: u64 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let frac: u64 = format!("{frac:0<3}").parse().ok()?;
    whole.checked_mul(1000)?.checked_add(frac)
}

/// `ms` as the seconds the custom timeout field shows: 2500 is "2.5", 30000 is "30".
pub fn millis_to_seconds(ms: u64) -> String {
    let (whole, frac) = (ms / 1000, ms % 1000);
    if frac == 0 {
        return whole.to_string();
    }
    let frac = format!("{frac:03}");
    format!("{whole}.{}", frac.trim_end_matches('0'))
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
        let transport = transport_defaults(cx);
        let tls_select = cx
            .new(|cx| SelectState::new(tls_rows(transport.0), Some(IndexPath::new(0)), window, cx));
        let version_select = cx.new(|cx| {
            SelectState::new(
                version_rows(transport.1),
                Some(IndexPath::new(0)),
                window,
                cx,
            )
        });
        let tags = cx.new(|cx| TagField::new(window, cx));
        let mut subs = vec![
            cx.subscribe_in(
                &tls_select,
                window,
                |this, _, event: &SelectEvent<Vec<ui::IdItem>>, window, cx| {
                    if let SelectEvent::Confirm(Some(label)) = event
                        && let Some(mode) = (TlsMode::ALL.into_iter())
                            .find(|m| m.label(this.transport_defaults.0) == *label)
                    {
                        this.set_tls_mode(mode, window, cx);
                    }
                },
            ),
            cx.subscribe_in(
                &version_select,
                window,
                |this, _, event: &SelectEvent<Vec<ui::IdItem>>, window, cx| {
                    if let SelectEvent::Confirm(Some(label)) = event
                        && let Some(mode) = (VersionMode::ALL.into_iter())
                            .find(|m| m.label(this.transport_defaults.1) == *label)
                    {
                        this.set_version_mode(mode, window, cx);
                    }
                },
            ),
            cx.subscribe(&tags, |_, _, e: &TagsChanged, cx| {
                cx.emit(SectionEdit::new(Edit::Tags(e.0.clone())))
            }),
            cx.subscribe(&max_redirects, |this, state, event: &InputEvent, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let text = state.read(cx).value().to_string();
                let n = digits::<u32>(&text);
                this.number(NumberField::MaxRedirects, n, |s, n| s.max_redirects = n, cx);
            }),
            cx.subscribe(&timeout, |this, state, event: &InputEvent, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let text = state.read(cx).value().to_string();
                let millis = seconds_to_millis(&text);
                if millis == Some(0) {
                    this.invalid = Some((NumberField::Timeout, TIMEOUT_ZERO_NOTE));
                    cx.notify();
                    return;
                }
                if millis.is_some() {
                    this.custom = millis;
                    this.custom_pending = false;
                }
                let timeout = millis.map(Timeout::Millis);
                this.number(NumberField::Timeout, timeout, |s, t| s.timeout = t, cx);
                if timeout.is_none() {
                    this.invalid = Some((NumberField::Timeout, SECONDS_NOTE));
                }
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
                let transport = transport_defaults(cx);
                if transport != this.transport_defaults {
                    this.transport_defaults = transport;
                    let rows = tls_rows(transport.0);
                    (this.tls_select).update(cx, |s, cx| s.set_items(rows, window, cx));
                    let rows = version_rows(transport.1);
                    (this.version_select).update(cx, |s, cx| s.set_items(rows, window, cx));
                    this.show_transport(window, cx);
                }
            }));
        }
        SettingsSection {
            follow: FocusFollow::default(),
            base,
            max_redirects,
            timeout_select,
            custom: None,
            custom_pending: false,
            default_timeout,
            timeout,
            tls_select,
            version_select,
            transport_defaults: transport,
            tags,
            invalid: None,
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
        // A pending Custom outlives the echo of another field's edit, which keeps the timeout.
        self.custom_pending &= settings.timeout == self.base.timeout;
        self.base = *settings;
        if let Timeout::Millis(n) = settings.timeout {
            self.custom = Some(n);
        }
        self.invalid = None;
        set_text(
            &self.max_redirects,
            &settings.max_redirects.to_string(),
            window,
            cx,
        );
        self.show_timeout(window, cx);
        self.show_transport(window, cx);
        cx.notify();
    }

    /// The text the certificate check select shows.
    pub fn tls_shown(&self, cx: &App) -> Option<SharedString> {
        self.tls_select.read(cx).selected_value().cloned()
    }

    /// The text the HTTP version select shows.
    pub fn version_shown(&self, cx: &App) -> Option<SharedString> {
        self.version_select.read(cx).selected_value().cloned()
    }

    /// Picks the request's certificate check and sends it when it changed.
    pub fn set_tls_mode(&mut self, mode: TlsMode, window: &mut Window, cx: &mut Context<Self>) {
        if self.base.verify_tls != mode.verify() {
            self.base.verify_tls = mode.verify();
            self.emit(cx);
        }
        self.show_transport(window, cx);
    }

    /// Picks the request's HTTP version and sends it when it changed.
    pub fn set_version_mode(
        &mut self,
        mode: VersionMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.base.http_version != mode.policy() {
            self.base.http_version = mode.policy();
            self.emit(cx);
        }
        self.show_transport(window, cx);
    }

    fn show_transport(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (insecure, pref) = self.transport_defaults;
        let tls = TlsMode::of(self.base.verify_tls).label(insecure);
        let version = VersionMode::of(self.base.http_version).label(pref);
        (self.tls_select).update(cx, |s, cx| s.set_selected_value(&tls, window, cx));
        (self.version_select).update(cx, |s, cx| s.set_selected_value(&version, window, cx));
        cx.notify();
    }

    /// Drops a Custom picked but never sent, as a discard does, so the select shows the
    /// timeout the settings hold.
    pub fn discard_unsent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.custom_pending) {
            self.show_timeout(window, cx);
        }
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

    /// The custom timeout field.
    pub fn timeout_input(&self) -> &Entity<InputState> {
        &self.timeout
    }

    /// The tag chip input.
    pub fn tags(&self) -> &Entity<TagField> {
        &self.tags
    }

    /// The refusal shown under a number field, if any.
    pub fn refusal_shown(&self) -> Option<&'static str> {
        self.invalid.map(|(_, note)| note)
    }

    /// The text the timeout select shows.
    pub fn timeout_shown(&self, cx: &App) -> Option<SharedString> {
        self.timeout_select.read(cx).selected_value().cloned()
    }

    /// Whether the timeout is inherited, lifted, or the request's own.
    pub fn timeout_mode(&self) -> TimeoutMode {
        if self.custom_pending {
            return TimeoutMode::Custom;
        }
        TimeoutMode::of(self.base.timeout)
    }

    /// Switches the timeout mode and sends it; Custom brings back the request's own limit, or
    /// shows an empty field and sends nothing until a limit is typed.
    pub fn set_timeout_mode(
        &mut self,
        mode: TimeoutMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.timeout_mode() == mode {
            return;
        }
        let timeout = match mode {
            TimeoutMode::Inherit => Some(Timeout::Inherit),
            TimeoutMode::Unlimited => Some(Timeout::Unlimited),
            TimeoutMode::Custom => self.custom.map(Timeout::Millis),
        };
        self.custom_pending = timeout.is_none();
        self.invalid = None;
        // Leaving a pending Custom for the mode the file already holds changes nothing.
        let changed = timeout.filter(|t| *t != self.base.timeout);
        if let Some(timeout) = changed {
            self.base.timeout = timeout;
        }
        self.show_timeout(window, cx);
        if changed.is_some() {
            self.emit(cx);
        }
    }

    fn show_timeout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let label = self.timeout_mode().label(self.default_timeout);
        self.timeout_select
            .update(cx, |s, cx| s.set_selected_value(&label, window, cx));
        if self.timeout_mode() == TimeoutMode::Custom {
            let text = self.custom.map(millis_to_seconds).unwrap_or_default();
            set_text(&self.timeout, &text, window, cx);
        }
        cx.notify();
    }

    // A typed number either updates one field and is sent, or is refused and only flagged.
    fn number<T>(
        &mut self,
        field: NumberField,
        value: Option<T>,
        apply: impl FnOnce(&mut RequestSettings, T),
        cx: &mut Context<Self>,
    ) {
        match value {
            Some(v) => {
                self.invalid = self.invalid.filter(|(f, _)| *f != field);
                apply(&mut self.base, v);
                self.emit(cx);
            }
            None => self.invalid = Some((field, DIGITS_NOTE)),
        }
        cx.notify();
    }

    // `field`'s input at the form width, marked while its last typed number stands refused.
    fn number_input(&self, field: NumberField, disabled: bool, window: &Window, cx: &App) -> Div {
        let id = field.id();
        ui::invalid_input(
            Input::new(self.number_state(field))
                .id(id)
                .w(FORM_INPUT_WIDTH)
                .disabled(disabled)
                .bg(ui::field_fill(disabled, cx)),
            div().w(FORM_INPUT_WIDTH).flex_none(),
            self.number_state(field),
            self.refusal(field).is_some(),
            (
                SharedString::from(format!("{id}-danger-ring")).into(),
                SharedString::from(format!("{id}-invalid-icon")).into(),
            ),
            window,
            cx,
        )
    }

    fn number_state(&self, field: NumberField) -> &Entity<InputState> {
        match field {
            NumberField::MaxRedirects => &self.max_redirects,
            NumberField::Timeout => &self.timeout,
        }
    }

    fn refusal(&self, field: NumberField) -> Option<&'static str> {
        self.invalid
            .filter(|(f, _)| *f == field)
            .map(|(_, note)| note)
    }

    // A row's control with `field`'s refusal, when it has one, right under it.
    fn with_refusal(&self, field: NumberField, control: impl IntoElement, cx: &App) -> Div {
        v_flex()
            .flex_1()
            .min_w_0()
            .gap(SPACE_1)
            .child(control)
            .when_some(self.refusal(field), |el, note| {
                let id = SharedString::from(format!("{}-error", field.id()));
                el.child(ui::field_error(id, note, cx))
            })
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let s = self.base;
        let mode = self.timeout_mode();
        let follow = self.follow.clone();
        let followed = move |row: ui::FormRow| Followed::new(row.wrapper_id(), &follow, row);
        let custom = mode == TimeoutMode::Custom;
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
                r.child(ui::with_unit(
                    self.number_input(NumberField::Timeout, false, window, cx),
                    "set-timeout-unit",
                    "s",
                    cx,
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
                "Keep Authorization header on redirect to another origin",
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
                self.with_refusal(
                    NumberField::MaxRedirects,
                    div()
                        .id("set-max-redirects-wrap")
                        .when(!s.follow_redirects, |d| {
                            d.tooltip(ui::text_tooltip(
                                "set-max-redirects-tooltip",
                                MAX_REDIRECTS_OFF_TIP,
                            ))
                        })
                        .child(self.number_input(
                            NumberField::MaxRedirects,
                            !s.follow_redirects,
                            window,
                            cx,
                        )),
                    cx,
                ),
            )))
            .child(followed(
                ui::form_row(
                    "set-row-timeout",
                    "Timeout",
                    self.with_refusal(NumberField::Timeout, timeout, cx),
                )
                .hint("set-timeout-hint", mode.hint()),
            ))
            .child(followed(ui::form_row(
                "set-row-verify-tls",
                "Certificate check",
                div()
                    .w(FORM_INPUT_WIDTH)
                    .child(Select::new(&self.tls_select).id("set-verify-tls")),
            )))
            .child(followed(ui::form_row(
                "set-row-http-version",
                "HTTP version",
                div()
                    .w(FORM_INPUT_WIDTH)
                    .child(Select::new(&self.version_select).id("set-http-version")),
            )))
            .child(followed(ui::form_row(
                "set-row-tags",
                "Tags",
                self.tags.clone(),
            )))
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
    vim: Entity<Vim>,
    _sub: Subscription,
    _vim_sub: Subscription,
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
        let vim = cx.new(|cx| Vim::new(&code, cx));
        let vim_sub = cx.observe(&vim, |_, _, cx| cx.notify());
        DocsSection {
            code,
            mode: DocsMode::Edit,
            mode_chosen: false,
            vim,
            _sub: sub,
            _vim_sub: vim_sub,
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

    /// The Vim key map of the markdown editor.
    pub fn vim(&self) -> &Entity<Vim> {
        &self.vim
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
            DocsMode::Edit => crate::vim::scope(div().id("docs-code"), &self.vim, cx).child(
                ui::ringed_editor(Editor::new(&self.code).h_full(), &self.code),
            ),
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
        let header = h_flex()
            .gap(SPACE_2)
            .items_center()
            .child(toggle)
            .children(crate::vim::mode_label("docs-vim-mode", &self.vim, cx));
        v_flex().w_full().flex_1().gap(SPACE_2).child(header).child(
            body.test_support()
                .w_full()
                .flex_1()
                .min_h(style::EDITOR_MIN_HEIGHT),
        )
    }
}
