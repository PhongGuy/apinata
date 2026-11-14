//! The single key registry: every binding the app makes, and the window-level actions the
//! `Shell` handles. Window keys bind in the `"Shell"` context so they work with focus
//! anywhere in the window; editor keys stay in their editor's context.

use std::rc::Rc;

use gpui_kit::base::actions::Confirm as SelectConfirm;
use gpui_kit::component::input::GoToDefinition;
#[cfg(not(target_os = "macos"))]
use gpui_kit::component::input::Redo as EditorRedo;
use gpui_kit::{Action, App, Global, KeyBinding, KeyBindingContextPredicate, NoAction, actions};

use crate::app::AppState;
use crate::body_find::{
    CloseFindInResponse, KEY_CONTEXT as RESPONSE_BODY, RowDown, RowFirst, RowLast, RowPageDown,
    RowPageUp, RowUp,
};
use crate::changes_dialog::{CommitChanges, MESSAGE_CONTEXT};
use crate::commands::registry;
use crate::graphql_section::GRAPHQL_QUERY_CONTEXT;
use crate::preferences::{
    ClearPreferencesSearch, FocusPreferencesSearch, KEY_CONTEXT as PREFERENCES,
};
use crate::shortcuts::{Overrides, effective};
use crate::sidebar::{
    ClearFilter, ClearSelection, EnterTree, ExtendDown, ExtendUp, FILTER_CONTEXT,
    KEY_CONTEXT as SIDEBAR, LeaveTree, OpenRowMenu, OpenSelected, PreviewSelected, RunSelected,
};
use crate::ui::{
    ActivateSegment, DIALOG_FOOTER_CONTEXT, DIALOG_SELECT_CONTEXT, NextSegment, OpenSelect,
    PrevSegment, SEGMENTED_CONTEXT,
};

/// The key context of the window's root view.
pub const SHELL_CONTEXT: &str = "Shell";

/// The key context the kit puts on every dialog's root, palettes included. The kit draws
/// dialogs outside the Shell, so window-wide chords that must work there bind it too.
pub const DIALOG_CONTEXT: &str = "Dialog";

actions!(
    apinata,
    [
        /// Saves every dirty target of the active tab.
        SaveActive,
        /// Opens a draft request in the current collection.
        NewRequest,
        /// Opens a draft GraphQL request in the current collection.
        NewGraphqlRequest,
        /// Hides or shows the sidebar.
        ToggleSidebar,
        /// Flips the response pane between below and beside the request.
        ToggleOrientation,
        /// Shows or hides the console dock.
        ToggleConsole,
        /// Switches between the light and dark theme.
        ToggleTheme,
        /// Opens the environments tab of the current collection.
        OpenEnvironments,
        /// Opens the settings tab of the current collection.
        OpenCollectionSettings,
        /// Opens the runner of the current collection.
        RunCollection,
        /// Opens the runner of the active request's folder, or of the current collection.
        RunFolder,
        /// Zooms the app, or the response body when it is focused or hovered, one step in.
        ZoomIn,
        /// Zooms the app, or the response body when it is focused or hovered, one step out.
        ZoomOut,
        /// Resets the app zoom, or the response body's size when it is focused or hovered.
        ResetZoom,
        /// Puts the caret in the active request tab's URL input.
        FocusUrl,
        /// Shows the next section of the active request editor, wrapping.
        NextSection,
        /// Shows the previous section of the active request editor, wrapping.
        PreviousSection,
        /// Moves focus to the active request's response body.
        FocusResponse,
        /// Opens the environment dropdown in the title bar.
        OpenEnvironmentSelector,
        /// Reverts the active tab's unsaved changes, after asking.
        DiscardChanges,
        /// Undoes the active request, settings or environments tab's last structured edit.
        UndoActive,
        /// Redoes the last structured edit undone in the active request, settings or
        /// environments tab.
        RedoActive
    ]
);

/// Every key binding of the app under `overrides`: each command's effective chords on each
/// of its targets, then the fixed bindings no command owns.
pub fn bindings_with(overrides: &Overrides, cx: &App) -> Vec<KeyBinding> {
    let mut b = Vec::new();
    // The kit binds Redo to Ctrl+Y off macOS and Cmd+Shift+Z on it; Ctrl+Shift+Z joins
    // Ctrl+Y so every code editor redoes as in most editors. It comes before the commands
    // because a later binding wins a tie, and the table cells' redo must beat it.
    #[cfg(not(target_os = "macos"))]
    b.push(KeyBinding::new("ctrl-shift-z", EditorRedo, Some("Input")));
    for cmd in registry() {
        for chord in effective(&cmd, overrides) {
            for target in &cmd.targets {
                b.push(bind(
                    &chord.to_file_string(),
                    target.action.as_ref(),
                    target.context,
                    cx,
                ));
            }
        }
    }
    b.extend(fixed_bindings());
    b
}

/// Every key binding of the app with each command on its default chords.
pub fn bindings(cx: &App) -> Vec<KeyBinding> {
    bindings_with(&Overrides::new(), cx)
}

// Every chord comes from `Chord`, which only holds parsable keystrokes, so a failure here is a bug.
#[allow(clippy::expect_used)]
fn bind(chord: &str, action: &dyn Action, context: &str, cx: &App) -> KeyBinding {
    let predicate = KeyBindingContextPredicate::parse(context)
        .expect("every binding context is a fixed predicate from the command registry");
    KeyBinding::load(
        chord,
        action.boxed_clone(),
        Some(Rc::new(predicate)),
        false,
        None,
        cx,
    )
    .expect("every chord comes from `Chord`, which only holds parsable keystrokes")
}

// `KeyBinding::new` parses without a host OS, so it refuses `secondary-`; this spells the
// secondary modifier out for the build target as a static chord.
macro_rules! secondary {
    ($key:literal) => {
        if $crate::shortcuts::HOST_OS == gpui_kit::OperatingSystem::Mac {
            concat!("cmd-", $key)
        } else {
            concat!("ctrl-", $key)
        }
    };
}

/// Bindings no command owns: the Preferences search keys, the segmented-control and tree
/// keys, and the filter guard.
pub fn fixed_bindings() -> Vec<KeyBinding> {
    // Segments bind Enter deeper than the dialog's `enter -> Confirm`, so it chooses
    // instead of closing; Space binds on key-down so a key-down-only press counts.
    let seg = Some(SEGMENTED_CONTEXT);
    // The kit's tree binds only the arrows; Enter and Space act on its selected row.
    let tree = Some("Sidebar > Tree");
    let body = Some(RESPONSE_BODY);
    vec![
        // Ctrl+enter is the sidebar filter's own and must not send from there.
        // Pinned by tests/keymap.rs `send_works_from_the_url_input_but_not_the_filter`.
        KeyBinding::new(
            secondary!("enter"),
            NoAction,
            Some(format!("{FILTER_CONTEXT} > Input").as_str()),
        ),
        // Escape in the filter clears it before the kit input sees the key; empty, it goes
        // back to the tree.
        KeyBinding::new(
            "escape",
            ClearFilter,
            Some(format!("{FILTER_CONTEXT} > Input").as_str()),
        ),
        // Anywhere else in the sidebar, Escape clears the filter too, after the tree's own
        // Escape has passed; with the filter empty it returns to the tree.
        KeyBinding::new("escape", ClearFilter, Some(SIDEBAR)),
        // Nor from the rename input inside the tree, while the tree itself still sends.
        KeyBinding::new(
            secondary!("enter"),
            NoAction,
            Some("Sidebar > Tree > Input"),
        ),
        // Down in the filter walks on into the tree, as in most filter-a-list fields.
        KeyBinding::new(
            "down",
            EnterTree,
            Some(format!("{FILTER_CONTEXT} > Input").as_str()),
        ),
        // Commits from the Changes dialog's message field, where plain Ctrl+Enter would
        // otherwise insert a newline.
        KeyBinding::new(
            secondary!("enter"),
            CommitChanges,
            Some(format!("{MESSAGE_CONTEXT} > Input").as_str()),
        ),
        // A focused footer button or checkbox takes Enter natively, outranking the dialog's
        // `enter -> Confirm`, which would otherwise answer the default instead.
        KeyBinding::new("enter", NoAction, Some(DIALOG_FOOTER_CONTEXT)),
        // A kit select in a dialog opens on Enter; its own Confirm would reach the dialog too.
        KeyBinding::new(
            "enter",
            OpenSelect,
            Some(format!("{DIALOG_SELECT_CONTEXT} > Select").as_str()),
        ),
        // The kit select binds only Enter; Space does the same, as on a native select.
        KeyBinding::new("space", SelectConfirm { secondary: false }, Some("Select")),
        KeyBinding::new(
            "space",
            OpenSelect,
            Some(format!("{DIALOG_SELECT_CONTEXT} > Select").as_str()),
        ),
        KeyBinding::new(secondary!("f"), FocusPreferencesSearch, Some(PREFERENCES)),
        KeyBinding::new("escape", ClearPreferencesSearch, Some(PREFERENCES)),
        KeyBinding::new("enter", ActivateSegment, seg),
        KeyBinding::new("space", ActivateSegment, seg),
        KeyBinding::new("right", NextSegment, seg),
        KeyBinding::new("down", NextSegment, seg),
        KeyBinding::new("left", PrevSegment, seg),
        KeyBinding::new("up", PrevSegment, seg),
        KeyBinding::new("enter", OpenSelected, tree),
        KeyBinding::new("space", PreviewSelected, tree),
        // The Menu key and Shift+F10 open the selected row's context menu, as on most desktops.
        KeyBinding::new("menu", OpenRowMenu, tree),
        KeyBinding::new("shift-f10", OpenRowMenu, tree),
        // F12 follows a name in the GraphQL query editor to its Docs page.
        KeyBinding::new(
            "f12",
            GoToDefinition,
            Some(format!("{GRAPHQL_QUERY_CONTEXT} > Input").as_str()),
        ),
        KeyBinding::new("shift-enter", RunSelected, tree),
        KeyBinding::new("escape", ClearSelection, tree),
        KeyBinding::new("shift-up", ExtendUp, tree),
        KeyBinding::new("shift-down", ExtendDown, tree),
        // The tree's Tab stop forwards focus, so the window's own shift+tab would land back
        // on it; from the tree, shift+tab goes to the stop above. Its own action keeps it
        // out of FocusFilter's key hints.
        KeyBinding::new("shift-tab", LeaveTree, tree),
        // The response body's rows list. An input inside the body binds these keys deeper and
        // wins; the handlers also act only while the body itself holds focus.
        KeyBinding::new("escape", CloseFindInResponse, body),
        KeyBinding::new("up", RowUp, body),
        KeyBinding::new("down", RowDown, body),
        KeyBinding::new("pageup", RowPageUp, body),
        KeyBinding::new("pagedown", RowPageDown, body),
        KeyBinding::new("home", RowFirst, body),
        KeyBinding::new("ctrl-home", RowFirst, body),
        KeyBinding::new("end", RowLast, body),
        KeyBinding::new("ctrl-end", RowLast, body),
    ]
}

// The kit binds its own keys once in `gpui_kit::init`; clearing the keymap would lose them,
// so they are kept here and re-bound first on every apply.
struct KitBindings(Vec<KeyBinding>);

impl Global for KitBindings {}

// Debug builds of the kit bind its inspector to Ctrl+Shift+I with no context, which beats
// the Shell's import chord whenever focus sits in an input.
const KIT_INSPECTOR: &str = "inspector::ToggleInspector";

/// Snapshots the kit's bindings, minus its inspector chord, once per app (later calls keep
/// the first snapshot), then applies no overrides. Call after `gpui_kit::init`, so an
/// `Input`-depth binding here outranks the input's own key.
pub fn init(cx: &mut App) {
    if !cx.has_global::<KitBindings>() {
        let kit = cx
            .key_bindings()
            .borrow()
            .bindings()
            .filter(|b| b.action().name() != KIT_INSPECTOR)
            .cloned()
            .collect();
        cx.set_global(KitBindings(kit));
    }
    apply(&Overrides::new(), cx);
}

/// Rebuilds the keymap: the kit snapshot first, then `bindings_with(overrides)`.
pub fn apply(overrides: &Overrides, cx: &mut App) {
    let kit = kit_bindings(cx);
    cx.clear_key_bindings();
    cx.bind_keys(kit);
    let bindings = bindings_with(overrides, cx);
    cx.bind_keys(bindings);
}

/// The kit's own bindings as snapshotted by `init`; empty before it.
pub fn kit_bindings(cx: &App) -> Vec<KeyBinding> {
    cx.try_global::<KitBindings>()
        .map(|k| k.0.clone())
        .unwrap_or_default()
}

/// Edits AppState's keymap (saved debounced) and applies the result; does nothing without
/// AppState.
pub fn set_overrides(f: impl FnOnce(&mut Overrides), cx: &mut App) {
    let Some(keymap) = cx.try_global::<AppState>().map(|s| s.keymap.clone()) else {
        return;
    };
    let overrides = keymap.update(cx, |p, cx| {
        p.update(cx, f);
        p.get().clone()
    });
    apply(&overrides, cx);
}
