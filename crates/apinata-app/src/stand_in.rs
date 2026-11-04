//! What held focus before the shell's own handle took it. The shell's handle is no Tab stop
//! and takes no typing, so focus that lands there goes back to the recorded holder.

use std::cell::{Cell, RefCell};

use gpui_kit::component::WindowExt as _;
use gpui_kit::{App, FocusHandle, WeakFocusHandle, Window};

/// The holder before the last key or press, outside dialogs and the open app menu.
#[derive(Default)]
pub struct StandIn {
    held: RefCell<Option<WeakFocusHandle>>,
    frozen: Cell<bool>,
}

impl StandIn {
    /// Records what holds focus now, unless that is `shell` itself, a dialog is open or the
    /// app menu is open (its popup and items are no place to return to).
    pub fn hold(&self, shell: &FocusHandle, window: &mut Window, cx: &mut App) {
        if self.frozen.get() || window.has_active_dialog(cx) {
            return;
        }
        if let Some(now) = window.focused(cx).filter(|f| f != shell) {
            *self.held.borrow_mut() = Some(now.downgrade());
        }
    }

    /// Stops recording while the app menu is open, so what held focus before it is kept.
    pub fn freeze(&self, frozen: bool) {
        self.frozen.set(frozen);
    }

    /// The recorded holder, if it still exists; the record is used up. It also ends a freeze,
    /// since a menu removed while open never announces its close.
    pub fn take(&self) -> Option<FocusHandle> {
        self.frozen.set(false);
        self.held.borrow_mut().take().and_then(|h| h.upgrade())
    }
}
