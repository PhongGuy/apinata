//! A single-line input at rest shows the start of its value, like a browser's text field.

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::{App, Entity, Window};

/// Marks that the blur reset is installed, so a second start does not add another.
struct Installed;

impl gpui_kit::Global for Installed {}

/// Makes every input created from now on put its caret at the start when it loses focus. The
/// kit keeps the scroll that followed the caret, so a long value would show only its tail.
pub fn install(cx: &mut App) {
    if cx.has_global::<Installed>() {
        return;
    }
    cx.set_global(Installed);
    cx.observe_new(
        |_: &mut InputState, window: Option<&mut Window>, cx: &mut Context<InputState>| {
            let Some(window) = window else {
                return;
            };
            cx.subscribe_in(&cx.entity(), window, on_event).detach();
        },
    )
    .detach();
}

// The kit scrolls to the caret only when the selection changes, so moving the caret to the
// start brings the start into view and Tab back shows it. A selection is kept for copying, and
// a blur from switching windows keeps the view the user left.
fn on_event(
    input: &mut InputState,
    _: &Entity<InputState>,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<InputState>,
) {
    if matches!(event, InputEvent::Blur)
        && window.is_window_active()
        && input.selected_range().is_empty()
    {
        input.set_selected_range(0..0, cx);
    }
}
