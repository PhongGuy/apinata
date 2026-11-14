//! Where a dialog sits: below the title bar and ending above the status bar, at every zoom.

use gpui_kit::component::dialog::Dialog;
use gpui_kit::component::window_paddings;
use gpui_kit::{Edges, Pixels, Rems, Window, px, rems};

use crate::style::{
    DIALOG_BODY_WIDTH, DIALOG_BOX_CHROME, DIALOG_SIDE_PADDINGS, SPACE_4, STATUS_BAR_HEIGHT,
    TITLE_BAR_HEIGHT, resolve, snap,
};

/// The dialog's top offset: the kit's tenth of the viewport, or just below the title bar
/// when that is lower.
pub fn top(viewport_height: Pixels, rem: Pixels) -> Pixels {
    (viewport_height / 10.).max(at_rem(TITLE_BAR_HEIGHT, rem) + gap(viewport_height, rem))
}

// The gap kept around a dialog. A view under 24 rems tall is too short to spare a full one,
// so the gap shrinks to a quarter and the dialog gets the room.
fn gap(viewport_height: Pixels, rem: Pixels) -> Pixels {
    let full = at_rem(SPACE_4, rem);
    if viewport_height < rem * 24. {
        full / 4.
    } else {
        full
    }
}

/// The tallest a dialog at `top` may be and still end a gap above the status bar.
pub fn room(viewport_height: Pixels, rem: Pixels) -> Pixels {
    let room = viewport_height
        - top(viewport_height, rem)
        - at_rem(STATUS_BAR_HEIGHT, rem)
        - gap(viewport_height, rem);
    room.max(px(0.))
}

/// The height the kit lays dialogs out in: the viewport less the window border's `paddings`.
pub fn view_height(viewport_height: Pixels, paddings: Edges<Pixels>) -> Pixels {
    viewport_height - paddings.top - paddings.bottom
}

/// `room` for `window`, inside its border padding.
pub fn window_room(window: &Window) -> Pixels {
    room(window_view_height(window), window.rem_size())
}

// A kit dialog's title is a 1.25 line of base text, 0.5 rem above the body.
const TITLE_CHROME: Rems = rems(1.75);

/// The tallest a dialog's content may be in `window` when the dialog has no title.
pub fn body_room(window: &Window) -> Pixels {
    (window_room(window) - DIALOG_BOX_CHROME).max(px(0.))
}

/// The tallest a titled dialog's body may be in `window`.
pub fn titled_body_room(window: &Window) -> Pixels {
    (body_room(window) - at_rem(TITLE_CHROME, window.rem_size())).max(px(0.))
}

/// `dialog` with `top` and the zoomed default width applied for `window`; a dialog sets its
/// own width after this. The kit offsets the dialog by the top border padding itself, so the
/// offset is measured in the padded view.
pub fn place(dialog: Dialog, window: &Window) -> Dialog {
    dialog
        .margin_top(top(window_view_height(window), window.rem_size()))
        .width(resolve(DIALOG_BODY_WIDTH, window) + DIALOG_SIDE_PADDINGS)
}

/// [`place`] for a dialog opened over another: lower by the one beneath's title and a gap
/// on each side, so that title stays in view above it.
pub fn place_nested(dialog: Dialog, window: &Window) -> Dialog {
    let rem = window.rem_size();
    let lower = at_rem(TITLE_CHROME, rem) + at_rem(SPACE_4, rem) * 2.;
    place(dialog, window).margin_top(top(window_view_height(window), rem) + lower)
}

fn window_view_height(window: &Window) -> Pixels {
    view_height(window.viewport_size().height, window_paddings(window))
}

fn at_rem(len: Rems, rem: Pixels) -> Pixels {
    snap(len.to_pixels(rem))
}
