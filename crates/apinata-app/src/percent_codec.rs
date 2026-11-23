//! Percent-encode and Percent-decode of the focused text field's selection, from the palette
//! and the field's right-click menu.

use apinata_core::{percent_decode, percent_encode};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{Copy, Cut, InputState, Paste, SelectAll};
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::prelude::*;
use gpui_kit::{Action, AnyWindowHandle, App, Entity, Focusable as _, WeakEntity, Window, actions};

use crate::ui::{self, ToastKind};

actions!(
    apinata,
    [
        /// Percent-encodes the focused field's selection.
        PercentEncode,
        /// Percent-decodes the focused field's selection.
        PercentDecode
    ]
);

/// Shown when the selection's escapes are not UTF-8 text.
pub const NOT_UTF8: &str = "The selection does not decode to UTF-8 text, so it was left as it was.";

// Every single-line field created since install, with its window. A field cannot be reached
// from the focus handle the window reports, so the commands look it up here.
#[derive(Default)]
struct Fields(Vec<(AnyWindowHandle, WeakEntity<InputState>)>);

impl gpui_kit::Global for Fields {}

#[derive(Clone, Copy)]
enum Codec {
    Encode,
    Decode,
}

/// Records every field created from now on and handles the two commands wherever focus is,
/// dialogs included. A second call does nothing.
pub fn install(cx: &mut App) {
    if cx.has_global::<Fields>() {
        return;
    }
    cx.set_global(Fields::default());
    cx.observe_new(
        |_: &mut InputState, window: Option<&mut Window>, cx: &mut Context<InputState>| {
            let Some(window) = window else {
                return;
            };
            let entry = (window.window_handle(), cx.entity().downgrade());
            cx.update_global::<Fields, _>(|fields, _| {
                fields.0.retain(|(_, field)| field.upgrade().is_some());
                fields.0.push(entry);
            });
        },
    )
    .detach();
    cx.on_action(|_: &PercentEncode, cx| run(Codec::Encode, cx));
    cx.on_action(|_: &PercentDecode, cx| run(Codec::Decode, cx));
}

/// Whether the commands have a field to act on in `input`: a visible, editable value.
pub fn applies(input: &InputState) -> bool {
    !input.context_menu_capabilities().is_masked() && input.is_editable()
}

/// Whether the commands act on `input` now: it applies and holds a selection.
pub fn offered(input: &InputState) -> bool {
    applies(input) && !input.selected_range().is_empty()
}

/// The recorded field that holds focus in `window`.
pub fn focused_field(window: &Window, cx: &App) -> Option<Entity<InputState>> {
    let handle = window.window_handle();
    let fields = cx.try_global::<Fields>()?;
    (fields.0.iter())
        .filter(|(owner, _)| *owner == handle)
        .filter_map(|(_, field)| field.upgrade())
        .find(|field| field.read(cx).focus_handle(cx).is_focused(window))
}

/// The kit's own Cut, Copy, Paste and Select All, then the two commands when `input` applies,
/// disabled while nothing is selected.
pub fn input_menu(menu: NativeMenu, input: &InputState, cx: &App) -> NativeMenu {
    let caps = input.context_menu_capabilities();
    let editable = !caps.is_disabled() && !caps.is_readonly();
    let item = |menu: NativeMenu, label: &'static str, off: bool, action: Box<dyn Action>| {
        menu.menu_with_disabled(ui::native_menu_label(label, off, cx), off, action)
    };
    let menu = item(
        menu,
        "Cut",
        !(editable && caps.is_copyable()),
        Box::new(Cut),
    );
    let menu = item(menu, "Copy", !caps.is_copyable(), Box::new(Copy));
    let menu = item(menu, "Paste", !editable, Box::new(Paste));
    let menu = menu.separator().menu("Select All", Box::new(SelectAll));
    if !applies(input) {
        return menu;
    }
    let off = !offered(input);
    let menu = item(
        menu.separator(),
        "Percent-encode",
        off,
        Box::new(PercentEncode),
    );
    item(menu, "Percent-decode", off, Box::new(PercentDecode))
}

// The handler runs inside the window's dispatch, so the edit waits until that ends.
fn run(codec: Codec, cx: &mut App) {
    let Some(window) = cx.active_window() else {
        return;
    };
    cx.defer(move |cx| {
        window
            .update(cx, |_, window, cx| apply(codec, window, cx))
            .ok();
    });
}

// The selection is replaced in one undoable edit and stays selected; a decode that is not
// UTF-8 changes nothing and says why.
fn apply(codec: Codec, window: &mut Window, cx: &mut App) {
    let Some(field) = focused_field(window, cx).filter(|f| offered(f.read(cx))) else {
        return;
    };
    let (range, selected) = {
        let state = field.read(cx);
        (state.selected_range(), state.selected_text().to_string())
    };
    let changed = match codec {
        Codec::Encode => Ok(percent_encode(&selected)),
        Codec::Decode => percent_decode(&selected),
    };
    match changed {
        Ok(text) if text != selected => field.update(cx, |s, cx| {
            let end = range.start + text.len();
            s.replace(text, window, cx);
            s.set_selected_range(range.start..end, cx);
        }),
        Ok(_) => {}
        Err(_) => window.push_notification(ui::kind_toast(ToastKind::Error, NOT_UTF8), cx),
    }
}
