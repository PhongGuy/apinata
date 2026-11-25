//! The title bar's environment picker: one menu that chooses the current collection's
//! environment and the global environment independently, and imports environment files.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use apinata_core::import::{ImportReport, ImportedEnvironment, parse_environments};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, WindowExt as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, App, AppContext as _, Context, DismissEvent, Entity, FocusHandle, Focusable as _, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, PathPromptOptions, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _,
    WeakEntity, Window, canvas, div,
};

use crate::command::Command;
use crate::commands::label as command_label;
use crate::env_color::{env_fill, picker_row, scoped};
use crate::global_env_editor::open_global_env_editor;
use crate::shell::NO_COLLECTION_ENV_TIP;
use crate::style::{ENV_CONTROL_SHOWN_FROM, ENV_CONTROL_WIDTH, resolve};
use crate::tabs::TabKind;
use crate::ui::{LockButton as _, SolidRing as _};
use crate::workspace::Workspace;

/// The tooltip and accessible name of the picker's button.
pub const ENV_SELECT_TIP: &str = "Environment";

/// The header of the menu section that lists the current collection's environments.
pub const COLLECTION_GROUP: &str = "Collection";

/// The header of the menu section that lists the global environments.
pub const GLOBAL_GROUP: &str = "Global";

/// The header of the menu section whose entries act instead of selecting.
pub const ACTIONS_GROUP: &str = "Actions";

/// The collection row that selects no environment.
pub const NO_ENVIRONMENT: &str = "No environment";

/// The global row that selects no global environment; it names the choice whether or not any
/// globals exist, so it never reads as "none exist" while some do.
pub const NO_GLOBAL_ENVIRONMENT: &str = "No global environment";

/// The action that opens the environment editor; the command that does the same takes it as
/// its label.
pub const MANAGE_ENVIRONMENTS: &str = "Manage environments";

/// The action that imports environment files; the command that does the same takes it as its
/// label.
pub const IMPORT_ENVIRONMENT: &str = "Import environment…";

/// What the button puts between the collection's environment and the global one.
pub const LABEL_SEPARATOR: &str = " · ";

/// The action that opens the global environment editor.
pub fn manage_global_environments() -> &'static str {
    command_label("environment.manage_globals")
}

/// The id of the collection row or collection action `item`; the prefixes keep an
/// environment named "none" or "import" apart from the fixed entries.
pub fn option_id(item: &str) -> SharedString {
    match item {
        NO_ENVIRONMENT => "env-none".into(),
        IMPORT_ENVIRONMENT => "env-import".into(),
        MANAGE_ENVIRONMENTS => "env-manage".into(),
        name => format!("env-option-{name}").into(),
    }
}

/// The id of the global row or global action `item`, kept apart from the collection's ids.
pub fn global_option_id(item: &str) -> SharedString {
    if item == NO_GLOBAL_ENVIRONMENT {
        "global-env-none".into()
    } else if item == manage_global_environments() {
        "global-env-manage".into()
    } else {
        format!("global-env-option-{item}").into()
    }
}

// Which choice a menu row makes: the collection's environment or the global one.
#[derive(Clone, Copy, PartialEq)]
enum Scope {
    Collection,
    Global,
}

// One menu row as drawn: its id, name, dot and whether it holds the current choice.
struct Row {
    scope: Scope,
    name: SharedString,
    dot: Option<Hsla>,
    checked: bool,
}

/// The picker: a button naming both choices whose menu lists the collection's environments,
/// then the global ones, then the actions; picking in one section leaves the other alone.
pub struct EnvPicker {
    workspace: Entity<Workspace>,
    focus: FocusHandle,
    menu: Option<Entity<PopupMenu>>,
    // Closes the picker when the open menu dismisses itself.
    menu_dismissed: Option<Subscription>,
    // What the button last showed: its label, its dot and whether a collection is open.
    drawn: Option<(SharedString, Option<Hsla>, bool)>,
    // The sections the open menu was built from.
    listed: Vec<(SharedString, Vec<SharedString>)>,
    last_import: Option<ImportReport>,
    // Whether the control's last laid-out room was too small to show the button.
    squeezed: Rc<Cell<bool>>,
    _subscriptions: Vec<Subscription>,
}

impl EnvPicker {
    /// Builds the picker over `workspace`, following its current collection and its globals.
    pub fn new(workspace: Entity<Workspace>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // The workspace notifies on every edit; the button redraws only when what it shows
        // changes, so typing elsewhere leaves the title bar alone.
        let subscriptions = vec![cx.observe_in(&workspace, window, |this, _, window, cx| {
            this.refresh_open_menu(window, cx);
            let shown = this.shown(cx);
            if this.drawn.as_ref() != Some(&shown) {
                this.drawn = Some(shown);
                cx.notify();
            }
        })];
        EnvPicker {
            workspace,
            focus: cx.focus_handle(),
            menu: None,
            menu_dismissed: None,
            listed: Vec::new(),
            drawn: None,
            last_import: None,
            squeezed: Rc::default(),
            _subscriptions: subscriptions,
        }
    }

    fn root(&self, cx: &App) -> Option<PathBuf> {
        self.workspace.read(cx).current_root()
    }

    /// [`NO_ENVIRONMENT`], then the current collection's environments; empty with no
    /// collection open.
    pub fn collection_items(&self, cx: &App) -> Vec<SharedString> {
        let workspace = self.workspace.read(cx);
        let Some(root) = workspace.current_root() else {
            return Vec::new();
        };
        std::iter::once(SharedString::from(NO_ENVIRONMENT))
            .chain(
                workspace
                    .environments(&root, cx)
                    .into_iter()
                    .map(Into::into),
            )
            .collect()
    }

    /// [`NO_GLOBAL_ENVIRONMENT`], then the loaded global environments.
    pub fn global_items(&self, cx: &App) -> Vec<SharedString> {
        let names = self.workspace.read(cx).global_picker_items();
        std::iter::once(SharedString::from(NO_GLOBAL_ENVIRONMENT))
            .chain(names.into_iter().map(Into::into))
            .collect()
    }

    fn actions(&self, cx: &App) -> Vec<SharedString> {
        let collection = [MANAGE_ENVIRONMENTS, IMPORT_ENVIRONMENT];
        let collection = self.root(cx).map(|_| collection).into_iter().flatten();
        collection
            .chain([manage_global_environments()])
            .map(SharedString::from)
            .collect()
    }

    /// The menu's sections in order, each as its header and its rows: [`COLLECTION_GROUP`]
    /// while a collection is open, [`GLOBAL_GROUP`], then [`ACTIONS_GROUP`].
    pub fn sections(&self, cx: &App) -> Vec<(SharedString, Vec<SharedString>)> {
        [
            (COLLECTION_GROUP, self.collection_items(cx)),
            (GLOBAL_GROUP, self.global_items(cx)),
            (ACTIONS_GROUP, self.actions(cx)),
        ]
        .into_iter()
        .filter(|(_, rows)| !rows.is_empty())
        .map(|(title, rows)| (title.into(), rows))
        .collect()
    }

    /// The rows that hold the current choices: the collection's, then the global one. A
    /// selection that is not listed, such as one whose file vanished, reads as none.
    pub fn checked(&self, cx: &App) -> (SharedString, SharedString) {
        let workspace = self.workspace.read(cx);
        let listed = |items: Vec<SharedString>, name: Option<String>| {
            name.filter(|n| items.iter().any(|i| i.as_ref() == n.as_str()))
                .map(SharedString::from)
        };
        let collection = (self.root(cx))
            .and_then(|r| workspace.selected_environment(&r, cx))
            .and_then(|name| listed(self.collection_items(cx), Some(name)))
            .unwrap_or_else(|| NO_ENVIRONMENT.into());
        let global = listed(self.global_items(cx), workspace.selected_picker_item())
            .unwrap_or_else(|| NO_GLOBAL_ENVIRONMENT.into());
        (collection, global)
    }

    /// What the button reads: the collection's environment, then the global one after
    /// [`LABEL_SEPARATOR`] while one is chosen.
    pub fn label(&self, cx: &App) -> SharedString {
        let (collection, global) = self.checked(cx);
        if global == NO_GLOBAL_ENVIRONMENT {
            collection
        } else {
            format!("{collection}{LABEL_SEPARATOR}{global}").into()
        }
    }

    /// The colour of the dot beside the collection environment `name`, if it sets one.
    pub fn collection_dot(&self, name: &str, cx: &App) -> Option<Hsla> {
        let root = self.root(cx)?;
        let color = self.workspace.read(cx).environment_color(&root, name, cx);
        env_fill(color.as_deref())
    }

    /// The colour of the dot beside the global environment `name`, if it sets one.
    pub fn global_dot(&self, name: &str, cx: &App) -> Option<Hsla> {
        let globals = self.workspace.read(cx).globals()?;
        env_fill(globals.environment(name)?.color().as_deref())
    }

    fn shown(&self, cx: &App) -> (SharedString, Option<Hsla>, bool) {
        let (collection, _) = self.checked(cx);
        let dot = (collection.as_ref() != NO_ENVIRONMENT)
            .then(|| self.collection_dot(&collection, cx))
            .flatten();
        (self.label(cx), dot, self.root(cx).is_some())
    }

    /// The button's focus handle.
    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    /// Whether the menu is open.
    pub fn is_open(&self) -> bool {
        self.menu.is_some()
    }

    /// Whether the bar squeezed the control too narrow to show its button, which is then
    /// hidden and no Tab stop.
    pub fn squeezed(&self) -> bool {
        self.squeezed.get()
    }

    /// Opens the menu with focus in it, re-reading the global directory first so a file added
    /// outside the app is listed.
    pub fn open_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.is_some() {
            return;
        }
        self.workspace.update(cx, |w, cx| w.reload_globals(cx));
        self.listed = self.sections(cx);
        let builder = self.menu_builder(cx);
        let menu = PopupMenu::build(window, cx, move |menu, _, _| builder(menu));
        let dismissed = cx.subscribe_in(&menu, window, |this, _, _: &DismissEvent, _, cx| {
            this.close_menu(cx)
        });
        menu.focus_handle(cx).focus(window, cx);
        self.menu = Some(menu);
        self.menu_dismissed = Some(dismissed);
        cx.notify();
    }

    // The open menu's rows, built from what the workspace holds now.
    fn menu_builder(&self, cx: &mut Context<Self>) -> impl FnOnce(PopupMenu) -> PopupMenu + use<> {
        let rows = self.rows(cx);
        let actions = self.actions(cx);
        let (this, focus) = (cx.entity().downgrade(), self.focus.clone());
        move |menu| build_menu(menu.action_context(focus), rows, actions, this)
    }

    // Rebuilds the open menu when its rows change underneath it, such as the globals the
    // opening reload brings in; an unchanged list keeps the highlighted row.
    fn refresh_open_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = self.menu.clone() else {
            return;
        };
        let listed = self.sections(cx);
        if listed == self.listed {
            return;
        }
        self.listed = listed;
        let builder = self.menu_builder(cx);
        menu.update(cx, |m, cx| {
            m.rebuild(window, cx, move |menu, _, _| builder(menu))
        });
    }

    /// Closes the menu without choosing.
    pub fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
            self.menu_dismissed = None;
            cx.notify();
        }
    }

    // The menu's choice rows by section, each with its dot and check.
    fn rows(&self, cx: &App) -> Vec<Row> {
        let (checked_collection, checked_global) = self.checked(cx);
        let mut rows = Vec::new();
        for (scope, items, checked) in [
            (
                Scope::Collection,
                self.collection_items(cx),
                checked_collection,
            ),
            (Scope::Global, self.global_items(cx), checked_global),
        ] {
            let dots: Vec<Option<Hsla>> = (items.iter())
                .enumerate()
                .map(|(ix, name)| match (ix, scope) {
                    (0, _) => None,
                    (_, Scope::Collection) => self.collection_dot(name, cx),
                    (_, Scope::Global) => self.global_dot(name, cx),
                })
                .collect();
            rows.extend(items.into_iter().zip(dots).map(|(name, dot)| Row {
                scope,
                checked: name == checked,
                name,
                dot,
            }));
        }
        rows
    }

    /// Selects the collection environment `item`; [`NO_ENVIRONMENT`] selects none,
    /// [`IMPORT_ENVIRONMENT`] starts [`Self::import`] and [`MANAGE_ENVIRONMENTS`] opens the
    /// Environments tab, both keeping the selection. Does nothing with no collection open.
    pub fn choose(&mut self, item: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.root(cx) else {
            return;
        };
        match item {
            IMPORT_ENVIRONMENT => {
                // The prompt hands focus back to the button, not to the closed menu.
                self.focus.focus(window, cx);
                self.import(window, cx)
            }
            MANAGE_ENVIRONMENTS => self.workspace.update(cx, |w, cx| {
                w.open_settings(&root, TabKind::Environments, cx);
            }),
            _ => {
                let name = (item != NO_ENVIRONMENT).then(|| item.to_owned());
                self.workspace
                    .update(cx, |w, cx| w.select_environment(&root, name, cx));
            }
        }
    }

    /// Selects the global environment `item`, or none for [`NO_GLOBAL_ENVIRONMENT`];
    /// [`manage_global_environments`] opens the global editor and keeps the selection.
    pub fn choose_global(&mut self, item: &str, window: &mut Window, cx: &mut Context<Self>) {
        if item == manage_global_environments() {
            // The editor hands focus back to its opener: the button, not the closed menu.
            self.focus.focus(window, cx);
            open_global_env_editor(self.workspace.clone(), window, cx);
            return;
        }
        let picked = if item == NO_GLOBAL_ENVIRONMENT {
            ""
        } else {
            item
        };
        self.workspace
            .update(cx, |w, cx| w.select_picker_item(picked, cx));
    }

    /// Asks the workspace to re-read the global directory.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.workspace.update(cx, |w, cx| w.reload_globals(cx));
    }

    /// Prompts for environment files (several allowed), parses them all, then sends one
    /// `ImportEnvironment` each to the current collection; a file that does not parse imports
    /// nothing. Does nothing when no collection is open.
    pub fn import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.root(cx) else {
            return;
        };
        let reply = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Import environment".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Some(files) = crate::file_dialog::picked(reply, "import an environment", cx).await
            else {
                return;
            };
            let parsed = cx.background_spawn(async move { parse_all(&files) }).await;
            let envs = match parsed {
                Ok(envs) => envs,
                Err(message) => {
                    let _ = this.update_in(cx, |_, window, cx| failed(message, window, cx));
                    return;
                }
            };
            let variables = envs.iter().flat_map(|e| &e.variables);
            let secrets = variables.clone().filter(|v| v.secret).count();
            let text_counts = (variables.count(), secrets);
            // One at a time, so a write that fails stops the rest and keeps what was written.
            let mut merged = ImportReport::default();
            let mut error = None;
            for env in envs {
                let (reply, replies) = async_channel::bounded(1);
                let command = Command::ImportEnvironment { env, reply };
                let Ok(Some(_)) = this.update(cx, |this, cx| {
                    this.workspace
                        .update(cx, |w, cx| w.send(&root, command, cx))
                }) else {
                    return;
                };
                match replies.recv().await {
                    Ok(Ok(report)) => merged.merge(report),
                    Ok(Err(message)) => {
                        error = Some(message);
                        break;
                    }
                    Err(_) => return,
                }
            }
            let _ = this.update_in(cx, |this, window, cx| {
                if !merged.notes.is_empty() {
                    crate::import_report::open("Import report", &merged, window, cx);
                } else if error.is_none() && !merged.environments.is_empty() {
                    let (variables, secrets) = text_counts;
                    let text = imported_text(&merged.environments, variables, secrets);
                    let toast = imported_toast(this.workspace.clone(), root, &merged, text);
                    window.push_notification(toast, cx);
                }
                if let Some(message) = error {
                    failed(message, window, cx);
                }
                if !merged.environments.is_empty() {
                    this.last_import = Some(merged);
                }
            });
        })
        .detach();
    }

    /// The merged report of the last import that wrote anything.
    pub fn last_import(&self) -> Option<&ImportReport> {
        self.last_import.as_ref()
    }
}

// The menu: each section under its header, a rule between sections, the actions last.
fn build_menu(
    mut menu: PopupMenu,
    rows: Vec<Row>,
    actions: Vec<SharedString>,
    this: WeakEntity<EnvPicker>,
) -> PopupMenu {
    let mut scope = None;
    for row in rows {
        if scope != Some(row.scope) {
            let title = match row.scope {
                Scope::Collection => COLLECTION_GROUP,
                Scope::Global => GLOBAL_GROUP,
            };
            menu = menu.separator().label(title);
            scope = Some(row.scope);
        }
        let (id, ids) = match row.scope {
            Scope::Collection => (option_id(&row.name), ("env-row-dot", "env-row-name")),
            Scope::Global => (
                global_option_id(&row.name),
                ("global-env-row-dot", "global-env-row-name"),
            ),
        };
        let (name, dot) = (row.name.clone(), row.dot);
        let item = PopupMenuItem::element(move |_, _| {
            div()
                .id(id.clone())
                .test_support()
                .child(picker_row(ids, dot, name.clone()))
        })
        .checked(row.checked)
        .on_click(chooser(this.clone(), row.scope, row.name));
        menu = menu.item(item);
    }
    menu = menu.separator().label(ACTIONS_GROUP);
    for action in actions {
        let scope = if action.as_ref() == manage_global_environments() {
            Scope::Global
        } else {
            Scope::Collection
        };
        let (id, ids) = match scope {
            Scope::Collection => (option_id(&action), ("env-action-dot", "env-action-name")),
            Scope::Global => (
                global_option_id(&action),
                ("global-env-action-dot", "global-env-action-name"),
            ),
        };
        // An action keeps a dot's space too, so its text lines up with the names above.
        let text = action.clone();
        let item = PopupMenuItem::element(move |_, _| {
            div()
                .id(id.clone())
                .test_support()
                .child(picker_row(ids, None, text.clone()))
        })
        .on_click(chooser(this.clone(), scope, action));
        menu = menu.item(item);
    }
    menu
}

// A row's click: the picker makes the row's choice in its own scope.
fn chooser(
    this: WeakEntity<EnvPicker>,
    scope: Scope,
    item: SharedString,
) -> impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static {
    move |_, window, cx| {
        let _ = this.update(cx, |picker, cx| match scope {
            Scope::Collection => picker.choose(&item, window, cx),
            Scope::Global => picker.choose_global(&item, window, cx),
        });
    }
}

impl Render for EnvPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A light ghost button that fills the host's slot: its scope icon, the collection
        // environment's dot and caret say it is a control, and "No environment" reads muted.
        let muted = cx.theme().muted_foreground;
        let (collection, _) = self.checked(cx);
        let label = self.label(cx);
        let none = label.as_ref() == NO_ENVIRONMENT;
        let dot = (collection.as_ref() != NO_ENVIRONMENT)
            .then(|| self.collection_dot(&collection, cx))
            .flatten();
        // The control's room comes from the bar, never from the button, so hiding the button
        // cannot change the measurement that hid it. A change redraws once the frame is done.
        let squeezed = self.squeezed.get();
        let locked = self.root(cx).is_none();
        let (cell, me) = (Rc::clone(&self.squeezed), cx.entity().downgrade());
        let measure = canvas(
            move |bounds, window, cx| {
                let now = bounds.size.width < resolve(ENV_CONTROL_SHOWN_FROM, window);
                if cell.replace(now) != now {
                    window.defer(cx, move |_, cx| {
                        me.update(cx, |_, cx| cx.notify()).ok();
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        let tip: SharedString = if locked {
            NO_COLLECTION_ENV_TIP.into()
        } else if none {
            ENV_SELECT_TIP.into()
        } else {
            format!("{ENV_SELECT_TIP}: {label}").into()
        };
        let title = scoped(
            ("env-scope-icon", IconName::Folder),
            ("env-trigger-dot", dot),
            label.clone(),
        );
        let button = Button::new("env-select")
            .solid_ring(cx)
            .ghost()
            .small()
            .w_full()
            .min_w_0()
            .justify_start()
            .dropdown_caret(true)
            .accessibility_label(format!("{ENV_SELECT_TIP}: {label}"))
            .aria_expanded(self.menu.is_some())
            // A hidden button tracks no focus, so Tab passes it and focus on it is stranded
            // for the shell to hand back.
            .when(!squeezed, |b| b.track_focus(&self.focus))
            .lock(squeezed, cx)
            .tab_stop(!squeezed)
            .when(none, |b| b.text_color(muted))
            .child(div().flex_1().min_w_0().child(title));
        let mut wrapper = gpui_kit::StyleRefinement::default();
        wrapper.min_size.width = Some(gpui_kit::px(0.).into());
        wrapper.flex_grow = Some(1.);
        let me = cx.entity().downgrade();
        let menu = self.menu.clone();
        // The popover moves focus into itself as it opens; pointing it at the menu keeps a
        // menu opened by command, not by a press, holding the keys.
        let menu_focus = menu.as_ref().map(|m| m.focus_handle(cx));
        let popover = Popover::new("env-picker")
            .appearance(false)
            .overlay_closable(false)
            .anchor(Anchor::TopRight)
            .open(self.menu.is_some())
            .when_some(menu_focus, |p, focus| p.track_focus(&focus))
            .on_open_change(move |open, window, cx| {
                let _ = me.update(cx, |picker, cx| {
                    if *open {
                        picker.open_menu(window, cx);
                    } else {
                        picker.close_menu(cx);
                    }
                });
            })
            .trigger(button)
            .trigger_style(wrapper)
            .content(move |_, _, _| match &menu {
                Some(menu) => menu.clone().into_any_element(),
                None => div().into_any_element(),
            });
        h_flex()
            .id("env-control")
            .test_support()
            .relative()
            .w_full()
            .max_w(ENV_CONTROL_WIDTH)
            .overflow_hidden()
            .rounded(cx.theme().radius)
            // The tip would draw over the open menu's first rows, so it shows only while closed.
            .when(self.menu.is_none(), |d| {
                d.tooltip(crate::ui::text_tooltip("env-control-tooltip", tip))
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .when(squeezed, |d| d.invisible())
                    .child(popover),
            )
            .child(measure)
    }
}

// Every environment of every file in file order, or the first failure's text naming its file.
fn parse_all(files: &[PathBuf]) -> Result<Vec<ImportedEnvironment>, String> {
    let mut envs = Vec::new();
    for file in files {
        let input = std::fs::read(file).map_err(|e| {
            crate::import_report::failure_at(file, &apinata_core::import::read_reason(&e))
        })?;
        envs.extend(
            parse_environments(None, &input)
                .map_err(|e| crate::import_report::failure_at(file, &e.to_string()))?,
        );
    }
    Ok(envs)
}

fn failed(message: String, window: &mut Window, cx: &mut App) {
    let text: SharedString = crate::import_report::failure_text(&message).into();
    crate::ui::open_alert_dialog(window, cx, move |dialog, _, _| {
        dialog
            .title("Could not import environment")
            .description(text.clone())
    });
}

/// The toast after a clean environment import: the one name or how many environments, then
/// the variable count and how many secrets were kept out of the files.
pub fn imported_text(names: &[String], variables: usize, secrets: usize) -> String {
    let what = match names {
        [one] => one.clone(),
        many => format!("{} environments", many.len()),
    };
    let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    let mut counts = plural(variables, "variable");
    if secrets > 0 {
        counts.push_str(&format!(
            ", {} kept out of the file",
            plural(secrets, "secret")
        ));
    }
    format!("Imported {what} ({counts})")
}

struct ImportedToast;

// The toast a clean import shows instead of a report. Use it selects the environment when one
// was imported, under the name it was written as; Open shows the environment editor.
fn imported_toast(
    workspace: Entity<Workspace>,
    root: PathBuf,
    report: &ImportReport,
    text: String,
) -> Notification {
    let single = match report.environments.as_slice() {
        [one] => Some(one.clone()),
        _ => None,
    };
    let open: Rc<dyn Fn(&mut App)> = {
        let (workspace, root) = (workspace.clone(), root.clone());
        Rc::new(move |cx: &mut App| {
            workspace.update(cx, |w, cx| {
                w.open_settings(&root, TabKind::Environments, cx);
            });
        })
    };
    let toast = Notification::new().id::<ImportedToast>().message(text);
    let open_button = move |open: Rc<dyn Fn(&mut App)>, this: Entity<Notification>, cx: &App| {
        Button::new("env-import-open")
            .solid_ring(cx)
            .ghost()
            .label("Open")
            .on_click(move |_, window, cx| {
                open(cx);
                this.update(cx, |n, cx| n.dismiss(window, cx));
            })
    };
    let Some(name) = single else {
        return toast
            .action(move |_, _, cx| open_button(Rc::clone(&open), cx.entity(), cx))
            .autohide(true);
    };
    let use_it = Rc::new(move |cx: &mut App| {
        workspace.update(cx, |w, cx| {
            w.select_environment(&root, Some(name.clone()), cx)
        });
    });
    toast
        .content(move |_, _, cx| {
            let button = open_button(Rc::clone(&open), cx.entity(), cx);
            h_flex().pt_2().child(button).into_any_element()
        })
        .action(move |_, _, cx| {
            let (use_it, this) = (Rc::clone(&use_it), cx.entity());
            Button::new("env-import-use")
                .solid_ring(cx)
                .ghost()
                .label("Use it")
                .on_click(move |_, window, cx| {
                    use_it(cx);
                    this.update(cx, |n, cx| n.dismiss(window, cx));
                })
        })
        .autohide(true)
}
