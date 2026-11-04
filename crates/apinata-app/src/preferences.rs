//! The preferences dialog: edits the user's settings in place. Theme, orientation, zoom and
//! the response font size apply at once; the network settings apply at the next start. The
//! global proxy password goes to the secret store, never to the settings file.

use gpui_kit::assets::IconName;
use gpui_kit::component::WindowExt;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::{Scrollbar, ScrollbarMode};
use gpui_kit::component::{ActiveTheme as _, ThemeMode, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::path::PathBuf;
use std::sync::Arc;

use apinata_core::SecretStore;

use crate::app::{AppState, VolatileSecrets};
use crate::dialog_frame;
use crate::executor::{parse_pac, proxy_password_key};
use crate::keyboard_prefs::KeyboardPrefs;
use crate::keymap::{ResetZoom, ZoomIn, ZoomOut};
use crate::persisted::Persisted;
use crate::prefs_section::PrefsSection;
use crate::settings::{
    HttpVersionPref, LinuxTitleBar, Orientation, ProxyMode, ProxyPrefs, REQUEST_TIMEOUT_SECS,
    RunScripts, SCRIPT_TIMEOUT_SECS, Settings, ThemeChoice,
};
use crate::style::{
    FORM_INPUT_WIDTH, PREFS_DIALOG_CHROME, PREFS_DIALOG_HEIGHT, PREFS_DIALOG_WIDTH,
    PREFS_NAV_WIDTH, PREFS_SEARCH_HINT_ROOM, PREFS_SHORT_ROOM, SCROLL_FADE_HEIGHT,
    SCROLLBAR_GUTTER, SPACE_1, SPACE_3, SPACE_4, SPACE_5, TEXT_SMALL, TEXT_TITLE, TEXT_UI, at_zoom,
    resolve,
};
use crate::theme;
use crate::theme::SelectedStyle;
use crate::ui::{self, ChooseButton as _, LockButton as _};
use crate::updates::{Updates, UpdatesEvent, report_text};
use crate::zoom::{self, ZoomStep, ZoomTarget};

actions!(
    apinata,
    [
        /// Opens the preferences dialog.
        OpenPreferences,
        /// Focuses the Preferences search box.
        FocusPreferencesSearch,
        /// Empties the Preferences search box, or closes the dialog when it is already empty.
        ClearPreferencesSearch
    ]
);

/// The key context on the dialog body, so its keys reach it past the kit's dialog host.
pub const KEY_CONTEXT: &str = "Preferences";

const MB: u64 = 1024 * 1024;
const THEMES: [(ThemeChoice, &str); 3] = [
    (ThemeChoice::System, "System"),
    (ThemeChoice::Light, "Light"),
    (ThemeChoice::Dark, "Dark"),
];
const TITLE_BARS: [(LinuxTitleBar, &str); 2] = [
    (LinuxTitleBar::Client, "Integrated"),
    (LinuxTitleBar::System, "System"),
];
const RUN_SCRIPTS: [(RunScripts, &str); 2] = [
    (RunScripts::Ask, "Ask for each collection"),
    (RunScripts::Never, "Never"),
];
const PROXY_MODES: [(ProxyMode, &str); 4] = [
    (ProxyMode::Off, "Off"),
    (ProxyMode::System, "System"),
    (ProxyMode::Manual, "Manual"),
    (ProxyMode::Pac, "PAC"),
];
const HTTP_VERSIONS: [(HttpVersionPref, &str); 3] = [
    (HttpVersionPref::Http1Only, "HTTP/1.1"),
    (HttpVersionPref::Auto, "Auto"),
    (HttpVersionPref::Http2PriorKnowledge, "HTTP/2"),
];
const HTTP_VERSION_HINT: &str = "HTTP/1.1 is the default. Auto uses HTTP/2 when an HTTPS server \
    offers it. HTTP/2 sends HTTP/2 without asking first, also over plain http, and fails if the \
    server does not speak it.";
const PROXY_PROTOCOLS: [&str; 4] = ["http", "https", "socks4", "socks5"];
const ORIENTATIONS: [(Orientation, &str); 2] = [
    (Orientation::Vertical, "Vertical"),
    (Orientation::Horizontal, "Horizontal"),
];

// The proxy inputs; the password one starts blank since the store is never read here.
struct ProxyInputs {
    host: Entity<InputState>,
    port: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    bypass: Entity<InputState>,
    pac: Entity<InputState>,
}

// A text field that commits on Enter, blur and dialog close; `id` is its input's element id.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Field {
    Zoom,
    EditorFont,
    ResponseFont,
    BodyCap,
    HistoryLimit,
    ScriptTimeout,
    RequestTimeout,
    Host,
    Port,
    Username,
    Bypass,
    PacSource,
    CaFile,
}

const FIELDS: [Field; 13] = [
    Field::Zoom,
    Field::EditorFont,
    Field::ResponseFont,
    Field::BodyCap,
    Field::HistoryLimit,
    Field::ScriptTimeout,
    Field::RequestTimeout,
    Field::Host,
    Field::Port,
    Field::Username,
    Field::Bypass,
    Field::PacSource,
    Field::CaFile,
];

impl Field {
    fn id(self) -> &'static str {
        match self {
            Field::Zoom => "zoom",
            Field::EditorFont => "editor-font-size",
            Field::ResponseFont => "response-font-size",
            Field::BodyCap => "body-cap",
            Field::HistoryLimit => "history-limit",
            Field::ScriptTimeout => "script-timeout",
            Field::RequestTimeout => "request-timeout",
            Field::Host => "global-proxy-host",
            Field::Port => "global-proxy-port",
            Field::Username => "global-proxy-username",
            Field::Bypass => "global-proxy-bypass",
            Field::PacSource => "global-proxy-pac",
            Field::CaFile => "ca-file",
        }
    }

    // The short name a close warning gives the field.
    fn label(self) -> &'static str {
        match self {
            Field::Zoom => "Zoom",
            Field::EditorFont => "Editor font size",
            Field::ResponseFont => "Response font size",
            Field::BodyCap => "Body cap",
            Field::HistoryLimit => "Responses kept per request",
            Field::ScriptTimeout => "Script time limit",
            Field::RequestTimeout => "Request timeout",
            Field::Host => "Host",
            Field::Port => "Port",
            Field::Username => "Username",
            Field::Bypass => "Bypass",
            Field::PacSource => "PAC file",
            Field::CaFile => "CA certificate file",
        }
    }
}

/// The dialog body over the app's settings.
pub struct PreferencesView {
    settings: Entity<Persisted<Settings>>,
    body_cap: Entity<InputState>,
    history_limit: Entity<InputState>,
    script_timeout: Entity<InputState>,
    request_timeout: Entity<InputState>,
    zoom: Entity<InputState>,
    editor_font: Entity<InputState>,
    response_font: Entity<InputState>,
    search: Entity<InputState>,
    proxy: ProxyInputs,
    ca_file: Entity<InputState>,
    secrets: Option<Arc<dyn SecretStore>>,
    keyboard: Entity<KeyboardPrefs>,
    // Set by a reset click, which does not take focus: the next refill covers focused inputs too.
    refill_focused: bool,
    updates: Option<Entity<Updates>>,
    // A failure outside any one field, such as the keychain refusing the password.
    error: Option<String>,
    // Each field whose typed text was refused, with the reason; its text stays as typed.
    errors: Vec<(Field, String)>,
    // Per field in `FIELDS` order, the text last shown or committed. A field still showing
    // it is unedited, so a blur or close leaves its setting alone.
    shown: [String; FIELDS.len()],
    // The result of the dialog's last button, shown in the dialog since toasts sit beneath it.
    status: Option<SharedString>,
    // The rows' scroll position, shared by the column, its always-shown bar and the fade.
    scroll: ScrollHandle,
    // The ids of the sections the last render showed, in order: the scroll column's children.
    shown_sections: Vec<SharedString>,
    focus: FocusHandle,
    _subs: Vec<Subscription>,
}

impl PreferencesView {
    /// A body showing `settings`, whose inputs commit on Enter, blur and close.
    pub fn new(
        settings: Entity<Persisted<Settings>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let current = settings.read(cx).get().clone();
        let cap = current
            .max_body_bytes
            .map(|b| b.div_ceil(MB).to_string())
            .unwrap_or_default();
        let body_cap = cx.new(|cx| InputState::new(window, cx).default_value(cap));
        let history_limit = cx
            .new(|cx| InputState::new(window, cx).default_value(current.history_limit.to_string()));
        let script_timeout = cx.new(|cx| {
            InputState::new(window, cx).default_value(current.script_timeout_secs.to_string())
        });
        let request_timeout = cx.new(|cx| {
            InputState::new(window, cx).default_value(current.request_timeout_secs.to_string())
        });
        let (zoom_text, editor_text, response_text) = shown_sizes(&current);
        let zoom = cx.new(|cx| InputState::new(window, cx).default_value(zoom_text));
        let editor_font = cx.new(|cx| InputState::new(window, cx).default_value(editor_text));
        let response_font = cx.new(|cx| InputState::new(window, cx).default_value(response_text));
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search settings"));
        let password = cx.new(|cx| InputState::new(window, cx).masked(true));
        let mut text_input = |text: &str| {
            let text = text.to_string();
            cx.new(|cx| InputState::new(window, cx).default_value(text))
        };
        let proxy = ProxyInputs {
            host: text_input(&current.proxy.host),
            port: text_input(&current.proxy.port),
            username: text_input(&current.proxy.username),
            password,
            bypass: text_input(&current.proxy.bypass),
            pac: {
                let text = current.proxy.pac_source.clone();
                cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder("https://example.com/proxy.pac or /path/to/proxy.pac")
                        .default_value(text)
                })
            },
        };
        let ca_file = cx.new(|cx| {
            let shown = current.ca_file.as_ref().map(|p| p.display().to_string());
            InputState::new(window, cx).default_value(shown.unwrap_or_default())
        });
        let keyboard = cx.new(KeyboardPrefs::new);
        let updates = cx.try_global::<AppState>().map(|s| s.updates.clone());
        let secrets = cx.try_global::<AppState>().map(|s| s.secrets.clone());
        let mut subs = vec![
            cx.observe(&keyboard, |_, _, cx| cx.notify()),
            // A key or scroll zoom changes the zoom while the dialog is open; refill each
            // input the user is not typing in, or every input after a reset.
            cx.observe_in(&settings, window, |this, settings, window, cx| {
                let (zoom_text, editor_text, response_text) = shown_sizes(settings.read(cx).get());
                let all = std::mem::take(&mut this.refill_focused);
                for (field, text) in [
                    (Field::Zoom, zoom_text),
                    (Field::EditorFont, editor_text),
                    (Field::ResponseFont, response_text),
                ] {
                    let state = this.state(field).clone();
                    let refused = this.error_of(field).is_some();
                    if all || !(refused || state.read(cx).focus_handle(cx).is_focused(window)) {
                        state.update(cx, |s, cx| s.set_value(text.clone(), window, cx));
                        this.shown[field as usize] = text;
                    }
                }
                cx.notify();
            }),
            cx.subscribe(&search, |_, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(&proxy.password, window, |this, _, e, window, cx| {
                if matches!(e, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    this.commit_password(window, cx);
                }
            }),
        ];
        for (field, state) in [
            (Field::Zoom, &zoom),
            (Field::EditorFont, &editor_font),
            (Field::ResponseFont, &response_font),
            (Field::BodyCap, &body_cap),
            (Field::HistoryLimit, &history_limit),
            (Field::ScriptTimeout, &script_timeout),
            (Field::RequestTimeout, &request_timeout),
            (Field::Host, &proxy.host),
            (Field::Port, &proxy.port),
            (Field::Username, &proxy.username),
            (Field::Bypass, &proxy.bypass),
            (Field::PacSource, &proxy.pac),
            (Field::CaFile, &ca_file),
        ] {
            subs.push(
                cx.subscribe(state, move |this, _, e: &InputEvent, cx| match e {
                    InputEvent::PressEnter { .. } | InputEvent::Blur => {
                        this.commit_field(field, true, cx)
                    }
                    // A shown error follows the typing; the value is stored only at Enter or blur.
                    InputEvent::Change if this.error_of(field).is_some() => {
                        this.commit_field(field, false, cx)
                    }
                    _ => {}
                }),
            );
        }
        // The disabled note follows the checker, which a test or `main` may set later.
        if let Some(updates) = &updates {
            subs.push(cx.observe(updates, |_, _, cx| cx.notify()));
            subs.push(cx.subscribe(updates, |this, _, e: &UpdatesEvent, cx| {
                let UpdatesEvent::Reported(report) = e;
                this.status = Some(report_text(report).into());
                cx.notify();
            }));
        }
        let shown = FIELDS.map(|f| {
            let state = match f {
                Field::Zoom => &zoom,
                Field::EditorFont => &editor_font,
                Field::ResponseFont => &response_font,
                Field::BodyCap => &body_cap,
                Field::HistoryLimit => &history_limit,
                Field::ScriptTimeout => &script_timeout,
                Field::RequestTimeout => &request_timeout,
                Field::Host => &proxy.host,
                Field::Port => &proxy.port,
                Field::Username => &proxy.username,
                Field::Bypass => &proxy.bypass,
                Field::PacSource => &proxy.pac,
                Field::CaFile => &ca_file,
            };
            state.read(cx).value().to_string()
        });
        PreferencesView {
            settings,
            body_cap,
            history_limit,
            script_timeout,
            request_timeout,
            zoom,
            editor_font,
            response_font,
            search,
            proxy,
            ca_file,
            secrets,
            keyboard,
            shown,
            refill_focused: false,
            updates,
            error: None,
            errors: Vec::new(),
            status: None,
            scroll: ScrollHandle::new(),
            shown_sections: Vec::new(),
            focus: cx.focus_handle(),
            _subs: subs,
        }
    }

    /// The search box's state; its text filters the sections.
    pub fn search(&self) -> &Entity<InputState> {
        &self.search
    }

    /// The Keyboard section's entity.
    pub fn keyboard(&self) -> &Entity<KeyboardPrefs> {
        &self.keyboard
    }

    /// The result of the last Clear or Check now in this dialog, or `None` before either.
    pub fn status(&self) -> Option<SharedString> {
        self.status.clone()
    }

    /// The texts the zoom and response font size inputs show.
    pub fn field_texts(&self, cx: &App) -> (String, String) {
        let text = |state: &Entity<InputState>| state.read(cx).value().to_string();
        (text(&self.zoom), text(&self.response_font))
    }

    /// The text of the input with element id `id`, or `None` for an id that is no field.
    pub fn input_text(&self, id: &str, cx: &App) -> Option<String> {
        let field = FIELDS.into_iter().find(|f| f.id() == id)?;
        Some(self.state(field).read(cx).value().to_string())
    }

    /// The warnings closing the dialog shows, one "Not saved: <label>: <reason>" per refused
    /// field.
    pub fn unsaved(&self) -> Vec<String> {
        self.errors
            .iter()
            .map(|(field, reason)| format!("Not saved: {}: {reason}", field.label()))
            .collect()
    }

    fn state(&self, field: Field) -> &Entity<InputState> {
        match field {
            Field::Zoom => &self.zoom,
            Field::EditorFont => &self.editor_font,
            Field::ResponseFont => &self.response_font,
            Field::BodyCap => &self.body_cap,
            Field::HistoryLimit => &self.history_limit,
            Field::ScriptTimeout => &self.script_timeout,
            Field::RequestTimeout => &self.request_timeout,
            Field::Host => &self.proxy.host,
            Field::Port => &self.proxy.port,
            Field::Username => &self.proxy.username,
            Field::Bypass => &self.proxy.bypass,
            Field::PacSource => &self.proxy.pac,
            Field::CaFile => &self.ca_file,
        }
    }

    fn error_of(&self, field: Field) -> Option<&str> {
        self.errors
            .iter()
            .find(|(f, _)| *f == field)
            .map(|(_, e)| e.as_str())
    }

    // Parses the field's text and, with `store`, stores it, unless it still shows what was last
    // shown or committed: rounded or stale text must not overwrite the setting behind it.
    // Without `store` only the field's error is updated.
    fn commit_field(&mut self, field: Field, store: bool, cx: &mut Context<Self>) {
        let text = self.state(field).read(cx).value().to_string();
        if text == self.shown[field as usize] {
            self.errors.retain(|(f, _)| *f != field);
            cx.notify();
            return;
        }
        let trimmed = |t: &str| t.trim().to_string();
        match field {
            Field::Zoom => self.commit(store, field, parse_zoom(&text), |s, v| s.zoom = v, cx),
            Field::EditorFont => {
                let parsed = parse_font_size(&text, "editor");
                self.commit(store, field, parsed, |s, v| s.editor_font_offset = v, cx);
            }
            Field::ResponseFont => {
                let parsed = parse_font_size(&text, "response");
                self.commit(store, field, parsed, |s, v| s.response_font_offset = v, cx);
            }
            Field::BodyCap => self.commit(
                store,
                field,
                parse_body_cap(&text),
                |s, v| s.max_body_bytes = v,
                cx,
            ),
            Field::HistoryLimit => {
                let parsed = parse_history_limit(&text);
                self.commit(store, field, parsed, |s, v| s.history_limit = v, cx)
            }
            Field::ScriptTimeout => {
                let parsed = parse_script_timeout(&text);
                self.commit(store, field, parsed, |s, v| s.script_timeout_secs = v, cx)
            }
            Field::RequestTimeout => {
                let parsed = parse_request_timeout(&text);
                self.commit(store, field, parsed, |s, v| s.request_timeout_secs = v, cx)
            }
            Field::Host => self.commit(
                store,
                field,
                Ok(trimmed(&text)),
                |s, v| s.proxy.host = v,
                cx,
            ),
            Field::Port => self.commit(
                store,
                field,
                parse_proxy_port(&text),
                |s, v| s.proxy.port = v,
                cx,
            ),
            Field::Username => self.commit(
                store,
                field,
                Ok(trimmed(&text)),
                |s, v| s.proxy.username = v,
                cx,
            ),
            Field::Bypass => self.commit(
                store,
                field,
                Ok(trimmed(&text)),
                |s, v| s.proxy.bypass = v,
                cx,
            ),
            // Blank clears the source; other text is kept trimmed only if it parses without
            // credentials, so a password in a PAC URL never reaches settings.json.
            Field::PacSource => {
                let text = text.trim();
                let parsed = if text.is_empty() {
                    Ok(String::new())
                } else {
                    parse_pac(text).map(|_| text.to_owned())
                };
                self.commit(store, field, parsed, |s, v| s.proxy.pac_source = v, cx)
            }
            // Blank clears the CA file; any other text is stored as the path typed.
            Field::CaFile => {
                let text = text.trim();
                let path = (!text.is_empty()).then(|| PathBuf::from(text));
                self.commit(store, field, Ok(path), |s, v| s.ca_file = v, cx)
            }
        }
        if store && self.error_of(field).is_none() {
            self.shown[field as usize] = text;
        }
    }

    // Commits every field and a typed password, as closing the dialog does.
    fn commit_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for field in FIELDS {
            self.commit_field(field, true, cx);
        }
        self.commit_password(window, cx);
    }

    // A typed password goes to the store and leaves the input; a blank one does nothing.
    fn commit_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.proxy.password.read(cx).value().to_string();
        if !text.is_empty() {
            self.proxy
                .password
                .update(cx, |s, cx| s.set_value("", window, cx));
            self.store_password(Some(text), cx);
        }
    }

    // Stores a valid value when it changes anything and clears the field's error; a refused
    // one leaves the settings alone and records why.
    fn commit<T>(
        &mut self,
        store: bool,
        field: Field,
        parsed: Result<T, String>,
        set: impl FnOnce(&mut Settings, T),
        cx: &mut Context<Self>,
    ) {
        self.errors.retain(|(f, _)| *f != field);
        match parsed {
            Ok(_) if !store => {}
            Ok(value) => {
                let current = self.settings.read(cx).get();
                let mut next = current.clone();
                set(&mut next, value);
                let changed = next != *current;
                if changed {
                    self.edit(|s| *s = next, cx);
                }
            }
            Err(message) => self.errors.push((field, message)),
        }
        cx.notify();
    }

    // Stores a reset value, clears the field's error and refills both size inputs.
    fn reset<T>(
        &mut self,
        field: Field,
        value: T,
        set: impl FnOnce(&mut Settings, T),
        cx: &mut Context<Self>,
    ) {
        self.refill_focused = true;
        self.errors.retain(|(f, _)| *f != field);
        self.edit(|s| set(s, value), cx);
        cx.notify();
    }

    fn edit(&self, f: impl FnOnce(&mut Settings), cx: &mut Context<Self>) {
        self.settings.update(cx, |p, cx| p.update(cx, f));
    }

    // The field's input at the form width, with its refusal under it when it has one.
    fn field(&self, field: Field, window: &Window, cx: &App) -> Div {
        self.with_error(field, self.framed(field, window, cx), cx)
    }

    // The field's input at the form width, marked while it is focused and refused.
    fn framed(&self, field: Field, window: &Window, cx: &App) -> Div {
        let id = field.id();
        ui::invalid_input(
            Input::new(self.state(field))
                .id(id)
                .w(FORM_INPUT_WIDTH)
                .bg(ui::field_fill(false, cx)),
            div().w(FORM_INPUT_WIDTH).flex_none(),
            self.state(field),
            self.error_of(field).is_some(),
            (
                SharedString::from(format!("{id}-danger-ring")).into(),
                SharedString::from(format!("{id}-invalid-icon")).into(),
            ),
            window,
            cx,
        )
    }

    fn with_error(&self, field: Field, control: impl IntoElement, cx: &App) -> Div {
        // The column takes the row's remaining width, so a long reason wraps inside the dialog.
        v_flex()
            .flex_1()
            .min_w_0()
            .gap(SPACE_1)
            .child(control)
            .when_some(self.error_of(field), |el, error| {
                el.child(
                    div()
                        .w_full()
                        .whitespace_normal()
                        .id(SharedString::from(format!("{}-error", field.id())))
                        .test_support()
                        .text_size(TEXT_SMALL)
                        .text_color(cx.theme().danger)
                        .child(error.to_string()),
                )
            })
    }

    // Saves the global proxy password, or forgets it for `None`. The keychain can block, so
    // the call runs off the UI thread and only a failure comes back, as the dialog's error.
    fn store_password(&mut self, password: Option<String>, cx: &mut Context<Self>) {
        let Some(store) = self.secrets.clone() else {
            return;
        };
        let task = cx.background_spawn(async move {
            let key = proxy_password_key();
            match password {
                Some(p) => store.set(&key, &p),
                None => store.delete(&key),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.error = result
                    .err()
                    .map(|e| format!("The proxy password was not saved: {e}"));
                cx.notify();
            });
        })
        .detach();
    }

    // Asks for one file and makes it the CA file; a cancelled or failed prompt does nothing.
    fn choose_ca_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
            let _ = this.update_in(cx, |this, window, cx| {
                let text = path.display().to_string();
                this.ca_file
                    .update(cx, |s, cx| s.set_value(text, window, cx));
                this.commit_field(Field::CaFile, true, cx);
            });
        })
        .detach();
    }

    // The PAC source input with its Refetch button, and the hint under them. Refetch drops
    // the process-wide PAC cache at once, unlike the settings, which apply at restart.
    fn pac_row(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        // Each Preferences button takes Enter as its press; the inputs beside them keep theirs.
        let refetch = Button::new("global-proxy-pac-refetch")
            .key_context(ui::DIALOG_FOOTER_CONTEXT)
            .label("Refetch")
            .on_click(cx.listener(|this, _, _, cx| {
                apinata_core::clear_pac_cache();
                this.status =
                    Some("The PAC file will be fetched again on the next request.".into());
                cx.notify();
            }));
        let input = self.framed(Field::PacSource, window, cx);
        let control = h_flex().gap(SPACE_1).child(input).child(refetch);
        // Not `form_note`: its label-width indent is for section notes and leaves no room
        // inside the field's column.
        self.with_error(Field::PacSource, control, cx).child(
            div()
                .w_full()
                .whitespace_normal()
                .id("prefs-proxy-pac-hint")
                .test_support()
                .text_size(TEXT_SMALL)
                .text_color(cx.theme().muted_foreground)
                .child(PAC_HINT),
        )
    }

    // The global proxy rows: the mode always, then the PAC source under PAC or the manual
    // proxy's fields under Manual.
    fn proxy_rows(
        &self,
        section: PrefsSection,
        current: &ProxyPrefs,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> PrefsSection {
        let view = cx.entity();
        let mode = ui::segmented(
            "global-proxy-mode",
            PROXY_MODES.map(|(_, label)| label),
            PROXY_MODES
                .iter()
                .position(|(m, _)| *m == current.mode)
                .unwrap_or_default(),
            {
                let view = view.clone();
                move |ix: &usize, _: &mut Window, cx: &mut App| {
                    if let Some(&(m, _)) = PROXY_MODES.get(*ix) {
                        view.update(cx, |this, cx| this.edit(|s| s.proxy.mode = m, cx));
                    }
                }
            },
        );
        let section = section.row("pref-proxy-mode", "Proxy", mode);
        match current.mode {
            ProxyMode::Manual => {}
            ProxyMode::Pac => {
                return section.row(
                    "pref-proxy-pac",
                    "PAC file (URL or path)",
                    self.pac_row(window, cx),
                );
            }
            ProxyMode::Off | ProxyMode::System => return section,
        }
        let protocol = ui::segmented(
            "global-proxy-protocol",
            PROXY_PROTOCOLS,
            PROXY_PROTOCOLS
                .iter()
                .position(|p| *p == current.protocol)
                .unwrap_or_default(),
            move |ix: &usize, _: &mut Window, cx: &mut App| {
                if let Some(&p) = PROXY_PROTOCOLS.get(*ix) {
                    view.update(cx, |this, cx| {
                        this.edit(|s| s.proxy.protocol = p.into(), cx)
                    });
                }
            },
        );
        let password = v_flex()
            .gap(SPACE_1)
            .child(
                h_flex()
                    .gap(SPACE_1)
                    .child(ui::padded_input(
                        Input::new(&self.proxy.password)
                            .id("global-proxy-password")
                            .w(FORM_INPUT_WIDTH)
                            .bg(ui::field_fill(false, cx)),
                        &self.proxy.password,
                    ))
                    .child(
                        Button::new("global-proxy-password-forget")
                            .key_context(ui::DIALOG_FOOTER_CONTEXT)
                            .label("Forget")
                            .on_click(cx.listener(|this, _, _, cx| this.store_password(None, cx))),
                    ),
            )
            .when(cx.has_global::<VolatileSecrets>(), |el| {
                el.child(ui::form_note(
                    "global-proxy-password-volatile",
                    "Keychain unavailable: the password is not remembered after quit",
                ))
            });
        section
            .row("pref-proxy-protocol", "Protocol", protocol)
            .row(
                "pref-proxy-host",
                "Host",
                self.field(Field::Host, window, cx),
            )
            .row(
                "pref-proxy-port",
                "Port",
                self.field(Field::Port, window, cx),
            )
            .row(
                "pref-proxy-username",
                "Username",
                self.field(Field::Username, window, cx),
            )
            .row("pref-proxy-password", "Password", password)
            .row(
                "pref-proxy-bypass",
                "Bypass (comma separated hosts)",
                self.field(Field::Bypass, window, cx),
            )
    }
}

/// The note under the Network section on what System mode reads.
pub const PROXY_NOTE: &str = "System uses the proxy environment variables (and the system \
     settings on macOS and Windows); a PAC file set in the system settings is not used. Choose \
     PAC to use one.";

/// The note under the PAC file input on what PAC mode leaves out.
pub const PAC_HINT: &str = "The bypass list and proxy login do not apply to PAC. Requests that \
     cannot use the PAC file connect directly and show a warning.";

/// A proxy port as typed: blank, or ASCII digits naming 1 to 65535, stored without
/// leading zeros. A sign, which `u16` parsing would accept, is refused.
pub fn parse_proxy_port(text: &str) -> Result<String, String> {
    let text = text.trim();
    let port = text
        .bytes()
        .all(|b| b.is_ascii_digit())
        .then(|| text.parse::<u16>().ok())
        .flatten()
        .filter(|&n| n > 0);
    if text.is_empty() {
        Ok(String::new())
    } else if let Some(n) = port {
        Ok(n.to_string())
    } else {
        Err(format!(
            "The proxy port must be blank or a whole number from 1 to 65535, not {text:?}"
        ))
    }
}

// Blank means no cap; otherwise a positive whole number of MB.
fn parse_body_cap(text: &str) -> Result<Option<u64>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    text.parse::<u64>()
        .ok()
        .filter(|&n| n > 0)
        .and_then(|n| n.checked_mul(MB))
        .map(Some)
        .ok_or_else(|| format!("The body cap must be a positive whole number of MB, not {text:?}"))
}

/// A whole number from 0 to 100.
pub fn parse_history_limit(text: &str) -> Result<u32, String> {
    let text = text.trim();
    text.parse::<u32>()
        .ok()
        .filter(|&n| n <= 100)
        .ok_or_else(|| {
            format!("The history limit must be a whole number from 0 to 100, not {text:?}")
        })
}

/// A whole number of seconds within [`SCRIPT_TIMEOUT_SECS`].
pub fn parse_script_timeout(text: &str) -> Result<u32, String> {
    let text = text.trim();
    let (low, high) = (SCRIPT_TIMEOUT_SECS.start(), SCRIPT_TIMEOUT_SECS.end());
    text.parse::<u32>()
        .ok()
        .filter(|n| SCRIPT_TIMEOUT_SECS.contains(n))
        .ok_or_else(|| {
            format!(
                "The script time limit must be whole seconds from {low} to {high}, not {text:?}"
            )
        })
}

/// A whole number of seconds within [`REQUEST_TIMEOUT_SECS`].
pub fn parse_request_timeout(text: &str) -> Result<u32, String> {
    let text = text.trim();
    let (low, high) = (REQUEST_TIMEOUT_SECS.start(), REQUEST_TIMEOUT_SECS.end());
    text.parse::<u32>()
        .ok()
        .filter(|n| REQUEST_TIMEOUT_SECS.contains(n))
        .ok_or_else(|| {
            format!("The request timeout must be whole seconds from {low} to {high}, not {text:?}")
        })
}

// The size inputs' texts: the zoom in whole percent, the editors and the response body in
// whole px. A font size shows the setting, its size at 100% zoom, so the zoom never moves it.
fn shown_sizes(settings: &Settings) -> (String, String, String) {
    let zoom = (settings.zoom * 100.0).round();
    let at_one = |offset| f32::from(zoom::response_font_size(base_mono(), offset)).round();
    (
        zoom.to_string(),
        at_one(settings.editor_font_offset).to_string(),
        at_one(settings.response_font_offset).to_string(),
    )
}

// The mono size at 100% zoom, which a font size setting is measured from.
fn base_mono() -> f32 {
    zoom::mono_font_size(1.0).into()
}

// Whole percent from 50 to 200, with or without a trailing `%`.
fn parse_zoom(text: &str) -> Result<f32, String> {
    let text = text.trim();
    let digits = text.strip_suffix('%').unwrap_or(text).trim();
    digits
        .parse::<u32>()
        .ok()
        .filter(|n| (50..=200).contains(n))
        .map(|n| n as f32 / 100.0)
        .ok_or_else(|| format!("The zoom must be a whole percent from 50 to 200, not {text:?}"))
}

// Whole px from 8 to 32, stored as the offset from the rounded mono size at 100% zoom. `what`
// names the field in the refusal.
fn parse_font_size(text: &str, what: &str) -> Result<f32, String> {
    let text = text.trim();
    text.parse::<u32>()
        .ok()
        .filter(|n| (8..=32).contains(n))
        .map(|n| n as f32 - base_mono().round())
        .ok_or_else(|| {
            format!("The {what} font size must be a whole px from 8 to 32, not {text:?}")
        })
}

// An input with a reset button beside it; the wrapper carries the id a click targets. The
// press lives on the button, outside the input, so Enter on it resets.
fn with_reset(
    input: impl IntoElement,
    reset_id: &'static str,
    on_reset: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    h_flex().gap(SPACE_1).child(input).child(
        div().id(reset_id).test_support().child(
            ui::icon_button(
                SharedString::from(format!("{reset_id}-button")),
                IconName::Undo2,
                "Reset",
                None,
            )
            .key_context(ui::DIALOG_FOOTER_CONTEXT)
            .on_click(on_reset),
        ),
    )
}

impl Focusable for PreferencesView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl PreferencesView {
    // The dialog's sections in display order, each with its rows, controls and notes.
    fn sections(&mut self, window: &Window, cx: &mut Context<Self>) -> Vec<PrefsSection> {
        let current = self.settings.read(cx).get().clone();
        let view = cx.entity();
        // Each segmented group shows its table's labels; a click maps the index back
        // through the same table, so the order on screen is the order of the choices.
        let edit = |f: fn(&mut Settings, usize)| {
            let view = view.clone();
            move |ix: &usize, _: &mut Window, cx: &mut App| {
                let ix = *ix;
                view.update(cx, |this, cx| this.edit(|s| f(s, ix), cx));
            }
        };
        let theme = ui::segmented(
            "theme",
            THEMES.map(|(_, label)| label),
            THEMES
                .iter()
                .position(|(c, _)| *c == current.theme)
                .unwrap_or_default(),
            edit(|s, ix| {
                if let Some(&(c, _)) = THEMES.get(ix) {
                    s.theme = c;
                }
            }),
        );
        // A select shows the saved name only if it is offered for the mode, else the default.
        let theme_select =
            |id: &'static str, name: &'static str, mode: ThemeMode, saved: &Option<String>| {
                let names = theme::choices(mode, cx);
                let selected = saved
                    .as_deref()
                    .and_then(|n| names.iter().position(|c| c.as_ref() == n))
                    .unwrap_or(0);
                let current = names[selected].clone();
                let view = view.clone();
                let picks = names.clone();
                ui::select(
                    id,
                    name,
                    current,
                    names,
                    selected,
                    move |ix, _, cx| {
                        // The first entry is the bundled default, which is stored as no name.
                        let name = (ix > 0).then(|| picks[ix].to_string());
                        view.update(cx, |this, cx| {
                            this.edit(
                                |s| match mode {
                                    ThemeMode::Light => s.light_theme = name,
                                    ThemeMode::Dark => s.dark_theme = name,
                                },
                                cx,
                            )
                        });
                    },
                    cx,
                )
            };
        let light_theme = theme_select(
            "light-theme",
            "Light theme",
            ThemeMode::Light,
            &current.light_theme,
        );
        let dark_theme = theme_select(
            "dark-theme",
            "Dark theme",
            ThemeMode::Dark,
            &current.dark_theme,
        );
        let orientation = ui::segmented(
            "orientation",
            ORIENTATIONS.map(|(_, label)| label),
            ORIENTATIONS
                .iter()
                .position(|(o, _)| *o == current.response_orientation)
                .unwrap_or_default(),
            edit(|s, ix| {
                if let Some(&(o, _)) = ORIENTATIONS.get(ix) {
                    s.response_orientation = o;
                }
            }),
        );
        let title_bar = ui::segmented(
            "title-bar-mode",
            TITLE_BARS.map(|(_, label)| label),
            TITLE_BARS
                .iter()
                .position(|(c, _)| *c == current.linux_title_bar)
                .unwrap_or_default(),
            edit(|s, ix| {
                if let Some(&(c, _)) = TITLE_BARS.get(ix) {
                    s.linux_title_bar = c;
                }
            }),
        );
        let updates_enabled = self.updates.as_ref().is_some_and(|u| u.read(cx).enabled());

        let mut appearance = PrefsSection::new("prefs-appearance", "Appearance")
            .row("pref-theme", "Theme", theme)
            .row("pref-light-theme", "Light theme", light_theme)
            .row("pref-dark-theme", "Dark theme", dark_theme)
            .row("pref-orientation", "Response layout", orientation);
        // Only Linux lets the app choose who draws the title bar.
        if cfg!(target_os = "linux") {
            appearance = appearance.row("pref-title-bar", "Title bar", title_bar);
        }
        let appearance = appearance
            .row(
                "pref-zoom",
                "Zoom (%)",
                self.with_error(
                    Field::Zoom,
                    with_reset(
                        self.framed(Field::Zoom, window, cx),
                        "zoom-reset",
                        cx.listener(|this, _, _, cx| {
                            this.reset(Field::Zoom, 1.0, |s, v| s.zoom = v, cx)
                        }),
                    ),
                    cx,
                ),
            )
            .row(
                "pref-editor-font",
                "Editor font size",
                self.with_error(
                    Field::EditorFont,
                    with_reset(
                        self.framed(Field::EditorFont, window, cx),
                        "editor-font-reset",
                        cx.listener(|this, _, _, cx| {
                            this.reset(Field::EditorFont, 0.0, |s, v| s.editor_font_offset = v, cx)
                        }),
                    ),
                    cx,
                ),
            )
            .row(
                "pref-response-font",
                "Response font size",
                self.with_error(
                    Field::ResponseFont,
                    with_reset(
                        self.framed(Field::ResponseFont, window, cx),
                        "response-font-reset",
                        cx.listener(|this, _, _, cx| {
                            let field = Field::ResponseFont;
                            this.reset(field, 0.0, |s, v| s.response_font_offset = v, cx)
                        }),
                    ),
                    cx,
                ),
            )
            .row(
                "pref-wrap-editors",
                "Wrap long lines in editors",
                ui::checkbox("wrap-editors", current.wrap_editors).on_click(cx.listener(
                    |this, on: &bool, _, cx| {
                        let on = *on;
                        this.edit(|s| s.wrap_editors = on, cx)
                    },
                )),
            );
        let run_scripts = ui::segmented(
            "run-scripts",
            RUN_SCRIPTS.map(|(_, label)| label),
            RUN_SCRIPTS
                .iter()
                .position(|(c, _)| *c == current.run_scripts)
                .unwrap_or_default(),
            edit(|s, ix| {
                if let Some(&(c, _)) = RUN_SCRIPTS.get(ix) {
                    s.run_scripts = c;
                }
            }),
        );
        let http_version = ui::segmented(
            "http-version",
            HTTP_VERSIONS.map(|(_, label)| label),
            HTTP_VERSIONS
                .iter()
                .position(|(v, _)| *v == current.http_version)
                .unwrap_or_default(),
            edit(|s, ix| {
                if let Some(&(v, _)) = HTTP_VERSIONS.get(ix) {
                    s.http_version = v;
                }
            }),
        );
        // The short labels need the hint to say what each choice does; like the PAC hint it
        // sits in the control's column rather than under the section.
        // Start-aligned, so the picker keeps its own width while the hint spans the column.
        let http_version = v_flex()
            .flex_1()
            .min_w_0()
            .items_start()
            .gap(SPACE_1)
            .child(http_version)
            .child(
                div()
                    .w_full()
                    .whitespace_normal()
                    .id("prefs-http-version-hint")
                    .test_support()
                    .text_size(TEXT_SMALL)
                    .text_color(cx.theme().muted_foreground)
                    .child(HTTP_VERSION_HINT),
            );
        let network = PrefsSection::new("prefs-network", "Network")
            .row(
                "pref-verify-tls",
                "Verify TLS certificates",
                ui::checkbox("verify-tls", !current.insecure).on_click(cx.listener(
                    |this, on: &bool, _, cx| {
                        let insecure = !*on;
                        this.edit(|s| s.insecure = insecure, cx)
                    },
                )),
            )
            .row(
                "pref-body-cap",
                "Body cap (MB, blank for none)",
                self.field(Field::BodyCap, window, cx),
            )
            .row_with_terms(
                "pref-request-timeout",
                "Request timeout (seconds, 0 means no limit)",
                ["request", "timeout", "limit"],
                self.field(Field::RequestTimeout, window, cx),
            )
            .row("pref-http-version", "HTTP version", http_version);
        let network = self
            .proxy_rows(network, &current.proxy, window, cx)
            .row(
                "pref-ca-file",
                "CA certificate file (blank for none)",
                self.with_error(
                    Field::CaFile,
                    h_flex()
                        .gap(SPACE_1)
                        .child(self.framed(Field::CaFile, window, cx))
                        .child(
                            Button::new("ca-file-choose")
                                .label("Choose")
                                .key_context(ui::DIALOG_FOOTER_CONTEXT)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.choose_ca_file(window, cx)
                                })),
                        ),
                    cx,
                ),
            )
            .row(
                "pref-keep-default-cas",
                "Also trust the default certificate authorities",
                ui::checkbox("keep-default-cas", current.keep_default_cas).on_click(cx.listener(
                    |this, on: &bool, _, cx| {
                        let on = *on;
                        this.edit(|s| s.keep_default_cas = on, cx)
                    },
                )),
            )
            // Unlike the settings above, the cookie switches reach the live executor at once.
            .row(
                "pref-store-cookies",
                "Store cookies automatically",
                ui::checkbox("store-cookies", current.store_cookies).on_click(cx.listener(
                    |this, on: &bool, _, cx| {
                        let on = *on;
                        this.edit(|s| s.store_cookies = on, cx);
                        if let Some(exec) = crate::executor::shared(cx) {
                            exec.set_store_cookies(on);
                        }
                    },
                )),
            )
            .row(
                "pref-send-cookies",
                "Send cookies automatically",
                ui::checkbox("send-cookies", current.send_cookies).on_click(cx.listener(
                    |this, on: &bool, _, cx| {
                        let on = *on;
                        this.edit(|s| s.send_cookies = on, cx);
                        if let Some(exec) = crate::executor::shared(cx) {
                            exec.set_send_cookies(on);
                        }
                    },
                )),
            )
            .note(ui::form_note(
                "prefs-restart-note",
                "Network settings, such as TLS checks, the proxy and certificates, apply the next \
                 time the app starts.",
            ))
            .note(ui::form_note(
                "prefs-cookies-note",
                "Cookie settings apply at once.",
            ))
            .note(ui::form_note("prefs-proxy-note", PROXY_NOTE));
        let scripts = PrefsSection::new("prefs-scripts", "Scripts")
            .row("pref-run-scripts", "Run scripts", run_scripts)
            .row_with_terms(
                "pref-script-timeout",
                "Script time limit (seconds)",
                ["script", "timeout", "limit"],
                self.field(Field::ScriptTimeout, window, cx),
            )
            .note(ui::form_note(
                "prefs-scripts-note",
                "Script settings apply at once.",
            ));
        // Disabled controls swallow the click, so an unconfigured build never checks and
        // the stored preference survives until a build can honour it. It is drawn unticked
        // meanwhile, so the box never claims a check that cannot happen.
        let mut updates = PrefsSection::new("prefs-updates", "Updates").row(
            "pref-auto-check",
            "Check automatically",
            ui::checkbox(
                "check-updates",
                updates_enabled && current.check_for_updates,
            )
            .disabled(!updates_enabled)
            .on_click(cx.listener(|this, on: &bool, _, cx| {
                let on = *on;
                this.edit(|s| s.check_for_updates = on, cx)
            })),
        );
        // The note explains the disabled checkbox, so it sits right under that row.
        if !updates_enabled {
            updates = updates.row_note(ui::form_note(
                "updates-disabled",
                "Update checks are not configured for this build",
            ));
        }
        updates = updates.row(
            "pref-check-now",
            "Manual check",
            Button::new("check-updates-now")
                .key_context(ui::DIALOG_FOOTER_CONTEXT)
                .label("Check now")
                .lock(!updates_enabled, cx)
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(updates) = &this.updates {
                        updates.update(cx, |u, cx| u.check_now(cx));
                    }
                })),
        );
        // Pressing Clear is the ask, so there is no confirm dialog.
        let history = PrefsSection::new("prefs-history", "History")
            .row(
                "pref-remember-responses",
                "Remember last responses",
                ui::checkbox("remember-responses", current.remember_responses).on_click(
                    cx.listener(|this, on: &bool, _, cx| {
                        let on = *on;
                        this.edit(|s| s.remember_responses = on, cx)
                    }),
                ),
            )
            .row(
                "pref-history-limit",
                "Responses kept per request (0 keeps the latest only)",
                self.field(Field::HistoryLimit, window, cx),
            )
            .row(
                "pref-clear-responses",
                "Saved responses",
                Button::new("clear-responses")
                    .key_context(ui::DIALOG_FOOTER_CONTEXT)
                    .label("Clear saved responses")
                    .on_click(cx.listener(|this, _, window, cx| {
                        const CLEARED: &str = "Saved responses cleared";
                        crate::last_response::clear_all(cx);
                        window.push_notification(ui::info_toast(CLEARED), cx);
                        this.status = Some(CLEARED.into());
                        cx.notify();
                    })),
            );
        let general = PrefsSection::new("prefs-general", "General").row(
            "pref-autosave",
            "Save changes automatically",
            ui::checkbox("autosave", current.autosave).on_click(cx.listener(
                |this, on: &bool, _, cx| {
                    let on = *on;
                    this.edit(|s| s.autosave = on, cx)
                },
            )),
        );
        let keyboard = self.keyboard.update(cx, |k, cx| k.section(cx));
        vec![
            general, appearance, network, scripts, updates, history, keyboard,
        ]
    }

    /// The shown section at the top of the rows' viewport, as of the last frame.
    fn current_section(&self) -> Option<SharedString> {
        self.shown_sections.get(self.scroll.top_item()).cloned()
    }

    /// Whether rows sit below the visible ones, with 1 px of tolerance for rounding at the end.
    fn more_below(&self) -> bool {
        let (offset, max) = (-self.scroll.offset().y, self.scroll.max_offset().y);
        max > px(1.) && offset < max - px(1.)
    }
}

impl Render for PreferencesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).value().to_string();
        // One pass yields both the rows and the list: each section's id and title, and its
        // element when the query leaves any of its rows.
        let sections: Vec<_> = self
            .sections(window, cx)
            .into_iter()
            .map(|section| {
                let (id, title) = (section.id().clone(), section.title().clone());
                (id, title, section.filtered(&query))
            })
            .collect();
        self.shown_sections = sections
            .iter()
            .filter(|(_, _, el)| el.is_some())
            .map(|(id, _, _)| id.clone())
            .collect();
        let current = self.current_section();
        // Only shown sections get an entry, which jumps to the section's index among them:
        // the index the scroll column's children carry. The kit button exposes no selected
        // state, so a wrapper carries the entry's id and its selection. The entries are a
        // pointer shortcut and stay out of the Tab order, which starts at the first row.
        let mut entries = Vec::new();
        let mut shown = Vec::new();
        for (ix, (id, title, el)) in sections
            .into_iter()
            .filter_map(|(id, title, el)| Some((id, title, el?)))
            .enumerate()
        {
            let selected = current.as_ref() == Some(&id);
            let name = id.trim_start_matches("prefs-");
            shown.push(el);
            entries.push(
                div()
                    .id(SharedString::from(format!("prefs-nav-{name}")))
                    .test_support()
                    .role(Role::ListItem)
                    .aria_selected(selected)
                    .child(
                        Button::new(("prefs-nav-entry", ix))
                            .ghost()
                            .w_full()
                            .justify_start()
                            .label(title)
                            .choose(selected, SelectedStyle::Ghost, cx)
                            .tab_stop(false)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.scroll.scroll_to_top_of_item(ix);
                                cx.notify();
                            })),
                    ),
            );
        }
        // A window too narrow for the whole dialog leaves the room to the rows: the list hides,
        // as the search still reaches every section.
        let margin = cx.theme().spacing_tokens().lg;
        let with_nav =
            window.viewport_size().width - margin * 2. >= resolve(PREFS_DIALOG_WIDTH, window);
        // The list takes the body's height and scrolls on its own in a window too short for it.
        let nav = v_flex()
            .id("prefs-nav")
            .test_support()
            .role(Role::List)
            .flex_none()
            .w(PREFS_NAV_WIDTH)
            .h_full()
            .overflow_y_scroll()
            .pt(SPACE_3)
            .pr(SPACE_4)
            .gap(SPACE_1)
            .children(entries);
        // The root fills the dialog body, so its box keeps the body's height however few rows
        // the search leaves; the column inside scrolls with the kit's scrollbar.
        // A short room gives the rows the column's top padding and the empty status line, so a
        // heading clears the fade.
        let short = dialog_frame::window_room(window) < resolve(PREFS_SHORT_ROOM, window);
        let column = v_flex()
            .gap(SPACE_4)
            .when(!short, |el| el.pt(SPACE_3))
            .when(shown.is_empty(), |el| {
                el.child(
                    div()
                        .id("prefs-no-match")
                        .test_support()
                        .child("No settings match"),
                )
            })
            .children(shown)
            .when_some(self.error.clone(), |el, error| {
                el.child(div().id("prefs-error").test_support().child(error))
            });
        // The status keeps its line even while empty, so a status appearing never shrinks the
        // scroll area and hides the row that was just clicked; a short room gives it to the rows.
        let status = div()
            .flex_none()
            .pt(SPACE_1)
            .text_size(TEXT_SMALL)
            .text_color(cx.theme().muted_foreground);
        let status = match self.status.clone() {
            Some(text) => status
                .id("prefs-status")
                .test_support()
                .child(text)
                .into_any_element(),
            None if short => div().into_any_element(),
            None => status.child("\u{a0}").into_any_element(),
        };
        // The fade never covers more than a quarter of the last frame's viewport.
        let fade_height =
            resolve(SCROLL_FADE_HEIGHT, window).min(self.scroll.bounds().size.height / 4.);
        // The handle reports the last frame's extent and position, so a search or a jump is
        // seen one frame late, as is a resize or zoom: recheck after this frame and redraw only
        // if an answer changed. The check holds the view weakly so a closed dialog is not kept
        // alive by it.
        let more_below = self.more_below();
        let viewport = self.scroll.bounds().size.height;
        let view = cx.entity().downgrade();
        window.on_next_frame(move |_, cx| {
            let _ = view.update(cx, |this, cx| {
                if this.more_below() != more_below
                    || this.current_section() != current
                    || this.scroll.bounds().size.height != viewport
                {
                    cx.notify();
                }
            });
        });
        let background = cx.theme().background;
        with_keys(v_flex().id("preferences").test_support(), &cx.entity())
            .track_focus(&self.focus)
            .flex_1()
            .min_h_0()
            .text_size(TEXT_UI)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .items_start()
                    .when(with_nav, |el| el.child(nav))
                    .child(
                        div()
                            .id("preferences-rows")
                            .test_support()
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .child(
                                column
                                    .id("preferences-scroll")
                                    .size_full()
                                    .pr(SCROLLBAR_GUTTER)
                                    .overflow_y_scroll()
                                    .track_scroll(&self.scroll),
                            )
                            .when(more_below, |el| {
                                el.child(
                                    div()
                                        .id("prefs-scroll-fade")
                                        .test_support()
                                        .absolute()
                                        .left_0()
                                        .right(SCROLLBAR_GUTTER)
                                        .bottom_0()
                                        .h(fade_height)
                                        .bg(linear_gradient(
                                            180.,
                                            linear_color_stop(background.opacity(0.), 0.),
                                            linear_color_stop(background, 1.),
                                        )),
                                )
                            })
                            .child(
                                div().absolute().inset_0().child(
                                    Scrollbar::vertical(&self.scroll)
                                        .mode(ScrollbarMode::Always)
                                        .viewport_from_layout(),
                                ),
                            ),
                    ),
            )
            .child(status)
    }
}

/// Puts the Preferences key context and its key actions on `el`, dispatching to `view`. Both
/// the body and the title row carry it, since the search box sits outside the body.
fn with_keys<E: InteractiveElement>(el: E, view: &Entity<PreferencesView>) -> E {
    // Body zoom is unreachable from the dialog, so the zoom keys always act on the app.
    let app = ZoomTarget::App;
    let (focus, clear) = (view.clone(), view.clone());
    el.key_context(KEY_CONTEXT)
        .on_action(move |_: &ZoomIn, _, cx| zoom::apply_step(app, ZoomStep::In, cx))
        .on_action(move |_: &ZoomOut, _, cx| zoom::apply_step(app, ZoomStep::Out, cx))
        .on_action(move |_: &ResetZoom, _, cx| zoom::apply_step(app, ZoomStep::Reset, cx))
        .on_action(move |_: &FocusPreferencesSearch, window, cx| {
            let search = focus.read(cx).search.clone();
            search.read(cx).focus_handle(cx).focus(window, cx);
        })
        // An empty search lets Escape fall through to the kit's Dialog binding, which closes.
        .on_action(move |_: &ClearPreferencesSearch, window, cx| {
            let search = clear.read(cx).search.clone();
            if search.read(cx).value().is_empty() {
                cx.propagate();
                return;
            }
            search.update(cx, |s, cx| s.set_value("", window, cx));
            clear.update(cx, |_, cx| cx.notify());
        })
}

/// Opens the "Preferences" dialog over `window` and returns its body; does nothing and
/// returns `None` before `app::start` has set up the settings.
pub fn open(window: &mut Window, cx: &mut App) -> Option<Entity<PreferencesView>> {
    let state = cx.try_global::<AppState>()?;
    let settings = state.settings.clone();
    let view = cx.new(|cx| PreferencesView::new(settings, window, cx));
    let dialog_view = view.clone();
    let search = view.read(cx).search.clone();
    // Enter commits the focused input; the dialog's own Enter confirm must not close it.
    window.open_dialog(cx, move |d, window, cx| {
        // The kit's title is body-sized, the same as a section heading, with a 1.25 relative
        // line height; ours outranks it in size and in line box.
        let title = div()
            .id("prefs-title")
            .test_support()
            .text_size(TEXT_TITLE)
            .line_height(relative(1.5))
            .font_weight(FontWeight::SEMIBOLD)
            .child("Preferences");
        // The search names its focus chord inside the box, as the sidebar filter does, unless
        // the row is too narrow for both the hint and the placeholder.
        let (narrow, watch) =
            ui::narrower_than("prefs-title-row".into(), PREFS_SEARCH_HINT_ROOM, window, cx);
        let search_hint = (!narrow)
            .then(|| Kbd::binding_for_action(&FocusPreferencesSearch, Some(KEY_CONTEXT), window))
            .flatten();
        // The absolute close button sits over the title row's right end; the padding keeps
        // the search box clear of it.
        let title_row = with_keys(h_flex(), &dialog_view)
            .relative()
            .w_full()
            .justify_between()
            .gap(SPACE_4)
            .pr(SPACE_5)
            .child(title)
            .child(ui::padded_input(
                Input::new(&search)
                    .id("prefs-search")
                    .w(FORM_INPUT_WIDTH)
                    .bg(ui::field_fill(false, cx))
                    .when_some(search_hint, |input, kbd| {
                        input.suffix(div().id("prefs-search-kbd").test_support().child(kbd))
                    }),
                &search,
            ))
            .child(watch);
        // A fixed height the search cannot shrink, cut to the room above the status bar.
        // Below 100% the kit's px chrome does not shrink, so it is added back to the zoomed
        // height, never past the 100% height.
        let room = dialog_frame::window_room(window);
        let (zoomed, full) = (
            resolve(PREFS_DIALOG_HEIGHT, window),
            at_zoom(PREFS_DIALOG_HEIGHT, 1.0),
        );
        let height = if zoomed >= full {
            zoomed
        } else {
            (zoomed + PREFS_DIALOG_CHROME).min(full)
        };
        ui::with_close(dialog_frame::place(d, window), title_row)
            .width(resolve(PREFS_DIALOG_WIDTH, window))
            .h(height.min(room))
            // As content rather than a child, the view fills the body instead of sitting in
            // the kit's scroll area at its natural height.
            .content({
                let view = dialog_view.clone();
                move |c, _, _| c.min_h_0().child(view.clone())
            })
            .on_ok(|_, _, _| false)
            // Escape, the close button and the backdrop all end here: what was typed but not
            // confirmed is committed, and each refused field is named in a warning.
            .on_close({
                let view = dialog_view.clone();
                move |_, window, cx| {
                    view.update(cx, |v, cx| v.commit_all(window, cx));
                    for warning in view.read(cx).unsaved() {
                        window.push_notification(Notification::warning(warning), cx);
                    }
                }
            })
    });
    // `open_dialog` focused the kit's host above our view; focus the view so its context
    // is on the dispatch path from the start.
    window.focus(&view.focus_handle(cx), cx);
    Some(view)
}
