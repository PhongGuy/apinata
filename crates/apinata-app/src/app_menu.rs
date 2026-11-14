//! The title bar's app menu: one dropdown over the window-level actions the shell handles.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::base::{ElementExt as _, Placement};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::prelude::*;
use gpui_kit::{Action, Anchor, App, Bounds, FocusHandle, IntoElement, TestSupportExt as _, div};

use crate::commands::registry;
use crate::stand_in::StandIn;

const MENU_GROUPS: [&[&str]; 5] = [
    &[
        "collection.open",
        "collection.close",
        "collection.new",
        "collection.import",
        "collection.import_curl",
        "collection.export",
        "collection.export_keep_scripts",
        "collection.export_openapi",
        "collection.export_html",
        "collection.run",
        "collection.settings",
    ],
    &[
        "collection.clone",
        "collection.switch_branch",
        "collection.show_changes",
    ],
    &[
        "collection.new_request",
        "collection.new_graphql_request",
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
    updates: bool,
    cx: &App,
) -> impl IntoElement {
    // The open popup holds focus, so entries dispatch from the shell's handle to reach
    // the same handlers as the shell's key bindings.
    let disabled = app_menu_disabled_reasons(scratch_only, in_repo, updates);
    crate::ui::solid_ring(Button::new("app-menu").ghost().compact(), cx)
        .icon(IconName::Menu)
        .accessibility_label("Application menu")
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
            app_menu_groups().into_iter().enumerate().fold(
                menu.action_context(shell.clone()),
                |menu, (i, group)| {
                    let menu = if i > 0 { menu.separator() } else { menu };
                    group.into_iter().fold(menu, |menu, (label, action)| {
                        match disabled.iter().find(|(l, _)| *l == label) {
                            Some(&(_, reason)) => menu.item(greyed_entry(label, reason)),
                            None => menu.menu(label, action),
                        }
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

/// The id of the tooltip that says why a greyed entry cannot be used.
pub const APP_MENU_REASON_TIP: &str = "app-menu-reason-tooltip";

/// Command ids of the entries that act on a collection, which need a real one open.
pub(crate) const COLLECTION_ONLY: [&str; 7] = [
    "collection.close",
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
/// scratch pad is open, those that act on a repository while the collection is in none, and
/// Check for updates on a build without an update source.
pub fn app_menu_disabled(scratch_only: bool, in_repo: bool, updates: bool) -> Vec<&'static str> {
    app_menu_disabled_reasons(scratch_only, in_repo, updates)
        .into_iter()
        .map(|(label, _)| label)
        .collect()
}

/// The entries `app_menu_disabled` greys out, each with the reason its tooltip gives.
pub fn app_menu_disabled_reasons(
    scratch_only: bool,
    in_repo: bool,
    updates: bool,
) -> Vec<(&'static str, &'static str)> {
    registry()
        .into_iter()
        .filter_map(|c| {
            let reason = if scratch_only && COLLECTION_ONLY.contains(&c.id) {
                "Open a collection to use this"
            } else if !in_repo && REPOSITORY_ONLY.contains(&c.id) {
                "Open a collection in a git repository"
            } else if !updates && c.id == "view.check_for_updates" {
                crate::updates::NOT_CONFIGURED
            } else {
                return None;
            };
            Some((c.label, reason))
        })
        .collect()
}

// A greyed entry drawn as an element, since the kit's plain items take no tooltip; it fills the
// row so a hover anywhere on it shows the reason, which opens beside the menu so it never hides
// the entries below.
fn greyed_entry(label: &'static str, reason: &'static str) -> PopupMenuItem {
    PopupMenuItem::element(move |_, cx| {
        let row = Rc::new(Cell::new(Bounds::default()));
        let painted = Rc::clone(&row);
        crate::ui::menu_label_cue(div(), cx)
            .id(label)
            .test_support()
            .w_full()
            .aria_label(label)
            .aria_description(reason)
            .tooltip(crate::ui::text_tooltip_beside(
                APP_MENU_REASON_TIP,
                reason,
                row,
                Placement::Right,
            ))
            .child(label)
            .on_prepaint(move |bounds, _, _| painted.set(bounds))
    })
    .disabled(true)
}
