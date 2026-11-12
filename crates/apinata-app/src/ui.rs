//! Small presentational primitives composed from the kit's components: method tags,
//! status pills, the unsaved dot, icon buttons, segmented choices, checkboxes, form rows, count
//! badges, empty states and overflow menus for tab strips.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::AsKeystroke as _;
use gpui_kit::EntityInputHandler as _;
use gpui_kit::Focusable as _;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Checkbox as BaseCheckbox, CheckboxIndicator, Radio as BaseRadio};
use gpui_kit::component::Colorize as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::Dialog;
use gpui_kit::component::input::{Editor, EditorState, Input, InputState, Textarea, TextareaState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::{Scrollbar, ScrollbarMode};
use gpui_kit::component::tab::TabBar;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, ThemeStyled as _, h_flex,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Action, Anchor, AnyElement, App, BorderStyle, Bounds, BoxShadow, Corners, Div, ElementId,
    Entity, FocusHandle, FontFeatures, FontWeight, Hsla, InteractiveElement as _, IntoElement,
    MouseButton, ParentElement as _, Pixels, Rems, Render, RenderOnce, ScrollEvent, ScrollHandle,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _,
    Window, actions, canvas, div, fill, linear_color_stop, linear_gradient, outline, px, relative,
    svg, transparent_black,
};

use crate::app::AppState;
use crate::git_status::GitChange;
use crate::keymap::SHELL_CONTEXT;
pub use crate::kv_grid::{KvCell, KvView, kv_view};
use crate::style::{
    CHECKBOX_SIZE, DIALOG_CLOSE_INSET, DIALOG_FOOTER_GAP, DIALOG_PADDING, DISABLED_OPACITY, DOT,
    FOCUS_RING_ROOM, FORM_LABEL_LINE_HEIGHT, FORM_LABEL_WIDTH, FORM_STACK_WIDTH, INPUT_HEIGHT,
    METHOD_COLUMN, RADIUS, SELECT_CHEVRON, SELECT_MAX_WIDTH, SPACE_1, SPACE_2, SPACE_3, SPACE_5,
    STRIP_FADE_WIDTH, TEXT_CONTROL, TEXT_SMALL, TEXT_UI, kit_size, resolve,
};
use crate::theme::{self, SelectedStyle, Status, selected_look};
use crate::zoom;

/// The four-character-or-shorter label for an HTTP method, case-insensitive. Known
/// methods get their usual abbreviation; anything else keeps its first four characters.
pub fn short_method(method: &str) -> SharedString {
    let upper = method.to_uppercase();
    match upper.as_str() {
        "GET" | "POST" | "PUT" | "HEAD" => upper.into(),
        "PATCH" => "PTCH".into(),
        "DELETE" => "DEL".into(),
        "OPTIONS" => "OPT".into(),
        "QUERY" => "QURY".into(),
        _ => upper.chars().take(4).collect::<String>().into(),
    }
}

/// `text` with every backtick turned into a straight double quote, so a name that core
/// messages fence in backticks reads as a quoted name on screen.
pub fn plain_quotes(text: &str) -> String {
    text.replace('`', "\"")
}

/// Ends the typing run of `input`'s own undo, so a later keystroke undoes apart from earlier
/// ones (at a save or a reload). A selection already ended the run and is left alone, and so
/// is an open IME composition: its commit is an undo step of its own, and unselecting would
/// move the caret inside it.
pub fn end_input_run(input: &Entity<InputState>, window: &mut Window, cx: &mut App) {
    input.update(cx, |s, cx| {
        if s.selected_range().is_empty() && s.marked_text_range(window, cx).is_none() {
            s.unselect(window, cx);
        }
    });
}

/// [`end_input_run`] for a code editor.
pub fn end_editor_run(editor: &Entity<EditorState>, window: &mut Window, cx: &mut App) {
    editor.update(cx, |s, cx| {
        if s.selected_range().is_empty() && s.marked_text_range(window, cx).is_none() {
            s.unselect(window, cx);
        }
    });
}

/// Shows the model's `text` in `input`, replacing its value only when they differ. An open IME
/// composition is typing in progress, so it is kept while the text around it is `text`.
pub fn sync_input_text(input: &Entity<InputState>, text: &str, window: &mut Window, cx: &mut App) {
    input.update(cx, |s, cx| {
        if committed_text(&s.value(), s.marked_text_range(window, cx)) != text {
            s.set_value(text.to_string(), window, cx);
        }
    });
}

/// [`sync_input_text`] for a code editor.
pub fn sync_editor_text(
    editor: &Entity<EditorState>,
    text: &str,
    window: &mut Window,
    cx: &mut App,
) {
    editor.update(cx, |s, cx| {
        if committed_text(&s.value(), s.marked_text_range(window, cx)) != text {
            s.set_value(text.to_string(), window, cx);
        }
    });
}

/// `value` without the IME composition at `marked`, a UTF-16 range as the input handler gives
/// it. A range that does not fit `value` leaves it whole.
pub fn committed_text(value: &str, marked: Option<std::ops::Range<usize>>) -> Cow<'_, str> {
    let Some(marked) = marked.filter(|m| !m.is_empty()) else {
        return Cow::Borrowed(value);
    };
    // The kit counts in UTF-16 units; slicing needs byte offsets.
    let byte_at = |utf16: usize| {
        let mut units = 0;
        for (byte, ch) in value.char_indices() {
            if units == utf16 {
                return Some(byte);
            }
            units += ch.len_utf16();
        }
        (units == utf16).then_some(value.len())
    };
    match (byte_at(marked.start), byte_at(marked.end)) {
        (Some(start), Some(end)) if start <= end => {
            Cow::Owned(format!("{}{}", &value[..start], &value[end..]))
        }
        _ => Cow::Borrowed(value),
    }
}

/// `input` locked or not. A locked input is read-only in every theme, so a click focuses it,
/// the ring shows and its text selects and copies; the kit dims only a disabled one. Standard
/// themes dim the text through [`locked_text_color`] until it is focused. While focused it carries a lock icon,
/// since the kit cannot hide the caret of a focused read-only input.
pub fn lock_input(
    input: Input,
    state: &Entity<InputState>,
    locked: bool,
    window: &Window,
    cx: &App,
) -> Input {
    // A selection exists only while focused, so a focused input draws the full foreground
    // and muted text is never painted on the selection.
    let focused = state.read(cx).focus_handle(cx).is_focused(window);
    let text = if focused {
        locked.then(|| cx.theme().foreground)
    } else {
        locked_text_color(locked, cx)
    };
    let input = input
        .readonly(locked)
        .when_some(text, |input, color| input.text_color(color));
    with_lock_hint(input, state, locked, window, cx)
}

/// Whether the active theme is a high-contrast one, where disabled controls cannot dim and
/// carry the muted foreground instead, with an edge no heavier than the enabled one's.
fn full_strength(cx: &App) -> bool {
    crate::theme::disabled_text_alpha(cx.theme().theme_name()) >= 1.0
}

/// The text colour a disabled button takes over the kit's own, or `None` to keep the kit's
/// `muted_foreground` at half alpha. The high-contrast themes need full strength to reach 4.5:1.
pub fn disabled_button_text(theme: &gpui_kit::component::Theme) -> Option<Hsla> {
    (crate::theme::disabled_text_alpha(theme.theme_name()) >= 1.0).then_some(theme.muted_foreground)
}

/// A button that can be marked selected with a label and fill the contrast test measures.
pub trait ChooseButton: Sized {
    /// `self` selected or not; while selected it draws [`selected_look`] for `style`, which
    /// is the only way a view marks a button selected.
    fn choose(self, selected: bool, style: SelectedStyle, cx: &App) -> Self {
        self.chosen(selected, selected_look(cx.theme(), style))
    }

    /// `self` selected or not, drawing `look` (a [`selected_look`] result) while selected.
    fn chosen(self, selected: bool, look: (Hsla, Hsla)) -> Self;
}

impl ChooseButton for Button {
    fn chosen(self, selected: bool, (text, fill): (Hsla, Hsla)) -> Self {
        self.selected(selected)
            .when(selected, |b| b.text_color(text).bg(fill))
    }
}

/// The edge a disabled control takes in High Contrast: `input` leaned toward the page, which
/// keeps 3:1 there yet stays lighter than the enabled edge.
pub fn disabled_edge(cx: &App) -> Hsla {
    let t = cx.theme();
    t.input.blend(t.background.opacity(0.25))
}

/// A button that can be disabled without the kit's half-alpha label.
pub trait LockButton: Sized {
    /// `self` disabled or not, with [`disabled_button_text`] and, in High Contrast, the
    /// [`disabled_edge`] colour applied while it is disabled. The `locked` marker child it adds
    /// then makes a locked Small or Large icon-only button widen, since the kit sizes one square
    /// only without children.
    fn lock(self, disabled: bool, cx: &App) -> Self;

    /// [`LockButton::lock`] for a filled, borderless button such as a primary: while disabled
    /// in High Contrast it also draws a 1px inset edge in the [`disabled_edge`] colour.
    fn lock_filled(self, disabled: bool, cx: &App) -> Self;
}

impl LockButton for Button {
    fn lock(self, disabled: bool, cx: &App) -> Self {
        let text = disabled.then(|| disabled_button_text(cx.theme())).flatten();
        // Only the colour changes; variants the kit draws borderless stay without an edge.
        self.disabled(disabled)
            .when_some(text, |button, color| {
                button.text_color(color).border_color(disabled_edge(cx))
            })
            .when(disabled, |button| button.child(LockedMarker::new()))
    }

    fn lock_filled(self, disabled: bool, cx: &App) -> Self {
        // An inset shadow outlines the fill without a border width, so the layout never shifts.
        let edged = disabled && full_strength(cx);
        self.lock(disabled, cx).when(edged, |button| {
            let edge = BoxShadow::new(px(0.), px(0.), disabled_edge(cx))
                .spread_radius(px(1.))
                .inset();
            button.shadow(vec![edge])
        })
    }
}

/// An out-of-flow child, id `locked`, covering a locked button: the kit's button reports no
/// disabled flag, so this marker carries it for snapshots and the pilot tree.
struct LockedMarker(Stateful<Div>);

impl LockedMarker {
    fn new() -> gpui_kit::base::ObservedElement<Self> {
        LockedMarker(div().id("locked").absolute().top_0().left_0().size_full()).test_support()
    }
}

impl IntoElement for LockedMarker {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl gpui_kit::Element for LockedMarker {
    type RequestLayoutState = <Stateful<Div> as gpui_kit::Element>::RequestLayoutState;
    type PrepaintState = <Stateful<Div> as gpui_kit::Element>::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        gpui_kit::Element::id(&self.0)
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        gpui_kit::Element::source_location(&self.0)
    }

    fn request_layout(
        &mut self,
        id: Option<&gpui_kit::GlobalElementId>,
        inspector: Option<&gpui_kit::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui_kit::LayoutId, Self::RequestLayoutState) {
        self.0.request_layout(id, inspector, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&gpui_kit::GlobalElementId>,
        inspector: Option<&gpui_kit::InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.0.prepaint(id, inspector, bounds, layout, window, cx)
    }

    fn paint(
        &mut self,
        id: Option<&gpui_kit::GlobalElementId>,
        inspector: Option<&gpui_kit::InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.0
            .paint(id, inspector, bounds, layout, prepaint, window, cx)
    }

    fn write_a11y_info(&self, node: &mut gpui_kit::accesskit::Node) {
        gpui_kit::Element::write_a11y_info(&self.0, node);
        node.set_disabled();
    }
}

impl gpui_kit::InteractiveElement for LockedMarker {
    fn interactivity(&mut self) -> &mut gpui_kit::Interactivity {
        self.0.interactivity()
    }
}

/// `textarea` locked or not, like [`lock_input`]: read-only in every theme, so a click focuses
/// it, the ring shows and its text selects, with muted text while unfocused and the full
/// foreground while focused. The kit's textarea takes no suffix, so it carries no lock icon.
pub fn lock_textarea(
    textarea: Textarea,
    state: &Entity<TextareaState>,
    locked: bool,
    window: &Window,
    cx: &App,
) -> Textarea {
    // Muted text never sits on a selection, which exists only while focused.
    let focused = state.read(cx).focus_handle(cx).is_focused(window);
    let text = if focused {
        locked.then(|| cx.theme().foreground)
    } else {
        locked_text_color(locked, cx)
    };
    textarea
        .readonly(locked)
        .when_some(text, |t, color| t.text_color(color))
}

/// `input` read-only in every theme when `locked`, for a value that must keep reading as
/// normal text. It carries the same lock icon as [`lock_input`] while focused.
pub fn readonly_input(
    input: Input,
    state: &Entity<InputState>,
    locked: bool,
    window: &Window,
    cx: &App,
) -> Input {
    with_lock_hint(input.readonly(locked), state, locked, window, cx)
}

fn with_lock_hint(
    input: Input,
    state: &Entity<InputState>,
    locked: bool,
    window: &Window,
    cx: &App,
) -> Input {
    let focused = locked && state.read(cx).focus_handle(cx).is_focused(window);
    input.when(focused, |input| input.suffix(lock_hint(state, cx)))
}

/// The lock icon's colour: full foreground, so it clears 3:1 on the locked fill in every theme.
pub fn lock_hint_color(cx: &App) -> Hsla {
    cx.theme().foreground
}

/// The text colour a locked input takes: `muted_foreground` in every theme, `None` (the
/// theme's own) when unlocked.
pub fn locked_text_color(locked: bool, cx: &App) -> Option<Hsla> {
    locked.then(|| cx.theme().muted_foreground)
}

/// The lock icon a focused locked input shows; tests find it by `("lock-hint", state id)`.
fn lock_hint(state: &Entity<InputState>, cx: &App) -> impl IntoElement + use<> {
    div()
        .id(("lock-hint", state.entity_id()))
        .test_support()
        .flex_none()
        .child(
            svg()
                .path(IconName::Lock.path())
                .size_3()
                .text_color(lock_hint_color(cx)),
        )
}

/// A table cell's input: small, monospace, on the [`field_fill`], so every key-value, assertion
/// and environment cell looks the same. Callers add the id and width.
pub fn table_input(input: &Entity<InputState>, disabled: bool, window: &Window, cx: &App) -> Input {
    lock_input(Input::new(input).small(), input, disabled, window, cx)
        .bg(field_fill(disabled, cx))
        .font_family(cx.theme().mono_font_family.clone())
}

/// Lets a press in the padding around `state`'s text act as a press on the text. The kit's
/// input takes mouse-downs only on the text element, so the frame's padding would otherwise
/// focus the frame instead (no caret, no drag, no word select). A press outside the text is
/// replayed, clamped just inside it. The replay is deferred, so it lands after the kit's own
/// handler whatever order the listeners ran in.
pub fn forward_padding_press(cell: Div, state: &Entity<InputState>) -> Div {
    let state = state.clone();
    cell.on_mouse_down(MouseButton::Left, move |e, window, cx| {
        replay_padding_press(&state, e, window, cx, None)
    })
}

/// Pointer events that arrived between a press and its deferred replay.
#[derive(Default)]
struct Held {
    pending: bool,
    last_move: Option<gpui_kit::MouseMoveEvent>,
    up: Option<gpui_kit::MouseUpEvent>,
}

/// Replays `e` just inside `state`'s text when it landed outside the text.
fn replay_padding_press(
    state: &Entity<InputState>,
    e: &gpui_kit::MouseDownEvent,
    window: &mut Window,
    cx: &mut App,
    held: Option<&std::rc::Rc<std::cell::RefCell<Held>>>,
) {
    use gpui_kit::InputEvent as _;
    let text = state.read(cx).input_bounds();
    if text.size.width <= px(2.) || text.contains(&e.position) {
        return;
    }
    let mut replay = e.clone();
    replay.position.x = e
        .position
        .x
        .clamp(text.left() + px(1.), text.right() - px(1.));
    replay.position.y = e
        .position
        .y
        .clamp(text.top() + px(1.), text.bottom() - px(1.));
    if let Some(held) = held {
        *held.borrow_mut() = Held {
            pending: true,
            ..Held::default()
        };
    }
    let held = held.cloned();
    window.defer(cx, move |window, cx| {
        window.dispatch_event(replay.to_platform_input(), cx);
        // A driver that sends the drag back to back reached the kit before the replay set it
        // selecting; once the replay's focus has settled, replay the last held move and release.
        let Some(held) = held else { return };
        window.defer(cx, move |window, cx| {
            let Held { last_move, up, .. } = std::mem::take(&mut *held.borrow_mut());
            for event in [
                last_move.map(|m| m.to_platform_input()),
                up.map(|u| u.to_platform_input()),
            ]
            .into_iter()
            .flatten()
            {
                window.dispatch_event(event, cx);
            }
        });
    });
}

/// `input` with [`forward_padding_press`] applied across its whole frame. The wrapper is a
/// transparent element: it lends the input's layout as its own, so the sizing a caller put on
/// the input still decides how it lays out. Every input the app draws goes through it.
pub fn padded_input(input: Input, state: &Entity<InputState>) -> PaddedInput {
    use gpui_kit::component::FocusableExt as _;
    PaddedInput {
        ring: input.is_focus_ring_enabled(),
        child: input.into_any_element(),
        state: state.clone(),
    }
}

/// The danger-coloured CircleAlert that marks an invalid input at its trailing edge.
pub fn invalid_icon(id: ElementId, danger: Hsla) -> impl IntoElement {
    div().id(id).test_support().flex_none().child(
        svg()
            .path(IconName::CircleAlert.path())
            .size_3()
            .text_color(danger),
    )
}

/// A refused name's reason, floating just under its field: placed after the field in an
/// empty box, it anchors at the field's bottom edge and shifts no row below it.
pub fn field_refusal(
    id: &'static str,
    message: SharedString,
    cx: &App,
) -> impl IntoElement + use<> {
    gpui_kit::deferred(
        gpui_kit::anchored().snap_to_window().child(
            refusal_bubble(id, message, cx)
                .max_w(crate::style::VAR_TIP_MAX_WIDTH)
                .test_support(),
        ),
    )
}

/// A refused name's reason as [`field_refusal`] draws it, but in the layout under its field:
/// it takes its column's width and wraps there, for a column too narrow to float it over.
pub fn field_refusal_in_column(
    id: &'static str,
    message: SharedString,
    cx: &App,
) -> impl IntoElement + use<> {
    refusal_bubble(id, message, cx).w_full().test_support()
}

fn refusal_bubble(id: &'static str, message: SharedString, cx: &App) -> Stateful<Div> {
    div()
        .id(id)
        .aria_label(message.clone())
        .px(crate::style::SPACE_2)
        .py(crate::style::SPACE_1)
        .rounded(crate::style::POPOVER_RADIUS)
        .border_1()
        .border_color(cx.theme().danger)
        .bg(cx.theme().popover)
        .text_color(cx.theme().danger)
        .text_size(crate::style::TEXT_SMALL)
        .child(message)
}

/// An `input` in `frame`, marked while it is invalid: a danger-coloured CircleAlert icon at
/// the input's trailing edge, focused or not, so the shape alone tells it from a valid input
/// in greyscale and the High Contrast themes. While it is also focused, a 2px danger ring
/// covers the frame. The icon is the input's suffix, present in both states so text never
/// shifts on a focus change. `frame` carries the caller's layout (width, border); tests find
/// the overlays by the two ids.
pub fn invalid_input(
    input: Input,
    frame: Div,
    state: &Entity<InputState>,
    invalid: bool,
    (ring_id, icon_id): (ElementId, ElementId),
    window: &Window,
    cx: &App,
) -> Div {
    let focused = state.read(cx).focus_handle(cx).is_focused(window);
    let danger = cx.theme().danger;
    let input = input.when(invalid, |input| input.suffix(invalid_icon(icon_id, danger)));
    frame
        .relative()
        .child(padded_input(input, state))
        .when(invalid && focused, |frame| {
            frame.child(danger_ring(ring_id, danger))
        })
}

/// Like [`invalid_input`], for a required input left blank: its danger ring shows whether
/// or not it is focused, so every blank field stands out at once.
pub fn missing_input(
    input: Input,
    frame: Div,
    state: &Entity<InputState>,
    missing: bool,
    (ring_id, icon_id): (ElementId, ElementId),
    window: &Window,
    cx: &App,
) -> Div {
    let focused = state.read(cx).focus_handle(cx).is_focused(window);
    let danger = cx.theme().danger;
    let ring = ring_id.clone();
    invalid_input(input, frame, state, missing, (ring_id, icon_id), window, cx)
        .when(missing && !focused, |frame| {
            frame.child(danger_ring(ring, danger))
        })
}

// The 2px danger-coloured outline laid over an input's frame.
fn danger_ring(id: ElementId, danger: Hsla) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .absolute()
        .inset_0()
        .rounded(crate::style::RADIUS)
        .border_2()
        .border_color(danger)
}

/// The element [`padded_input`] returns.
pub struct PaddedInput {
    child: AnyElement,
    state: Entity<InputState>,
    /// Whether a pointer-focused input paints the accent ring over its own frame.
    ring: bool,
}

impl PaddedInput {
    /// For an input that draws no frame of its own (`appearance(false)`), whose wrapper
    /// shows focus instead.
    pub fn frameless(mut self) -> Self {
        self.ring = false;
        self
    }
}

impl IntoElement for PaddedInput {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl gpui_kit::Element for PaddedInput {
    type RequestLayoutState = ();
    type PrepaintState = Option<gpui_kit::Hitbox>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&gpui_kit::GlobalElementId>,
        _: Option<&gpui_kit::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui_kit::LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&gpui_kit::GlobalElementId>,
        _: Option<&gpui_kit::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Option<gpui_kit::Hitbox> {
        self.child.prepaint(window, cx);
        Some(window.insert_hitbox(bounds, gpui_kit::HitboxBehavior::Normal))
    }

    fn paint(
        &mut self,
        _: Option<&gpui_kit::GlobalElementId>,
        _: Option<&gpui_kit::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        hitbox: &mut Option<gpui_kit::Hitbox>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
        if self.ring {
            let input = self.state.read(cx);
            let (focus, disabled) = (input.focus_handle(cx), input.presentation().is_disabled());
            paint_pointer_focus(bounds, &focus, disabled, window, cx);
        }
        let (Some(hitbox), state) = (hitbox.clone(), self.state.clone()) else {
            return;
        };
        let held = std::rc::Rc::new(std::cell::RefCell::new(Held::default()));
        window.on_mouse_event({
            let held = Rc::clone(&held);
            move |e: &gpui_kit::MouseMoveEvent, _, _, _| {
                let mut held = held.borrow_mut();
                if held.pending && e.pressed_button == Some(MouseButton::Left) {
                    held.last_move = Some(e.clone());
                }
            }
        });
        window.on_mouse_event({
            let held = Rc::clone(&held);
            move |e: &gpui_kit::MouseUpEvent, _, _, _| {
                let mut held = held.borrow_mut();
                if held.pending && e.button == MouseButton::Left {
                    held.up = Some(e.clone());
                }
            }
        });
        // Bubble listeners run last-registered first, so this one runs before the kit's own;
        // the deferred replay is what puts the forwarded press after it.
        window.on_mouse_event(move |e: &gpui_kit::MouseDownEvent, phase, window, cx| {
            if phase == gpui_kit::DispatchPhase::Bubble
                && e.button == MouseButton::Left
                && hitbox.is_hovered(window)
            {
                replay_padding_press(&state, e, window, cx, Some(&held));
            }
        });
    }
}

/// `textarea` with the accent ring a pointer-focused input paints, as [`padded_input`] gives
/// an input. The wrapper lends the textarea its layout and adds no press handling: the kit
/// already takes presses in a textarea's padding.
pub fn ringed_textarea(textarea: Textarea, state: &Entity<TextareaState>) -> RingedField {
    RingedField {
        child: textarea.into_any_element(),
        source: RingSource::Textarea(state.clone()),
        by_key: false,
    }
}

/// The key context every [`ringed_editor`] wraps its editor in, where the keys that leave a
/// code editor bind.
pub const CODE_EDITOR_CONTEXT: &str = "CodeEditor";

actions!(
    apinata,
    [
        /// Moves focus out of a code editor to the next control, as Tab does elsewhere.
        FocusNextControl,
        /// Moves focus out of a code editor to the previous control.
        FocusPreviousControl
    ]
);

// Leaving goes through the kit root's own Tab actions, so a dialog's focus trap still holds;
// they are private to the kit, hence the lookup by name.
pub(crate) fn leave_editor(forward: bool, window: &mut Window, cx: &mut App) {
    let name = if forward {
        "root::Tab"
    } else {
        "root::TabPrev"
    };
    match cx.build_action(name, None) {
        Ok(action) => window.dispatch_action(action, cx),
        Err(_) if forward => window.focus_next(cx),
        Err(_) => window.focus_prev(cx),
    }
    // The dispatch is deferred, so the keystroke check has already seen focus unmoved.
    crate::focus_mode::key_moves_focus(window, cx);
}

/// `editor` with the 2px accent ring while it holds focus, by pointer and by key alike, since
/// the kit's editor draws no focus ring of its own. A disabled editor shows none; a readonly
/// one keeps it, as it still takes focus. The wrapper lends the editor its layout.
pub fn ringed_editor(editor: Editor, state: &Entity<EditorState>) -> RingedField {
    RingedField {
        child: editor.into_any_element(),
        source: RingSource::Editor(state.clone()),
        by_key: true,
    }
}

/// The state a ringed field reads its focus and disabled flag from at paint time.
enum RingSource {
    Textarea(Entity<TextareaState>),
    Editor(Entity<EditorState>),
}

/// The paint-only element [`ringed_textarea`] and [`ringed_editor`] return: it adds no layout
/// node, so the ring costs no relayout as focus moves.
pub struct RingedField {
    child: AnyElement,
    source: RingSource,
    by_key: bool,
}

impl IntoElement for RingedField {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl gpui_kit::Element for RingedField {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&gpui_kit::GlobalElementId>,
        _: Option<&gpui_kit::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui_kit::LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&gpui_kit::GlobalElementId>,
        _: Option<&gpui_kit::InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&gpui_kit::GlobalElementId>,
        _: Option<&gpui_kit::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        // The context and listeners go on this element's own dispatch node, the editor's
        // parent, so they must be set before the child's paint moves to its node.
        if let RingSource::Editor(_) = self.source {
            let mut context = gpui_kit::KeyContext::default();
            context.add(CODE_EDITOR_CONTEXT);
            window.set_key_context(context);
            window.on_action(
                std::any::TypeId::of::<FocusNextControl>(),
                |_, phase, window, cx| {
                    if phase == gpui_kit::DispatchPhase::Bubble {
                        leave_editor(true, window, cx);
                    }
                },
            );
            window.on_action(
                std::any::TypeId::of::<FocusPreviousControl>(),
                |_, phase, window, cx| {
                    if phase == gpui_kit::DispatchPhase::Bubble {
                        leave_editor(false, window, cx);
                    }
                },
            );
        }
        self.child.paint(window, cx);
        let (focus, disabled) = match &self.source {
            RingSource::Textarea(s) => {
                let s = s.read(cx);
                (s.focus_handle(cx), s.presentation().is_disabled())
            }
            RingSource::Editor(s) => {
                let s = s.read(cx);
                (s.focus_handle(cx), s.presentation().is_disabled())
            }
        };
        // Without `by_key` the field leaves the keyboard ring to the kit, as a textarea does,
        // so it paints only by pointer; with it, the field paints in both modalities.
        let kit_rings = !self.by_key && crate::focus_mode::is_keyboard(cx);
        if !kit_rings && !disabled && focus.is_focused(window) {
            paint_accent_ring(bounds, window, cx);
        }
    }
}

/// A text field keeps showing focus after a click, as on the web. The kit draws the focus
/// edge in `ring`, which a pointer press swaps for the plain border so buttons stay quiet, so
/// a focused input paints the accent ring itself while the pointer last moved focus.
fn paint_pointer_focus(
    bounds: Bounds<Pixels>,
    focus: &FocusHandle,
    disabled: bool,
    window: &mut Window,
    cx: &App,
) {
    if crate::focus_mode::is_keyboard(cx) || disabled || !focus.is_focused(window) {
        return;
    }
    paint_accent_ring(bounds, window, cx);
}

/// The accent ring's width at `rem`: 2 px at 100%, growing with zoom, never thinner.
pub fn accent_ring_width(rem: Pixels) -> Pixels {
    crate::style::snap(crate::style::ui(2.).to_pixels(rem)).max(px(2.))
}

/// The accent ring, at `bounds` with the theme's corner radius, painted inside them.
fn paint_accent_ring(bounds: Bounds<Pixels>, window: &mut Window, cx: &App) {
    let theme = cx.theme();
    let mut edge = outline(
        bounds,
        crate::focus_mode::accent_ring(theme),
        BorderStyle::Solid,
    )
    .corner_radii(Corners::all(theme.radius));
    edge.border_widths = gpui_kit::Edges::all(accent_ring_width(window.rem_size()));
    window.paint_quad(edge);
}

/// The fill every text input takes: the raised fill when editable, the muted fill when
/// disabled, so a locked field reads as locked in both themes.
pub fn field_fill(disabled: bool, cx: &App) -> Hsla {
    if disabled {
        cx.theme().muted
    } else {
        cx.theme().secondary
    }
}

/// The sides of a box that take [`RingRoom`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RingSides {
    /// The top side.
    pub top: bool,
    /// The right side.
    pub right: bool,
    /// The bottom side.
    pub bottom: bool,
    /// The left side.
    pub left: bool,
}

impl RingSides {
    /// Every side.
    pub const ALL: Self = Self::new(true, true, true, true);
    /// The top side only.
    pub const TOP: Self = Self::new(true, false, false, false);
    /// The right side only.
    pub const RIGHT: Self = Self::new(false, true, false, false);
    /// The bottom side only.
    pub const BOTTOM: Self = Self::new(false, false, true, false);
    /// The left side only.
    pub const LEFT: Self = Self::new(false, false, false, true);

    const fn new(top: bool, right: bool, bottom: bool, left: bool) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }
}

impl std::ops::BitOr for RingSides {
    type Output = Self;
    fn bitor(self, o: Self) -> Self {
        Self::new(
            self.top || o.top,
            self.right || o.right,
            self.bottom || o.bottom,
            self.left || o.left,
        )
    }
}

/// Room for the kit's keyboard ring, which paints `FOCUS_RING_ROOM` outside a control and is
/// cut wherever a clipping ancestor sits flush with that control.
pub trait RingRoom: gpui_kit::Styled + Sized {
    /// Pads the `sides` by the ring's room, so a child flush with them keeps its ring inside
    /// this box's clip. It replaces any padding already set on those sides, so a box that
    /// needs more sets the sum itself.
    fn ring_room(mut self, sides: RingSides) -> Self {
        let pad = self.style();
        let room = Some(gpui_kit::DefiniteLength::from(FOCUS_RING_ROOM));
        if sides.top {
            pad.padding.top = room;
        }
        if sides.right {
            pad.padding.right = room;
        }
        if sides.bottom {
            pad.padding.bottom = room;
        }
        if sides.left {
            pad.padding.left = room;
        }
        self
    }

    /// Grows this box out by the ring's room on `sides` and pads it back in, so its children
    /// keep their place while its clip moves out. The parent must have that room inside its
    /// own clip, and the box must stretch rather than set its size on those axes.
    fn ring_outset(mut self, sides: RingSides) -> Self {
        let out = Some(gpui_kit::Length::from(-FOCUS_RING_ROOM));
        let margin = &mut self.style().margin;
        if sides.top {
            margin.top = out;
        }
        if sides.right {
            margin.right = out;
        }
        if sides.bottom {
            margin.bottom = out;
        }
        if sides.left {
            margin.left = out;
        }
        self.ring_room(sides)
    }
}

impl<T: gpui_kit::Styled> RingRoom for T {}

/// A ring in the theme's ring colour over its `relative` parent, for a borderless kit
/// control that draws no focus ring of its own; tests find it by `id`.
pub fn ring_overlay(id: &'static str, cx: &App) -> impl IntoElement + use<> {
    div()
        .id(id)
        .test_support()
        .absolute()
        .inset_0()
        .border_1()
        .border_color(cx.theme().ring)
}

/// A monospace method label occupying the fixed method column, coloured by its method.
#[derive(IntoElement)]
pub struct MethodTag {
    label: SharedString,
    class: StatusClass,
    record: Option<(TagPaints, SharedString)>,
}

/// The text colour each recorded method tag applied in its latest render, by record id.
pub type TagPaints = Rc<RefCell<HashMap<SharedString, Hsla>>>;

impl MethodTag {
    /// The abbreviated method this tag shows.
    pub fn label(&self) -> &SharedString {
        &self.label
    }

    /// Replaces the shown label, keeping the colour of the tag's method.
    pub fn with_label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = label.into();
        self
    }

    /// The colour class of the tag's method.
    pub fn class(&self) -> StatusClass {
        self.class
    }

    /// The text colour: the class's theme colour, on a selected row as anywhere else.
    pub fn color(&self, cx: &App) -> Hsla {
        self.class.color(cx)
    }

    /// Writes the colour this tag applies into `paints` under `id`, from its own render.
    pub fn record(mut self, paints: &TagPaints, id: SharedString) -> Self {
        self.record = Some((Rc::clone(paints), id));
        self
    }
}

/// A method tag for `method`, abbreviated by [`short_method`].
pub fn method_tag(method: &str) -> MethodTag {
    MethodTag {
        label: short_method(method),
        class: method_class(method),
        record: None,
    }
}

impl RenderOnce for MethodTag {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let color = self.color(cx);
        if let Some((paints, id)) = self.record {
            paints.borrow_mut().insert(id, color);
        }
        div()
            .w(METHOD_COLUMN)
            .flex_none()
            .overflow_hidden()
            .whitespace_nowrap()
            .font_family(cx.theme().mono_font_family.clone())
            .text_size(TEXT_SMALL)
            .text_color(color)
            .child(self.label)
    }
}

/// The colour class of an HTTP status code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusClass {
    /// 2xx.
    Success,
    /// 3xx.
    Info,
    /// 4xx.
    Warning,
    /// 5xx.
    Danger,
    /// Anything outside 200 to 599.
    Muted,
}

/// Classifies `status` by its hundreds digit; informational and out-of-range codes are muted.
pub fn status_class(status: u16) -> StatusClass {
    match status {
        200..=299 => StatusClass::Success,
        300..=399 => StatusClass::Info,
        400..=499 => StatusClass::Warning,
        500..=599 => StatusClass::Danger,
        _ => StatusClass::Muted,
    }
}

/// Classifies an HTTP `method`, ignoring case. The theme has no purple, so PUT and PATCH
/// share the info colour; uncommon methods stay muted.
pub fn method_class(method: &str) -> StatusClass {
    match method.to_ascii_uppercase().as_str() {
        "GET" => StatusClass::Success,
        "POST" => StatusClass::Warning,
        "PUT" | "PATCH" => StatusClass::Info,
        "DELETE" => StatusClass::Danger,
        _ => StatusClass::Muted,
    }
}

impl StatusClass {
    /// The class's text colour in the current theme.
    pub fn color(self, cx: &App) -> Hsla {
        let t = cx.theme();
        match self {
            StatusClass::Success => t.success,
            StatusClass::Info => t.info,
            StatusClass::Warning => t.warning,
            StatusClass::Danger => t.danger,
            StatusClass::Muted => t.muted_foreground,
        }
    }
}

/// A status code and reason drawn in the status colour on its tinted fill.
#[derive(IntoElement)]
pub struct StatusPill {
    class: StatusClass,
    text: SharedString,
}

impl StatusPill {
    /// The colour class the pill renders with.
    pub fn class(&self) -> StatusClass {
        self.class
    }

    /// The pill's text: "200 OK", or just the code when the reason is empty.
    pub fn text(&self) -> SharedString {
        self.text.clone()
    }
}

/// A status pill for `status` and its `reason` phrase.
pub fn status_pill(status: u16, reason: &str) -> StatusPill {
    let text = if reason.is_empty() {
        status.to_string()
    } else {
        format!("{status} {reason}")
    };
    StatusPill {
        class: status_class(status),
        text: text.into(),
    }
}

impl RenderOnce for StatusPill {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let color = self.class.color(cx);
        let bg = match self.class {
            StatusClass::Success => theme::status_fill(Status::Success, cx),
            StatusClass::Info => theme::status_fill(Status::Info, cx),
            StatusClass::Warning => theme::status_fill(Status::Warning, cx),
            StatusClass::Danger => theme::status_fill(Status::Danger, cx),
            StatusClass::Muted => cx.theme().muted,
        };
        let t = cx.theme();
        div()
            .flex_none()
            .px(SPACE_2)
            .rounded(t.radius)
            .bg(bg)
            .text_color(color)
            .text_size(TEXT_SMALL)
            .child(self.text)
    }
}

/// The unsaved-changes dot: a filled circle in the theme foreground.
pub fn dirty_dot(id: impl Into<ElementId>) -> Stateful<Div> {
    // The builder has no `App`, so the theme is read when the canvas paints.
    div().id(id).size(DOT).flex_none().child(
        canvas(
            |_, _, _| {},
            |bounds, _, window, cx| {
                let color = cx.theme().foreground;
                window.paint_quad(
                    fill(bounds, color).corner_radii(Corners::all(resolve(DOT, window) / 2.)),
                );
            },
        )
        .size_full(),
    )
}

/// The roll-up mark of a collapsed folder: a muted dot, quieter than [`dirty_dot`], naming in
/// words that something inside changed.
pub fn rollup_dot(id: ElementId) -> impl IntoElement + use<> {
    div()
        .id(id)
        .test_support()
        .size(DOT)
        .flex_none()
        .aria_label("Changes inside")
        .child(
            canvas(
                |_, _, _| {},
                |bounds, _, window, cx| {
                    let color = cx.theme().muted_foreground;
                    window.paint_quad(
                        fill(bounds, color).corner_radii(Corners::all(resolve(DOT, window) / 2.)),
                    );
                },
            )
            .size_full(),
        )
        .tooltip(|window, cx| Tooltip::new("Changes inside").build(window, cx))
}

/// A tooltip of `text` whose element tests find by `id`, named by the text itself.
pub fn text_tooltip(
    id: impl Into<SharedString>,
    text: impl Into<SharedString>,
) -> impl Fn(&mut Window, &mut App) -> gpui_kit::AnyView + 'static {
    let (id, text): (SharedString, SharedString) = (id.into(), text.into());
    move |window, cx| {
        let (id, text) = (id.clone(), text.clone());
        Tooltip::element(move |_, _| {
            div()
                .id(id.clone())
                .test_support()
                .aria_label(text.clone())
                .child(text.clone())
        })
        .build(window, cx)
    }
}

/// A file's git letter, named in words by its label and tooltip so colour never carries it
/// alone: success for added and untracked, danger for conflicted, muted otherwise.
pub fn git_letter(id: ElementId, change: &GitChange, cx: &App) -> impl IntoElement + use<> {
    let color = match change {
        GitChange::Added | GitChange::Untracked => cx.theme().success,
        GitChange::Conflicted => cx.theme().danger,
        _ => cx.theme().muted_foreground,
    };
    let words = change.description();
    div()
        .id(id)
        .test_support()
        .flex_none()
        .aria_label(words)
        .text_color(color)
        .child(change.letter())
        .tooltip(move |window, cx| Tooltip::new(words).build(window, cx))
}

/// A request mode marker: a hollow circle the size of [`dirty_dot`], so a chosen mode never
/// reads as unsaved changes. `words` names it and is its hover text.
pub fn mode_ring(id: impl Into<ElementId>, words: &'static str) -> Stateful<Div> {
    // Painted like `dirty_dot`, since the builder has no `App` to read the theme from.
    div()
        .id(id)
        .size(DOT)
        .flex_none()
        .aria_label(words)
        .tooltip(move |window, cx| Tooltip::new(words).build(window, cx))
        .child(
            canvas(
                |_, _, _| {},
                |bounds, _, window, cx| {
                    let color = cx.theme().muted_foreground;
                    window.paint_quad(
                        outline(bounds, color, BorderStyle::Solid)
                            .corner_radii(Corners::all(resolve(DOT, window) / 2.)),
                    );
                },
            )
            .size_full(),
        )
}

/// The key context an [`icon_button`]'s tooltip finds its action's shortcut in. Each such
/// action is a Shell command, and naming the context keeps the hint when focus is elsewhere.
pub const ICON_TIP_CONTEXT: Option<&str> = Some(SHELL_CONTEXT);

thread_local! {
    /// The key context the latest action-carrying [`icon_button`] gave its tooltip.
    static LAST_TIP_CONTEXT: Cell<Option<Option<&'static str>>> = const { Cell::new(None) };
}

/// The key context the latest [`icon_button`] with an action gave its tooltip, on this
/// thread: `None` before any such button, `Some(None)` for a tooltip with no context. Tests
/// use it because the harness cannot see tooltips.
pub fn last_icon_tip_context() -> Option<Option<&'static str>> {
    LAST_TIP_CONTEXT.with(Cell::get)
}

thread_local! {
    /// The icon each response header and body bar icon control was last built with, by
    /// element id.
    static GLYPHS: RefCell<HashMap<&'static str, IconName>> = RefCell::new(HashMap::new());
}

/// Records that the response header or body bar control `id` shows `icon`, and hands the
/// icon back for the control's builder. Always on, since tests cannot see an svg's path.
/// It stores the icon, not its path, so an unchanged render builds and allocates nothing.
/// A new header or body bar icon control goes through here and into the id lists of
/// `no_two_shown_icon_controls_share_a_glyph`.
pub fn glyph(id: &'static str, icon: IconName) -> IconName {
    GLYPHS.with(|g| {
        let mut g = g.borrow_mut();
        if g.get(id) != Some(&icon) {
            g.insert(id, icon);
        }
    });
    icon
}

/// Every `(id, glyph path)` [`glyph`] recorded on this thread since the last clear.
pub fn recorded_glyphs() -> Vec<(&'static str, SharedString)> {
    GLYPHS.with(|g| g.borrow().iter().map(|(k, v)| (*k, v.path())).collect())
}

/// Forgets every glyph [`glyph`] recorded on this thread.
pub fn clear_recorded_glyphs() {
    GLYPHS.with(|g| g.borrow_mut().clear());
}

/// A compact ghost button showing `icon`. With an action, its tooltip shows the action's
/// key binding and a click dispatches the action from the focused element.
pub fn icon_button(
    id: impl Into<ElementId>,
    icon: IconName,
    tooltip: &str,
    action: Option<&dyn Action>,
) -> Button {
    // The kit names a button from its label or `accessibility_label`, never its tooltip, so
    // an icon-only button would be nameless without this.
    debug_assert!(!tooltip.trim().is_empty(), "an icon button needs a tooltip");
    let button = Button::new(id)
        .ghost()
        .compact()
        .icon(icon)
        .accessibility_label(tooltip.to_string());
    match action {
        Some(action) => {
            let owned = action.boxed_clone();
            let context = ICON_TIP_CONTEXT;
            LAST_TIP_CONTEXT.with(|c| c.set(Some(context)));
            button
                .tooltip_with_action(tooltip.to_string(), action, context)
                .on_click(move |_, window, cx| window.dispatch_action(owned.boxed_clone(), cx))
        }
        None => button.tooltip(tooltip.to_string()),
    }
}

/// An [`icon_button`] that flips between two faces under one element `id`, so its focus
/// handle, and with it keyboard focus, survives the flip. `face` names the current face on an
/// out-of-flow marker child that covers the button, for tests to find and click.
pub fn swap_icon_button(
    id: impl Into<ElementId>,
    face: impl Into<ElementId>,
    icon: IconName,
    tooltip: &str,
) -> Button {
    // An icon-only button sizes itself square only without children; the compact padding
    // around the one icon comes to the same square.
    icon_button(id, icon, tooltip, None).child(
        div()
            .id(face)
            .test_support()
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
    )
}

/// `label` followed by the key bound to `action`, as in "Run (Shift+Enter)", read from the
/// keymap in any context. For a tooltip of an action bound in a nested context, which the
/// kit's own lookup cannot name; plain `label` when the action has no key.
pub fn label_with_key(label: &str, action: &dyn Action, cx: &App) -> String {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let key = keymap
        .bindings_for_action(action)
        .next_back()
        .and_then(|b| b.keystrokes().first().map(|k| k.as_keystroke().clone()));
    match key {
        Some(key) => format!("{label} ({})", Kbd::format(&key)),
        None => label.to_string(),
    }
}

/// A kit button whose keyboard focus draws the app's solid ring, the colour and weight of a
/// tab's, in place of the kit's half-transparent band. A mouse press draws none.
pub fn solid_ring(button: Button, cx: &App) -> Button {
    use gpui_kit::component::FocusableExt as _;
    let ring = cx.theme().ring;
    // The border is reserved in every state, transparent until the ring shows, so focus
    // never moves the control's content.
    let button = button
        .focus_ring(false)
        .border_1()
        .border_color(gpui_kit::transparent_black());
    if crate::focus_mode::is_keyboard(cx) {
        button.focus(move |s| s.border_color(ring))
    } else {
        button
    }
}

/// Marks `control` as a title-bar control: a press that lands on it never arms the bar's
/// window move, whether or not the control marks the press handled.
pub fn no_window_move<E: gpui_kit::InteractiveElement>(control: E) -> E {
    control.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

/// How bad the news in a failure toast is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    /// Part of the work was done.
    Warning,
    /// None of the work was done.
    Error,
}

/// A neutral toast: a message with no type, so the kit draws no icon.
pub fn info_toast(message: impl Into<SharedString>) -> Notification {
    Notification::new().message(message)
}

/// A failure toast of `kind` saying `message`. It holds an empty marker, `toast-warning` or
/// `toast-error`, since the kit offers no way to read a drawn toast's kind.
pub fn kind_toast(kind: ToastKind, message: impl Into<SharedString>) -> Notification {
    let (toast, marker) = match kind {
        ToastKind::Warning => (Notification::warning(message), "toast-warning"),
        ToastKind::Error => (Notification::error(message), "toast-error"),
    };
    toast.content(move |_, _, _| div().id(marker).test_support().into_any_element())
}

/// A compact ghost chevron (id `id`) whose dropdown lists `labels` in order with `selected`
/// checked; choosing one calls `on_pick` with its index. For a strip whose tabs overflow.
pub fn strip_overflow(
    id: impl Into<ElementId>,
    labels: Vec<SharedString>,
    selected: usize,
    on_pick: impl Fn(usize, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let on_pick = Rc::new(on_pick);
    strip_chevron(
        Button::new(id)
            .ghost()
            .compact()
            .icon(IconName::ChevronDown)
            .accessibility_label("All tabs")
            .dropdown_menu_with_anchor(Anchor::TopRight, move |mut menu, window, cx| {
                menu = menu.scrollable(true);
                start_on(&menu, selected, window, cx);
                for (ix, label) in labels.iter().enumerate() {
                    let on_pick = Rc::clone(&on_pick);
                    menu = menu.item(
                        PopupMenuItem::new(label.clone())
                            .checked(ix == selected)
                            .on_click(move |_, window, cx| on_pick(ix, window, cx)),
                    );
                }
                menu
            }),
    )
}

/// A tab strip's overflow `chevron`, boxed with the ring's room on both sides: the strip ends
/// flush with a clipping edge on its right, and the tabs' viewport and its right fade, painted
/// after the chevron, end at its left. The room comes out of the viewport, not the window.
pub fn strip_chevron(chevron: impl IntoElement) -> impl IntoElement {
    div()
        .flex_none()
        .ring_room(RingSides::LEFT | RingSides::RIGHT)
        .child(chevron)
}

/// A compact outline button (id `id`) showing `current` with a chevron; its dropdown lists
/// `labels` in order with `selected` checked, and choosing one calls `on_pick` with its index.
pub fn select(
    id: impl Into<SharedString>,
    name: &str,
    current: SharedString,
    labels: Vec<SharedString>,
    selected: usize,
    on_pick: impl Fn(usize, &mut Window, &mut App) + 'static,
    cx: &App,
) -> impl IntoElement {
    let on_pick = Rc::new(on_pick);
    let id: SharedString = id.into();
    // The value text carries the shown name, so tests and assistive tools can read it.
    let value_id = SharedString::from(format!("{id}-value"));
    let chevron_id = SharedString::from(format!("{id}-chevron"));
    let accessible = format!("{name}: {current}");
    Button::new(id)
        .outline()
        .compact()
        .when_some(crate::theme::select_open_fill(cx.theme()), |b, fill| {
            b.bg(fill)
        })
        .max_w(SELECT_MAX_WIDTH)
        .accessibility_label(accessible)
        .child(
            h_flex()
                .gap(SPACE_1)
                .min_w_0()
                .child(
                    div()
                        .id(value_id)
                        .min_w_0()
                        .truncate()
                        .test_support()
                        .aria_label(current.clone())
                        .child(current),
                )
                // An svg paints only with a text colour of its own; it does not inherit one.
                .child(
                    svg()
                        .flex_none()
                        .path(IconName::ChevronDown.path())
                        .size(SELECT_CHEVRON)
                        .text_color(cx.theme().muted_foreground)
                        .id(chevron_id)
                        .test_support(),
                ),
        )
        .dropdown_menu_with_anchor(Anchor::TopLeft, move |mut menu, window, cx| {
            menu = menu.scrollable(true);
            start_on(&menu, selected, window, cx);
            for (ix, label) in labels.iter().enumerate() {
                let on_pick = Rc::clone(&on_pick);
                menu = menu.item(
                    PopupMenuItem::new(label.clone())
                        .checked(ix == selected)
                        .on_click(move |_, window, cx| on_pick(ix, window, cx)),
                );
            }
            menu
        })
}

/// Highlights item `selected` of a dropdown `menu` being built, so the first Down moves past
/// the checked item. The kit builds the menu while rendering and keeps its cursor private, so
/// once that frame is drawn, the kit's own Down steps the cursor there on the menu alone.
pub(crate) fn start_on(menu: &PopupMenu, selected: usize, window: &mut Window, cx: &mut App) {
    let handle = menu.focus_handle(cx);
    window.defer(cx, move |window, cx| {
        for _ in 0..=selected {
            handle.dispatch_action(&gpui_kit::base::actions::SelectDown, window, cx);
        }
    });
}

/// Whether the strip `scroll` tracks was wider than its box when last laid out.
pub fn strip_overflows(scroll: &ScrollHandle) -> bool {
    scroll.max_offset().x > px(0.)
}

/// A wheel handler for a kit tab strip, which scrolls only sideways: a purely vertical
/// trackpad swipe moves `handle` along x by the same amount, clamped to the strip. A mouse
/// wheel is the kit strip's own, and events with an x part are left to its scroller.
pub(crate) fn wheel_sideways(
    handle: &ScrollHandle,
) -> impl Fn(&ScrollEvent, &mut Window, &mut App) + 'static {
    let handle = handle.clone();
    move |event, window, cx| {
        let delta = event.delta.pixel_delta(window.line_height());
        if !event.delta.precise() || delta.x != px(0.) || delta.y == px(0.) {
            return;
        }
        let mut offset = handle.offset();
        let old = offset.x;
        offset.x = (old + delta.y).clamp(-handle.max_offset().x, px(0.));
        if offset.x == old {
            return;
        }
        // A strip that moved keeps the wheel from the scrolling page around it, whose
        // listeners run after this one; at either end the wheel bubbles on to the page.
        handle.set_offset(offset);
        cx.stop_propagation();
        window.refresh();
    }
}

/// A wheel handler for a strip inside a scrolling column: the strip moves only on horizontal
/// intent. An x delta is the strip's own scroller's to take; Shift with a vertical wheel is
/// mapped onto x here. Either way the wheel stops at the strip, since the column would read
/// an x delta as vertical. A plain vertical wheel is left to bubble to the column.
pub(crate) fn wheel_on_intent(
    handle: &ScrollHandle,
) -> impl Fn(&ScrollEvent, &mut Window, &mut App) + 'static {
    let handle = handle.clone();
    move |event, window, cx| {
        let delta = event.delta.pixel_delta(window.line_height());
        // A mostly vertical swipe with a little x noise still belongs to the column.
        if delta.x != px(0.) && delta.x.abs() >= delta.y.abs() {
            cx.stop_propagation();
            return;
        }
        if !event.modifiers.shift || delta.y == px(0.) {
            return;
        }
        let mut offset = handle.offset();
        let old = offset.x;
        offset.x = (old + delta.y).clamp(-handle.max_offset().x, px(0.));
        if offset.x == old {
            return;
        }
        handle.set_offset(offset);
        cx.stop_propagation();
        window.refresh();
    }
}

/// Whether the column `scroll` tracks has content below its viewport, from the last layout.
pub fn more_below(scroll: &ScrollHandle) -> bool {
    let max = scroll.max_offset().y;
    max > px(1.) && scroll.offset().y > -max + px(1.)
}

/// The quiet cue that a column scrolls: a fade into the background along its bottom edge
/// (id `id`) while content lies below, plus a marker that asks one more frame when this
/// frame's layout disagrees with what was drawn. Add both inside the column's `relative()`
/// wrapper, after the column.
pub fn scroll_cue(id: &'static str, scroll: &ScrollHandle, cx: &App) -> Vec<AnyElement> {
    let drawn = more_below(scroll);
    let handle = scroll.clone();
    let watch = canvas(
        move |_, window, _| {
            if more_below(&handle) != drawn {
                window.request_animation_frame();
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_0()
    .into_any_element();
    let mut out = vec![watch];
    if drawn {
        let background = cx.theme().background;
        out.push(
            div()
                .id(id)
                .test_support()
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .h(STRIP_FADE_WIDTH)
                .bg(linear_gradient(
                    180.,
                    linear_color_stop(background.opacity(0.), 0.),
                    linear_color_stop(background, 1.),
                ))
                .into_any_element(),
        );
    }
    out
}

/// Whether the box `scroll` tracks is wider than its viewport, from the last layout.
pub fn overflows_sideways(scroll: &ScrollHandle) -> bool {
    scroll.max_offset().x > px(1.)
}

/// The cue that a box scrolls sideways: a thin always-visible scrollbar (id `id`) under it
/// while it overflows, plus a marker that asks one more frame when layout disagrees with what
/// was drawn. Visible wherever the columns end. Add both as children of the box's wrapper, after the box.
pub fn scroll_bar_x(id: &'static str, scroll: &ScrollHandle) -> Vec<AnyElement> {
    let drawn = overflows_sideways(scroll);
    let handle = scroll.clone();
    let watch = canvas(
        move |_, window, _| {
            if overflows_sideways(&handle) != drawn {
                window.request_animation_frame();
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_0()
    .into_any_element();
    let mut out = vec![watch];
    if drawn {
        out.push(
            div()
                .id(id)
                .test_support()
                .relative()
                .w_full()
                .h(gpui_kit::rems(0.625))
                .flex_none()
                .child(
                    div().absolute().inset_0().child(
                        Scrollbar::horizontal(scroll)
                            .mode(ScrollbarMode::Always)
                            .viewport_from_layout(),
                    ),
                )
                .into_any_element(),
        );
    }
    out
}

/// Whether the box `scroll` tracks is taller than its viewport, from the last layout.
pub fn overflows_down(scroll: &ScrollHandle) -> bool {
    scroll.max_offset().y > px(1.)
}

/// The cue that a box scrolls down: an always-visible scrollbar (id `id`) over its right edge
/// while it overflows, plus a marker that asks one more frame when layout disagrees with what
/// was drawn. It overlays the box and takes no layout; the box keeps `SCROLLBAR_GUTTER` of
/// right padding. Add both as children of the box's `relative()` wrapper, after the box.
pub fn scroll_bar_y(id: impl Into<ElementId>, scroll: &ScrollHandle) -> Vec<AnyElement> {
    let drawn = overflows_down(scroll);
    let handle = scroll.clone();
    let watch = canvas(
        move |_, window, _| {
            if overflows_down(&handle) != drawn {
                window.request_animation_frame();
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_0()
    .into_any_element();
    let mut out = vec![watch];
    if drawn {
        out.push(
            div()
                .id(id.into())
                .test_support()
                .absolute()
                .top_0()
                .right_0()
                .bottom_0()
                .w(crate::style::SCROLLBAR_GUTTER)
                .child(
                    div().absolute().inset_0().child(
                        Scrollbar::vertical(scroll)
                            .mode(ScrollbarMode::Always)
                            .viewport_from_layout(),
                    ),
                )
                .into_any_element(),
        );
    }
    out
}

/// A zero-size marker drawn after a strip that rendered assuming `assumed` overflow: once
/// layout shows otherwise, it asks the view for another frame so the menu follows.
pub fn overflow_watch(scroll: &ScrollHandle, assumed: bool) -> impl IntoElement {
    let scroll = scroll.clone();
    canvas(
        move |_, window, _| {
            if strip_overflows(&scroll) != assumed {
                window.request_animation_frame();
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_0()
}

/// Keeps a scrolling strip's chosen item wholly in view. A zero-size marker drawn after the
/// strip reads that frame's layout; when the item, its place or the strip's width changed,
/// an item outside the viewport is scrolled to and one more frame asked for. A wheel scroll
/// changes none of these, so the user's own scroll is left alone.
#[derive(Clone, Default)]
pub struct Reveal(Rc<Cell<Option<RevealKey>>>);

// The layout a check last saw: the item, the viewport width, and the item's unscrolled
// place in it.
#[derive(Clone, Copy, PartialEq)]
struct RevealKey {
    item: usize,
    viewport: Pixels,
    range: Pixels,
    left: Pixels,
    width: Pixels,
}

impl Reveal {
    /// Checks the item again on the next frame, as for a newly chosen one.
    pub fn request(&self) {
        self.0.set(None);
    }

    /// Whether a check is owed: requested, or not yet run on a laid-out item.
    pub fn pending(&self) -> bool {
        self.0.get().is_none()
    }

    /// The marker that keeps item `ix` of the strip `scroll` tracks in view; draw it after
    /// the strip so the strip is measured first.
    pub fn watch(&self, scroll: &ScrollHandle, ix: usize) -> impl IntoElement + use<> {
        let (seen, scroll) = (Rc::clone(&self.0), scroll.clone());
        canvas(
            move |_, window, _| {
                let viewport = scroll.bounds();
                let Some(item) = scroll.bounds_for_item(ix) else {
                    return;
                };
                let key = RevealKey {
                    item: ix,
                    viewport: viewport.size.width,
                    range: scroll.max_offset().x,
                    left: item.left() - viewport.left(),
                    width: item.size.width,
                };
                if seen.replace(Some(key)) == Some(key) {
                    return;
                }
                // Scroll the least that shows the item: stay put when it lies whole, align the
                // edge it is cut at clear of that edge's fade otherwise, and an item wider
                // than the strip on its left edge past the fade. "Whole" means clear of the fades
                // now shown, unless the item cannot fit between them: it then keeps the raw
                // viewport test, so the two edges never trade places. A pixel of slack absorbs
                // layout rounding.
                let fade = resolve(STRIP_FADE_WIDTH, window);
                let x = scroll.offset().x;
                let (left, right) = (item.left() + x, item.right() + x);
                let (vl, vr) = (viewport.left(), viewport.right());
                let fits = item.size.width <= viewport.size.width - fade * 2.;
                let shown_left = if fits && x < -px(1.) { fade } else { px(0.) };
                let shown_right = if fits && x > -scroll.max_offset().x + px(1.) {
                    fade
                } else {
                    px(0.)
                };
                let wide = item.size.width > viewport.size.width;
                let want = if wide || left < vl + shown_left - px(1.) {
                    vl + fade - item.left()
                } else if right > vr - shown_right + px(1.) {
                    vr - fade - item.right()
                } else {
                    return;
                };
                let want = want.clamp(-scroll.max_offset().x, px(0.));
                if (want - x).abs() >= px(1.) {
                    scroll.set_offset(gpui_kit::point(want, scroll.offset().y));
                    window.request_animation_frame();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_0()
    }

    /// The marker that keeps the focused item `focused` of the strip `scroll` tracks in view,
    /// for a reveal kept apart from the chosen item's. With no item focused it draws nothing
    /// and resets, so the next focus that lands in the strip is checked afresh.
    pub fn watch_focused(
        &self,
        scroll: &ScrollHandle,
        focused: Option<usize>,
    ) -> Option<impl IntoElement + use<>> {
        let Some(ix) = focused else {
            self.request();
            return None;
        };
        Some(self.watch(scroll, ix))
    }
}

/// The strip's own gap, the space between the last two of its `items` from the last layout;
/// nothing before any layout or with fewer than two items.
pub fn strip_gap(scroll: &ScrollHandle, items: usize) -> Pixels {
    let at = |ix: usize| scroll.bounds_for_item(ix);
    let (Some(last), Some(prev)) = (
        items.checked_sub(1).and_then(at),
        items.checked_sub(2).and_then(at),
    ) else {
        return px(0.);
    };
    (last.left() - prev.right()).max(px(0.))
}

/// Fades over a sideways strip's edges while items lie beyond them. The fades sit over the
/// strip's scroll viewport only, placed from the previous frame's layout like the overflow
/// chevron; they take no mouse input, so clicks reach the items beneath.
#[derive(Clone, Default)]
pub struct StripFades(Rc<Cell<FadeLayout>>);

// What the last frame laid out: the wrapper's and viewport's bounds and which edges had
// items beyond them.
#[derive(Clone, Copy, Default, PartialEq)]
struct FadeLayout {
    host: Bounds<Pixels>,
    view: Bounds<Pixels>,
    edges: (bool, bool),
}

impl StripFades {
    /// The fades for the strip `scroll` tracks, which holds `items` items, ids `<id>-fade-left`
    /// and `<id>-fade-right`. Add them after the strip inside its `relative()` wrapper.
    pub fn render(
        &self,
        id: &str,
        scroll: &ScrollHandle,
        items: usize,
        cx: &App,
    ) -> Vec<AnyElement> {
        let seen = self.0.get();
        let edges = hidden_edges(scroll, items);
        // The marker measures the wrapper and redraws once when it or the edges changed
        // after this render, so a resize or a reveal is not left a frame behind.
        let (cell, handle) = (Rc::clone(&self.0), scroll.clone());
        let view = scroll.bounds();
        let drawn = FadeLayout {
            view,
            edges,
            ..seen
        };
        let watch = canvas(
            move |host, window, _| {
                let now = FadeLayout {
                    host,
                    view: handle.bounds(),
                    edges: hidden_edges(&handle, items),
                };
                cell.set(now);
                if now != drawn {
                    window.request_animation_frame();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
        .into_any_element();
        let mut out = vec![watch];
        let host = seen.host;
        if host.size.width <= px(0.) || view.size.width <= px(0.) {
            return out;
        }
        // An empty box over the viewport names it, `<id>-viewport`, for layout checks; the
        // kit's own scroll box carries no findable id.
        out.push(
            div()
                .id(SharedString::from(format!("{id}-viewport")))
                .test_support()
                .absolute()
                .top_0()
                .bottom_0()
                .left(view.left() - host.left())
                .w(view.size.width)
                .into_any_element(),
        );
        let background = cx.theme().background;
        let fade = |side: &str| {
            div()
                .id(SharedString::from(format!("{id}-fade-{side}")))
                .test_support()
                .absolute()
                .top_0()
                .bottom_0()
                .w(STRIP_FADE_WIDTH)
        };
        if edges.0 {
            out.push(
                fade("left")
                    .left(view.left() - host.left())
                    .bg(linear_gradient(
                        90.,
                        linear_color_stop(background, 0.),
                        linear_color_stop(background.opacity(0.), 1.),
                    ))
                    .into_any_element(),
            );
        }
        if edges.1 {
            out.push(
                fade("right")
                    .right(host.right() - view.right())
                    .bg(linear_gradient(
                        90.,
                        linear_color_stop(background.opacity(0.), 0.),
                        linear_color_stop(background, 1.),
                    ))
                    .into_any_element(),
            );
        }
        out
    }
}

// Whether the first item starts left of the viewport and the last ends right of it. The
// item bounds are unscrolled, so the offset is added; a pixel of slack absorbs rounding.
// The kit's trailing filler is not an item, so a strip scrolled to its end shows no fade.
fn hidden_edges(scroll: &ScrollHandle, items: usize) -> (bool, bool) {
    if items == 0 {
        return (false, false);
    }
    let (view, x) = (scroll.bounds(), scroll.offset().x);
    let left = scroll
        .bounds_for_item(0)
        .is_some_and(|b| b.left() + x < view.left() - px(1.));
    let right = scroll
        .bounds_for_item(items - 1)
        .is_some_and(|b| b.right() + x > view.right() + px(1.));
    (left, right)
}

/// Whether the last `tail` of item `ix` of the strip `scroll` tracks lies wholly outside both
/// of the strip's fades, as of the last layout. True before any layout, so nothing is held
/// back that was never measured.
pub fn tail_clear_of_fades(
    scroll: &ScrollHandle,
    items: usize,
    ix: usize,
    tail: Pixels,
    fade: Pixels,
) -> bool {
    let Some(item) = scroll.bounds_for_item(ix) else {
        return true;
    };
    let (hidden_left, hidden_right) = hidden_edges(scroll, items);
    let (view, x) = (scroll.bounds(), scroll.offset().x);
    let from = view.left() + if hidden_left { fade } else { px(0.) };
    let to = view.right() - if hidden_right { fade } else { px(0.) };
    let end = item.right() + x;
    end - tail >= from - px(1.) && end <= to + px(1.)
}

/// The key context of a [`Segmented`] track, where its keys bind.
pub const SEGMENTED_CONTEXT: &str = "Segmented";

actions!(
    segmented,
    [
        /// Chooses the focused segment.
        ActivateSegment,
        /// Chooses and focuses the next segment, wrapping at the end.
        NextSegment,
        /// Chooses and focuses the previous segment, wrapping at the start.
        PrevSegment
    ]
);

/// The callback a [`Segmented`] row shares among its buttons.
type OnSelect = Rc<dyn Fn(&usize, &mut Window, &mut App)>;

/// A row of choices where exactly one is current; see [`segmented`].
#[derive(IntoElement)]
pub struct Segmented {
    id: ElementId,
    options: Vec<SharedString>,
    selected: usize,
    on_select: OnSelect,
    ids: Vec<ElementId>,
    tips: Vec<(SharedString, SharedString)>,
}

/// A row of ghost buttons, one per option, ids `0..n` inside `id` unless [`Segmented::ids`]
/// names them; the `selected` one sits on a raised, edged fill and reports itself toggled. A click
/// calls `on_select(ix)`, as do Enter and Space on a focused segment; the arrow keys choose
/// and focus a neighbour.
pub fn segmented(
    id: impl Into<ElementId>,
    options: impl IntoIterator<Item = impl Into<SharedString>>,
    selected: usize,
    on_select: impl Fn(&usize, &mut Window, &mut App) + 'static,
) -> Segmented {
    Segmented {
        id: id.into(),
        options: options.into_iter().map(Into::into).collect(),
        selected,
        on_select: Rc::new(on_select),
        ids: Vec::new(),
        tips: Vec::new(),
    }
}

impl Segmented {
    /// Gives segment `ix` the element id `ids[ix]` instead of `ix`; focus and arrow keys follow it.
    pub fn ids(mut self, ids: impl IntoIterator<Item = impl Into<ElementId>>) -> Self {
        self.ids = ids.into_iter().map(Into::into).collect();
        self
    }

    /// Gives segment `ix` the hover text `tips[ix].1`, found by tests as element `tips[ix].0`.
    pub fn tooltips(
        mut self,
        tips: impl IntoIterator<Item = (impl Into<SharedString>, impl Into<SharedString>)>,
    ) -> Self {
        self.tips = tips
            .into_iter()
            .map(|(id, text)| (id.into(), text.into()))
            .collect();
        self
    }
}

/// The fill of a segment: `popover` when selected, transparent otherwise. It never reads as
/// focus or brand; the selected segment's edge carries the contrast.
pub fn segment_fill(selected: bool, cx: &App) -> Hsla {
    if selected {
        cx.theme().popover
    } else {
        transparent_black()
    }
}

/// The 1 px edge around the selected segment: the control outline colour, which clears 3:1 on
/// the track and the raised fill where the fill alone does not. Unselected segments have none.
pub fn segment_edge(selected: bool, cx: &App) -> Option<Hsla> {
    selected.then(|| cx.theme().input)
}

/// The app's underline tab strip: `TabBar::new(id).underline()` sized for the current zoom.
/// Callers add `Tab` children, the selected index, `on_click`, scroll tracking and suffixes.
pub fn tab_strip(id: impl Into<ElementId>, cx: &App) -> TabBar {
    TabBar::new(id)
        .underline()
        .with_size(kit_size(zoom::current(cx)))
}

impl RenderOnce for Segmented {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        // The track keeps each segment's focus handle and hands it to the kit button, so the
        // track's key handlers can tell which segment holds focus and move it to a neighbour.
        let ids: Vec<ElementId> = (0..self.options.len())
            .map(|ix| self.ids.get(ix).cloned().unwrap_or(ix.into()))
            .collect();
        let handles: Rc<[FocusHandle]> = window.with_id(self.id.clone(), |window| {
            ids.iter()
                .map(|id| {
                    window
                        .use_keyed_state(id.clone(), cx, |_, cx| cx.focus_handle())
                        .read(cx)
                        .clone()
                })
                .collect()
        });
        let t = cx.theme();
        let (fg, muted, radius) = (t.foreground, t.muted_foreground, t.radius);
        let track = t.muted;
        let focused = {
            let handles = Rc::clone(&handles);
            move |window: &Window| handles.iter().position(|h| h.is_focused(window))
        };
        // An arrow steps from the focused segment, wrapping, then chooses and focuses it.
        let step = {
            let (handles, on_select, focused) = (
                Rc::clone(&handles),
                Rc::clone(&self.on_select),
                focused.clone(),
            );
            move |forward: bool, window: &mut Window, cx: &mut App| {
                let n = handles.len();
                let Some(ix) = focused(window) else { return };
                let to = if forward {
                    (ix + 1) % n
                } else {
                    (ix + n - 1) % n
                };
                window.focus(&handles[to], cx);
                on_select(&to, window, cx);
            }
        };
        let step = Rc::new(step);
        let buttons = self
            .options
            .into_iter()
            .zip(ids)
            .enumerate()
            .map(|(ix, (label, id))| {
                let sel = ix == self.selected;
                let on_select = Rc::clone(&self.on_select);
                // `selected` stops the kit painting its hover over the current choice.
                let button = Button::new(id)
                    .track_focus(&handles[ix])
                    .ghost()
                    .label(label)
                    .toggled(sel)
                    .selected(sel)
                    .on_click(move |_, window, cx| on_select(&ix, window, cx));
                let button = if sel {
                    button.bg(segment_fill(true, cx)).text_color(fg)
                } else {
                    button.text_color(muted)
                };
                // The edge floats over the button so the segment keeps a ghost button's size.
                let edge = segment_edge(sel, cx).map(|edge| {
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(radius)
                        .border_1()
                        .border_color(edge)
                });
                // The kit button's own tooltip is not findable, so a hint rides on a wrapper.
                match (self.tips.get(ix).cloned(), edge) {
                    (Some((tip_id, text)), edge) => div()
                        .id(("segment-tip", ix))
                        .tooltip(text_tooltip(tip_id, text))
                        .relative()
                        .child(button)
                        .children(edge)
                        .into_any_element(),
                    (None, Some(edge)) => div()
                        .relative()
                        .child(button)
                        .child(edge)
                        .into_any_element(),
                    (None, None) => button.into_any_element(),
                }
            });
        // The track hugs its segments; `form-control` is a flex row, so it keeps its content width.
        let on_select = Rc::clone(&self.on_select);
        let (next, prev) = (Rc::clone(&step), step);
        h_flex()
            .id(self.id)
            .test_support()
            .key_context(SEGMENTED_CONTEXT)
            .on_action(move |_: &ActivateSegment, window, cx| {
                if let Some(ix) = focused(window) {
                    on_select(&ix, window, cx);
                }
            })
            .on_action(move |_: &NextSegment, window, cx| next(true, window, cx))
            .on_action(move |_: &PrevSegment, window, cx| prev(false, window, cx))
            .flex_none()
            .gap(SPACE_1)
            .p(px(2.))
            .rounded(RADIUS)
            .bg(track)
            .children(buttons)
    }
}

/// The callback a [`Checkbox`] calls with the requested value.
type OnToggle = Rc<dyn Fn(&bool, &mut Window, &mut App)>;

/// A checkbox; see [`checkbox`].
#[derive(IntoElement)]
pub struct Checkbox {
    id: ElementId,
    checked: bool,
    disabled: bool,
    label: Option<SharedString>,
    name: Option<SharedString>,
    tooltip: Option<SharedString>,
    tab_stop: bool,
    on_click: Option<OnToggle>,
    /// Drawn with a dash instead of a tick: some of what the box stands for is on.
    mixed: bool,
    /// Drawn as a lock icon instead of a box; see [`lock_toggle`].
    lock: bool,
    /// Takes keyboard focus when drawn, for a box standing where focus just was.
    take_focus: bool,
    /// Drawn round with a dot instead of a tick: one choice among its neighbours.
    radio: bool,
}

/// A controlled checkbox with the kit control's behaviour (click, Enter and Space toggle; a
/// click never takes focus; Tab reaches it unless `tab_stop(false)`) and a neutral look
/// from [`checkbox_look`]. `on_click` receives the requested value, `!checked`.
pub fn checkbox(id: impl Into<ElementId>, checked: bool) -> Checkbox {
    Checkbox {
        id: id.into(),
        checked,
        disabled: false,
        label: None,
        name: None,
        tooltip: None,
        tab_stop: true,
        on_click: None,
        mixed: false,
        lock: false,
        take_focus: false,
        radio: false,
    }
}

/// A [`Checkbox`] drawn as a lock icon with no visible text and the accessibility label
/// "Secret"; ids, keys, focus and `on_click(&!on)` as `checkbox`.
pub fn lock_toggle(id: impl Into<ElementId>, on: bool) -> Checkbox {
    Checkbox {
        lock: true,
        ..checkbox(id, on)
    }
}

/// The icon a lock toggle paints; see [`lock_look`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LockLook {
    /// `Lock` when on, `LockOpen` when off.
    pub icon: IconName,
    /// The icon's colour.
    pub color: Hsla,
    /// The control's opacity.
    pub opacity: f32,
    /// A 1px outline in this colour, drawn only by an enabled on lock in high contrast.
    pub outline: Option<Hsla>,
    /// The filled box behind the glyph: set for every secret (on) lock, never for an off one.
    /// A disabled one in high contrast is filled with the grey, the `border` token leaned toward `foreground`. The kit has only
    /// outline lock glyphs, so the fill is what makes a secret read as solid.
    pub fill: Option<Hsla>,
}

/// The opacity of a control: 1 when enabled, `DISABLED_OPACITY` when disabled, and 1 for a
/// disabled one in a high-contrast theme, where the half-opacity glyph falls under 4.5:1 and
/// the look itself carries the disabled cue.
pub(crate) fn control_opacity(disabled: bool, cx: &App) -> f32 {
    if disabled && !full_strength(cx) {
        DISABLED_OPACITY
    } else {
        1.
    }
}

/// An on lock is an inverted chip: a `Lock` in `background` on a `foreground` fill, which
/// stands out from any surface; an off lock is a bare `LockOpen` in `muted_foreground`.
/// Disabled dims the whole control to `DISABLED_OPACITY`. In a high-contrast theme, where it
/// cannot dim, a disabled on lock is a grey chip instead (a `Lock` in whichever of
/// `foreground` and `background` reads better on that grey), and a disabled off lock's glyph
/// leans toward `background`. Only the enabled on lock is outlined.
pub fn lock_look(on: bool, disabled: bool, cx: &App) -> LockLook {
    let t = cx.theme();
    let high = full_strength(cx);
    let (icon, color, fill) = if on && disabled && high {
        // The bare border token is under 3:1 on HC Dark's popover, so lean it toward the text.
        let grey = t.border.blend(t.foreground.opacity(0.15));
        let glyph = if luminance_ratio(t.foreground, grey) >= luminance_ratio(t.background, grey) {
            t.foreground
        } else {
            t.background
        };
        (IconName::Lock, glyph, Some(grey))
    } else if on {
        (IconName::Lock, t.background, Some(t.foreground))
    } else if disabled && high {
        // Dimmer than the enabled off glyph, yet still 4.5:1 on every surface.
        let dim = t.muted_foreground.blend(t.background.opacity(0.25));
        (IconName::LockOpen, dim, None)
    } else {
        (IconName::LockOpen, t.muted_foreground, None)
    };
    // A disabled lock never carries an edge, so it can never out-weigh the enabled one.
    let outline = (high && on && !disabled).then_some(t.input);
    LockLook {
        icon,
        color,
        opacity: control_opacity(disabled, cx),
        outline,
        fill,
    }
}

/// A colour from text that may come from a file: `#rgb`, `#rrggbb` or `#rrggbbaa` in ASCII hex,
/// the `#` optional; anything else is `None`.
pub fn parse_color(text: &str) -> Option<Hsla> {
    // The kit's parser slices by byte offset, so non-ASCII text of a valid length would panic
    // inside it; only plain hex digits ever reach it.
    let digits = text.strip_prefix('#').unwrap_or(text);
    if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let full = match digits.len() {
        3 => digits.chars().flat_map(|c| [c, c]).collect(),
        6 | 8 => digits.to_owned(),
        _ => return None,
    };
    Hsla::parse_hex(&full).ok()
}

/// The WCAG contrast ratio of two colours, ignoring alpha.
pub fn luminance_ratio(a: Hsla, b: Hsla) -> f32 {
    let lum = |c: Hsla| {
        let c = c.to_rgb();
        let ch = |v: f32| {
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(c.r) + 0.7152 * ch(c.g) + 0.0722 * ch(c.b)
    };
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// How a popup menu entry is drawn beyond the kit's own muted colour for a disabled one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuItemLook {
    /// Whether the label is set in italics.
    pub italic: bool,
}

/// The look of a menu entry, `disabled` or not.
pub fn menu_item_look(disabled: bool, cx: &App) -> MenuItemLook {
    MenuItemLook {
        italic: disabled && full_strength(cx),
    }
}

/// `el` with the disabled-menu cue applied: italics in the muted foreground where
/// [`menu_item_look`] asks for it, `el` untouched elsewhere.
pub fn menu_label_cue<E: gpui_kit::Styled>(el: E, cx: &App) -> E {
    if menu_item_look(true, cx).italic {
        el.italic().text_color(cx.theme().muted_foreground)
    } else {
        el
    }
}

/// The label of a menu the OS draws, which takes text only: a disabled entry in a
/// high-contrast theme gets an "(unavailable)" marker where the other menus use italics.
pub fn native_menu_label(label: &'static str, disabled: bool, cx: &App) -> SharedString {
    if menu_item_look(disabled, cx).italic {
        format!("{label} (unavailable)").into()
    } else {
        label.into()
    }
}

/// A menu entry labelled `label`, drawn per [`menu_item_look`]. Where the look adds nothing it
/// is the kit's own item; otherwise an element item carrying [`menu_label_cue`].
pub fn menu_item(label: impl Into<SharedString>, disabled: bool, cx: &App) -> PopupMenuItem {
    let label: SharedString = label.into();
    if !menu_item_look(disabled, cx).italic {
        return PopupMenuItem::new(label).disabled(disabled);
    }
    PopupMenuItem::element(move |_, cx| {
        menu_label_cue(div(), cx)
            .id(label.clone())
            .test_support()
            .aria_label(label.clone())
            .child(label.clone())
    })
    .disabled(true)
}

impl Checkbox {
    /// A disabled box ignores clicks and keys and leaves the tab order.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Text beside the box, also its accessibility label.
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The accessibility label alone, for a box whose row already shows the text; set beside a
    /// visible `label`, it replaces that label as the name.
    pub fn accessible_name(mut self, name: impl Into<SharedString>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Draw a dash in an unchecked box; the click still asks for `!checked`.
    pub fn mixed(mut self, mixed: bool) -> Self {
        self.mixed = mixed;
        self
    }

    /// A tooltip shown on hover.
    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    /// Called with the requested value when the box is clicked or toggled by key.
    pub fn on_click(mut self, f: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }

    /// Moves keyboard focus to the box when it is drawn; a row that moved carries focus this way.
    pub fn take_focus(mut self, take: bool) -> Self {
        self.take_focus = take;
        self
    }

    /// Whether Tab stops on the box; true by default.
    pub fn tab_stop(mut self, tab_stop: bool) -> Self {
        self.tab_stop = tab_stop;
        self
    }

    /// Draw a round mark with a dot when checked, for a box that picks one of several rows;
    /// clicks and keys behave as for any checkbox.
    pub fn radio(mut self, radio: bool) -> Self {
        self.radio = radio;
        self
    }
}

/// The colours a checkbox paints; see [`checkbox_look`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CheckboxLook {
    /// The box's edge.
    pub border: Hsla,
    /// The box's fill.
    pub fill: Hsla,
    /// The tick's colour, `None` when unchecked.
    pub tick: Option<Hsla>,
    /// The box's opacity.
    pub opacity: f32,
}

/// A checkbox's look: the `input` border on the [`field_fill`], a `foreground` tick when
/// checked (`muted_foreground` when also disabled), dimmed to `DISABLED_OPACITY` when
/// disabled, except in a high-contrast theme, where the edge leans toward `background`.
/// Never the accent, so a checked box does not read as focus or brand.
pub fn checkbox_look(checked: bool, disabled: bool, cx: &App) -> CheckboxLook {
    let t = cx.theme();
    let tick = if disabled {
        t.muted_foreground
    } else {
        t.foreground
    };
    // The lighter edge mirrors the standard themes' half-opacity box and keeps 3:1.
    let border = if disabled && full_strength(cx) {
        disabled_edge(cx)
    } else {
        t.input
    };
    CheckboxLook {
        border,
        fill: field_fill(disabled, cx),
        tick: checked.then_some(tick),
        opacity: control_opacity(disabled, cx),
    }
}

/// The colour of a checkbox's mixed-state dash: the colour a tick would take, so the dash reads
/// like one (`muted_foreground` when disabled).
pub fn mixed_dash_color(disabled: bool, cx: &App) -> Hsla {
    let t = cx.theme();
    if disabled {
        t.muted_foreground
    } else {
        t.foreground
    }
}

/// Focus `handle` from inside a draw. Controls drawn earlier in this frame already painted
/// their focus state and a draw ignores refresh requests, so a redraw is queued for after it.
pub fn focus_in_draw(handle: &FocusHandle, window: &mut Window, cx: &mut App) {
    if handle.is_focused(window) {
        return;
    }
    handle.focus(window, cx);
    window.defer(cx, |window, _| window.refresh());
}

impl RenderOnce for Checkbox {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        // The base control owns toggling, keys, focus and accessibility; sharing its keyed
        // focus handle tells this render whether to draw the ring.
        let focus = window
            .use_keyed_state(self.id.clone(), cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        if self.take_focus {
            focus_in_draw(&focus, window, cx);
        }
        let is_focused = focus.is_focused(window);
        let look = checkbox_look(self.checked, self.disabled, cx);
        let t = cx.theme();
        let (fg, muted) = (t.foreground, t.muted_foreground);
        let checked = self.checked;
        // A lock carries its label for accessibility only, so nothing is drawn beside it.
        let (a11y_label, label) = if self.lock {
            (Some(SharedString::from("Secret")), None)
        } else {
            (self.name.or(self.label.clone()), self.label)
        };
        let labelled = label.is_some();
        let lock = self.lock.then(|| lock_look(checked, self.disabled, cx));
        let indicator = CheckboxIndicator::new()
            .checked(checked)
            .disabled(self.disabled)
            .relative()
            .flex_shrink_0()
            .size(CHECKBOX_SIZE);
        let indicator = match lock {
            Some(lock) => indicator
                .opacity(lock.opacity)
                // The fill is an overlay drawn first, so the glyph paints over it.
                .when_some(lock.fill, |d, color| {
                    d.child(div().absolute().inset_0().rounded(RADIUS).bg(color))
                })
                .child(
                    svg()
                        .size_full()
                        .path(lock.icon.path())
                        .text_color(lock.color),
                )
                // The outline is an overlay, so it never changes the icon's size or the layout.
                .when_some(lock.outline, |d, color| {
                    d.child(
                        div()
                            .absolute()
                            .inset_0()
                            .rounded(RADIUS)
                            .border_1()
                            .border_color(color),
                    )
                }),
            None => indicator
                // Centre the box on the label's first line, including when the label wraps.
                .when(labelled, |d| d.mt(CHECKBOX_SIZE * 0.125))
                .border_1()
                .rounded(RADIUS)
                .border_color(look.border)
                .bg(look.fill)
                .opacity(look.opacity)
                // A round mark centres a dot where a box would draw its tick; the empty marker
                // is a test hook naming the shape.
                .when(self.radio, |d| {
                    d.rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .id("checkbox-radio")
                                .test_support()
                                .absolute()
                                .size_0(),
                        )
                        .when_some(look.tick, |d, tick| {
                            d.child(div().size(CHECKBOX_SIZE * 0.5).rounded_full().bg(tick))
                        })
                })
                .when_some(look.tick.filter(|_| !self.radio), |d, tick| {
                    d.child(
                        svg()
                            .absolute()
                            .top_px()
                            .left_px()
                            .size_3()
                            .path(IconName::Check.path())
                            .text_color(tick),
                    )
                })
                // Mixed is unchecked, so the dash takes the colour a tick would have.
                .when(self.mixed && !checked, |d| {
                    d.child(
                        svg()
                            .absolute()
                            .top_px()
                            .left_px()
                            .size_3()
                            .path(IconName::Minus.path())
                            .text_color(mixed_dash_color(self.disabled, cx)),
                    )
                }),
        };
        let label = label.map(|label| {
            div()
                .flex_1()
                .overflow_hidden()
                .line_height(relative(1.25))
                .text_color(if self.disabled { muted } else { fg })
                .child(label)
        });
        let parts = ControlParts {
            focused: is_focused,
            indicator: indicator.into_any_element(),
            label: label.map(IntoElement::into_any_element),
            tooltip: self.tooltip,
        };
        // A radio mark uses the kit's radio primitive, which announces a radio button and
        // ignores a click on the picked row, so `on_click` only ever hears `true`.
        if self.radio {
            let base = BaseRadio::new(self.id)
                .key_context(DIALOG_FOOTER_CONTEXT)
                .checked(checked)
                .disabled(self.disabled)
                .tab_stop(self.tab_stop)
                .track_focus(&focus)
                .when_some(a11y_label, |b, label| b.accessibility_label(label))
                .when_some(self.on_click, |b, on_click| {
                    b.on_change(move |_, _, window, cx| {
                        window.prevent_default();
                        on_click(&true, window, cx);
                    })
                });
            return control_frame(base, parts, fg, window, cx).into_any_element();
        }
        // In a dialog, Enter toggles the box as Space does rather than answering the dialog.
        let base = BaseCheckbox::new(self.id)
            .key_context(DIALOG_FOOTER_CONTEXT)
            .checked(checked)
            .disabled(self.disabled)
            .tab_stop(self.tab_stop)
            .track_focus(&focus)
            .when_some(a11y_label, |b, label| b.accessibility_label(label))
            .when_some(self.on_click, |b, on_click| {
                b.on_change(move |_, _, window, cx| {
                    window.prevent_default();
                    on_click(&!checked, window, cx);
                })
            });
        control_frame(base, parts, fg, window, cx).into_any_element()
    }
}

/// What a [`Checkbox`] draws inside its base control, box or radio alike.
struct ControlParts {
    focused: bool,
    indicator: AnyElement,
    label: Option<AnyElement>,
    tooltip: Option<SharedString>,
}

// The layout, focus ring, mark, label and tooltip shared by the box and radio controls.
fn control_frame<E>(base: E, parts: ControlParts, fg: Hsla, window: &Window, cx: &App) -> E
where
    E: gpui_kit::Styled
        + gpui_kit::ParentElement
        + gpui_kit::StatefulInteractiveElement
        + IntoElement,
{
    base.flex()
        .flex_row()
        .items_start()
        .gap_2()
        .line_height(relative(1.))
        .text_size(TEXT_CONTROL)
        .text_color(fg)
        .rounded(RADIUS * 0.5)
        // The marker is absolute and empty, so focusing never shifts the layout.
        .when(parts.focused, |b| {
            b.focus_ring_style(window, cx).child(
                div()
                    .id("checkbox-focus")
                    .test_support()
                    .absolute()
                    .size_0(),
            )
        })
        .child(parts.indicator)
        .children(parts.label)
        // A press never moves focus, as with the kit control.
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .when_some(parts.tooltip, |b, text| {
            b.tooltip(move |window, cx| Tooltip::new(text.clone()).build(window, cx))
        })
}

/// The key context of a [`dialog_footer`] and of each [`checkbox`], where Enter is unbound so a
/// focused button or checkbox takes it natively instead of the dialog's own Confirm.
pub const DIALOG_FOOTER_CONTEXT: &str = "DialogFooter";

/// The key context of a [`dialog_select`], where Enter on the kit select inside opens it.
pub const DIALOG_SELECT_CONTEXT: &str = "DialogSelect";

actions!(
    dialog_select,
    [
        /// Opens the focused kit select inside a dialog, as Down does.
        OpenSelect
    ]
);

/// A kit select's menu height that shows `rows` medium rows without a scroll, in place of the
/// kit's 20 rem cap; the kit still caps the popup to the window. A row is about 1.92 rem, plus
/// the list's padding.
pub fn menu_fits(rows: usize) -> Rems {
    gpui_kit::rems(rows as f32 * 2. + 1.)
}

/// Wraps a kit `Select` that sits in a dialog. The kit select passes its Enter (Confirm) on
/// to the dialog, which answers it; here Enter opens the select as Down does, and stops.
pub fn dialog_select(select: impl IntoElement) -> Div {
    div()
        .key_context(DIALOG_SELECT_CONTEXT)
        .on_action(|_: &OpenSelect, window, cx| {
            window.dispatch_action(Box::new(gpui_kit::base::actions::SelectDown), cx)
        })
        .child(select)
}

/// `dialog` titled `title`, with the app's close button in place of the kit's: the same look
/// and place, the name "Close", the last Tab stop, and Enter, Space or a click run the dialog's
/// cancel (and so its `on_cancel` veto), as the kit's does.
pub fn with_close(dialog: Dialog, title: impl IntoElement) -> Dialog {
    // The kit draws its x half a padding inside the panel, above where the title starts, and
    // the body clips there. The panel's top padding gives up that half to the title, so the x
    // fits inside the body while the title and the gap below it stay put. The title takes the
    // ring's room from the padding too, so the x's keyboard ring clears that clip.
    dialog
        .close_button(false)
        .pt(DIALOG_PADDING - DIALOG_CLOSE_INSET - FOCUS_RING_ROOM)
        .gap(DIALOG_PADDING)
        .title(
            div()
                .id("dialog-title")
                .test_support()
                .relative()
                .w_full()
                .pt(DIALOG_CLOSE_INSET + FOCUS_RING_ROOM)
                .child(title)
                .child(DialogX),
        )
}

/// The close button [`with_close`] puts in a dialog's title.
#[derive(IntoElement)]
struct DialogX;

impl RenderOnce for DialogX {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        // Cancel is dispatched from a focus node inside the dialog, as the kit's x does, so it
        // reaches this dialog whatever holds focus when the button is pressed.
        let anchor = window
            .use_keyed_state("dialog-x-anchor", cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        let target = anchor.clone();
        // The footer context unbinds Enter, so the focused button takes it natively instead of
        // the dialog's Confirm; after every other control in Tab order, as the kit's was.
        div()
            .id("dialog-x")
            .key_context(DIALOG_FOOTER_CONTEXT)
            .absolute()
            .top(FOCUS_RING_ROOM)
            .right(-DIALOG_CLOSE_INSET)
            .child(div().absolute().size_0().track_focus(&anchor))
            .child(
                Button::new("close")
                    .small()
                    .ghost()
                    .icon(IconName::Close)
                    .accessibility_label("Close")
                    .tab_index(1)
                    .on_click(move |_, window, cx| {
                        target.dispatch_action(&gpui_kit::base::actions::Cancel, window, cx)
                    }),
            )
    }
}

/// A dialog's button row; see [`dialog_footer`].
#[derive(IntoElement)]
pub struct DialogFooter {
    hint: Option<AnyElement>,
    buttons: Vec<Button>,
}

/// A dialog's button row, id `dialog-footer`: [hint] then, at the right edge, cancel, others,
/// primary, `SPACE_2` apart, with `DIALOG_FOOTER_GAP` of top padding. Buttons keep their styling.
pub fn dialog_footer(
    cancel: Option<Button>,
    others: impl IntoIterator<Item = Button>,
    primary: Button,
) -> DialogFooter {
    DialogFooter {
        hint: None,
        buttons: cancel.into_iter().chain(others).chain([primary]).collect(),
    }
}

impl DialogFooter {
    /// Muted text or any element on the left, before the flexible space.
    pub fn hint(mut self, hint: impl IntoElement) -> Self {
        self.hint = Some(hint.into_any_element());
        self
    }
}

impl RenderOnce for DialogFooter {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        // The flexible hint cell pushes the buttons to the right edge whether or not it holds text.
        h_flex()
            .id("dialog-footer")
            .key_context(DIALOG_FOOTER_CONTEXT)
            .test_support()
            .w_full()
            .pt(DIALOG_FOOTER_GAP)
            // The footer ends the kit body, which clips flush with its bottom.
            .ring_room(RingSides::BOTTOM)
            .gap(SPACE_2)
            .items_center()
            .child(div().flex_1().children(self.hint))
            .children(self.buttons)
    }
}

/// Whether the element keyed `id` measured narrower than `FORM_STACK_WIDTH` last frame, and a
/// canvas that measures it again. Stacking leaves the width alone, so the answer cannot flip back.
fn narrow_watch(
    id: ElementId,
    window: &mut Window,
    cx: &mut App,
) -> (bool, impl IntoElement + use<>) {
    narrower_than(id, FORM_STACK_WIDTH, window, cx)
}

/// Whether the element keyed `id` measured narrower than `limit` last frame, and an absolute
/// canvas to place in it that measures it again, refreshing once when the answer changes. The
/// element's own width must not depend on the answer, or it could oscillate.
pub fn narrower_than(
    id: ElementId,
    limit: Rems,
    window: &mut Window,
    cx: &mut App,
) -> (bool, impl IntoElement + use<>) {
    let narrow = Rc::clone(
        window
            .use_keyed_state(id, cx, |_, _| Rc::new(Cell::new(false)))
            .read(cx),
    );
    let was = narrow.get();
    let watch = canvas(
        move |bounds, window, _| {
            let now = bounds.size.width < resolve(limit, window);
            if narrow.replace(now) != now {
                window.request_animation_frame();
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0();
    (was, watch)
}

/// A labelled form row; see [`form_row`].
#[derive(IntoElement)]
pub struct FormRow {
    id: ElementId,
    label: SharedString,
    label_width: Option<Rems>,
    hint: Option<(ElementId, SharedString)>,
    control: AnyElement,
}

/// One form row: a muted, left-aligned label in a `FORM_LABEL_WIDTH` column that wraps long
/// text, then `control`. Narrower than `FORM_STACK_WIDTH`, the label sits above the control.
/// Stacking measures the row, so its parent needs a definite width.
pub fn form_row(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    control: impl IntoElement,
) -> FormRow {
    FormRow {
        id: id.into(),
        label: label.into(),
        label_width: None,
        hint: None,
        control: control.into_any_element(),
    }
}

impl FormRow {
    /// An id for a wrapper around this row, distinct from the row's own.
    pub fn wrapper_id(&self) -> ElementId {
        (self.id.clone(), "wrapper").into()
    }

    /// Sets the label column's width in place of `FORM_LABEL_WIDTH`; such a row never stacks.
    pub fn label_width(mut self, width: Rems) -> Self {
        self.label_width = Some(width);
        self
    }

    /// Muted `TEXT_SMALL` text with id `id` under the control, in the control column.
    pub fn hint(mut self, id: impl Into<ElementId>, text: impl Into<SharedString>) -> Self {
        self.hint = Some((id.into(), text.into()));
        self
    }
}

impl RenderOnce for FormRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        // A custom label width never stacks.
        let stacks = self.label_width.is_none();
        let (narrow, watch) = narrow_watch(self.id.clone(), window, cx);
        let stacked = stacks && narrow;
        let watch = stacks.then_some(watch);
        // Stacked, an empty label would only leave a blank band; side by side it holds the column.
        let show_label = !(stacked && self.label.is_empty());
        // Side by side, the label's padded line band is exactly `INPUT_HEIGHT`, so one line
        // centres on the control's first line and a wrapped label starts there.
        let label = div()
            .id("form-label")
            .aria_label(self.label.clone())
            .test_support()
            .text_color(cx.theme().muted_foreground)
            .line_height(relative(FORM_LABEL_LINE_HEIGHT))
            .whitespace_normal()
            .map(|label| {
                if stacked {
                    label.w_full()
                } else {
                    label
                        .w(self.label_width.unwrap_or(FORM_LABEL_WIDTH))
                        .flex_none()
                        .min_h(INPUT_HEIGHT)
                        .py(SPACE_1)
                }
            })
            .child(self.label);
        let control = v_flex()
            .id("form-control")
            .test_support()
            .min_w_0()
            .map(|c| if stacked { c.w_full() } else { c.flex_1() })
            .child(
                h_flex()
                    .w_full()
                    .min_h(INPUT_HEIGHT)
                    .items_center()
                    .child(self.control),
            )
            .children(self.hint.map(|(id, text)| {
                div()
                    .id(id)
                    .test_support()
                    .text_size(TEXT_SMALL)
                    .text_color(cx.theme().muted_foreground)
                    .child(text)
            }));
        let row = if stacked {
            v_flex().gap(SPACE_1)
        } else {
            h_flex().items_start().gap(SPACE_2)
        };
        row.id(self.id)
            .test_support()
            .relative()
            .w_full()
            .text_size(TEXT_UI)
            .when(show_label, |row| row.child(label))
            .child(control)
            .children(watch)
    }
}

/// A form section heading; see [`form_heading`].
#[derive(IntoElement)]
pub struct FormHeading {
    id: ElementId,
    title: SharedString,
}

/// A section heading: `TEXT_UI`, semibold, theme foreground.
pub fn form_heading(id: impl Into<ElementId>, title: impl Into<SharedString>) -> FormHeading {
    FormHeading {
        id: id.into(),
        title: title.into(),
    }
}

impl RenderOnce for FormHeading {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .id(self.id)
            .test_support()
            .text_size(TEXT_UI)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(cx.theme().foreground)
            .child(self.title)
    }
}

/// Muted helper text under a form; see [`form_note`].
#[derive(IntoElement)]
pub struct FormNote {
    id: ElementId,
    text: SharedString,
    danger: bool,
}

/// Muted helper text whose left edge lines up with the control column, or with the stacked
/// rows when its parent is narrower than `FORM_STACK_WIDTH`. Its parent needs a definite width.
pub fn form_note(id: impl Into<ElementId>, text: impl Into<SharedString>) -> FormNote {
    FormNote {
        id: id.into(),
        text: text.into(),
        danger: false,
    }
}

impl FormNote {
    /// Danger-coloured text, for a field error under the rows.
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
}

impl RenderOnce for FormNote {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (narrow, watch) = narrow_watch(self.id.clone(), window, cx);
        let theme = cx.theme();
        let color = if self.danger {
            theme.danger
        } else {
            theme.muted_foreground
        };
        // The wrapper spans the parent so the watch measures the same width a row does; the
        // indent is its padding, which leaves the text element itself starting at the column.
        div()
            .relative()
            .w_full()
            .when(!narrow, |n| n.pl(FORM_LABEL_WIDTH + SPACE_2))
            .child(
                div()
                    .id(self.id)
                    .test_support()
                    .aria_label(self.text.clone())
                    .text_size(TEXT_SMALL)
                    .text_color(color)
                    .child(self.text),
            )
            .child(watch)
    }
}

/// A small rounded count with tabular figures, or `None` for zero so callers can skip it.
pub fn count_badge(n: usize) -> Option<AnyElement> {
    badge(n, Tag::secondary())
}

/// An unpainted, zero-width count badge: it gives a row a counted tab's height at any zoom.
pub fn badge_strut() -> AnyElement {
    div()
        .w_0()
        .overflow_hidden()
        .invisible()
        .children(count_badge(1))
        .into_any_element()
}

/// `count_badge` in the danger colour, for a count of failures.
pub fn danger_count_badge(n: usize) -> Option<AnyElement> {
    badge(n, Tag::danger())
}

/// `count_badge` in the warning colour, for a count where a check was skipped.
pub fn warning_count_badge(n: usize) -> Option<AnyElement> {
    badge(n, Tag::warning())
}

/// A warning-coloured badge carrying `text` instead of a number, for a pass beside skips.
pub fn warning_text_badge(text: String) -> AnyElement {
    text_badge(text, Tag::warning())
}

/// A danger-coloured badge carrying `text` instead of a number, for a count of failures.
pub fn danger_text_badge(text: String) -> AnyElement {
    text_badge(text, Tag::danger())
}

fn text_badge(text: String, tag: Tag) -> AnyElement {
    let tnum = FontFeatures(Arc::new(vec![("tnum".into(), 1)]));
    tag.small()
        .rounded_full()
        .font_features(tnum)
        .whitespace_nowrap()
        .child(text)
        .into_any_element()
}

/// `count_badge` in the success colour, for a count where every check passed.
pub fn success_count_badge(n: usize) -> Option<AnyElement> {
    badge(n, Tag::success())
}

/// A semantic colour: muted, success (all passed), warning (passed but not fully verified) or
/// danger (something failed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Neither all passed nor anything failed.
    Muted,
    /// Every check passed.
    Success,
    /// Passed, but a check was skipped or never verified.
    Warning,
    /// Something failed or errored.
    Danger,
}

impl Tone {
    /// The tone's colour in the current theme.
    pub fn colour(self, cx: &App) -> Hsla {
        let t = cx.theme();
        match self {
            Tone::Muted => t.muted_foreground,
            Tone::Success => t.success,
            Tone::Warning => t.warning,
            Tone::Danger => t.danger,
        }
    }
}

fn badge(n: usize, tag: Tag) -> Option<AnyElement> {
    if n == 0 {
        return None;
    }
    let tnum = FontFeatures(Arc::new(vec![("tnum".into(), 1)]));
    Some(
        tag.small()
            .rounded_full()
            .font_features(tnum)
            .child(n.to_string())
            .into_any_element(),
    )
}

/// The key bound to `action`, as a hint. A context-bound key is found along the focused
/// element's path, then in `context` and the Shell's, so hints hold while focus sits outside
/// them; the context-free lookup still covers globally bound actions.
pub fn key_hint(
    action: &dyn Action,
    context: Option<&'static str>,
    window: &Window,
    cx: &App,
) -> Option<Kbd> {
    window
        .focused(cx)
        .and_then(|focus| Kbd::binding_for_action_in(action, &focus, window))
        .or_else(|| context.and_then(|ctx| Kbd::binding_for_action(action, Some(ctx), window)))
        .or_else(|| Kbd::binding_for_action(action, Some(SHELL_CONTEXT), window))
        .or_else(|| Kbd::binding_for_action(action, None, window))
}

/// A centred title with one button per action, each followed by its key hint when bound.
#[derive(IntoElement)]
pub struct EmptyState {
    id: ElementId,
    title: SharedString,
    actions: Vec<(SharedString, Box<dyn Action>)>,
    key_context: Option<&'static str>,
    note: Option<(usize, ElementId, SharedString)>,
}

impl EmptyState {
    /// Also looks key hints up in `context`, for actions bound only inside a view that may
    /// not hold focus.
    pub fn key_context(mut self, context: &'static str) -> EmptyState {
        self.key_context = Some(context);
        self
    }

    /// A muted line `id` under action `ix` that qualifies that action alone.
    pub fn note(
        mut self,
        ix: usize,
        id: impl Into<ElementId>,
        text: impl Into<SharedString>,
    ) -> EmptyState {
        self.note = Some((ix, id.into(), text.into()));
        self
    }
}

/// An empty state scoped by `id`, offering `actions` as labelled buttons.
pub fn empty_state(
    id: impl Into<ElementId>,
    title: &str,
    actions: &[(&str, &dyn Action)],
) -> EmptyState {
    EmptyState {
        id: id.into(),
        title: title.to_string().into(),
        actions: actions
            .iter()
            .map(|(label, action)| (label.to_string().into(), action.boxed_clone()))
            .collect(),
        key_context: None,
        note: None,
    }
}

impl RenderOnce for EmptyState {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let context = self.key_context;
        let muted = cx.theme().muted_foreground;
        let mut note = self.note;
        let rows = self
            .actions
            .into_iter()
            .enumerate()
            .map(|(ix, (label, action))| {
                let hint = key_hint(action.as_ref(), context, window, cx);
                // The hint hangs off the button's right edge, out of the row's layout, so a
                // hint that appears when focus arrives on press never moves the button.
                h_flex()
                    .relative()
                    .child(
                        Button::new(("empty-action", ix))
                            .ghost()
                            .label(label)
                            .on_click(move |_, window, cx| {
                                window.dispatch_action(action.boxed_clone(), cx)
                            }),
                    )
                    .children(hint.map(|kbd| {
                        h_flex()
                            .id(("empty-kbd", ix))
                            .test_support()
                            .absolute()
                            .top_0()
                            .h_full()
                            .left(relative(1.))
                            .ml(SPACE_2)
                            .child(kbd)
                    }))
                    .into_any_element()
            })
            .enumerate()
            .map(|(ix, row)| match note.take_if(|(at, ..)| *at == ix) {
                Some((_, id, text)) => v_flex()
                    .items_center()
                    .child(row)
                    .child(
                        div()
                            .id(id)
                            .test_support()
                            .aria_label(text.clone())
                            .text_size(TEXT_SMALL)
                            .text_color(muted)
                            .child(text),
                    )
                    .into_any_element(),
                None => row,
            });
        v_flex()
            .id(self.id)
            .test_support()
            .size_full()
            .items_center()
            .justify_center()
            .gap(SPACE_3)
            .p(SPACE_5)
            .child(
                div()
                    .id("empty-title")
                    .test_support()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.title),
            )
            .child(v_flex().items_center().gap(SPACE_1).children(rows))
    }
}

/// Whether `text`, trimmed, is exactly one `{{name}}` reference; the name may be dotted.
pub fn is_variable_reference(text: &str) -> bool {
    let Some(name) = text
        .trim()
        .strip_prefix("{{")
        .and_then(|rest| rest.strip_suffix("}}"))
    else {
        return false;
    };
    !name.is_empty()
        && !name
            .chars()
            .any(|c| c.is_whitespace() || c == '{' || c == '}')
}

/// Masks a secret input unless its whole text, trimmed, is exactly one `{{var}}` reference,
/// which shows so the user sees what it names; anything else stays hidden.
pub fn mask_unless_template(
    input: &gpui_kit::Entity<gpui_kit::component::input::InputState>,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::App,
) {
    let state = input.read(cx);
    let masked = !is_variable_reference(&state.value());
    if state.presentation().is_masked() != masked {
        input.update(cx, |s, cx| s.set_masked(masked, window, cx));
    }
}

/// Masks secret inputs by their text, as [`mask_unless_template`] does, but only when an
/// input's text turns into or out of one `{{var}}` reference, so a reveal by the input's eye
/// button holds while the user types.
#[derive(Default)]
pub struct SecretMasks(HashMap<gpui_kit::EntityId, bool>);

impl SecretMasks {
    /// Masks `input` unless its text is one reference, when that has changed since last seen.
    pub fn apply(&mut self, input: &Entity<InputState>, window: &mut Window, cx: &mut App) {
        let reference = is_variable_reference(&input.read(cx).value());
        if self.0.insert(input.entity_id(), reference) == Some(reference) {
            return;
        }
        if input.read(cx).presentation().is_masked() == reference {
            input.update(cx, |s, cx| s.set_masked(!reference, window, cx));
        }
    }

    /// Puts `text` in `input` for a value the user did not type; a changed text is masked
    /// again by the next [`SecretMasks::apply`], so a reveal never shows it.
    pub fn fill(
        &mut self,
        input: &Entity<InputState>,
        text: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        if input.read(cx).value() != text {
            self.0.remove(&input.entity_id());
        }
        sync_input_text(input, text, window, cx);
    }

    /// Forgets every reveal, so the next [`SecretMasks::apply`] masks each input by its text.
    pub fn forget(&mut self) {
        self.0.clear();
    }
}

/// What a secret field shows while its value is in `.env`, or held for it until the save:
/// a note and a Replace button in place of the input, so the reference text is never shown
/// or edited by accident. The tooltip says where the value lives.
pub fn stored_secret(
    id: impl Into<ElementId>,
    replace_id: impl Into<ElementId>,
    home: crate::snapshot::SecretHome,
    on_replace: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> impl IntoElement {
    use crate::snapshot::SecretHome;
    let (text, tip) = match home {
        SecretHome::Held => (
            "Saved to .env when you save",
            "When you save, the value goes into this collection's .env file, which is kept \
             out of git.",
        ),
        SecretHome::Stored => (
            "Stored in .env",
            "The value lives in this collection's .env file, which is kept out of git.",
        ),
    };
    h_flex()
        .id(id)
        .test_support()
        .gap(SPACE_2)
        .items_center()
        .min_w_0()
        .aria_label(text)
        .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
        .child(div().text_color(cx.theme().muted_foreground).child(text))
        .child(
            Button::new(replace_id)
                .label("Replace")
                .small()
                .on_click(on_replace),
        )
}

/// The step a Left or Right key takes along a tab strip: -1, +1, or none for any other key.
pub fn arrow_step(key: &str) -> Option<isize> {
    match key {
        "left" => Some(-1),
        "right" => Some(1),
        _ => None,
    }
}

/// `view` drawn from its last frame until it is notified, so a key typed elsewhere in the
/// window does not lay it out and paint it again; it fills its parent.
pub(crate) fn cached<V: Render>(view: &Entity<V>) -> impl IntoElement + use<V> {
    view.clone()
        .cached(gpui_kit::StyleRefinement::default().size_full())
}

/// Redraws the calling view on every settings change. A view drawn `cached` needs it:
/// theme writes refresh every window, but a setting the theme does not carry refreshes
/// nothing. `None` without an `AppState`.
pub(crate) fn redraw_on_settings<V: 'static>(
    cx: &mut gpui_kit::Context<V>,
) -> Option<gpui_kit::Subscription> {
    let settings = cx.try_global::<AppState>()?.settings.clone();
    Some(cx.observe(&settings, |_, _, cx| cx.notify()))
}

/// A select row valued by its label, whose drawn row registers as `{select}-option-{key}`.
///
/// The kit draws select rows without `.test_support()`, so the row content registers itself;
/// a click on it bubbles to the kit row's confirm handler.
#[derive(Debug, Clone)]
pub struct IdItem {
    id: SharedString,
    label: SharedString,
}

/// A kit select over [`IdItem`] rows.
pub type IdSelect = gpui_kit::component::select::SelectState<Vec<IdItem>>;

impl IdItem {
    /// A row of the select `select` showing `label`, found by the stable `key`.
    pub fn new(select: &str, key: &str, label: impl Into<SharedString>) -> Self {
        IdItem {
            id: format!("{select}-option-{key}").into(),
            label: label.into(),
        }
    }
}

/// Rows of `select`, one per label, each keyed by its label as shown so distinct labels
/// never share an id.
pub fn id_items<L: Into<SharedString>>(
    select: &str,
    labels: impl IntoIterator<Item = L>,
) -> Vec<IdItem> {
    labels
        .into_iter()
        .map(|l| {
            let label = l.into();
            IdItem::new(select, &label, label.clone())
        })
        .collect()
}

impl gpui_kit::component::select::SelectItem for IdItem {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &SharedString {
        &self.label
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .id(self.id.clone())
            .test_support()
            .child(self.label.clone())
    }
}
