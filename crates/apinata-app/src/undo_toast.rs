//! The toast that follows a delete into the trash: an Undo button for as long as it shows.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::{
    AnyElement, App, ElementId, InteractiveElement as _, IntoElement as _, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window, div,
};

use crate::sidebar::UndoDelete;

/// How long a delete's Undo toast shows, twice the kit's own toast timeout.
pub const UNDO_TOAST_TIMEOUT: Duration = Duration::from_secs(10);

// How often a toast under the pointer checks whether the pointer has left.
const HOVER_POLL: Duration = Duration::from_millis(250);

// How many of the toast's watched areas the pointer is on, and a count of its every arrival,
// so a countdown can tell the pointer came by while it ran.
#[derive(Default)]
struct Hover {
    over: Cell<u8>,
    enters: Cell<u64>,
}

impl Hover {
    fn set(&self, hovered: bool) {
        if hovered {
            self.over.set(self.over.get().saturating_add(1));
            self.enters.set(self.enters.get() + 1);
        } else {
            self.over.set(self.over.get().saturating_sub(1));
        }
    }

    // A layer over its parent that only reports the pointer to `hover`; it takes no clicks.
    fn watch(self: &Rc<Self>, id: &'static str) -> AnyElement {
        let hover = Rc::clone(self);
        div()
            .id(id)
            .absolute()
            .inset_0()
            .on_hover(move |hovered, _, _| hover.set(*hovered))
            .test_support()
            .into_any_element()
    }
}

/// The id type of every delete's Undo toast; its key is the sidebar's undo entry id.
pub struct UndoToast;

/// The element key the Undo toast of undo entry `entry` is pushed under.
pub fn toast_key(entry: u64) -> ElementId {
    ElementId::from(entry as usize)
}

/// Shows a toast saying `message` with an Undo button that calls `on_undo`, for the sidebar's
/// undo entry `entry`, and closes it after `UNDO_TOAST_TIMEOUT` without the pointer on it. The
/// entry outlives it, so the key the button's tooltip names can still undo the delete.
pub fn show_undo_toast(
    message: impl Into<SharedString>,
    entry: u64,
    on_undo: impl Fn(&mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let hover = Rc::new(Hover::default());
    let watch = Rc::downgrade(&hover);
    window.push_notification(undo_toast(message, entry, on_undo, hover), cx);
    // The kit's own timeout is fixed at five seconds, so the toast stays up and this timer
    // closes it. A pointer on the toast holds it open, and one that came by restarts the
    // countdown; the toast only holds `hover`, so once it is gone the timer stops.
    window
        .spawn(cx, async move |cx| {
            let timer = |d| cx.background_executor().timer(d);
            loop {
                let seen = watch.upgrade().map(|h| h.enters.get());
                timer(UNDO_TOAST_TIMEOUT).await;
                while watch.upgrade().is_some_and(|h| h.over.get() > 0) {
                    timer(HOVER_POLL).await;
                }
                match watch.upgrade() {
                    Some(h) if Some(h.enters.get()) != seen => continue,
                    Some(_) => break,
                    None => return,
                }
            }
            cx.update(|window, cx| window.remove_notification1::<UndoToast>(toast_key(entry), cx))
                .ok();
        })
        .detach();
}

fn undo_toast(
    message: impl Into<SharedString>,
    entry: u64,
    on_undo: impl Fn(&mut Window, &mut App) + 'static,
    hover: Rc<Hover>,
) -> Notification {
    let on_undo = Rc::new(on_undo);
    // Setting an action turns the kit's timer off, which `show_undo_toast` replaces.
    Notification::new()
        .id1::<UndoToast>(toast_key(entry))
        .message(message)
        // The message column and the Undo button each carry a watch layer: the kit's button
        // spends its own hover listener on its tooltip.
        .content({
            let hover = Rc::clone(&hover);
            move |_, _, _| hover.watch("undo-toast-hover")
        })
        .action(move |_, _, cx| {
            let on_undo = Rc::clone(&on_undo);
            let this = cx.entity();
            // Read from the keymap at each render, so a rebound key shows at once.
            Button::new("undo-delete")
                .ghost()
                .label("Undo")
                .tooltip(crate::ui::label_with_key("Undo", &UndoDelete, cx))
                .child(hover.watch("undo-button-hover"))
                .on_click(move |_, window, cx| {
                    on_undo(window, cx);
                    this.update(cx, |n, cx| n.dismiss(window, cx));
                })
        })
}
