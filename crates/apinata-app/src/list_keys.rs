//! Up, Down and Enter for a flat focusable list the kit does not draw itself, such as the
//! GraphQL Docs rows and the network dock. The list sets [`KEY_CONTEXT`] where it tracks
//! focus and handles the three actions; the keymap binds them.

use gpui_kit::actions;

actions!(
    list_keys,
    [
        /// Moves the focused list's selection one row up.
        ListUp,
        /// Moves the focused list's selection one row down.
        ListDown,
        /// Opens the focused list's selected row, as a click on it does.
        ListOpen
    ]
);

/// The key context of a list that takes [`ListUp`], [`ListDown`] and [`ListOpen`].
pub const KEY_CONTEXT: &str = "KeyList";

/// The row a step lands on among `stops`, the selectable row indices in order. With nothing
/// selected, or a selection that is no longer a stop, either step lands on the first stop;
/// otherwise it moves one stop and holds at either end. `None` only when there are no stops.
pub fn step(stops: &[usize], at: Option<usize>, down: bool) -> Option<usize> {
    let Some(pos) = at.and_then(|at| stops.iter().position(|&s| s == at)) else {
        return stops.first().copied();
    };
    let next = if down {
        (pos + 1).min(stops.len() - 1)
    } else {
        pos.saturating_sub(1)
    };
    stops.get(next).copied()
}
