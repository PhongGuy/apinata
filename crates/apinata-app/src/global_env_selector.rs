//! The title bar's global environment picker, which follows the workspace rather than a
//! collection and selects the one global environment every collection's sends use.

use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, Focusable as _, Hsla,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _, Window, div,
};

use crate::commands::label;
use crate::env_color::{PickerGroups, env_fill, picker_row, scoped};
use crate::env_selector::{ACTIONS_GROUP, scope_tip};
use crate::global_env_editor::open_global_env_editor;
use crate::shell::GLOBAL_ENV_ICON;
use crate::style::ENV_CONTROL_WIDTH;
use crate::ui::ring_overlay;
use crate::workspace::Workspace;

/// The tooltip and accessible name of the global environment select.
pub const GLOBAL_ENV_SELECT_TIP: &str = "Global environment";

/// The header of the picker group that lists the global environments.
pub const GLOBALS_GROUP: &str = "Global environments";

/// The picker item that selects no global environment. It names the choice as the
/// collection picker's does, whether or not any globals exist, so it never reads as "none
/// exist" while some do.
pub const NO_GLOBAL_ENVIRONMENT: &str = "No global environment";

/// The last picker item, which opens the global environment editor instead of selecting.
pub fn manage_global_environments() -> &'static str {
    label("environment.manage_globals")
}

/// The id of the picker row for `item`; the prefix keeps a global named "none" apart from
/// the fixed row.
pub fn option_id(item: &str) -> SharedString {
    if item == NO_GLOBAL_ENVIRONMENT {
        "global-env-none".into()
    } else if item == manage_global_environments() {
        "global-env-manage".into()
    } else {
        format!("global-env-option-{item}").into()
    }
}

/// One picker row, valued by its name.
#[derive(Clone)]
pub struct GlobalEnvItem(
    /// The global environment name, or the fixed row.
    pub SharedString,
    /// The environment's colour, drawn as a dot beside the name.
    pub Option<Hsla>,
    /// Whether some row has a dot, so a row without one keeps its space and names line up.
    pub bool,
);

// As with the collection picker, the row content registers itself under `option_id` because
// the kit draws select rows without `.test_support()`, and the dot leaves the title bare.
impl SelectItem for GlobalEnvItem {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.0.clone()
    }

    fn display_title(&self) -> Option<AnyElement> {
        let icon = ("global-env-scope-icon", GLOBAL_ENV_ICON);
        let title = scoped(icon, ("global-env-trigger-dot", self.1), self.0.clone());
        Some(title.into_any_element())
    }

    fn value(&self) -> &SharedString {
        &self.0
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let ids = ("global-env-row-dot", "global-env-row-name");
        div()
            .id(option_id(&self.0))
            .test_support()
            .child(picker_row(ids, self.1, self.2, self.0.clone()))
    }
}

/// The picker's rows: the global environments, then the manage row in a group of its own.
pub type GlobalEnvRows = PickerGroups<GlobalEnvItem>;

/// A select over the workspace's global environments that picks the active one.
pub struct GlobalEnvSelector {
    workspace: Entity<Workspace>,
    select: Entity<SelectState<GlobalEnvRows>>,
    listed: Vec<SharedString>,
    // The dot colour of each `listed` entry.
    dots: Vec<Option<Hsla>>,
    shown: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl GlobalEnvSelector {
    /// Builds the picker over `workspace`, following its global environments.
    pub fn new(workspace: Entity<Workspace>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let rows = GlobalEnvRows::new("global-env-group", Vec::new());
        let select = cx.new(|cx| SelectState::new(rows, None, window, cx));
        let subscriptions = vec![
            cx.observe_in(&workspace, window, |this, _, window, cx| {
                this.sync(window, cx)
            }),
            cx.subscribe_in(
                &select,
                window,
                |this, _, event: &SelectEvent<GlobalEnvRows>, window, cx| {
                    if let SelectEvent::Confirm(Some(item)) = event {
                        let item = item.clone();
                        this.choose(&item, window, cx);
                    }
                },
            ),
        ];
        let mut selector = GlobalEnvSelector {
            workspace,
            select,
            listed: Vec::new(),
            dots: Vec::new(),
            shown: None,
            _subscriptions: subscriptions,
        };
        selector.sync(window, cx);
        selector
    }

    /// [`NO_GLOBAL_ENVIRONMENT`], the loaded global environment names, then
    /// [`manage_global_environments`].
    pub fn items(&self, cx: &App) -> Vec<SharedString> {
        let names = self.workspace.read(cx).global_picker_items();
        std::iter::once(SharedString::from(NO_GLOBAL_ENVIRONMENT))
            .chain(names.into_iter().map(SharedString::from))
            .chain([SharedString::from(manage_global_environments())])
            .collect()
    }

    /// `items()` as the picker groups it: [`GLOBALS_GROUP`], then [`ACTIONS_GROUP`] holding
    /// [`manage_global_environments`].
    pub fn groups(&self) -> Vec<(SharedString, Vec<SharedString>)> {
        let (globals, actions) = self.listed.split_at(self.listed.len().saturating_sub(1));
        vec![
            (GLOBALS_GROUP.into(), globals.to_vec()),
            (ACTIONS_GROUP.into(), actions.to_vec()),
        ]
    }

    /// The select itself, for its focus handle.
    pub fn select(&self) -> &Entity<SelectState<GlobalEnvRows>> {
        &self.select
    }

    /// The item the select shows.
    pub fn shown(&self) -> Option<SharedString> {
        self.shown.clone()
    }

    /// Selects `item`'s global environment, or none for [`NO_GLOBAL_ENVIRONMENT`];
    /// [`manage_global_environments`] opens the editor and keeps the selection.
    pub fn choose(&mut self, item: &str, window: &mut Window, cx: &mut Context<Self>) {
        if item == manage_global_environments() {
            // The kit already shows the manage row; forget what we showed so the resync
            // pushes the selection back into the select.
            self.shown = None;
            self.sync(window, cx);
            open_global_env_editor(self.workspace.clone(), window, cx);
            return;
        }
        self.show(SharedString::from(item.to_owned()), window, cx);
        let picked = if item == NO_GLOBAL_ENVIRONMENT {
            ""
        } else {
            item
        };
        self.workspace
            .update(cx, |w, cx| w.select_picker_item(picked, cx));
    }

    /// Asks the workspace to re-read the global directory; wired to a mouse-down on the picker.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.workspace.update(cx, |w, cx| w.reload_globals(cx));
    }

    // Resets the items only when the names or their colours change, so an open popup is not
    // rebuilt on every workspace notify; the shown value follows the selection.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items = self.items(cx);
        let dots: Vec<Option<Hsla>> = {
            let globals = self.workspace.read(cx).globals();
            (items.iter())
                .map(|item| {
                    let fixed = [NO_GLOBAL_ENVIRONMENT, manage_global_environments()];
                    if fixed.contains(&item.as_ref()) {
                        return None;
                    }
                    let color = globals.as_deref()?.environment(item)?.color();
                    env_fill(color.as_deref())
                })
                .collect()
        };
        if items != self.listed || dots != self.dots {
            let any = dots.iter().any(Option::is_some);
            let mut globals: Vec<GlobalEnvItem> = (items.iter().cloned())
                .zip(dots.iter().copied())
                .map(|(name, dot)| GlobalEnvItem(name, dot, any))
                .collect();
            // `items()` always ends with the manage row.
            let manage = globals.split_off(globals.len().saturating_sub(1));
            let groups = vec![
                (GLOBALS_GROUP.into(), globals),
                (ACTIONS_GROUP.into(), manage),
            ];
            let rows = GlobalEnvRows::new("global-env-group", groups);
            self.listed = items;
            self.dots = dots;
            self.select
                .update(cx, |s, cx| s.set_items(rows, window, cx));
            self.shown = None;
        }
        // An unlisted selection would leave the kit select on its placeholder, so it reads
        // as none instead.
        let selected = self
            .workspace
            .read(cx)
            .selected_picker_item()
            .filter(|name| {
                self.listed
                    .iter()
                    .any(|item| item.as_ref() == name.as_str())
            })
            .unwrap_or_else(|| NO_GLOBAL_ENVIRONMENT.to_owned());
        self.show(selected.into(), window, cx);
    }

    fn show(&mut self, item: SharedString, window: &mut Window, cx: &mut Context<Self>) {
        if self.shown.as_ref() != Some(&item) {
            self.select
                .update(cx, |s, cx| s.set_selected_value(&item, window, cx));
            self.shown = Some(item);
            cx.notify();
        }
    }
}

impl Render for GlobalEnvSelector {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A light, borderless select like the collection picker that fills the host's slot;
        // none reads as a placeholder, and the app draws the ring the select lacks.
        let muted = cx.theme().muted_foreground;
        let none = self
            .shown
            .as_deref()
            .is_none_or(|s| s == NO_GLOBAL_ENVIRONMENT);
        let focused =
            self.select.focus_handle(cx).is_focused(window) && crate::focus_mode::is_keyboard(cx);
        div()
            .id("global-env-selector")
            .test_support()
            .relative()
            .w_full()
            .max_w(ENV_CONTROL_WIDTH)
            .overflow_hidden()
            .rounded(cx.theme().radius)
            .tooltip(crate::ui::text_tooltip(
                "global-env-selector-tooltip",
                scope_tip(GLOBAL_ENV_SELECT_TIP, self.shown.as_ref().filter(|_| !none)),
            ))
            .hover({
                let fill = crate::ui::ghost_hover_fill(cx);
                move |s| s.bg(fill)
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.refresh(cx)),
            )
            .child(
                Select::new(&self.select)
                    .id("global-env-select")
                    .accessibility_label(GLOBAL_ENV_SELECT_TIP)
                    .appearance(false)
                    .small()
                    .when(none, |s| s.text_color(muted)),
            )
            .when(focused, |d| {
                d.child(ring_overlay("global-env-select-ring", cx))
            })
    }
}
