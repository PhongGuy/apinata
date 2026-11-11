//! The toast that follows a delete into the trash: an Undo button for as long as it shows.

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::{App, ElementId, SharedString, Window};

use crate::sidebar::UndoDelete;

/// How long a delete's Undo toast shows, twice the kit's own toast timeout.
pub const UNDO_TOAST_TIMEOUT: Duration = Duration::from_secs(10);

/// The id type of every delete's Undo toast; its key is the sidebar's undo entry id.
pub struct UndoToast;

/// The element key the Undo toast of undo entry `entry` is pushed under.
pub fn toast_key(entry: u64) -> ElementId {
    ElementId::from(entry as usize)
}

/// Shows a toast saying `message` with an Undo button that calls `on_undo`, for the sidebar's
/// undo entry `entry`, and closes it after `UNDO_TOAST_TIMEOUT`. The entry outlives it, so the
/// key the button's tooltip names can still undo the delete.
pub fn show_undo_toast(
    message: impl Into<SharedString>,
    entry: u64,
    on_undo: impl Fn(&mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    window.push_notification(undo_toast(message, entry, on_undo), cx);
    // The kit's own timeout is fixed at five seconds, so the toast stays up and this timer
    // closes it; one already closed by Undo or Ctrl+Z is simply not found.
    window
        .spawn(cx, async move |cx| {
            cx.background_executor().timer(UNDO_TOAST_TIMEOUT).await;
            cx.update(|window, cx| window.remove_notification1::<UndoToast>(toast_key(entry), cx))
                .ok();
        })
        .detach();
}

fn undo_toast(
    message: impl Into<SharedString>,
    entry: u64,
    on_undo: impl Fn(&mut Window, &mut App) + 'static,
) -> Notification {
    let on_undo = Rc::new(on_undo);
    // Setting an action turns the kit's timer off, which `show_undo_toast` replaces.
    Notification::new()
        .id1::<UndoToast>(toast_key(entry))
        .message(message)
        .action(move |_, _, cx| {
            let on_undo = Rc::clone(&on_undo);
            let this = cx.entity();
            // Read from the keymap at each render, so a rebound key shows at once.
            Button::new("undo-delete")
                .ghost()
                .label("Undo")
                .tooltip(crate::ui::label_with_key("Undo", &UndoDelete, cx))
                .on_click(move |_, window, cx| {
                    on_undo(window, cx);
                    this.update(cx, |n, cx| n.dismiss(window, cx));
                })
        })
}
