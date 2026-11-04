//! The shared key-value table look: column descriptions, the heading row, the row frame,
//! the check and action slots, the borderless cell with its own hover and focus edge, and the
//! read-only `kv_view`.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::base::{ObservedElement, SelectableText};
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AnyView, App, AppContext as _, Context, Div, ElementId, Entity, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Rems, Render, RenderOnce,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled, TestSupportExt as _, Window,
    canvas, div, transparent_black,
};

use crate::scroll_follow::{FocusFollow, Followed};
use crate::style::{
    KV_ACTION_WIDTH, KV_CHECK_WIDTH, KV_GRIP_WIDTH, KV_HEAD_HEIGHT, KV_VIEW_LINE_HEIGHT,
    KV_VIEW_PAD_Y, RADIUS, ROW_HEIGHT, SPACE_0, SPACE_2, TEXT_SMALL,
};
use crate::ui::{self, LockButton as _};

/// The key context around the rows of a structured table (never its bulk text editor), so
/// undo and redo keys can bind to a cell input inside it.
pub const STRUCTURED_ROWS_CONTEXT: &str = "StructuredRows";

/// The hover group a row forms, so its action slot can show on hover.
pub const KV_ROW_GROUP: &str = "kv-row";
/// The hover group a cell forms, so its edge can show on hover.
pub const KV_CELL_GROUP: &str = "kv-cell";

/// How wide a column is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum KvWidth {
    /// Shares the free width with the other flexible columns.
    Flex,
    /// Exactly this wide.
    Fixed(Rems),
}

/// One column of a key-value grid: its heading id and text, the id its cells carry, its width.
#[derive(Debug, Clone, PartialEq)]
pub struct KvColumn {
    /// The heading cell's id.
    pub head_id: ElementId,
    /// The id each row's cell in this column carries, scoped by the row.
    pub cell_id: &'static str,
    /// The heading text.
    pub heading: SharedString,
    /// The column's width.
    pub width: KvWidth,
    /// In a read-only view, the column's text stays on one line and, when it does not fit,
    /// shows whole on hover.
    pub tip_when_cut: bool,
}

impl KvColumn {
    /// A column sharing the free width.
    pub fn flex(
        head_id: impl Into<ElementId>,
        cell_id: &'static str,
        heading: impl Into<SharedString>,
    ) -> KvColumn {
        KvColumn {
            head_id: head_id.into(),
            cell_id,
            heading: heading.into(),
            width: KvWidth::Flex,
            tip_when_cut: false,
        }
    }

    /// A column exactly `width` wide.
    pub fn fixed(
        head_id: impl Into<ElementId>,
        cell_id: &'static str,
        heading: impl Into<SharedString>,
        width: Rems,
    ) -> KvColumn {
        KvColumn {
            width: KvWidth::Fixed(width),
            ..KvColumn::flex(head_id, cell_id, heading)
        }
    }

    /// A column of a read-only view whose text stays on one line and, when cut, shows whole
    /// in a tooltip (`kv-cut-tooltip`) on hover.
    pub fn tip_when_cut(mut self) -> Self {
        self.tip_when_cut = true;
        self
    }

    /// `el` sized to the column: `flex_1().min_w_0()`, or the fixed width and `flex_none()`.
    pub fn size<E: Styled>(&self, el: E) -> E {
        match self.width {
            KvWidth::Flex => el.flex_1().min_w_0(),
            KvWidth::Fixed(w) => el.w(w).flex_none(),
        }
    }
}

/// The heading row's fill and text colour: `table_head`, `muted_foreground`.
pub fn head_colors(cx: &App) -> (Hsla, Hsla) {
    (cx.theme().table_head, cx.theme().muted_foreground)
}

/// A heading row for `columns`, built with `kv_head`.
#[derive(IntoElement)]
pub struct KvHead {
    id: ElementId,
    columns: Vec<KvColumn>,
    grip: bool,
    check: bool,
    check_box: Option<AnyElement>,
    extras: Vec<AnyElement>,
    action: Option<Option<AnyElement>>,
    pad_end: bool,
}

/// The heading row with id `id` over `columns`.
pub fn kv_head(id: impl Into<ElementId>, columns: Vec<KvColumn>) -> KvHead {
    KvHead {
        id: id.into(),
        columns,
        grip: false,
        check: false,
        check_box: None,
        extras: Vec::new(),
        action: None,
        pad_end: false,
    }
}

impl KvHead {
    /// Lead with an empty drag-handle slot, lining the headings up with movable rows.
    pub fn grip_column(mut self) -> Self {
        self.grip = true;
        self
    }

    /// Lead with an empty `kv-check` slot, lining the headings up with checked rows.
    pub fn check_column(mut self) -> Self {
        self.check = true;
        self
    }

    /// Lead with `box` in the `kv-check` slot, for a heading that carries its own checkbox.
    pub fn check_box(mut self, check_box: impl IntoElement) -> Self {
        self.check = true;
        self.check_box = Some(check_box.into_any_element());
        self
    }

    /// Add `slot` after the columns; extras keep their call order.
    pub fn extra(mut self, slot: impl IntoElement) -> Self {
        self.extras.push(slot.into_any_element());
        self
    }

    /// Keep the read-only view's right gutter, so the headings line up with its rows.
    pub fn pad_end(mut self) -> Self {
        self.pad_end = true;
        self
    }

    /// End with a `kv-action` slot holding `slot`, or empty for `None`.
    pub fn action(mut self, slot: Option<AnyElement>) -> Self {
        self.action = Some(slot);
        self
    }
}

impl RenderOnce for KvHead {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let (fill, text) = head_colors(cx);
        h_flex()
            .id(self.id)
            .test_support()
            .w_full()
            .flex_none()
            .gap(SPACE_2)
            .items_center()
            .py(SPACE_0)
            // Edit grids share one band height whether or not their action slot holds a toggle;
            // read-only view heads (the only ones padded at the end) keep their natural height.
            .when(self.action.is_some() && !self.pad_end, |r| {
                r.h(KV_HEAD_HEIGHT)
            })
            .when(self.pad_end, |r| r.pr(SPACE_2))
            .rounded(RADIUS)
            .bg(fill)
            .when(self.grip, |r| r.child(div().flex_none().w(KV_GRIP_WIDTH)))
            .when(self.check, |r| {
                r.child(match self.check_box {
                    Some(check_box) => check_slot(check_box),
                    None => check_slot(div()),
                })
            })
            .children(self.columns.iter().map(|c| {
                c.size(
                    div()
                        .id(c.head_id.clone())
                        .test_support()
                        .text_size(TEXT_SMALL)
                        .text_color(text)
                        .truncate()
                        .child(c.heading.clone()),
                )
            }))
            .children(self.extras)
            .when_some(self.action, |r, slot| {
                r.child(match slot {
                    Some(slot) => action_slot(slot, ActionShow::Always),
                    None => action_slot(div(), ActionShow::Never),
                })
            })
    }
}

/// A row frame: id with test support, `w_full`, `min_h(ROW_HEIGHT)`, `gap(SPACE_2)`, items
/// centred, group `KV_ROW_GROUP`.
pub fn kv_row(id: impl Into<ElementId>) -> ObservedElement<Stateful<Div>> {
    h_flex()
        .id(id)
        .test_support()
        .group(KV_ROW_GROUP)
        .w_full()
        .flex_none()
        .min_h(ROW_HEIGHT)
        .gap(SPACE_2)
        .items_center()
}

/// `check` in the `kv-check` slot, `KV_CHECK_WIDTH` wide.
pub fn check_slot(check: impl IntoElement) -> AnyElement {
    h_flex()
        .id("kv-check")
        .test_support()
        .flex_none()
        .w(KV_CHECK_WIDTH)
        .justify_center()
        .child(check)
        .into_any_element()
}

/// When a row's action slot shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionShow {
    /// Transparent until the row is hovered.
    OnHover,
    /// Always shown.
    Always,
    /// Laid out but never shown.
    Never,
}

/// `button` in the `kv-action` slot, `KV_ACTION_WIDTH` wide. The opacity sits on the slot,
/// never on the button, so a click on a transparent button still lands; a hover-only slot
/// also shows while focus is within it.
pub fn action_slot(button: impl IntoElement, show: ActionShow) -> AnyElement {
    action_slot_focusing(button, show, None)
}

/// [`action_slot`] whose kit button, with element id `take_focus`, takes keyboard focus on
/// this draw when one is given.
pub fn action_slot_focusing(
    button: impl IntoElement,
    show: ActionShow,
    take_focus: Option<ElementId>,
) -> AnyElement {
    ActionSlot {
        button: button.into_any_element(),
        show,
        take_focus,
    }
    .into_any_element()
}

#[derive(IntoElement)]
struct ActionSlot {
    button: AnyElement,
    show: ActionShow,
    take_focus: Option<ElementId>,
}

/// Focus the kit button with element id `id` that is drawn as a child of the element being
/// laid out now, with no id between them.
fn focus_kit_button(id: ElementId, window: &mut Window, cx: &mut App) {
    // The kit button keys its focus handle under its id inside its component scope; reading
    // that keyed state is the only way to focus it from outside. This relies on the kit's
    // internal id scoping, guarded by the reload focus tests of kv_table, env and assertions.
    let button = window.with_id(std::any::type_name::<Button>(), |window| {
        window
            .use_keyed_state(id, cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone()
    });
    ui::focus_in_draw(&button, window, cx);
}

/// `button`, a kit button with element id `take_focus`, taking keyboard focus on this draw
/// when one is given.
pub fn button_focusing(button: impl IntoElement, take_focus: Option<ElementId>) -> AnyElement {
    FocusingButton {
        button: button.into_any_element(),
        take_focus,
    }
    .into_any_element()
}

#[derive(IntoElement)]
struct FocusingButton {
    button: AnyElement,
    take_focus: Option<ElementId>,
}

impl RenderOnce for FocusingButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if let Some(id) = self.take_focus {
            focus_kit_button(id, window, cx);
        }
        self.button
    }
}

impl RenderOnce for ActionSlot {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if let Some(id) = self.take_focus {
            window.with_id("kv-action", |window| focus_kit_button(id, window, cx));
        }
        // A focus handle that is no tab stop, only to see focus inside the slot.
        let focus = window
            .use_keyed_state("kv-action-focus", cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        let within = focus.contains_focused(window, cx);
        // The tracker hugs the button, which takes focus itself on a press, so a click in
        // the slot's margin moves no focus.
        h_flex()
            .id("kv-action")
            .test_support()
            .flex_none()
            .w(KV_ACTION_WIDTH)
            .justify_center()
            .map(|d| match self.show {
                ActionShow::OnHover if !within => {
                    d.opacity(0.).group_hover(KV_ROW_GROUP, |s| s.opacity(1.))
                }
                ActionShow::OnHover | ActionShow::Always => d,
                ActionShow::Never => d.invisible(),
            })
            .child(div().track_focus(&focus).child(self.button))
    }
}

/// `field` with a transparent border, plus the edge `edge_id`: the input colour while the
/// cell is hovered, the ring colour while `focused`.
pub fn kv_cell(
    edge_id: impl Into<ElementId>,
    field: Input,
    state: &Entity<InputState>,
    focused: bool,
    cx: &App,
) -> Div {
    edged_cell(edge_id.into(), field, state, focused, false, None, cx)
}

/// `kv_cell` whose edge is drawn at rest too, for a field on a fill the same as its own
/// (a dialog), where the hover-only edge leaves a light theme's field unmarked.
pub fn kv_cell_resting(
    edge_id: impl Into<ElementId>,
    field: Input,
    state: &Entity<InputState>,
    focused: bool,
    cx: &App,
) -> Div {
    edged_cell(edge_id.into(), field, state, focused, true, None, cx)
}

/// `kv_cell` for a cell the last edit was refused on: the alert icon trails the field (id
/// `icon_id`) and the edge, now `danger_id`, is danger-coloured at rest and focused alike.
pub fn kv_cell_marked(
    (danger_id, icon_id): (ElementId, ElementId),
    field: Input,
    state: &Entity<InputState>,
    focused: bool,
    cx: &App,
) -> Div {
    let marked = Some((danger_id.clone(), icon_id));
    edged_cell(danger_id, field, state, focused, false, marked, cx)
}

fn edged_cell(
    edge_id: ElementId,
    field: Input,
    state: &Entity<InputState>,
    focused: bool,
    resting: bool,
    marked: Option<(ElementId, ElementId)>,
    cx: &App,
) -> Div {
    // A light theme has no fill step to mark a cell, so its edge rests visible; dark stays
    // hover-only.
    // A refused cell's edge always shows, so it never hides behind the hover-only edge.
    let resting = resting || marked.is_some() || !cx.theme().mode.is_dark();
    // Focus draws the same 2px accent ring as a padded input, as an overlay so nothing shifts;
    // a refused cell's danger colour outranks that ring and the resting edge.
    let colour = if marked.is_some() {
        cx.theme().danger
    } else if focused {
        crate::focus_mode::accent_ring(cx.theme())
    } else {
        cx.theme().input
    };
    let field = match &marked {
        Some((_, icon)) => field.suffix(ui::invalid_icon(icon.clone(), cx.theme().danger)),
        None => field,
    };
    // The kit sets the field's border first, so this refinement wins over it.
    ui::forward_padding_press(div(), state)
        .relative()
        .w_full()
        .group(KV_CELL_GROUP)
        .child(field.border_color(transparent_black()).w_full())
        .child(
            div()
                .id(edge_id)
                .test_support()
                .absolute()
                .inset_0()
                .rounded(RADIUS)
                .when(focused, |d| d.border_2())
                .when(!focused, |d| d.border_1())
                .border_color(colour)
                .when(!focused && !resting, |d| {
                    d.opacity(0.).group_hover(KV_CELL_GROUP, |s| s.opacity(1.))
                }),
        )
}

/// One cell of a read-only `kv_view`: selectable text, or a flag drawn as a tick or a dash.
#[derive(Debug, Clone, PartialEq)]
pub enum KvCell {
    /// Text, which is also the cell's aria label.
    Text(SharedString),
    /// Text in the muted colour, for a value that reads as a warning; also its aria label.
    Muted(SharedString),
    /// A yes/no value: a tick (aria "Yes") or a muted dash (aria "No").
    Flag(bool),
}

type DeleteFn = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// A read-only key-value table built with `kv_view`.
#[derive(IntoElement)]
pub struct KvView {
    id: ElementId,
    columns: Vec<KvColumn>,
    rows: Vec<Vec<KvCell>>,
    row_prefix: &'static str,
    head: Option<ElementId>,
    delete: Option<(&'static str, &'static str, DeleteFn)>,
    copy: Option<(&'static str, &'static str, DeleteFn)>,
    reveal: Option<Reveal>,
    reveal_column: Option<usize>,
    follow: Option<FocusFollow>,
    // Rows whose eye stays masked and disabled, with the tooltip saying why.
    locked: Option<(Vec<bool>, &'static str)>,
}

// A per-row eye: `rows[ix]` is None for a row with no eye, else whether it is revealed.
struct Reveal {
    ids: (&'static str, &'static str, &'static str),
    rows: Vec<Option<bool>>,
    on_toggle: DeleteFn,
}

/// A read-only table `id` of `rows` under `columns`, one cell per column in order.
pub fn kv_view(id: impl Into<ElementId>, columns: Vec<KvColumn>, rows: Vec<Vec<KvCell>>) -> KvView {
    KvView {
        id: id.into(),
        columns,
        rows,
        row_prefix: "kv-view-row",
        head: None,
        delete: None,
        copy: None,
        reveal: None,
        reveal_column: None,
        follow: None,
        locked: None,
    }
}

impl KvView {
    /// Rows are `(prefix, ix)` and cells `(column.cell_id, ix)`; the default prefix is
    /// "kv-view-row".
    pub fn row_ids(mut self, prefix: &'static str) -> Self {
        self.row_prefix = prefix;
        self
    }

    /// Draws `head_row(id)` above the rows.
    pub fn headed(mut self, id: impl Into<ElementId>) -> Self {
        self.head = Some(id.into());
        self
    }

    /// The heading row that lines up with these rows, for a caller that places it apart.
    pub fn head_row(&self, id: impl Into<ElementId>) -> KvHead {
        let head = kv_head(id, self.columns.clone()).pad_end();
        // One empty slot per row action, so the headings line up with the rows.
        let head = if self.copy.is_some() && self.delete.is_some() {
            head.extra(action_slot(div(), ActionShow::Never))
        } else {
            head
        };
        if self.copy.is_some() || self.delete.is_some() {
            head.action(None)
        } else {
            head
        }
    }

    /// An eye button per row that has one: `rows[ix]` is None for no eye, else whether the row is
    /// revealed. `ids` are `(eye, show, hide)`, `(id, ix)` each: the button keeps the eye id and
    /// its face takes the show or hide id. A click calls `f(ix, ..)`.
    pub fn on_reveal(
        mut self,
        ids: (&'static str, &'static str, &'static str),
        rows: Vec<Option<bool>>,
        f: impl Fn(usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.reveal = Some(Reveal {
            ids,
            rows,
            on_toggle: Rc::new(f),
        });
        self
    }

    /// Reports each eye to `follow`, so the scroll area around the table brings a focused eye
    /// into view.
    pub fn follow(mut self, follow: &FocusFollow) -> Self {
        self.follow = Some(follow.clone());
        self
    }

    /// Locks the eye of each row `rows[ix]` marks: masked, disabled and labelled `tooltip`, so a
    /// value that cannot be revealed says why.
    pub fn lock_eyes(mut self, rows: Vec<bool>, tooltip: &'static str) -> Self {
        self.locked = Some((rows, tooltip));
        self
    }

    /// Puts the eyes after the value in column `column` instead of the last one.
    pub fn reveal_column(mut self, column: usize) -> Self {
        self.reveal_column = Some(column);
        self
    }

    /// A hover-only copy button `(id, ix)` per row (ghost Copy icon, `tooltip`, which is also
    /// its label) calling `f(ix, ..)`. It sits before the delete button.
    pub fn on_copy(
        mut self,
        id: &'static str,
        tooltip: &'static str,
        f: impl Fn(usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.copy = Some((id, tooltip, Rc::new(f)));
        self
    }

    /// A hover-only remove button `(id, ix)` per row (ghost Close icon, `tooltip`) calling
    /// `f(ix, ..)`.
    pub fn on_delete(
        mut self,
        id: &'static str,
        tooltip: &'static str,
        f: impl Fn(usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.delete = Some((id, tooltip, Rc::new(f)));
        self
    }
}

impl RenderOnce for KvView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let head = self.head.clone().map(|id| self.head_row(id));
        let mono = cx.theme().mono_font_family.clone();
        let muted = cx.theme().muted_foreground;
        // A flexible column that tips when cut records the width each frame lays it out at, for
        // its tooltip to measure against when it shows.
        let widths = window.with_id(self.id.clone(), |window| {
            window.use_keyed_state("kv-flex-widths", cx, |_, _| Vec::<Option<Pixels>>::new())
        });
        let rem = window.rem_size();
        let rows = self.rows.into_iter().enumerate().map(|(ix, cells)| {
            let last = self
                .reveal_column
                .unwrap_or(self.columns.len().saturating_sub(1));
            let cells = self
                .columns
                .iter()
                .zip(cells)
                .enumerate()
                .map(|(c, (col, cell))| {
                    // A fixed column knows its width now. A flexible one is laid out after
                    // render, so its tooltip decides in every frame it shows, from the width
                    // that frame recorded.
                    let tip = match (&cell, col.width) {
                        (KvCell::Text(text) | KvCell::Muted(text), width) if col.tip_when_cut => {
                            match width {
                                KvWidth::Fixed(room) => {
                                    cut_text(text, &mono, room.to_pixels(rem), window)
                                        .then(|| (text.clone(), None))
                                }
                                KvWidth::Flex => Some((text.clone(), Some(widths.clone()))),
                            }
                        }
                        _ => None,
                    };
                    let mono_tip = mono.clone();
                    let measure = (col.tip_when_cut && col.width == KvWidth::Flex).then(|| {
                        let widths = widths.clone();
                        canvas(
                            move |bounds, _, cx| {
                                widths.update(cx, |v, _| {
                                    v.resize(v.len().max(c + 1), None);
                                    v[c] = Some(bounds.size.width);
                                })
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full()
                    });
                    let (label, content) = match cell {
                        KvCell::Text(text) => (
                            text.clone(),
                            div()
                                .id("kv-text")
                                .test_support()
                                .child(SelectableText::new("text", text))
                                .into_any_element(),
                        ),
                        KvCell::Muted(text) => (
                            text.clone(),
                            div()
                                .id("kv-text")
                                .test_support()
                                .text_color(muted)
                                .child(SelectableText::new("text", text))
                                .into_any_element(),
                        ),
                        KvCell::Flag(on) => (
                            SharedString::from(if on { "Yes" } else { "No" }),
                            h_flex()
                                .h(KV_VIEW_LINE_HEIGHT)
                                .child(if on {
                                    Icon::new(IconName::Check)
                                } else {
                                    Icon::new(IconName::Minus).text_color(muted)
                                })
                                .into_any_element(),
                        ),
                    };
                    // A revealable row's eye follows its value, in the last column.
                    let content = match self.reveal.as_ref().filter(|_| c == last) {
                        Some(reveal) => match reveal.rows.get(ix).copied().flatten() {
                            Some(revealed) => {
                                let locked = (self.locked.as_ref())
                                    .filter(|(rows, _)| rows.get(ix).copied().unwrap_or(false));
                                let (face, icon, words) = match locked {
                                    Some((_, tip)) => (reveal.ids.1, IconName::Eye, *tip),
                                    None if revealed => {
                                        (reveal.ids.2, IconName::EyeOff, "Hide value")
                                    }
                                    None => (reveal.ids.1, IconName::Eye, "Show value"),
                                };
                                let f = reveal.on_toggle.clone();
                                // One button across the flip, so a keyboard press keeps focus.
                                let eye = ui::swap_icon_button(
                                    (reveal.ids.0, ix),
                                    (face, ix),
                                    icon,
                                    words,
                                );
                                let eye = if locked.is_some() {
                                    eye.lock(true, cx)
                                } else {
                                    eye.on_click(move |_, window, cx| f(ix, window, cx))
                                };
                                let slot = action_slot(eye, ActionShow::Always);
                                let slot = match &self.follow {
                                    Some(f) => Followed::new(("kv-eye-follow", ix), f, slot)
                                        .into_any_element(),
                                    None => slot.into_any_element(),
                                };
                                h_flex()
                                    .items_start()
                                    .child(div().min_w_0().child(content))
                                    .child(slot)
                                    .into_any_element()
                            }
                            None => content,
                        },
                        None => content,
                    };
                    // Rows align to the top and every cell pads its first line into the
                    // middle of a row height. Fixed columns hold a line each; the flexible
                    // ones wrap unless they tip when cut.
                    col.size(
                        div()
                            .id((col.cell_id, ix))
                            .aria_label(label)
                            .test_support()
                            .py(KV_VIEW_PAD_Y)
                            .when(col.width != KvWidth::Flex || col.tip_when_cut, |d| {
                                d.truncate()
                            })
                            .when_some(tip, |d, (text, flex)| {
                                d.tooltip(move |w, cx| {
                                    let tip = cut_tip(&text, w, cx);
                                    match &flex {
                                        None => tip,
                                        Some(widths) => cx
                                            .new(|_| FlexTip {
                                                text: text.clone(),
                                                mono: mono_tip.clone(),
                                                widths: widths.clone(),
                                                col: c,
                                                tip,
                                            })
                                            .into(),
                                    }
                                })
                            })
                            .when_some(measure, |d, m| d.relative().child(m))
                            .child(content),
                    )
                });
            // The right gutter keeps a long value off the pane's edge.
            kv_row((self.row_prefix, ix))
                .pr(SPACE_2)
                .items_start()
                .font_family(mono.clone())
                .line_height(KV_VIEW_LINE_HEIGHT)
                .children(cells)
                .when_some(self.copy.as_ref(), |row, (id, tooltip, f)| {
                    let f = f.clone();
                    row.child(
                        h_flex()
                            .id(("kv-copy-slot", ix))
                            .test_support()
                            .min_h(ROW_HEIGHT)
                            .child(action_slot(
                                ui::icon_button((*id, ix), IconName::Copy, tooltip, None)
                                    .on_click(move |_, window, cx| f(ix, window, cx)),
                                ActionShow::OnHover,
                            )),
                    )
                })
                .when_some(self.delete.as_ref(), |row, (id, tooltip, f)| {
                    let f = f.clone();
                    row.child(
                        h_flex()
                            .id(("kv-delete-slot", ix))
                            .test_support()
                            .min_h(ROW_HEIGHT)
                            .child(action_slot(
                                ui::icon_button((*id, ix), IconName::Close, tooltip, None)
                                    .on_click(move |_, window, cx| f(ix, window, cx)),
                                ActionShow::OnHover,
                            )),
                    )
                })
        });
        v_flex()
            .id(self.id)
            .test_support()
            .w_full()
            .children(head)
            .children(rows)
    }
}

// Whether `text` in the mono font at the window's text size is wider than `width`.
fn cut_text(text: &SharedString, mono: &SharedString, width: Pixels, window: &Window) -> bool {
    let mut style = window.text_style();
    style.font_family = mono.clone();
    let size = style.font_size.to_pixels(window.rem_size());
    let run = style.to_run(text.len());
    let shaped = window
        .text_system()
        .shape_line(text.clone(), size, &[run], None);
    shaped.width > width
}

// A flexible cell's tooltip, which decides each frame it shows whether the cell is cut. GPUI
// renders the showing tooltip after the frame's prepaint, where the cell's canvas has recorded
// this frame's width, so a resize under a still pointer re-decides in the same frame.
struct FlexTip {
    text: SharedString,
    mono: SharedString,
    widths: Entity<Vec<Option<Pixels>>>,
    col: usize,
    tip: AnyView,
}

impl Render for FlexTip {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let room = self.widths.read(cx).get(self.col).copied().flatten();
        if room.is_some_and(|room| cut_text(&self.text, &self.mono, room, window)) {
            self.tip.clone().into_any_element()
        } else {
            div().into_any_element()
        }
    }
}

// The tooltip of a cut cell: exactly the cell's text, so a masked value stays masked.
fn cut_tip(text: &SharedString, window: &mut Window, cx: &mut App) -> AnyView {
    let text = text.clone();
    Tooltip::element(move |_, _| {
        div()
            .id("kv-cut-tooltip")
            .test_support()
            .aria_label(text.clone())
            .child(text.clone())
    })
    .build(window, cx)
}

/// The row that takes focus once row `ix` is gone from `rows`: the one now at `ix`, else the
/// one before it, else the table's empty row.
pub fn after_delete<'a, T>(rows: &'a [T], ix: usize, blank: &'a T) -> &'a T {
    rows.get(ix)
        .or(ix.checked_sub(1).and_then(|p| rows.get(p)))
        .unwrap_or(blank)
}

/// Where focus on a non-input control of row `ix` goes when a reload replaces `old` by `new`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reloaded {
    /// The row survives at this index, where the same control takes focus.
    Kept(usize),
    /// The row is gone; focus goes where deleting row `ix` sends it, see [`after_delete`].
    Dropped(usize),
}

/// Whether row `ix` of `old` survives into `new`, rows matched by `same`. A same-length reload
/// that one move explains (rows `pinned` judges true keep their slot) follows that move, as
/// an undone or redone move shows it. Otherwise the tables are aligned on content, so the row
/// follows through rows inserted, dropped or edited around it; a row with no match that sits
/// where one new row replaced it was edited in place and keeps that slot. Among alignments
/// that explain the reload, the one leaving the row nearest its old index wins.
pub fn after_reload<T>(
    old: &[T],
    new: &[T],
    ix: usize,
    pinned: impl Fn(&T) -> bool,
    same: impl Fn(&T, &T) -> bool,
) -> Reloaded {
    if new.len() == old.len()
        && let Some(to) = moved(old, new, ix, &pinned, &same)
    {
        return Reloaded::Kept(to);
    }
    aligned(old, new, ix, &same)
}

/// Where row `ix` of `old` goes under a longest common subsequence of `old` and `new`.
fn aligned<T>(old: &[T], new: &[T], ix: usize, same: impl Fn(&T, &T) -> bool) -> Reloaded {
    let Some(row) = old.get(ix) else {
        return Reloaded::Dropped(ix);
    };
    let (n, m) = (old.len(), new.len());
    // `before[i][j]` is the common subsequence length of old[..i] and new[..j], `after[i][j]`
    // that of old[i..] and new[j..]; a pairing lies on a longest alignment when the two sides
    // around it add up to the best length.
    let mut before = vec![vec![0usize; m + 1]; n + 1];
    for i in 0..n {
        for j in 0..m {
            before[i + 1][j + 1] = match same(&old[i], &new[j]) {
                true => before[i][j] + 1,
                false => before[i][j + 1].max(before[i + 1][j]),
            };
        }
    }
    let mut after = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            after[i][j] = match same(&old[i], &new[j]) {
                true => after[i + 1][j + 1] + 1,
                false => after[i + 1][j].max(after[i][j + 1]),
            };
        }
    }
    let best = before[n][m];
    let nearest =
        |fits: &dyn Fn(usize) -> bool| (0..m).filter(|&j| fits(j)).min_by_key(|j| j.abs_diff(ix));
    let matched =
        nearest(&|j| same(row, &new[j]) && before[ix][j] + 1 + after[ix + 1][j + 1] == best);
    // Unmatched, the row may still face one unmatched new row with the same alignment on
    // both sides: that is the row edited in place.
    let edited = || nearest(&|j| before[ix][j] + after[ix + 1][j + 1] == best);
    match matched.or_else(edited) {
        Some(j) => Reloaded::Kept(j),
        None => Reloaded::Dropped(ix),
    }
}

/// Where row `ix` of `old` sits in `new` when one row's move turns `old` into `new`; that row
/// may be another one, whose move shifts row `ix`.
fn moved<T>(
    old: &[T],
    new: &[T],
    ix: usize,
    pinned: impl Fn(&T) -> bool,
    same: impl Fn(&T, &T) -> bool,
) -> Option<usize> {
    // Twin rows can let several moves explain the reload; the one shifting row `ix` least
    // wins, so focus never jumps to an identical row that did not move.
    (0..old.len())
        .filter_map(|from| {
            let dest = crate::row_move::follow(old, new, from, &pinned, &same)?;
            let mut order: Vec<usize> = (0..old.len()).collect();
            crate::row_move::reorder_pinned(&mut order, |&k| pinned(&old[k]), from, dest);
            order.iter().position(|&k| k == ix)
        })
        .min_by_key(|to| to.abs_diff(ix))
}
