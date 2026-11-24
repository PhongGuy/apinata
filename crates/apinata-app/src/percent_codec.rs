//! Percent-encode and Percent-decode of the focused text field's selection, from the palette
//! and the field's right-click menu.

use apinata_core::{percent_decode, percent_encode, widen_to_placeholders};
use gpui_kit::base::input::InputContextMenuCapabilities;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{
    Copy, Cut, GoToDefinition, InputState, Paste, SelectAll, ToggleCodeActions,
};
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::prelude::*;
use gpui_kit::{Action, App, Entity, Focusable as _, WeakEntity, Window, actions};

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

/// Shown when decoding would put a line break or another control character in the field.
pub const CONTROL: &str = "The decoded text holds a line break or another control character, which this field cannot hold, so it was left as it was.";

/// Shown when the selection's escapes are not UTF-8 text.
pub const NOT_UTF8: &str = "The selection does not decode to UTF-8 text, so it was left as it was.";

// Every single-line field created since install. A field cannot be reached from the focus
// handle the window reports, so the commands look it up here by its own focus.
#[derive(Default)]
struct Fields(Vec<WeakEntity<InputState>>);

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
    cx.observe_new(|_: &mut InputState, _, cx: &mut Context<InputState>| {
        let field = cx.entity().downgrade();
        cx.update_global::<Fields, _>(|fields, _| {
            fields.0.retain(|field| field.upgrade().is_some());
            fields.0.push(field);
        });
    })
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

/// The recorded field that holds focus in `window`, whichever window created it.
pub fn focused_field(window: &Window, cx: &App) -> Option<Entity<InputState>> {
    let fields = cx.try_global::<Fields>()?;
    (fields.0.iter())
        .filter_map(WeakEntity::upgrade)
        .find(|field| field.read(cx).focus_handle(cx).is_focused(window))
}

/// The two commands' menu entries for `input`, each with whether it is disabled; none when
/// the field does not allow them.
pub fn codec_entries(input: &InputState) -> Vec<(&'static str, bool)> {
    if !applies(input) {
        return Vec::new();
    }
    let off = !offered(input);
    vec![("Percent-encode", off), ("Percent-decode", off)]
}

/// The kit's own Cut, Copy, Paste and Select All, then the two commands when `input` applies,
/// disabled while nothing is selected.
pub fn input_menu(menu: NativeMenu, input: &InputState, cx: &App) -> NativeMenu {
    let menu = edit_entries(menu, &input.context_menu_capabilities(), cx);
    let item = |menu: NativeMenu, label: &'static str, off: bool, action: Box<dyn Action>| {
        menu.menu_with_disabled(ui::native_menu_label(label, off, cx), off, action)
    };
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

/// The entries the kit's own menu gives a field with `caps`: a code editor's Go to Definition
/// and Show Code Actions, then Cut, Copy, Paste and Select All.
pub fn edit_entries(menu: NativeMenu, caps: &InputContextMenuCapabilities, cx: &App) -> NativeMenu {
    let enabled = !caps.is_disabled();
    let editable = enabled && !caps.is_readonly();
    let item = |menu: NativeMenu, label: &'static str, off: bool, action: Box<dyn Action>| {
        menu.menu_with_disabled(ui::native_menu_label(label, off, cx), off, action)
    };
    let menu = match caps.is_code_editor() {
        true => {
            let off = !(enabled && caps.has_definition());
            let menu = item(menu, "Go to Definition", off, Box::new(GoToDefinition));
            let off = !(editable && caps.has_code_actions());
            item(menu, "Show Code Actions", off, Box::new(ToggleCodeActions)).separator()
        }
        false => menu,
    };
    let menu = item(
        menu,
        "Cut",
        !(editable && caps.is_copyable()),
        Box::new(Cut),
    );
    let menu = item(menu, "Copy", !caps.is_copyable(), Box::new(Copy));
    let menu = item(menu, "Paste", !editable, Box::new(Paste));
    menu.separator().menu("Select All", Box::new(SelectAll))
}

// The handler runs inside the window's dispatch, so the edit waits until that ends. That
// window is the one open window that cannot be updated now, since its update is running.
fn run(codec: Codec, cx: &mut App) {
    let dispatching = (cx.windows().into_iter()).find(|w| w.update(cx, |_, _, _| ()).is_err());
    let Some(window) = dispatching.or_else(|| cx.active_window()) else {
        return;
    };
    cx.defer(move |cx| {
        window
            .update(cx, |_, window, cx| apply(codec, window, cx))
            .ok();
    });
}

// The selection, grown to whole placeholders, is replaced in one undoable edit and stays
// selected. A decode that is not UTF-8, or that would put any control character into the
// single-line field, changes nothing and says why; the field would drop a tab without a word.
fn apply(codec: Codec, window: &mut Window, cx: &mut App) {
    let Some(field) = focused_field(window, cx).filter(|f| offered(f.read(cx))) else {
        return;
    };
    let (range, selected) = {
        let state = field.read(cx);
        let value = state.value();
        let range = widen_to_placeholders(&value, state.selected_range());
        let selected = value[range.clone()].to_owned();
        (range, selected)
    };
    let changed = match codec {
        Codec::Encode => Ok(percent_encode(&selected)),
        Codec::Decode => percent_decode(&selected)
            .map_err(|_| NOT_UTF8)
            .and_then(|text| {
                let control = text.chars().any(char::is_control);
                if control { Err(CONTROL) } else { Ok(text) }
            }),
    };
    match changed {
        Ok(text) if text != selected => field.update(cx, |s, cx| {
            let end = range.start + text.len();
            s.set_selected_range(range.clone(), cx);
            s.replace(text, window, cx);
            s.set_selected_range(range.start..end, cx);
        }),
        Ok(_) => {}
        Err(message) => window.push_notification(ui::kind_toast(ToastKind::Error, message), cx),
    }
}
