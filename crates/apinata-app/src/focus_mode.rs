//! App-wide focus modality: the accent focus ring shows only after a key moves focus, and a
//! pointer press puts back the plain input border. A change swaps the kit theme's `ring` and
//! `focus_ring`, so every kit control follows without per-site code.

use gpui_kit::component::Theme;
use gpui_kit::{
    App, DispatchPhase, Global, Hsla, IntoElement, MouseDownEvent, Styled as _, Subscription,
    Window, canvas,
};

/// How the user last moved focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Modality {
    /// A pointer press; focus shows as the plain input border.
    #[default]
    Pointer,
    /// A key that moved focus; focus shows as the accent ring.
    Keyboard,
}

#[derive(Default)]
struct State {
    modality: Modality,
    swaps: usize,
    interceptor: Option<Subscription>,
}

impl Global for State {}

/// Registers the app-wide keystroke interceptor once; later calls do nothing.
pub fn install(cx: &mut App) {
    if cx.default_global::<State>().interceptor.is_some() {
        return;
    }
    // A key counts only when focus differs once its handlers ran, so typing stays Pointer.
    let sub = cx.intercept_keystrokes(|_, window, cx| key_moves_focus(window, cx));
    cx.global_mut::<State>().interceptor = Some(sub);
}

/// Sets Keyboard once the effects queued so far have run, if focus then differs from now. A
/// key handler that moves focus through a deferred action calls this after dispatching it,
/// since the interceptor's own check runs before that action does.
pub fn key_moves_focus(window: &mut Window, cx: &mut App) {
    let before = window.focused(cx);
    window.defer(cx, move |window, cx| {
        if window.focused(cx) != before {
            set(Modality::Keyboard, cx);
        }
    });
}

/// The current modality; Pointer before `install`.
pub fn modality(cx: &App) -> Modality {
    cx.try_global::<State>()
        .map(|s| s.modality)
        .unwrap_or_default()
}

/// Whether the keyboard last moved focus.
pub fn is_keyboard(cx: &App) -> bool {
    modality(cx) == Modality::Keyboard
}

/// How many times the modality actually changed.
pub fn swaps(cx: &App) -> usize {
    cx.try_global::<State>().map_or(0, |s| s.swaps)
}

/// Moves to `modality`; only a real change swaps the ring, counts, and refreshes windows.
pub fn set(modality: Modality, cx: &mut App) {
    let state = cx.default_global::<State>();
    if state.modality == modality {
        return;
    }
    state.modality = modality;
    state.swaps += 1;
    if cx.has_global::<Theme>() {
        Theme::update(cx, |t| apply_ring(t, modality));
    }
}

/// The mode config's `ring` colour (fallback `primary`), whatever the live `ring` holds.
pub fn accent_ring(theme: &Theme) -> Hsla {
    let config = if theme.mode.is_dark() {
        &theme.dark_theme
    } else {
        &theme.light_theme
    };
    config
        .colors
        .ring
        .as_deref()
        .and_then(crate::ui::parse_color)
        .unwrap_or(theme.primary)
}

/// Keyboard: `ring` = the current mode config's `ring` (fallback `primary`), `focus_ring` on.
/// Pointer: `ring` = `input`, `focus_ring` off.
pub(crate) fn apply_ring(theme: &mut Theme, modality: Modality) {
    match modality {
        Modality::Keyboard => {
            theme.ring = accent_ring(theme);
            theme.focus_ring = true;
        }
        Modality::Pointer => {
            theme.ring = theme.input;
            theme.focus_ring = false;
        }
    }
}

/// An absolute full-size canvas whose paint registers a capture-phase `MouseDownEvent`
/// listener that sets Pointer. Each window root paints one.
pub fn pointer_watch() -> impl IntoElement {
    canvas(
        |_, _, _| {},
        |_, _, window, _| {
            // Window-level and never stopping propagation, so dialogs still get the press.
            window.on_mouse_event(|_: &MouseDownEvent, phase, _, cx| {
                if phase == DispatchPhase::Capture {
                    set(Modality::Pointer, cx);
                }
            });
        },
    )
    .absolute()
    .size_full()
}
