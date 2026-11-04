//! Moving a table row: where a drop or an Alt step puts it, and the handle and drop line
//! the key/value, assertion and environment tables share.

use gpui_kit::assets::IconName;
use gpui_kit::component::h_flex;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, Context, CursorStyle, EntityId,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window, actions, div, rems,
};

use crate::style::{KV_GRIP_WIDTH, SPACE_1};
use crate::tabs::Side;

actions!(
    apinata,
    [
        /// Moves the focused table row one place up.
        MoveRowUp,
        /// Moves the focused table row one place down.
        MoveRowDown
    ]
);

/// The index a row at `from` takes in a table of `len` rows when dropped on the `side` of the
/// row at `to`; `None` when it would not move or an index is outside the table.
pub fn move_target(len: usize, from: usize, to: usize, side: Side) -> Option<usize> {
    if from >= len || to >= len {
        return None;
    }
    let slot = match side {
        Side::Before => to,
        Side::After => to + 1,
    };
    // Taking the row out shifts every later slot up by one.
    let dest = if from < slot { slot - 1 } else { slot };
    (dest != from).then_some(dest)
}

/// The index an Alt+Up (`up`) or Alt+Down step takes row `from` to, swapping it with its
/// neighbour; `None` at either end.
pub fn step_target(len: usize, from: usize, up: bool) -> Option<usize> {
    if from >= len {
        return None;
    }
    if up {
        from.checked_sub(1)
    } else {
        (from + 1 < len).then_some(from + 1)
    }
}

/// Take the element at `from` out and put it back at index `dest`.
pub fn reorder<T>(items: &mut Vec<T>, from: usize, dest: usize) {
    let item = items.remove(from);
    items.insert(dest, item);
}

/// The indices of the rows that are not pinned, in order.
fn real_rows(pinned: &[bool]) -> Vec<usize> {
    (0..pinned.len()).filter(|&i| !pinned[i]).collect()
}

/// The row an Alt step of `from` swaps with when `pinned` rows stay where they are; `None`
/// when `from` is pinned or has no real neighbour that way.
pub fn real_step_target(pinned: &[bool], from: usize, up: bool) -> Option<usize> {
    let real = real_rows(pinned);
    let at = real.iter().position(|&i| i == from)?;
    step_target(real.len(), at, up).map(|dest| real[dest])
}

/// The index a drop takes real row `from` to, counting real rows only; `None` when either row
/// is pinned or the drop changes nothing.
pub fn real_move_target(pinned: &[bool], from: usize, to: usize, side: Side) -> Option<usize> {
    let real = real_rows(pinned);
    let from = real.iter().position(|&i| i == from)?;
    let to = real.iter().position(|&i| i == to)?;
    move_target(real.len(), from, to, side).map(|dest| real[dest])
}

/// Like [`reorder`] for the row at `from` and the destination row `dest`, but every pinned
/// item keeps its index: only the others change places.
pub fn reorder_pinned<T>(
    items: &mut Vec<T>,
    pinned: impl Fn(&T) -> bool,
    from: usize,
    dest: usize,
) {
    let slots: Vec<usize> = (0..items.len()).filter(|&i| !pinned(&items[i])).collect();
    let (Some(f), Some(d)) = (
        slots.iter().position(|&i| i == from),
        slots.iter().position(|&i| i == dest),
    ) else {
        return;
    };
    let mut cells: Vec<Option<T>> = items.drain(..).map(Some).collect();
    let mut real: Vec<T> = slots.iter().filter_map(|&i| cells[i].take()).collect();
    reorder(&mut real, f, d);
    for (&slot, item) in slots.iter().zip(real) {
        cells[slot] = Some(item);
    }
    items.extend(cells.into_iter().flatten());
}

/// The colour of a row's drag grip: what the delete icon paints, so the two read alike.
pub fn grip_color(cx: &App) -> gpui_kit::Hsla {
    cx.theme().secondary_foreground
}

/// The side of a row's `bounds` a pointer at height `y` is on: the upper half is before.
pub fn drop_side(y: Pixels, bounds: Bounds<Pixels>) -> Side {
    if y < bounds.center().y {
        Side::Before
    } else {
        Side::After
    }
}

/// Where focus goes when `incoming` is `current` with one row moved, as an undo or redo of a
/// move shows it: the new index of the row at `focused`. Rows `pinned` judges true keep their
/// slot in every candidate move, as in [`reorder_pinned`]. `None` when nothing moved, the lists
/// differ in any other way, or no move of that row explains `incoming`.
pub fn follow<T>(
    current: &[T],
    incoming: &[T],
    focused: usize,
    pinned: impl Fn(&T) -> bool,
    same: impl Fn(&T, &T) -> bool,
) -> Option<usize> {
    let len = current.len();
    if len != incoming.len() || focused >= len || pinned(&current[focused]) {
        return None;
    }
    let equal = |order: &[usize]| {
        order
            .iter()
            .zip(incoming)
            .all(|(&c, i)| same(&current[c], i))
    };
    let identity: Vec<usize> = (0..len).collect();
    if equal(&identity) {
        return None;
    }
    // Candidate orders come from the same pinned-aware move the table applies.
    (0..len).filter(|&dest| dest != focused).find(|&dest| {
        let mut order = identity.clone();
        reorder_pinned(&mut order, |&c| pinned(&current[c]), focused, dest);
        equal(&order)
    })
}

/// The drag preview's text for a row named `name`: the name, or "Row" when it is empty.
pub fn preview_label(name: &str) -> String {
    if name.is_empty() {
        "Row".into()
    } else {
        name.into()
    }
}

/// The payload of a row drag: the table it came from and the row's index there.
#[derive(Clone)]
pub struct DraggedRow {
    /// The table entity, so a row dropped on another table is ignored.
    pub owner: EntityId,
    /// The dragged row's index.
    pub from: usize,
    label: SharedString,
}

impl DraggedRow {
    /// A drag of row `from` of the table `owner`, previewed as `label`.
    pub fn new(owner: EntityId, from: usize, label: SharedString) -> Self {
        DraggedRow { owner, from, label }
    }
}

/// The label that follows the pointer while a row is dragged.
struct RowDragPreview(SharedString);

impl Render for RowDragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(SPACE_1)
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.0.clone())
    }
}

/// The handle slot at the left of a row. With `drag` it holds the grip `grip_id`, which starts
/// the drag; without it the slot is laid out empty under `slot_id`, so columns line up.
pub fn grip_slot(
    grip_id: (&'static str, usize),
    slot_id: (&'static str, usize),
    drag: Option<DraggedRow>,
    cx: &App,
) -> AnyElement {
    let slot = h_flex().flex_none().w(KV_GRIP_WIDTH).justify_center();
    match drag {
        Some(payload) => slot
            .child(
                div()
                    .id(grip_id)
                    .test_support()
                    .cursor(CursorStyle::OpenHand)
                    .text_color(grip_color(cx))
                    .on_drag(payload, |d: &DraggedRow, _, _, cx| {
                        cx.new(|_| RowDragPreview(d.label.clone()))
                    })
                    .child(Icon::new(IconName::GripVertical).xsmall()),
            )
            .into_any_element(),
        None => slot.id(slot_id).test_support().into_any_element(),
    }
}

/// The line that marks where a dropped row lands: across the top or bottom edge of the row
/// it is drawn in, which must be positioned.
pub fn drop_line(id: &'static str, side: Side, cx: &App) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .absolute()
        .left_0()
        .right_0()
        .h(rems(0.125))
        .bg(cx.theme().primary)
        .map(|d| match side {
            Side::Before => d.top_0(),
            Side::After => d.bottom_0(),
        })
}
