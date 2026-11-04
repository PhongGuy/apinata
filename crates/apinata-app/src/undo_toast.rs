//! The toast that follows a delete into the trash: an Undo button for as long as it shows.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::{App, ElementId, SharedString, Window};

/// The id type of every delete's Undo toast; its key is the sidebar's undo entry id.
pub struct UndoToast;

/// The element key the Undo toast of undo entry `entry` is pushed under.
pub fn toast_key(entry: u64) -> ElementId {
    ElementId::from(entry as usize)
}

/// A toast saying `message` with an Undo button that calls `on_undo`, for the sidebar's undo
/// entry `entry`. It hides itself after a few seconds; the entry outlives it, so Ctrl+Z in
/// the sidebar can still undo the delete.
pub fn undo_toast(
    message: impl Into<SharedString>,
    entry: u64,
    on_undo: impl Fn(&mut Window, &mut App) + 'static,
) -> Notification {
    let on_undo = Rc::new(on_undo);
    Notification::new()
        .id1::<UndoToast>(toast_key(entry))
        .message(message)
        .action(move |_, _, cx| {
            let on_undo = on_undo.clone();
            let this = cx.entity();
            Button::new("undo-delete")
                .ghost()
                .label("Undo")
                .on_click(move |_, window, cx| {
                    on_undo(window, cx);
                    this.update(cx, |n, cx| n.dismiss(window, cx));
                })
        })
        // Setting an action turns the timer off; a delete's Undo is only for a few seconds.
        .autohide(true)
}
