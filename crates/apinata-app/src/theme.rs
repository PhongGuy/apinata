//! Maps the user's settings onto gpui-kit's global `Theme`: the light or dark mode, and the
//! zoomed UI and code editor font sizes.

use std::borrow::Cow;
use std::path::Path;
use std::rc::Rc;

use gpui_kit::component::{
    ActiveTheme, Colorize, Theme, ThemeConfig, ThemeMode, ThemeRegistry, ThemeSet,
    try_parse_background, try_parse_color,
};
use gpui_kit::{Anchor, App, Entity, Hsla, Pixels, SharedString, Window, WindowAppearance};

use crate::persisted::Persisted;
use crate::settings::{Settings, ThemeChoice};
use crate::style::{SPACE_3, STATUS_BAR_HEIGHT, at_zoom};
use crate::zoom;

/// The shipped theme set: one dark and one light theme setting every kit colour key.
pub const THEME_JSON: &str = include_str!("../assets/theme/apinata.json");
/// The name of the shipped dark theme.
pub const DARK: &str = "Apinata Dark";
/// The name of the shipped light theme.
pub const LIGHT: &str = "Apinata Light";
/// The high-contrast theme set: a light and a dark theme whose text reaches 7:1.
pub const HIGH_CONTRAST_JSON: &str = include_str!("../assets/theme/apinata-high-contrast.json");
/// The name of the high-contrast light theme.
pub const HC_LIGHT: &str = "Apinata High Contrast Light";
/// The name of the high-contrast dark theme.
pub const HC_DARK: &str = "Apinata High Contrast Dark";

/// The alpha disabled input text is painted at in the theme called `name`: the kit's 0.5, or
/// full strength in the high-contrast themes, where a locked cell is drawn read-only instead.
pub fn disabled_text_alpha(name: &str) -> f32 {
    if name == HC_LIGHT || name == HC_DARK {
        1.0
    } else {
        0.5
    }
}
/// The bundled TrueType fonts: Inter Regular, Medium, SemiBold and Italic, then JetBrains
/// Mono Regular and Bold.
pub const FONTS: [&[u8]; 6] = [
    include_bytes!("../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../assets/fonts/Inter-Italic.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf"),
];

/// Registers the bundled fonts and themes, makes the themes the kit's light and dark themes
/// and reloads the current mode. Safe to call twice. Fonts or a theme that fail to load leave
/// the system fonts or the kit default and log once to stderr.
pub fn install(cx: &mut App) {
    crate::focus_mode::install(cx);
    // Fonts go first so the theme reload below already resolves "Inter" on the first frame.
    let fonts = FONTS.iter().map(|f| Cow::Borrowed(*f)).collect();
    if let Err(err) = cx.text_system().add_fonts(fonts) {
        eprintln!("apinata: bundled fonts failed to register ({err}); using system fonts");
    }
    for json in [THEME_JSON, HIGH_CONTRAST_JSON] {
        if let Err(err) = ThemeRegistry::global_mut(cx).load_themes_from_str(json) {
            eprintln!("apinata: a bundled theme failed to load: {err}");
            return;
        }
    }
    // The registry keeps the first theme loaded under a name, so a second call finds the
    // same configs and the reassignment below is idempotent.
    let themes = ThemeRegistry::global(cx).themes();
    let (dark, light) = (themes.get(DARK).cloned(), themes.get(LIGHT).cloned());
    Theme::update(cx, |t| {
        if let Some(dark) = dark {
            t.dark_theme = dark;
        }
        if let Some(light) = light {
            t.light_theme = light;
        }
        // Bottom right clears the tab strip, whose overflow button the kit default covers.
        t.notification.placement = Anchor::BottomRight;
        t.notification.margins.bottom = toast_bottom(1.0);
    });
    // `change` reloads the current mode's config even when the mode is unchanged.
    set_mode(Theme::global(cx).mode, None, cx);
}

/// Registers every `*.json` theme set in `dir`, in file-name order, and returns one warning
/// per skipped file. The warnings are also written to the Console log. A missing `dir` is
/// normal and is neither created nor reported. A file is skipped whole when it cannot be
/// read, is not a theme set, or sets a colour the kit would silently drop; the registry
/// keeps the first theme under a name, so a bundled name is never replaced.
pub fn load_user_themes(dir: &Path, cx: &mut App) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<_> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json") && p.is_file())
        .collect();
    files.sort();
    let mut warnings = Vec::new();
    for path in files {
        let file = path.file_name().unwrap_or_default().to_string_lossy();
        let loaded = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| validated(&text).map(|()| text))
            .and_then(|text| {
                let registry = ThemeRegistry::global_mut(cx);
                registry
                    .load_themes_from_str(&text)
                    .map_err(|e| e.to_string())
            });
        if let Err(why) = loaded {
            warnings.push(format!("Skipped theme file {file}: {why}"));
        }
    }
    // Warnings go to the app-wide log, which `start` creates before the window opens.
    let log = crate::log_feed::ensure(cx);
    for w in &warnings {
        log.note("Themes", apinata_core::ConsoleLevel::Warn, w);
    }
    warnings
}

/// Parses `text` as a theme set and checks that every colour string is one the kit parses;
/// the kit itself falls back to a default for a bad value without saying so.
fn validated(text: &str) -> Result<(), String> {
    let set: ThemeSet = serde_json::from_str(text).map_err(|e| e.to_string())?;
    for theme in &set.themes {
        let colors = serde_json::to_value(&theme.colors).map_err(|e| e.to_string())?;
        let bad = colors
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(key, v)| Some((key, v.as_str()?)))
            .find(|(_, v)| try_parse_color(v).is_err() && try_parse_background(v).is_err());
        if let Some((key, v)) = bad {
            return Err(format!("theme \"{}\" sets {key} to \"{v}\"", theme.name));
        }
    }
    Ok(())
}

/// The toasts' bottom margin at `zoom`: clear of the status bar, whose height follows the zoom.
fn toast_bottom(zoom: f32) -> Pixels {
    at_zoom(STATUS_BAR_HEIGHT, zoom) + at_zoom(SPACE_3, zoom)
}

/// Switches the kit to `mode`, reloads its theme, restores the opaque selected-row fill and
/// reapplies the focus modality's ring. Every mode change in the app goes through here.
pub fn set_mode(mode: ThemeMode, window: Option<&mut Window>, cx: &mut App) {
    Theme::change(mode, window, cx);
    let modality = crate::focus_mode::modality(cx);
    // The kit clamps `list.active.background` to 20% alpha on load, which hides the fill.
    Theme::update(cx, |t| {
        crate::focus_mode::apply_ring(t, modality);
        let config = if t.mode.is_dark() {
            &t.dark_theme
        } else {
            &t.light_theme
        };
        let opaque = config
            .colors
            .list_active
            .as_deref()
            .and_then(|hex| Hsla::parse_hex(hex).ok());
        if let Some(fill) = opaque {
            t.list_active = fill;
            t.tokens.list_active = fill.into();
        }
    });
}

/// A semantic status with a kit colour key (`success`, `info`, `warning`, `danger`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// A completed or passing outcome.
    Success,
    /// A neutral notice.
    Info,
    /// A recoverable problem worth attention.
    Warning,
    /// A failure or destructive outcome.
    Danger,
}

/// The tinted fill behind a status colour (pill, banner): not a kit key.
pub fn status_fill_hex(status: Status, mode: ThemeMode) -> &'static str {
    match (mode, status) {
        (ThemeMode::Dark, Status::Success) => "#173322",
        (ThemeMode::Dark, Status::Info) => "#14283D",
        (ThemeMode::Dark, Status::Warning) => "#3A2F0E",
        (ThemeMode::Dark, Status::Danger) => "#3D1A1E",
        (ThemeMode::Light, Status::Success) => "#E3F4E6",
        (ThemeMode::Light, Status::Info) => "#E1EEFA",
        (ThemeMode::Light, Status::Warning) => "#FBF0D2",
        (ThemeMode::Light, Status::Danger) => "#FDEDED",
    }
}

/// `status_fill_hex` for the current theme mode, parsed.
pub fn status_fill(status: Status, cx: &App) -> Hsla {
    // Every fill is a fixed six-digit hex literal, so the muted fallback is unreachable.
    Hsla::parse_hex(status_fill_hex(status, cx.theme().mode)).unwrap_or(cx.theme().muted)
}

/// The fill behind an added or removed diff row: a quiet wash a little stronger than the pill
/// and banner tints, with row text at 4.5:1 or better; other statuses keep `status_fill_hex`.
pub fn diff_fill_hex(status: Status, mode: ThemeMode) -> &'static str {
    match (mode, status) {
        (ThemeMode::Dark, Status::Success) => "#0D3517",
        (ThemeMode::Dark, Status::Danger) => "#521418",
        (ThemeMode::Light, Status::Success) => "#B8E2C2",
        (ThemeMode::Light, Status::Danger) => "#F5CDD0",
        _ => status_fill_hex(status, mode),
    }
}

/// The +/- mark colour on a diff wash: the status token when it reads at 4.5:1 there, else
/// that token stepped toward black (light mode) or white (dark mode) until it does.
pub fn diff_glyph_color(token: Hsla, wash: Hsla, mode: ThemeMode) -> Hsla {
    let end = if mode.is_dark() {
        Hsla::white()
    } else {
        Hsla::black()
    };
    let mut glyph = token;
    for step in 1..=20 {
        if crate::ui::luminance_ratio(glyph, wash) >= 4.5 {
            break;
        }
        glyph = mix_toward(token, end, step as f32 * 0.05);
    }
    glyph
}

// Linear blend in RGB, keeping the hue of `from` close for small `t`.
fn mix_toward(from: Hsla, to: Hsla, t: f32) -> Hsla {
    let (a, b) = (from.to_rgb(), to.to_rgb());
    let lerp = |x: f32, y: f32| x + (y - x) * t;
    gpui_kit::Rgba {
        r: lerp(a.r, b.r),
        g: lerp(a.g, b.g),
        b: lerp(a.b, b.b),
        a: from.a,
    }
    .into()
}

/// The colour a diff mark starts from before `diff_glyph_color` adjusts it. High Contrast Dark's
/// danger token is a pale pink that must clear 7:1 as text, so a removed mark uses a clearer red
/// that still reads at 4.5:1 on the dark wash.
pub fn diff_token(status: Status, cx: &App) -> Hsla {
    let theme = cx.theme();
    match status {
        Status::Danger if theme.theme_name() == HC_DARK => HC_DARK_REMOVED_MARK,
        Status::Danger => theme.danger,
        Status::Success => theme.success,
        Status::Warning => theme.warning,
        Status::Info => theme.info,
    }
}

// #FF5A5A: 4.7:1 on the dark removed wash (#521418).
const HC_DARK_REMOVED_MARK: Hsla = Hsla {
    h: 0.0,
    s: 1.0,
    l: 0.676,
    a: 1.0,
};

/// The +/- mark colour for an added (`Success`), removed (`Danger`) or changed (`Warning`)
/// row on its own wash.
pub fn diff_glyph(status: Status, cx: &App) -> Hsla {
    let wash = match status {
        Status::Warning => status_fill(status, cx),
        _ => diff_fill(status, cx),
    };
    diff_glyph_color(diff_token(status, cx), wash, cx.theme().mode)
}

/// `diff_fill_hex` for the current theme mode, parsed.
pub fn diff_fill(status: Status, cx: &App) -> Hsla {
    Hsla::parse_hex(diff_fill_hex(status, cx.theme().mode)).unwrap_or(cx.theme().muted)
}

/// The label colour and opaque fill of a chosen collection button in the update picker. The
/// fill is the kit's outline-primary selected bg (`outline_background`, Active) over the dialog
/// background; the view sets both, so the contrast test measures what is drawn.
pub fn chosen_label_colors(theme: &Theme) -> (Hsla, Hsla) {
    let fill = theme.background.blend(theme.primary_active.opacity(0.4));
    (theme.foreground, fill)
}

/// A selected button's kit style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectedStyle {
    /// A ghost button, such as a toolbar toggle or a navigation entry.
    Ghost,
    /// A plain (default variant) button.
    Plain,
    /// The chosen collection button, outline primary; see [`chosen_label_colors`].
    ChosenOutlinePrimary,
}

/// The label colour and opaque fill a selected button of `style` draws.
pub fn selected_look(theme: &Theme, style: SelectedStyle) -> (Hsla, Hsla) {
    match style {
        SelectedStyle::Ghost => (theme.secondary_foreground, theme.secondary_active),
        SelectedStyle::Plain => (theme.button_foreground, theme.button_active),
        SelectedStyle::ChosenOutlinePrimary => chosen_label_colors(theme),
    }
}

/// The fill a select button draws in every state, or `None` for the kit's own. In High
/// Contrast Dark the kit's open fill (`input` at 70%) is too light for the normal-strength
/// label, so the button keeps its idle fill and a hover still shows the kit's lighter one.
pub fn select_open_fill(theme: &Theme) -> Option<Hsla> {
    (theme.theme_name() == HC_DARK).then(|| theme.input_background())
}

/// How a shadowed row of the variables overview is drawn: the opacity of the whole row and the
/// text colour that replaces `foreground`, if any. The row view applies both as returned.
pub fn shadowed_look(theme: &Theme) -> (f32, Option<Hsla>) {
    // The high-contrast themes mute the text instead, since 0.6 opacity breaks 4.5:1 there.
    if disabled_text_alpha(theme.theme_name()) >= 1.0 {
        (1.0, Some(theme.muted_foreground))
    } else {
        (0.6, None)
    }
}

/// How a `{{name}}` placeholder resolves, which picks its tint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarKind {
    /// A name some scope defines.
    Defined,
    /// A name no scope defines; sending it fails.
    Undefined,
    /// A `{{$guid}}`-style name generated on each send.
    Dynamic,
    /// A `{{?label}}` prompt, asked for when the request is sent.
    Prompt,
}

impl VarKind {
    /// Every kind, in declaration order.
    pub const ALL: [VarKind; 4] = [
        VarKind::Defined,
        VarKind::Undefined,
        VarKind::Dynamic,
        VarKind::Prompt,
    ];
}

/// The translucent `#RRGGBBAA` tint on a placeholder: a status colour at an alpha
/// low enough that text keeps 4.5:1 whether the tint sits over or under it.
pub fn var_tint_hex(kind: VarKind, mode: ThemeMode) -> &'static str {
    match (mode, kind) {
        (ThemeMode::Dark, VarKind::Defined) => "#51CF6640",
        (ThemeMode::Dark, VarKind::Undefined) => "#FF6B6B40",
        (ThemeMode::Dark, VarKind::Dynamic) => "#4DABF740",
        (ThemeMode::Dark, VarKind::Prompt) => "#FCC41940",
        (ThemeMode::Light, VarKind::Defined) => "#23703224",
        (ThemeMode::Light, VarKind::Undefined) => "#C92A2A24",
        (ThemeMode::Light, VarKind::Dynamic) => "#1864AB24",
        (ThemeMode::Light, VarKind::Prompt) => "#9A5A0024",
    }
}

/// `var_tint_hex` for the current theme mode, parsed.
pub fn var_tint(kind: VarKind, cx: &App) -> Hsla {
    // Every tint is a fixed eight-digit hex literal, so the muted fallback is unreachable.
    Hsla::parse_hex(var_tint_hex(kind, cx.theme().mode)).unwrap_or(cx.theme().muted)
}

/// The mode for `choice`: Light and Dark are fixed, System follows `appearance`.
pub fn mode_for(choice: ThemeChoice, appearance: WindowAppearance) -> ThemeMode {
    match choice {
        ThemeChoice::Light => ThemeMode::Light,
        ThemeChoice::Dark => ThemeMode::Dark,
        ThemeChoice::System => appearance.into(),
    }
}

/// The names of the registry themes of `mode`, the bundled default first and the rest
/// alphabetical ignoring case.
pub fn choices(mode: ThemeMode, cx: &App) -> Vec<SharedString> {
    let default = if mode.is_dark() { DARK } else { LIGHT };
    let registry = ThemeRegistry::global(cx);
    // The kit registers its own default pair; those are not Apinata's to offer.
    let kit = [
        registry.default_light_theme().name.clone(),
        registry.default_dark_theme().name.clone(),
    ];
    let own = |n: &str| n == DARK || n == LIGHT;
    let mut names: Vec<SharedString> = registry
        .themes()
        .values()
        .filter(|c| c.mode == mode && c.name.as_ref() != default)
        .filter(|c| own(&c.name) || !kit.contains(&c.name))
        .map(|c| c.name.clone())
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.insert(0, default.into());
    names
}

/// The registry config that `name` selects for `mode`, or the bundled default when the name
/// is absent, unknown or of the other mode. `None` only if the default is not registered.
fn resolve(name: Option<&str>, mode: ThemeMode, cx: &App) -> Option<Rc<ThemeConfig>> {
    let themes = ThemeRegistry::global(cx).themes();
    let default = if mode.is_dark() { DARK } else { LIGHT };
    name.and_then(|n| themes.get(n))
        .filter(|c| c.mode == mode)
        .or_else(|| themes.get(default))
        .cloned()
}

/// Applies `settings` to the global theme for `window`'s appearance; both writes refresh
/// every window, so each happens only when its value changes.
pub fn apply(settings: &Settings, window: &mut Window, cx: &mut App) {
    let mode = mode_for(settings.theme, window.appearance());
    // Both slots are kept in step so a later mode switch shows the right theme; the reload
    // (which refreshes every window) runs only if the active mode's config actually changed.
    let light = resolve(settings.light_theme.as_deref(), ThemeMode::Light, cx);
    let dark = resolve(settings.dark_theme.as_deref(), ThemeMode::Dark, cx);
    let t = Theme::global(cx);
    let differs = |want: &Option<Rc<ThemeConfig>>, have: &Rc<ThemeConfig>| {
        want.as_ref().is_some_and(|w| w.name != have.name)
    };
    let (light_changed, dark_changed) = (
        differs(&light, &t.light_theme),
        differs(&dark, &t.dark_theme),
    );
    let active_changed = if mode.is_dark() {
        dark_changed
    } else {
        light_changed
    };
    if light_changed || dark_changed {
        Theme::update(cx, |t| {
            if let (true, Some(light)) = (light_changed, light) {
                t.light_theme = light;
            }
            if let (true, Some(dark)) = (dark_changed, dark) {
                t.dark_theme = dark;
            }
        });
    }
    // Changing the mode reloads its config too, so one reload covers both causes.
    if Theme::global(cx).mode != mode || active_changed {
        set_mode(mode, Some(window), cx);
    }
    // The kit Root applies `font_size` as the rem each frame, so text in rems follows it.
    let ui = zoom::ui_font_size(settings.zoom);
    // The editor size is the zoomed base plus its own offset, clamped like the response size.
    let mono = zoom::response_font_size(
        zoom::mono_font_size(settings.zoom).into(),
        settings.editor_font_offset,
    );
    let toast_bottom = toast_bottom(settings.zoom);
    let t = Theme::global(cx);
    if t.font_size != ui
        || t.mono_font_size != mono
        || t.notification.margins.bottom != toast_bottom
    {
        Theme::update(cx, |t| {
            t.font_size = ui;
            t.mono_font_size = mono;
            t.notification.margins.bottom = toast_bottom;
        });
    }
}

/// Applies `settings` now, again on each change, and on each change of `window`'s system
/// appearance, for as long as the window and the settings live.
pub fn follow(settings: Entity<Persisted<Settings>>, window: &mut Window, cx: &mut App) {
    let current = settings.read(cx).get().clone();
    apply(&current, window, cx);
    window
        .observe(&settings, cx, |settings, window, cx| {
            let current = settings.read(cx).get().clone();
            apply(&current, window, cx);
        })
        .detach();
    window
        .observe_window_appearance(move |window, cx| {
            let current = settings.read(cx).get().clone();
            apply(&current, window, cx);
        })
        .detach();
}
