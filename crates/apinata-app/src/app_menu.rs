//! The title bar's app menu: one dropdown over the window-level actions the shell handles.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::{Action, Anchor, App, FocusHandle, IntoElement};

use crate::commands::registry;
use crate::stand_in::StandIn;

const MENU_GROUPS: [&[&str]; 4] = [
    &[
        "collection.open",
        "collection.new",
        "collection.clone",
        "collection.import",
        "collection.import_curl",
        "collection.export",
        "collection.export_keep_scripts",
        "collection.export_openapi",
        "collection.export_html",
        "collection.run",
        "collection.settings",
        "collection.switch_branch",
        "collection.show_changes",
    ],
    &[
        "collection.new_request",
        "request.save",
        "request.save_all",
        "request.generate_code",
    ],
    &[
        "environment.open",
        "environment.manage_globals",
        "environment.import",
        "environment.cookies",
        "environment.runtime_variables",
        "view.history",
    ],
    &[
        "view.preferences",
        "view.check_for_updates",
        "view.command_palette",
    ],
];

/// The menu's groups, in order: registry commands looked up by id, each as its label and
/// first target's action.
// Menu ids and their targets are fixed registry entries, so a missing one is a bug every launch hits.
#[allow(clippy::expect_used)]
pub fn app_menu_groups() -> Vec<Vec<(&'static str, Box<dyn Action>)>> {
    let mut commands = registry();
    MENU_GROUPS
        .iter()
        .map(|ids| {
            ids.iter()
                .map(|id| {
                    let ix = commands.iter().position(|c| c.id == *id).expect(id);
                    let c = commands.swap_remove(ix);
                    (
                        c.label,
                        c.targets
                            .into_iter()
                            .next()
                            .expect("every menu command has a target")
                            .action,
                    )
                })
                .collect()
        })
        .collect()
}

/// Every menu entry in order, without the separators between groups.
pub fn app_menu_items() -> Vec<(&'static str, Box<dyn Action>)> {
    app_menu_groups().into_iter().flatten().collect()
}

/// The popup row that dispatches `action`, counting one separator row between groups.
pub fn app_menu_index(action: &dyn Action) -> Option<usize> {
    let mut row = 0;
    for (i, group) in app_menu_groups().iter().enumerate() {
        if i > 0 {
            row += 1;
        }
        for (_, a) in group {
            if a.partial_eq(action) {
                return Some(row);
            }
            row += 1;
        }
    }
    None
}

/// `#app-menu`: a ghost Menu-icon button whose dropdown lists `app_menu_groups`, split by
/// separators, and dispatches from `shell` (the PopupMenu's action_context). `stand_in` keeps
/// what held focus before the menu opened while it is open, and `open` follows the popup.
pub fn app_menu(
    shell: FocusHandle,
    stand_in: Rc<StandIn>,
    open: Rc<Cell<bool>>,
    scratch_only: bool,
    in_repo: bool,
    cx: &App,
) -> impl IntoElement {
    // The open popup holds focus, so entries dispatch from the shell's handle to reach
    // the same handlers as the shell's key bindings.
    let disabled = app_menu_disabled(scratch_only, in_repo);
    crate::ui::solid_ring(Button::new("app-menu").ghost().compact(), cx)
        .icon(IconName::Menu)
        .accessibility_label("Application menu")
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
            app_menu_groups().into_iter().enumerate().fold(
                menu.action_context(shell.clone()),
                |menu, (i, group)| {
                    let menu = if i > 0 { menu.separator() } else { menu };
                    group.into_iter().fold(menu, |menu, (label, action)| {
                        menu.menu_with_disabled(label, action, disabled.contains(&label))
                    })
                },
            )
        })
        .on_open_change(move |now, window, _| {
            stand_in.freeze(*now);
            // The shell draws the slot's tooltip from this, so it redraws on each change.
            open.set(*now);
            window.refresh();
        })
}

/// Command ids of the entries that act on a collection, which need a real one open.
pub(crate) const COLLECTION_ONLY: [&str; 6] = [
    "collection.export",
    "collection.export_keep_scripts",
    "collection.export_openapi",
    "collection.export_html",
    "collection.run",
    "collection.settings",
];

/// Command ids of the entries that act on the current collection's git repository.
const REPOSITORY_ONLY: [&str; 2] = ["collection.switch_branch", "collection.show_changes"];

/// The labels of the entries to grey out: those that act on a collection while only the
/// scratch pad is open, and those that act on a repository while the collection is in none.
pub fn app_menu_disabled(scratch_only: bool, in_repo: bool) -> Vec<&'static str> {
    registry()
        .into_iter()
        .filter(|c| {
            (scratch_only && COLLECTION_ONLY.contains(&c.id))
                || (!in_repo && REPOSITORY_ONLY.contains(&c.id))
        })
        .map(|c| c.label)
        .collect()
}
