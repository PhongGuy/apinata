//! Focus repair for open dialogs: a focused control that is removed, re-keyed or disabled
//! leaves focus outside every key context, where the dialog's Escape and Tab never arrive.

use gpui_kit::component::WindowExt as _;
use gpui_kit::{App, FocusHandle, Window};

/// While a dialog is open, puts focus lost inside it on the first Tab stop within the closest
/// still-drawn ancestor of the lost control, or on that ancestor when it holds no stop; focus
/// lost behind it goes to the topmost dialog's first Tab stop. Call only from a focus-lost listener.
/// Returns false and leaves focus as it was when no dialog is open, no restore target exists,
/// or no Tab stop lies inside a focus trap (the dialog's).
pub fn repair_lost(window: &mut Window, cx: &mut App) -> bool {
    if !window.has_active_dialog(cx) {
        return false;
    }
    let Some(target) = window.focus_lost_restore_target(cx) else {
        return false;
    };
    // Containment in the open dialog shows only once the target holds focus. A target behind
    // it would leave keys outside the dialog, so focus walks the Tab order into the dialog.
    let lost = window.focused(cx);
    target.focus(window, cx);
    if gpui_kit::base::active_focus_trap(window, cx).is_none() {
        if focus_into_trap(window, cx) {
            crate::focus_redraw::after_draw_move(window, cx);
            return true;
        }
        match lost {
            Some(lost) => lost.focus(window, cx),
            None => window.blur(cx),
        }
        return false;
    }
    crate::focus_redraw::after_draw_move(window, cx);
    // Walk the Tab order from the target until a stop inside it holds focus. The frame's stops
    // are fixed during the walk, so it cycles back to its first stop after at most one lap;
    // reaching that stop again means the target holds none and keeps focus.
    let mut first = None;
    loop {
        window.focus_next(cx);
        let now = window.focused(cx);
        if now.as_ref() != Some(&target) && target.contains_focused(window, cx) {
            return true;
        }
        if now.is_none() || (first.is_some() && first == now) {
            break;
        }
        first = first.or(now);
    }
    target.focus(window, cx);
    true
}

// Steps through one lap of the frame's Tab stops and focuses the first stop of the last focus
// trap met: stacked dialogs paint in order, so the topmost one's stops come last. False, with
// focus moved, when no stop lies inside a trap.
fn focus_into_trap(window: &mut Window, cx: &mut App) -> bool {
    let mut first = None;
    let mut pick: Option<(FocusHandle, FocusHandle)> = None;
    loop {
        window.focus_next(cx);
        let Some(now) = window.focused(cx) else {
            break;
        };
        if first.as_ref() == Some(&now) {
            break;
        }
        first = first.or(Some(now.clone()));
        if let Some(trap) = gpui_kit::base::active_focus_trap(window, cx)
            && pick.as_ref().is_none_or(|(t, _)| *t != trap)
        {
            pick = Some((trap, now));
        }
    }
    match pick {
        Some((_, stop)) => {
            stop.focus(window, cx);
            true
        }
        None => false,
    }
}
