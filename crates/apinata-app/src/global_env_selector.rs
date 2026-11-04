//! The title bar's global environment picker, which follows the workspace rather than a
//! collection and selects the one global environment every collection's sends use.

use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, Focusable as _, InteractiveElement as _, IntoElement,
    MouseButton, ParentElement as _, Render, SharedString, Styled as _, Subscription,
    TestSupportExt as _, Window, div,
};

use crate::commands::label;
use crate::global_env_editor::open_global_env_editor;
use crate::style::ENV_CONTROL_WIDTH;
use crate::ui::ring_overlay;
use crate::workspace::Workspace;

/// The picker item that selects no global environment.
pub const NO_GLOBAL_ENVIRONMENT: &str = "No globals";

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
);

// As with the collection picker, the row content registers itself under `option_id` because
// the kit draws select rows without `.test_support()`.
impl SelectItem for GlobalEnvItem {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.0.clone()
    }

    fn value(&self) -> &SharedString {
        &self.0
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .id(option_id(&self.0))
            .test_support()
            .child(self.0.clone())
    }
}

/// A select over the workspace's global environments that picks the active one.
pub struct GlobalEnvSelector {
    workspace: Entity<Workspace>,
    select: Entity<SelectState<Vec<GlobalEnvItem>>>,
    listed: Vec<SharedString>,
    shown: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl GlobalEnvSelector {
    /// Builds the picker over `workspace`, following its global environments.
    pub fn new(workspace: Entity<Workspace>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let select = cx.new(|cx| SelectState::new(Vec::new(), None, window, cx));
        let subscriptions = vec![
            cx.observe_in(&workspace, window, |this, _, window, cx| {
                this.sync(window, cx)
            }),
            cx.subscribe_in(
                &select,
                window,
                |this, _, event: &SelectEvent<Vec<GlobalEnvItem>>, window, cx| {
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

    /// The select itself, for its focus handle.
    pub fn select(&self) -> &Entity<SelectState<Vec<GlobalEnvItem>>> {
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

    // Resets the items only when the names change, so an open popup is not rebuilt on every
    // workspace notify; the shown value follows the selection.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items = self.items(cx);
        if items != self.listed {
            let rows = items.iter().cloned().map(GlobalEnvItem).collect();
            self.listed = items;
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
        // A borderless select like the collection picker that fills the host's slot;
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
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.refresh(cx)),
            )
            .child(
                Select::new(&self.select)
                    .id("global-env-select")
                    .appearance(false)
                    .small()
                    .when(none, |s| s.text_color(muted)),
            )
            .when(focused, |d| {
                d.child(ring_overlay("global-env-select-ring", cx))
            })
    }
}
