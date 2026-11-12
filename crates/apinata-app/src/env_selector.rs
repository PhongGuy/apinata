//! The title bar's environment selector for the current collection, whose dropdown also
//! imports environment files into it.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use apinata_core::import::{ImportReport, ImportedEnvironment, parse_environments};
use gpui_kit::base::actions::SelectDown;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, Focusable as _, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, PathPromptOptions, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _,
    Window, canvas, div,
};

use crate::command::Command;
use crate::shell::NO_COLLECTION_ENV_TIP;
use crate::style::{ENV_CONTROL_SHOWN_FROM, ENV_CONTROL_WIDTH, SPACE_1, resolve};
use crate::tabs::TabKind;
use crate::ui::ring_overlay;
use crate::workspace::Workspace;

/// The selector item that selects no environment.
pub const NO_ENVIRONMENT: &str = "No environment";

/// The dropdown entry that opens the environment editor instead of selecting one.
pub const MANAGE_ENVIRONMENTS: &str = "Manage environments…";

/// The dropdown entry that imports environment files instead of selecting one.
pub const IMPORT_ENVIRONMENT: &str = "Import environment…";

/// The id of the dropdown row for `item`; the prefixes keep an environment named "none" or
/// "import" apart from the fixed entries.
pub fn option_id(item: &str) -> SharedString {
    match item {
        NO_ENVIRONMENT => "env-none".into(),
        IMPORT_ENVIRONMENT => "env-import".into(),
        MANAGE_ENVIRONMENTS => "env-manage".into(),
        name => format!("env-option-{name}").into(),
    }
}

/// One dropdown row, valued by its name.
#[derive(Clone)]
pub struct EnvItem(
    /// The environment name, or one of the fixed entries.
    pub SharedString,
    /// The environment's colour, drawn as a dot beside the name.
    pub Option<Hsla>,
);

// The kit draws select rows without `.test_support()`, so the row content registers itself
// under `option_id`; a click on it bubbles to the kit row's confirm handler. The colour dot is
// presentation only: the title, and so the accessible value, stays the bare name.
impl SelectItem for EnvItem {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.0.clone()
    }

    fn display_title(&self) -> Option<AnyElement> {
        let fill = self.1?;
        Some(dotted("env-trigger-dot", fill, self.0.clone()).into_any_element())
    }

    fn value(&self) -> &SharedString {
        &self.0
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let row = div().id(option_id(&self.0)).test_support();
        match self.1 {
            Some(fill) => row.child(dotted("env-row-dot", fill, self.0.clone())),
            None => row.child(self.0.clone()),
        }
    }
}

// The name led by a dot of `fill`, kept on one line so a long name still ellipsizes.
fn dotted(dot_id: &'static str, fill: Hsla, name: SharedString) -> impl IntoElement {
    h_flex()
        .min_w_0()
        .gap(SPACE_1)
        .child(crate::env_color::dot(dot_id, fill))
        .child(div().min_w_0().truncate().child(name))
}

/// A select over the current collection's environments that picks its selected one.
pub struct EnvSelector {
    workspace: Entity<Workspace>,
    select: Entity<SelectState<Vec<EnvItem>>>,
    root: Option<PathBuf>,
    listed: Vec<SharedString>,
    // The dot colour of each `listed` entry.
    dots: Vec<Option<Hsla>>,
    shown: Option<SharedString>,
    last_import: Option<ImportReport>,
    // Whether the control's last laid-out room was too small to show the select.
    squeezed: Rc<Cell<bool>>,
    _subscriptions: Vec<Subscription>,
}

impl EnvSelector {
    /// Builds the selector over `workspace`, following its current collection.
    pub fn new(workspace: Entity<Workspace>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let select = cx.new(|cx| SelectState::new(Vec::new(), None, window, cx));
        let subscriptions = vec![
            cx.observe_in(&workspace, window, |this, _, window, cx| {
                this.sync(window, cx)
            }),
            cx.subscribe_in(
                &select,
                window,
                |this, _, event: &SelectEvent<Vec<EnvItem>>, window, cx| {
                    if let SelectEvent::Confirm(Some(item)) = event {
                        let item = item.clone();
                        this.choose(&item, window, cx);
                    }
                },
            ),
        ];
        let mut selector = EnvSelector {
            workspace,
            select,
            root: None,
            listed: Vec::new(),
            dots: Vec::new(),
            shown: None,
            last_import: None,
            squeezed: Rc::default(),
            _subscriptions: subscriptions,
        };
        selector.sync(window, cx);
        selector
    }

    /// [`NO_ENVIRONMENT`], then the current collection's environments.
    pub fn items(&self, cx: &App) -> Vec<SharedString> {
        let workspace = self.workspace.read(cx);
        let names = workspace
            .current_root()
            .map(|root| workspace.environments(&root, cx))
            .unwrap_or_default();
        std::iter::once(SharedString::from(NO_ENVIRONMENT))
            .chain(names.into_iter().map(SharedString::from))
            .collect()
    }

    /// What the select lists: `items()`, then [`MANAGE_ENVIRONMENTS`] and [`IMPORT_ENVIRONMENT`]
    /// while a collection is open.
    pub fn listed(&self) -> &[SharedString] {
        &self.listed
    }

    /// The kit select state behind `#env-select`.
    pub fn select(&self) -> &Entity<SelectState<Vec<EnvItem>>> {
        &self.select
    }

    /// Whether the bar squeezed the control too narrow to show its select, which is then
    /// hidden and no Tab stop.
    pub fn squeezed(&self) -> bool {
        self.squeezed.get()
    }

    /// Opens the dropdown and puts focus in its list, as the Down key does on the trigger.
    pub fn open_menu(&self, window: &mut Window, cx: &mut App) {
        self.select.update(cx, |s, cx| s.focus(window, cx));
        window.dispatch_action(Box::new(SelectDown), cx);
    }

    /// The colour of the dot drawn beside `item`, if any.
    pub fn dot(&self, item: &str) -> Option<Hsla> {
        let ix = self.listed.iter().position(|l| l.as_ref() == item)?;
        self.dots.get(ix).copied().flatten()
    }

    /// The item the select shows.
    pub fn shown(&self) -> Option<SharedString> {
        self.shown.clone()
    }

    /// Selects the environment `item` of the current collection; [`NO_ENVIRONMENT`] selects none
    /// and [`IMPORT_ENVIRONMENT`] starts [`Self::import`], keeping the selection, as
    /// [`MANAGE_ENVIRONMENTS`] does while it opens the Environments tab.
    pub fn choose(&mut self, item: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.workspace.read(cx).current_root() else {
            return;
        };
        if item == IMPORT_ENVIRONMENT || item == MANAGE_ENVIRONMENTS {
            // The kit already shows the chosen entry; forget what we showed so the resync
            // pushes the selected environment back into the select.
            self.shown = None;
            self.sync(window, cx);
            if item == IMPORT_ENVIRONMENT {
                self.import(window, cx);
            } else {
                self.workspace.update(cx, |w, cx| {
                    w.open_settings(&root, TabKind::Environments, cx);
                });
            }
            return;
        }
        let name = (item != NO_ENVIRONMENT).then(|| item.to_owned());
        self.show(SharedString::from(item.to_owned()), window, cx);
        self.workspace
            .update(cx, |w, cx| w.select_environment(&root, name, cx));
    }

    /// Prompts for environment files (several allowed), parses them all, then sends one
    /// `ImportEnvironment` each to the current collection; a file that does not parse imports
    /// nothing. Does nothing when no collection is open.
    pub fn import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.workspace.read(cx).current_root() else {
            return;
        };
        let reply = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Import environment".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(files))) = reply.await else {
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
                    Ok(Ok(report)) => merge(&mut merged, report),
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

    // Resets the items only when the collection or its environments change, so an open
    // popup is not rebuilt on every snapshot; the shown value follows the selection.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.workspace.read(cx).current_root();
        let mut items = self.items(cx);
        if root.is_some() {
            items.push(MANAGE_ENVIRONMENTS.into());
            items.push(IMPORT_ENVIRONMENT.into());
        }
        let dots: Vec<Option<Hsla>> = {
            let workspace = self.workspace.read(cx);
            (items.iter())
                .map(|item| {
                    let fixed = [NO_ENVIRONMENT, MANAGE_ENVIRONMENTS, IMPORT_ENVIRONMENT];
                    if fixed.contains(&item.as_ref()) {
                        return None;
                    }
                    let r = root.as_deref()?;
                    let color = workspace.environment_color(r, item, cx);
                    crate::env_color::env_fill(color.as_deref())
                })
                .collect()
        };
        if root != self.root || items != self.listed || dots != self.dots {
            self.root = root.clone();
            let rows = (items.iter().cloned())
                .zip(dots.iter().copied())
                .map(|(name, dot)| EnvItem(name, dot))
                .collect();
            self.listed = items;
            self.dots = dots;
            self.select
                .update(cx, |s, cx| s.set_items(rows, window, cx));
            self.shown = None;
        }
        // A selection whose file vanished is not listed; showing it would leave the kit
        // select on its own placeholder, so it reads as no environment instead.
        let selected = root
            .and_then(|r| self.workspace.read(cx).selected_environment(&r, cx))
            .filter(|name| {
                self.listed
                    .iter()
                    .any(|item| item.as_ref() == name.as_str())
            })
            .unwrap_or_else(|| NO_ENVIRONMENT.to_owned());
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

impl EnvSelector {
    /// The opacity the select draws at: dimmed as a disabled button is while no collection is
    /// open, and at full strength in a high-contrast theme, where its muted text says locked.
    pub fn select_opacity(&self, cx: &App) -> f32 {
        crate::ui::control_opacity(self.root.is_none(), cx)
    }
}

impl Render for EnvSelector {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A borderless select that fills the host's slot; "no environment" reads as a
        // placeholder, and the app draws the ring the borderless select lacks.
        let muted = cx.theme().muted_foreground;
        let none = self.shown.as_deref().is_none_or(|s| s == NO_ENVIRONMENT);
        let focused =
            self.select.focus_handle(cx).is_focused(window) && crate::focus_mode::is_keyboard(cx);
        // The control's room comes from the bar, never from the select, so hiding the select
        // cannot change the measurement that hid it. A change redraws once the frame is done.
        let squeezed = self.squeezed.get();
        // With no collection there is nothing to choose; the tip says why instead.
        let locked = self.root.is_none();
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
        h_flex()
            .id("env-control")
            .test_support()
            .relative()
            .w_full()
            .max_w(ENV_CONTROL_WIDTH)
            .overflow_hidden()
            .when(locked, |d| {
                d.tooltip(crate::ui::text_tooltip(
                    "env-control-tooltip",
                    NO_COLLECTION_ENV_TIP,
                ))
            })
            // The select fills its parent's full width, so a shrinking wrapper sizes it to the
            // control's room and its own ellipsis shows instead of the control clipping.
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .when(squeezed, |d| d.invisible())
                    .opacity(self.select_opacity(cx))
                    .child(
                        Select::new(&self.select)
                            .id("env-select")
                            .appearance(false)
                            .small()
                            // A disabled select tracks no focus, so Tab passes it.
                            .disabled(squeezed || locked)
                            .when(none, |s| s.text_color(muted)),
                    ),
            )
            .child(measure)
            .when(focused, |d| d.child(ring_overlay("env-select-ring", cx)))
    }
}

// Every environment of every file in file order, or the first failure's text naming its file.
fn parse_all(files: &[PathBuf]) -> Result<Vec<ImportedEnvironment>, String> {
    let mut envs = Vec::new();
    for file in files {
        let input = std::fs::read(file)
            .map_err(|e| crate::import_report::failure_at(file, &e.to_string()))?;
        envs.extend(
            parse_environments(None, &input)
                .map_err(|e| crate::import_report::failure_at(file, &e.to_string()))?,
        );
    }
    Ok(envs)
}

fn merge(into: &mut ImportReport, report: ImportReport) {
    into.notes.extend(report.notes);
    into.items.extend(report.items);
    into.environments.extend(report.environments);
    into.secrets_to_enter.extend(report.secrets_to_enter);
}

fn failed(message: String, window: &mut Window, cx: &mut App) {
    let text: SharedString = crate::import_report::failure_text(&message).into();
    window.open_alert_dialog(cx, move |dialog, _, _| {
        dialog
            .title("Could not import environment")
            .description(text.clone())
    });
}
