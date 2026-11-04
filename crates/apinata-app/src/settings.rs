//! The user's settings, stored in `settings.json`.

use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::time::Duration;

use apinata_core::VarContext;
use gpui_kit::App;
use serde::{Deserialize, Serialize};

use crate::app::AppState;

/// User settings. Missing fields take their defaults and unknown fields are ignored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The colour theme.
    pub theme: ThemeChoice,
    /// How the response panel sits relative to the request editor.
    pub response_orientation: Orientation,
    /// The response body cap in bytes; `None` for no cap.
    pub max_body_bytes: Option<u64>,
    /// Skip TLS certificate verification.
    pub insecure: bool,
    /// The app zoom factor, 1.0 for 100%; see `zoom::step_zoom` for its steps and range.
    pub zoom: f32,
    /// Pixels added to the zoomed monospace size for the response body.
    pub response_font_offset: f32,
    /// Pixels added to the zoomed monospace size for the code editors.
    pub editor_font_offset: f32,
    /// The theme used in light mode, by name; `None` is Apinata Light.
    pub light_theme: Option<String>,
    /// The theme used in dark mode, by name; `None` is Apinata Dark.
    pub dark_theme: Option<String>,
    /// Check for a newer release at most once a day.
    pub check_for_updates: bool,
    /// Who draws the window's title bar on Linux.
    pub linux_title_bar: LinuxTitleBar,
    /// Keep each request's last response in the data dir and show it when the request opens.
    pub remember_responses: bool,
    /// How many sends each request keeps; 0 keeps only the latest.
    pub history_limit: u32,
    /// Save edited files automatically, a moment after the last edit.
    pub autosave: bool,
    /// Wrap long lines in the body and script editors; each editor can still toggle it.
    pub wrap_editors: bool,
    /// The global proxy; its password lives in the secret store, never here.
    pub proxy: ProxyPrefs,
    /// A PEM file of extra CA certificates to trust.
    pub ca_file: Option<PathBuf>,
    /// Whether the default CA roots stay trusted alongside `ca_file`.
    pub keep_default_cas: bool,
    /// Whether sends store the cookies responses set in the shared jar.
    pub store_cookies: bool,
    /// Whether sends attach the jar's matching cookies to requests.
    pub send_cookies: bool,
    /// Whether collections with scripts are asked about or never run them.
    pub run_scripts: RunScripts,
    /// How long one script may run, in whole seconds within [`SCRIPT_TIMEOUT_SECS`].
    pub script_timeout_secs: u32,
    /// The timeout, in whole seconds within [`REQUEST_TIMEOUT_SECS`], of requests that
    /// inherit theirs; 0 is no limit.
    pub request_timeout_secs: u32,
    /// The HTTP version sends use; applies at the next start.
    pub http_version: HttpVersionPref,
    /// Ids of the one-off tips the user chose never to see again.
    pub dismissed_tips: Vec<String>,
    /// The folder the last clone went into.
    pub clone_parent: Option<PathBuf>,
}

/// The script time limits, in whole seconds, that Preferences accepts and a send applies.
pub const SCRIPT_TIMEOUT_SECS: RangeInclusive<u32> = 1..=3600;

/// The request timeouts, in whole seconds, that Preferences accepts; 0 is no limit.
pub const REQUEST_TIMEOUT_SECS: RangeInclusive<u32> = 0..=86400;

impl Settings {
    /// The default for requests that inherit their timeout; 0 is `None`, no limit. A
    /// hand-edited value past [`REQUEST_TIMEOUT_SECS`] is held to its end.
    pub fn request_timeout(&self) -> Option<Duration> {
        let secs = self.request_timeout_secs.min(*REQUEST_TIMEOUT_SECS.end());
        (secs > 0).then(|| Duration::from_secs(secs.into()))
    }
}

/// The preference from `AppState`, or `None` when the app state is absent (tests).
pub fn request_timeout(cx: &App) -> Option<Duration> {
    let state = cx.try_global::<AppState>()?;
    state.settings.read(cx).get().request_timeout()
}

/// The Preferences default for wrapping long lines in editors; on when the app state is
/// absent (tests).
pub fn wrap_editors(cx: &App) -> bool {
    cx.try_global::<AppState>()
        .is_none_or(|state| state.settings.read(cx).get().wrap_editors)
}

/// `context` with the Preferences timeout as the default for requests that inherit theirs.
/// Send, the runner and Get token each call this after planning, so all read the same value.
pub fn with_request_timeout(context: &VarContext, cx: &App) -> VarContext {
    context.with_default_timeout(request_timeout(cx))
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: ThemeChoice::default(),
            response_orientation: Orientation::default(),
            // Bruno's 10 MB large-response threshold is 10 * 1024 * 1024 bytes.
            max_body_bytes: Some(10 * 1024 * 1024),
            insecure: false,
            zoom: 1.0,
            response_font_offset: 0.0,
            editor_font_offset: 0.0,
            light_theme: None,
            dark_theme: None,
            check_for_updates: true,
            linux_title_bar: LinuxTitleBar::default(),
            remember_responses: true,
            history_limit: 20,
            autosave: false,
            wrap_editors: true,
            proxy: ProxyPrefs::default(),
            ca_file: None,
            keep_default_cas: true,
            store_cookies: true,
            send_cookies: true,
            run_scripts: RunScripts::default(),
            script_timeout_secs: 30,
            request_timeout_secs: 0,
            http_version: HttpVersionPref::default(),
            dismissed_tips: Vec::new(),
            clone_parent: None,
        }
    }
}

/// The colour theme choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeChoice {
    /// Always light.
    Light,
    /// Always dark.
    Dark,
    /// Follow the operating system.
    #[default]
    System,
}

/// Whether the app runs collection scripts at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunScripts {
    /// Ask once per collection before its scripts first run.
    #[default]
    Ask,
    /// Never run scripts and never ask.
    Never,
}

/// Which HTTP versions sends may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HttpVersionPref {
    /// HTTP/1.1 only, never offering HTTP/2.
    #[default]
    Http1Only,
    /// Offer HTTP/2 over TLS and fall back to HTTP/1.1.
    Auto,
    /// Speak HTTP/2 from the first byte, also over plain HTTP.
    Http2PriorKnowledge,
}

/// Where the response panel sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    /// Response below the request.
    #[default]
    Vertical,
    /// Response beside the request.
    Horizontal,
}

/// Who draws the window's title bar and controls on Linux.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinuxTitleBar {
    /// The app draws an integrated title bar with its own window controls.
    #[default]
    Client,
    /// The compositor draws its own title bar.
    System,
}

/// Which proxy requests use when their collection inherits the global one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyMode {
    /// Connect directly.
    Off,
    /// The operating system's proxy.
    #[default]
    System,
    /// The proxy in `ProxyPrefs`.
    Manual,
    /// A PAC file.
    Pac,
}

/// The global proxy as the user typed it; checked only when the executor is built.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProxyPrefs {
    /// Off, system, manual or PAC.
    pub mode: ProxyMode,
    /// `http`, `https`, `socks4` or `socks5`.
    pub protocol: String,
    /// The proxy host.
    pub host: String,
    /// The proxy port, blank or 1 to 65535.
    pub port: String,
    /// The proxy user; blank for no credentials.
    pub username: String,
    /// Comma separated hosts that connect directly.
    pub bypass: String,
    /// The PAC file URL or absolute path.
    pub pac_source: String,
}

impl Default for ProxyPrefs {
    fn default() -> Self {
        ProxyPrefs {
            mode: ProxyMode::default(),
            protocol: "http".into(),
            host: String::new(),
            port: String::new(),
            username: String::new(),
            bypass: String::new(),
            pac_source: String::new(),
        }
    }
}
