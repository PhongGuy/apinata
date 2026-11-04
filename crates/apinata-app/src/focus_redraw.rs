//! Redraw after a focus move made while a frame draws, which GPUI does not schedule itself.

use gpui_kit::{App, Window};

/// Asks for a frame once the current draw ends. Call right after moving focus inside a
/// focus-lost listener: GPUI's own refresh is a no-op mid-draw, so the new focus would show
/// no ring, take no keyboard click and get no input handler until something else redraws.
pub fn after_draw_move(window: &mut Window, cx: &mut App) {
    window.defer(cx, |window, _| window.refresh());
}
