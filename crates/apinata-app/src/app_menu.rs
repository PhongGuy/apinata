//! The title bar's app menu: one dropdown over the window-level actions the shell handles.

use crate::ui::SolidRing as _;
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
        "view.about",
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

/// The label of the submenu that holds the exports, in this menu and a collection's row menu.
pub const EXPORT_MENU: &str = "Export collection";

/// Command ids of the exports the Export submenu holds, in order.
pub const EXPORT_COMMANDS: [&str; 4] = [
    "collection.export",
    "collection.export_keep_scripts",
    "collection.export_openapi",
    "collection.export_html",
];

// One top-level row of the open menu: a command, or the Export submenu with its commands.
enum Row {
    Command(&'static str, Box<dyn Action>),
    Exports(Vec<(&'static str, Box<dyn Action>)>),
}

impl Row {
    fn label(&self) -> &'static str {
        match self {
            Row::Command(label, _) => label,
            Row::Exports(_) => EXPORT_MENU,
        }
    }
}

fn is_export(label: &str) -> bool {
    EXPORT_COMMANDS
        .iter()
        .any(|id| crate::commands::label(id) == label)
}

// The groups as the menu draws them: the exports fold into one Export row where the first of
// them stood.
fn rows() -> Vec<Vec<Row>> {
    app_menu_groups()
        .into_iter()
        .map(|group| {
            let mut rows: Vec<Row> = Vec::new();
            for (label, action) in group {
                if !is_export(label) {
                    rows.push(Row::Command(label, action));
                } else if let Some(Row::Exports(exports)) =
                    rows.iter_mut().find(|r| matches!(r, Row::Exports(_)))
                {
                    exports.push((label, action));
                } else {
                    rows.push(Row::Exports(vec![(label, action)]));
                }
            }
            rows
        })
        .collect()
}

/// The labels of the open menu's top-level rows by group, the exports folded into
/// [`EXPORT_MENU`].
pub fn app_menu_rows() -> Vec<Vec<&'static str>> {
    rows()
        .iter()
        .map(|g| g.iter().map(Row::label).collect())
        .collect()
}

/// Every menu entry in order, without the separators between groups.
pub fn app_menu_items() -> Vec<(&'static str, Box<dyn Action>)> {
    app_menu_groups().into_iter().flatten().collect()
}

/// The popup row that dispatches `action`, or the Export row for an export, counting one
/// separator row between groups.
pub fn app_menu_index(action: &dyn Action) -> Option<usize> {
    let mut row = 0;
    for (i, group) in rows().iter().enumerate() {
        if i > 0 {
            row += 1;
        }
        for r in group {
            let holds = match r {
                Row::Command(_, a) => a.partial_eq(action),
                Row::Exports(exports) => exports.iter().any(|(_, a)| a.partial_eq(action)),
            };
            if holds {
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
    Button::new("app-menu")
        .solid_ring(cx)
        .ghost()
        .compact()
        .icon(IconName::Menu)
        .accessibility_label("Application menu")
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, window, cx| {
            let mut menu = menu.action_context(shell.clone());
            for (i, group) in rows().into_iter().enumerate() {
                if i > 0 {
                    menu = menu.separator();
                }
                for row in group {
                    let label = row.label();
                    menu = match (row, disabled.iter().find(|(l, _)| *l == label)) {
                        (_, Some(&(_, reason))) => menu.item(greyed_entry(label, reason)),
                        (Row::Command(_, action), None) => menu.menu(label, action),
                        // The submenu inherits the shell as its action context from this menu.
                        (Row::Exports(exports), None) => {
                            menu.submenu(label, window, cx, move |sub, _, _| {
                                exports.iter().fold(sub, |sub, (label, action)| {
                                    sub.menu(*label, action.boxed_clone())
                                })
                            })
                        }
                    };
                }
            }
            menu
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
    let mut off = registry()
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
        .collect::<Vec<_>>();
    // The Export row greys out, for the same reason, when every export in it does.
    let exports: Vec<_> = (off.iter())
        .filter(|(label, _)| is_export(label))
        .copied()
        .collect();
    if let Some(&(_, reason)) = exports.first()
        && exports.len() == EXPORT_COMMANDS.len()
    {
        off.push((EXPORT_MENU, reason));
    }
    off
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
