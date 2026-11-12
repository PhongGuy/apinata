//! The environment editor of one collection: its environments, and the selected one's
//! variables, color and parent.
//!
//! Every change goes through the workspace to the actor. Secret values leave as
//! `SetSecretValue` and come back only on a reveal's reply channel, never in a snapshot.

use crate::ui::LockButton as _;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use apinata_core::EnvVariable;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, ThemeStyled as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::prelude::*;
use gpui_kit::{
    Anchor, AnyElement, App, ClickEvent, Div, ElementId, Entity, FocusHandle, Focusable,
    FontWeight, Hsla, Result, ScrollHandle, SharedString, Subscription, Task, TestSupportExt,
    Window, actions, div, px,
};

use crate::banner::{self, Banner, Choice};
use crate::command::{Command, Edit, EnvOp, Field, Notice, NoticeKind, Target};
use crate::edit_history::EditHistory;
use crate::edit_track::{self, LoadStamps, same_value};
use crate::env_table::{
    DUPLICATE_SECRET_NAMES, EnvTableEvent, EnvVarTable, SecretRecord, env_rows, env_vars_from,
    secret_records, secrets_collide,
};
use crate::orphan_section::{self, Fileless, Orphans};
use crate::request_editor::{RedoEdit, UndoEdit};
use crate::scroll_follow::{FocusFollow, Followed};
use crate::secret_lock::{self, LockStep, ReplaceAnswer};
use crate::snapshot::{CollectionSnapshot, DiskState, EnvView};
use crate::style::{self, PANE_INSET};
use crate::ui;
use crate::workspace::{Workspace, WorkspaceEvent};

actions!(
    apinata,
    [
        /// Saves the selected environment to its file.
        SaveEnvironment
    ]
);

/// The "…" menu's entries by index; the separator sits at 3. Rename is first.
pub const ENV_MENU_RENAME: usize = 0;
/// The "…" menu's Export entry.
pub const ENV_MENU_EXPORT: usize = 1;
/// The "…" menu's entry exporting every environment to a folder.
pub const ENV_MENU_EXPORT_ALL: usize = 2;
/// The "…" menu's Delete entry, after the separator.
pub const ENV_MENU_DELETE: usize = 4;

/// Tooltip and accessibility label of `env-more`; one const, so they agree.
pub const MORE_ACTIONS: &str = "More actions";
/// Why Rename and Duplicate are off while the environment has unsaved edits: the locked
/// controls' tooltip and accessibility label.
pub const RENAME_LOCKED_REASON: &str = "Save or discard changes first";

/// The editor's notice after an outside change replaced a clean environment under typing.
pub const SECRETS_CLEARED: &str = "This file changed on disk. Unsent secret values were cleared.";

/// The line under a disk banner of an environment with secrets: a Reload restores no value.
pub const SECRETS_KEPT: &str = "Secret values already sent are kept.";

/// One entry of the `extends` picker; `None` is no parent.
#[derive(Clone)]
struct Parent {
    title: SharedString,
    value: Option<String>,
}

impl SelectItem for Parent {
    type Value = Option<String>;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn value(&self) -> &Option<String> {
        &self.value
    }

    // The kit draws rows without `.test_support()`, so the row registers itself, keyed by
    // the parent's name or `none`.
    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let key = self.value.as_deref().unwrap_or("none");
        div()
            .id(SharedString::from(format!("env-extends-option-{key}")))
            .test_support()
            .child(self.title.clone())
    }
}

// "None", then every other environment, then a named parent that is missing, so the
// picker can always show what the file says.
fn parents(names: &[String], selected: &str, extends: Option<&str>) -> Vec<Parent> {
    let mut items = vec![Parent {
        title: "None".into(),
        value: None,
    }];
    let others = names.iter().filter(|n| *n != selected);
    let missing = extends.filter(|e| !names.iter().any(|n| n == e) || *e == selected);
    for name in others.map(String::as_str).chain(missing) {
        items.push(Parent {
            title: name.to_string().into(),
            value: Some(name.to_string()),
        });
    }
    items
}

// What a sent command means for the selection when its notice comes back.
enum Sent {
    Plain,
    Create,
    Rename { from: String, to: String },
}

/// The reference client's preset colours in its order; "None" leads the row.
pub const ENV_SWATCHES: [&str; 6] = [
    "#CE4F3B", "#2E8A54", "#346AB2", "#C77A0F", "#B83D7F", "#8D44B2",
];

// Which name the name input is collecting.
#[derive(Clone, Copy, PartialEq)]
enum Naming {
    Create,
    Rename,
}

/// `#rgb` or `#rrggbb`.
fn valid_color(text: &str) -> bool {
    text.strip_prefix('#')
        .is_some_and(|hex| matches!(hex.len(), 3 | 6) && hex.chars().all(|c| c.is_ascii_hexdigit()))
}

/// Edits the environments of one collection through the workspace; each control follows
/// the snapshots only while it has no edit in flight, as the request editor does.
pub struct EnvEditor {
    workspace: Entity<Workspace>,
    compare: Option<Entity<crate::compare_dialog::CompareDialog>>,
    root: PathBuf,
    selected: Option<String>,
    table: Entity<EnvVarTable>,
    base: Vec<EnvVariable>,
    /// Each environment's variable names as last seen clean, so rows not yet in its file
    /// can be told apart from saved ones, also after switching away and back.
    saved: HashMap<String, Vec<String>>,
    /// The names in `saved` that are secrets.
    saved_secrets: HashMap<String, Vec<String>>,
    /// The secrets of each file as last seen clean, which a collision check compares against.
    saved_records: HashMap<String, Vec<SecretRecord>>,
    extends: Entity<SelectState<Vec<Parent>>>,
    parent_names: Vec<Option<String>>,
    name_input: Entity<InputState>,
    naming: Option<Naming>,
    /// The rename in flight as (from, to, id): the selection and the name field wait for it.
    renaming: Option<(String, String, u64)>,
    /// The name the last rename or create refused while its field held it; the error is its.
    refused: Option<String>,
    sent: HashMap<u64, Sent>,
    /// The snapshots each environment's fields loaded; their edits are built from that content.
    loaded: LoadStamps<(String, Field)>,
    /// Set while a table change is sent whose rows the table no longer shows.
    lagging: bool,
    /// Steps of the selected environment's variables, color and parent. It has no target of
    /// its own, so every change of selection or name clears it.
    history: EditHistory,
    /// The id of the last create or rename sent; until it is acknowledged a selected name
    /// missing from the snapshot may simply not have arrived yet.
    awaiting: u64,
    /// The selected environment's name, the outside-reload stamp and the dirty flag the editor
    /// last saw.
    seen: Option<(String, u64, bool)>,
    /// A one-line notice under the table, cleared by the next command or selection.
    notice: Option<&'static str>,
    locked: bool,
    error: Option<String>,
    /// A variable to add once the environment being created is selected.
    pending_add: Option<String>,
    orphans: Orphans,
    /// Stored secrets of environments with no file, listed with the orphans.
    fileless: Fileless,
    /// The number of the newest fileless list sent; an older answer arriving late is dropped.
    fileless_asked: u64,
    /// Set while a lock or Save check is reading the store or asking; a further one is dropped,
    /// and a blur stores nothing, since a dialog taking focus must not commit the typing.
    checking: bool,
    /// The (name, text) pairs the Save check under way read, confirmed once it agrees.
    checked: Vec<(String, String)>,
    focus: FocusHandle,
    /// Tracked on the list column, so focus on any row counts as "in the list".
    list_focus: FocusHandle,
    scroll: ScrollHandle,
    // Scrolls the editor to the control Tab lands on.
    follow: FocusFollow,
    _subs: Vec<Subscription>,
}

impl EnvEditor {
    /// An editor for the environments of the collection at `root`, with the active environment
    /// selected, else the first.
    pub fn new(
        workspace: Entity<Workspace>,
        root: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let root = std::path::absolute(&root).unwrap_or(root);
        let follow = FocusFollow::default();
        let table = cx.new(|cx| {
            let mut table = EnvVarTable::new(window, cx);
            table.set_follow(follow.clone());
            table
        });
        let extends = cx.new(|cx| SelectState::new(Vec::new(), None, window, cx));
        let name_input = cx.new(|cx| InputState::new(window, cx).placeholder("Environment name"));
        let subs = vec![
            cx.observe_in(&workspace, window, |this, _, window, cx| {
                this.refresh(window, cx)
            }),
            cx.subscribe_in(
                &workspace,
                window,
                |this, _, event, window, cx| match event {
                    WorkspaceEvent::Notices { root, notices, .. } => {
                        this.notices(root, notices, window, cx)
                    }
                    WorkspaceEvent::EnvReverted { root, name } => {
                        this.reverted(root, name, window, cx)
                    }
                    WorkspaceEvent::Delivered { root, acked, .. } => this.delivered(root, *acked),
                    _ => {}
                },
            ),
            cx.subscribe_in(&table, window, |this, _, event, window, cx| {
                this.table_event(event, window, cx)
            }),
            cx.subscribe_in(
                &extends,
                window,
                |this, _, event: &SelectEvent<Vec<Parent>>, window, cx| {
                    if let SelectEvent::Confirm(Some(parent)) = event {
                        this.set_extends(parent.clone(), window, cx);
                    }
                },
            ),
            cx.subscribe_in(
                &name_input,
                window,
                |this, state, event: &InputEvent, window, cx| match event {
                    InputEvent::PressEnter { .. } => {
                        let text = state.read(cx).value().trim().to_string();
                        this.commit_name(&text, window, cx);
                    }
                    // Editing the name answers the refusal it drew; Enter's own echo is no edit.
                    InputEvent::Change
                        if this.refused.as_deref() != Some(state.read(cx).value().trim()) =>
                    {
                        this.drop_refusal(cx)
                    }
                    _ => {}
                },
            ),
        ];
        let mut editor = EnvEditor {
            workspace,
            compare: None,
            root,
            selected: None,
            table,
            base: Vec::new(),
            saved: HashMap::new(),
            saved_secrets: HashMap::new(),
            saved_records: HashMap::new(),
            extends,
            parent_names: Vec::new(),
            name_input,
            naming: None,
            renaming: None,
            refused: None,
            sent: HashMap::new(),
            loaded: LoadStamps::default(),
            lagging: false,
            history: EditHistory::default(),
            awaiting: 0,
            seen: None,
            notice: None,
            locked: false,
            error: None,
            pending_add: None,
            orphans: Orphans::default(),
            fileless: Fileless::default(),
            fileless_asked: 0,
            checking: false,
            checked: Vec::new(),
            focus: cx.focus_handle(),
            list_focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            follow,
            _subs: subs,
        };
        editor.refresh(window, cx);
        editor.list_orphans(cx);
        editor
    }

    /// The selected environment.
    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// How a prompt over several collections names the selected environment: "dev of Shop".
    pub fn place(&self, cx: &App) -> String {
        let snapshot = self.workspace.read(cx).snapshot(&self.root, cx);
        let name = snapshot
            .as_ref()
            .and_then(|s| s.content.collection.name.clone());
        let collection = crate::workspace::label_or_dir(name.as_deref(), &self.root);
        format!(
            "{} of {collection}",
            self.selected.as_deref().unwrap_or_default()
        )
    }

    /// Selects the environment `name` and shows it as the snapshot has it. Secret typing not
    /// yet committed is stored in the environment being left first, except in rows not yet
    /// saved: the switch saves nothing, so their typing is dropped. Selecting the environment
    /// already shown is no switch: it stores nothing and its typing stays in place.
    pub fn select(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let same = self.selected.as_deref() == Some(name);
        if !same {
            self.store_pending_secrets(false, cx);
            self.orphans = Orphans::Unknown;
        }
        self.history.clear();
        self.selected = Some(name.to_string());
        self.naming = None;
        self.drop_refusal(cx);
        self.notice = None;
        // A create in flight may be what is being selected; its notice adds the variable.
        if !self.sent.values().any(|s| matches!(s, Sent::Create)) {
            self.pending_add = None;
        }
        // Empty first, so no secret row's text carries over by name into another environment.
        if !same {
            self.table
                .update(cx, |t, cx| t.load(Vec::new(), window, cx));
        }
        self.load(true, window, cx);
    }

    /// A person's selection of `name`: [`Self::select`], then a list of its orphans. A switch
    /// that would drop a new row's typing over a stored value asks first, as Create does. Paths
    /// the editor takes by itself use `select`, so nothing unattended reaches the keychain.
    pub fn choose(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.as_deref() == Some(name) {
            return self.select_chosen(name, window, cx);
        }
        let name = name.to_owned();
        self.after_save_check(window, cx, move |e, window, cx| {
            e.select_chosen(&name, window, cx)
        });
    }

    fn select_chosen(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.error = None;
        self.select(name, window, cx);
        self.list_orphans(cx);
    }

    /// Lists the selected environment's orphans, and with them the stored secrets of
    /// environments with no file; an orphan answer for an environment no longer selected is
    /// dropped.
    pub fn list_orphans(&mut self, cx: &mut Context<Self>) {
        let (fileless, fileless_answer) = async_channel::bounded(1);
        let root = self.root.clone();
        let Some(env) = self.selected.clone() else {
            let command = Command::FilelessSecrets { reply: fileless };
            self.workspace
                .update(cx, |w, cx| w.send(&root, command, cx));
            return self.take_fileless(fileless_answer, cx);
        };
        let (reply, answer) = async_channel::bounded(1);
        let command = Command::OrphanedSecrets {
            env: env.clone(),
            reply,
            fileless: Some(fileless),
        };
        self.workspace
            .update(cx, |w, cx| w.send(&root, command, cx));
        self.take_fileless(fileless_answer, cx);
        cx.spawn(async move |this, cx| {
            // A closed channel is a list the collection never took, which is no answer either.
            let answer = answer.recv().await.unwrap_or_else(|e| Err(e.to_string()));
            this.update(cx, |e, cx| {
                if e.selected.as_deref() == Some(env.as_str()) {
                    e.orphans = answer.map_or_else(Orphans::Failed, Orphans::Names);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn take_fileless(
        &mut self,
        answer: async_channel::Receiver<Result<Vec<orphan_section::FilelessGroup>, String>>,
        cx: &mut Context<Self>,
    ) {
        self.fileless_asked += 1;
        let asked = self.fileless_asked;
        cx.spawn(async move |this, cx| {
            let answer = answer.recv().await.unwrap_or_else(|e| Err(e.to_string()));
            this.update(cx, |e, cx| {
                if e.fileless_asked == asked {
                    e.fileless = answer.map_or_else(Fileless::Failed, Fileless::Groups);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// The stored secrets of environments with no file, as last listed.
    pub fn fileless(&self) -> &Fileless {
        &self.fileless
    }

    /// Asks to remove the stored value of `name` in `env`, which has no file, warning that the
    /// file may only be elsewhere; removes it on "Delete".
    pub fn ask_remove_fileless(
        &mut self,
        env: &str,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        orphan_section::ask_remove_fileless(env, name, window, cx, Self::remove_fileless);
    }

    // The reply's warnings or refusal show on the error line, then both sections list again.
    fn remove_fileless(&mut self, env: String, name: String, cx: &mut Context<Self>) {
        let (reply, answer) = async_channel::bounded(1);
        let names = vec![name];
        let command = Command::RemoveFilelessSecrets { env, names, reply };
        let root = self.root.clone();
        self.workspace
            .update(cx, |w, cx| w.send(&root, command, cx));
        self.error = None;
        cx.spawn(async move |this, cx| {
            let Ok(answer) = answer.recv().await else {
                return;
            };
            this.update(cx, |e, cx| {
                match answer {
                    Ok(warnings) if warnings.is_empty() => {}
                    Ok(warnings) => e.error = Some(warnings.join("; ")),
                    Err(message) => e.error = Some(message),
                }
                e.list_orphans(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The selected environment's orphans as last listed, unsaved secrets included.
    pub fn orphans(&self) -> &Orphans {
        &self.orphans
    }

    /// Asks to remove the stored value of orphan `name`; removes it on "Delete".
    pub fn ask_remove_orphan(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(env) = self.selected.clone() else {
            return;
        };
        orphan_section::ask_remove(env, name, window, cx, Self::remove_orphan);
    }

    // A name that gained an unsaved secret while the prompt was open is left out, since a save
    // may be about to declare it. The reply's warnings or refusal show on the error line.
    fn remove_orphan(&mut self, env: String, name: String, cx: &mut Context<Self>) {
        if self.selected.as_deref() != Some(env.as_str()) {
            return;
        }
        let names = orphan_section::visible(&[name], &self.draft_variables(cx));
        if names.is_empty() {
            return;
        }
        let (reply, answer) = async_channel::bounded(1);
        let command = Command::RemoveOrphanedSecrets { env, names, reply };
        let root = self.root.clone();
        self.workspace
            .update(cx, |w, cx| w.send(&root, command, cx));
        self.error = None;
        cx.spawn(async move |this, cx| {
            let Ok(answer) = answer.recv().await else {
                return;
            };
            this.update(cx, |e, cx| {
                match answer {
                    Ok(warnings) if warnings.is_empty() => {}
                    Ok(warnings) => e.error = Some(warnings.join("; ")),
                    Err(message) => e.error = Some(message),
                }
                e.list_orphans(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn draft_variables(&self, cx: &App) -> Vec<EnvVariable> {
        self.view(cx)
            .map_or_else(Vec::new, |view| view.variables.clone())
    }

    /// The selected environment's variables table.
    pub fn table(&self) -> &Entity<EnvVarTable> {
        &self.table
    }

    /// What the name input is for while it is open: a new environment's name, or the new
    /// name for the selected one. `None` while it is closed.
    pub fn naming_label(&self) -> Option<String> {
        match self.naming? {
            Naming::Create => Some("New environment name".into()),
            Naming::Rename => Some(format!(
                "Rename \"{}\"",
                self.selected.as_deref().unwrap_or_default()
            )),
        }
    }

    /// The input that takes a new or renamed environment's name.
    pub fn name_input(&self) -> &Entity<InputState> {
        &self.name_input
    }

    /// Creates the environment `name`; core suffixes a taken name, which the New field
    /// refuses before calling this. It is selected once its creation is reported.
    pub fn create(&mut self, name: &str, cx: &mut Context<Self>) {
        let op = EnvOp::Create {
            name: name.to_string(),
        };
        match self.send(Command::Env(op), Sent::Create, cx) {
            Some(id) => self.awaiting = id,
            // Nothing is in flight to bring the environment, so nothing will add the variable.
            None => self.pending_add = None,
        }
    }

    /// Copies the selected environment and selects the copy once its creation is reported.
    /// Nothing is sent while the environment is dirty: the copy is of what the disk holds.
    pub fn duplicate(&mut self, cx: &mut Context<Self>) {
        let Some(from) = self.selected.clone() else {
            return;
        };
        if self.is_locked(cx) {
            return;
        }
        // Typing still in a secret row is stored first, so the copy carries it.
        self.store_pending_secrets(false, cx);
        if let Some(id) = self.send(Command::Env(EnvOp::Duplicate { from }), Sent::Create, cx) {
            self.awaiting = id;
        }
    }

    /// Renames the selected environment to `to`; `to` is selected and the name field closes
    /// once the rename lands. Nothing is sent while the environment is dirty, since core's
    /// rename saves its draft; a taken name is refused here. A refusal keeps the source
    /// selected and the field open on the typed name, marked invalid.
    pub fn rename(&mut self, to: &str, cx: &mut Context<Self>) {
        // One rename at a time: a second Enter would rename a source about to be gone.
        if self.renaming.is_some() {
            return;
        }
        let Some(from) = self.selected.clone() else {
            return;
        };
        if from == to {
            return;
        }
        if let Some(taken) = name_taken(to, &self.names(cx)) {
            self.error = Some(taken);
            self.refused = (self.naming == Some(Naming::Rename)).then(|| to.to_owned());
            cx.notify();
            return;
        }
        if self.is_locked(cx) {
            return;
        }
        // Typing still in a secret row belongs to the old name; the actor remaps it. A rename
        // waits for a clean environment, so no row is unsaved here.
        self.store_pending_secrets(false, cx);
        let op = EnvOp::Rename {
            from: from.clone(),
            to: to.to_string(),
        };
        let sent = Sent::Rename {
            from: from.clone(),
            to: to.to_string(),
        };
        if let Some(id) = self.send(Command::Env(op), sent, cx) {
            self.history.clear();
            self.awaiting = id;
            self.renaming = Some((from, to.to_string(), id));
            self.refused = None;
        }
        cx.notify();
    }

    /// Asks to delete the selected environment; deletes it on "Delete".
    pub fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.selected.clone() else {
            return;
        };
        // A parent others extend is not offered for deletion; core refuses it anyway.
        let by = self.extended_by(&name, cx);
        if !by.is_empty() {
            crate::confirm::ask(
                crate::confirm::extended_ask("Environment", &name, &by),
                window,
                cx,
            );
            return;
        }
        let answer = crate::confirm::ask(
            crate::confirm::delete_ask(format!("Delete environment \"{name}\"?"), None),
            window,
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.recv().await == Ok(1) {
                this.update(cx, |e, cx| {
                    e.history.clear();
                    e.send(Command::Env(EnvOp::Delete { name }), Sent::Plain, cx);
                })
                .ok();
            }
        })
        .detach();
    }

    /// Sets the selected environment's color: "" clears it, `#rgb` or `#rrggbb` sets it, and
    /// anything else is refused with false.
    pub fn set_color(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        let color = match text {
            "" => None,
            t if valid_color(t) => Some(t.to_string()),
            _ => return false,
        };
        self.edit(Edit::Color(color), cx);
        true
    }

    /// Sets the environment the selected one extends, or none.
    pub fn set_extends(
        &mut self,
        parent: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.extends
            .update(cx, |s, cx| s.set_selected_value(&parent, window, cx));
        self.edit(Edit::Extends(parent), cx);
    }

    /// Stores secret typing not yet committed, then writes the selected environment to its
    /// file; a key or button press does not blur the secret input.
    pub fn save(&mut self, cx: &mut Context<Self>) {
        if self.refuse_colliding_secrets(cx) {
            return;
        }
        self.store_pending_secrets(true, cx);
        if let Some(env) = self.selected.clone() {
            self.send(Command::Save(Target::Environment(env)), Sent::Plain, cx);
            // Queued behind the save, so the actor answers from the file it just wrote.
            self.list_orphans(cx);
        }
    }

    /// Asks, then returns the selected environment to its saved file, as the changed-on-disk
    /// banner's Reload does. A clean environment has nothing to discard, so nothing is asked.
    pub fn discard_asking(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(name), true) = (self.selected.clone(), self.is_locked(cx)) else {
            return;
        };
        let title = format!("Discard changes to \"{name}\"?");
        let answer = crate::confirm::ask(crate::confirm::discard_ask(title), window, cx);
        cx.spawn(async move |this, cx| {
            if answer.recv().await == Ok(1) {
                this.update(cx, |e, cx| {
                    e.send(Command::Revert(Target::Environment(name)), Sent::Plain, cx);
                })
                .ok();
            }
        })
        .detach();
    }

    // Refused before any typing is stored: two secrets of one name would share one value. A
    // collision the file already holds blocks only a write that changes a secret.
    fn refuse_colliding_secrets(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(env) = self.selected.as_ref() else {
            return false;
        };
        let table = self.table.read(cx);
        let draft = table.secret_records(cx);
        let saved = self.saved_records.get(env).map(Vec::as_slice);
        let changed = draft != saved.unwrap_or_default() || table.has_secret_typing(cx);
        if !(secrets_collide(&draft) && changed) {
            return false;
        }
        self.error = Some(DUPLICATE_SECRET_NAMES.to_owned());
        cx.notify();
        true
    }

    /// The variable an "Add to environment" waits to add once the environment being created
    /// arrives.
    pub fn pending_add(&self) -> Option<&str> {
        self.pending_add.as_deref()
    }

    /// The last refusal or failure of a command this editor sent, as the error line shows it.
    pub fn error(&self) -> Option<String> {
        self.error.as_deref().map(ui::plain_quotes)
    }

    // The error that refused the open name field's text, as a sentence; it shows under the
    // field, so the pane's error line leaves it out.
    fn field_error(&self) -> Option<String> {
        let refused = self.refused.is_some() && self.naming.is_some();
        refused
            .then(|| self.error())
            .flatten()
            .map(|message| crate::import_report::failure_text(&message))
    }

    /// Exports the selected environment as saved, noting unsaved edits in the toast; nothing
    /// without a selection.
    pub fn export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.selected.clone() else {
            return;
        };
        let dirty = self.view(cx).is_some_and(|v| v.dirty);
        crate::export::export_environment(&self.root, &name, dirty, window, cx);
    }

    /// Exports every environment of the collection as saved to a folder the user picks, noting
    /// unsaved edits in any of them in the toast.
    pub fn export_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let snapshot = self.snapshot(cx);
        let envs = snapshot
            .as_ref()
            .map_or(&[][..], |s| &s.content.environments[..]);
        let dirty = envs.iter().any(|e| e.dirty);
        let source = crate::export::EnvironmentSource::Collection(self.root.clone());
        crate::export::export_environments(source, envs.len(), dirty, window, cx);
    }

    fn snapshot(&self, cx: &App) -> Option<Arc<CollectionSnapshot>> {
        self.workspace.read(cx).snapshot(&self.root, cx)
    }

    fn names(&self, cx: &App) -> Vec<String> {
        self.workspace.read(cx).environments(&self.root, cx)
    }

    // The other environments extending `name`. This mirrors core's `extended_by` over the
    // snapshot on purpose: the editor cannot reach the actor's `Collection`.
    fn extended_by(&self, name: &str, cx: &App) -> Vec<String> {
        self.snapshot(cx).map_or_else(Vec::new, |s| {
            s.content
                .environments
                .iter()
                .filter(|e| e.name != name && e.extends.as_deref() == Some(name))
                .map(|e| e.name.clone())
                .collect()
        })
    }

    fn view(&self, cx: &App) -> Option<Arc<EnvView>> {
        let name = self.selected.as_deref()?;
        let snapshot = self.snapshot(cx)?;
        snapshot
            .content
            .environments
            .iter()
            .find(|e| e.name == name)
            .cloned()
    }

    fn busy(&self, field: Field, cx: &App) -> bool {
        self.selected.as_ref().is_some_and(|env| {
            let target = Target::Environment(env.clone());
            self.workspace
                .read(cx)
                .in_flight(&self.root, &target, field, cx)
        })
    }

    // Dirty, or about to be: an edit in flight will make it so.
    fn is_locked(&self, cx: &App) -> bool {
        self.view(cx).is_some_and(|v| v.dirty)
            || [Field::Variables, Field::Color, Field::Extends]
                .into_iter()
                .any(|f| self.busy(f, cx))
    }

    fn send(&mut self, command: Command, sent: Sent, cx: &mut Context<Self>) -> Option<u64> {
        let root = self.root.clone();
        // An edit carries the snapshot its field last loaded, so the actor holds it back when
        // a reload replaced that content before the field could show it. Lagging rows came
        // from the load before that.
        let basis = match &command {
            Command::Edit {
                target: Target::Environment(env),
                edit,
            } => (self.loaded).basis(&(env.clone(), edit.field()), self.lagging),
            _ => None,
        };
        let id = self
            .workspace
            .update(cx, |w, cx| w.send_from(&root, command, basis, cx));
        self.history.bind(id);
        if let Some(id) = id {
            self.sent.insert(id, sent);
        }
        // A new command's outcome is not the rename's refusal, so the field is unmarked.
        self.refused = None;
        self.error = None;
        self.notice = None;
        self.sync_lock(cx);
        id
    }

    // Variables, color and parent become undo steps; `typed` is the cell a keystroke landed
    // in, so a run of keystrokes there is one step.
    fn edit(&mut self, edit: Edit, cx: &mut Context<Self>) {
        let typed = match &edit {
            Edit::Variables(_) => self.table.read(cx).last_typed(),
            _ => None,
        };
        match self.view(cx) {
            Some(view) => {
                let busy = self.busy(edit.field(), cx);
                let read = |e: &Edit| view_edit(&view, e);
                edit_track::track(&mut self.history, &edit, typed, busy, &read)
            }
            None => {
                self.history.note_sent(&edit);
            }
        }
        self.dispatch(edit, cx);
    }

    fn dispatch(&mut self, edit: Edit, cx: &mut Context<Self>) {
        if let Some(env) = self.selected.clone() {
            let command = Command::Edit {
                target: Target::Environment(env),
                edit,
            };
            self.send(command, Sent::Plain, cx);
        }
    }

    /// Takes back the last variable, color or parent edit by sending its earlier value. Does
    /// nothing when there is none or when the field was changed another way, which forgets
    /// that field's steps.
    pub fn undo_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.step_asking(true, window, cx);
    }

    /// Redoes the edit [`Self::undo_edit`] took back, under the same conditions.
    pub fn redo_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.step_asking(false, window, cx);
    }

    // A variables step reloads the rows and may take a new row's lock, and its typing, with it.
    fn step_asking(&mut self, undo: bool, window: &mut Window, cx: &mut Context<Self>) {
        let step =
            move |e: &mut Self, _: &mut Window, cx: &mut Context<Self>| e.step_history(undo, cx);
        match self.history.next_field(undo) {
            Some(Field::Variables) => self.after_save_check(window, cx, step),
            _ => self.after_held(window, cx, step),
        }
    }

    // Applies the history's next value as an ordinary, unrecorded edit; the controls follow
    // the echo. An edit in flight on the field makes the history trust its own last value.
    fn step_history(&mut self, undo: bool, cx: &mut Context<Self>) {
        let Some(field) = self.history.next_field(undo) else {
            return;
        };
        let Some(view) = self.view(cx) else {
            return;
        };
        let busy = self.busy(field, cx);
        let holds = |edit: &Edit| view_edit(&view, edit).is_some_and(|h| same_value(&h, edit));
        if let Some(edit) = edit_track::step(&mut self.history, undo, busy, &holds) {
            self.dispatch(edit, cx);
        }
    }

    // A secret flag change rewrites variables the history cannot restore, so the variables
    // steps retire; the flag itself is not an edit.
    fn retire_variables(&mut self, cx: &App) {
        match self.view(cx) {
            Some(view) => {
                self.history
                    .note_sent(&Edit::Variables(view.variables.clone()));
            }
            None => self.history.clear(),
        }
    }

    /// Stores secret typing not yet committed in the selected environment; a key or button
    /// press does not blur the secret input, so callers flush it before saving or closing.
    /// Rows not yet in the file are stored only when `saving`, since a discard orphans them.
    pub fn store_pending_secrets(&mut self, saving: bool, cx: &mut Context<Self>) {
        let Some(env) = self.selected.clone() else {
            return;
        };
        let pending = self
            .table
            .update(cx, |t, cx| t.take_pending_secrets(saving, cx));
        for (var, value) in pending {
            let env = env.clone();
            let op = EnvOp::SetSecretValue { env, var, value };
            self.send(Command::Env(op), Sent::Plain, cx);
        }
    }

    fn table_event(&mut self, event: &EnvTableEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(env) = self.selected.clone() else {
            return;
        };
        match event {
            EnvTableEvent::Changed(rows) => {
                // Rows the table no longer shows predate a load that came in the same update.
                self.lagging = self.table.read(cx).rows(cx) != *rows;
                self.edit(Edit::Variables(env_vars_from(&self.base, rows)), cx);
                self.lagging = false;
            }
            EnvTableEvent::SetSecret { name, secret } => {
                self.check_lock(env, name.clone(), *secret, window, cx);
            }
            EnvTableEvent::SecretValue { name, value } => {
                let (var, value) = (name.clone(), value.clone());
                let op = EnvOp::SetSecretValue { env, var, value };
                self.send(Command::Env(op), Sent::Plain, cx);
            }
            EnvTableEvent::Reveal(name) => self.reveal(env, name.clone(), window, cx),
            EnvTableEvent::TypingHeld => self.after_held(window, cx, |_, _, _| {}),
        }
    }

    fn set_checking(&mut self, checking: bool, cx: &mut Context<Self>) {
        self.checking = checking;
        self.sync_lock(cx);
    }

    // Asks the store what a lock click would lose, and only then toggles. Nothing is sent
    // before the answer, so a Cancel leaves the store, the file and the typing as they were.
    fn check_lock(
        &mut self,
        env: String,
        var: String,
        secret: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.checking || self.is_locked(cx) {
            return;
        }
        self.set_checking(true, cx);
        let read = self.read_stored(&env, &var, cx);
        cx.spawn_in(window, async move |this, cx| {
            let read = read.recv().await;
            let step = this.update_in(cx, |e, window, cx| {
                e.lock_step(&env, &var, secret, read, window, cx)
            });
            let go = step.unwrap_or(LockStep::Stop).confirmed().await;
            this.update(cx, |e, cx| {
                e.set_checking(false, cx);
                if go {
                    e.toggle_lock(env, var, secret, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn lock_step(
        &mut self,
        env: &str,
        var: &str,
        secret: bool,
        read: Result<Result<Option<String>, String>, async_channel::RecvError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> LockStep {
        if self.selected.as_deref() != Some(env) {
            return LockStep::Stop;
        }
        // A read that answers ends whatever an earlier failed read put on the error line.
        let stored = match read {
            Ok(Ok(stored)) => {
                self.error = None;
                cx.notify();
                stored
            }
            Ok(Err(message)) => {
                self.error = Some(message);
                cx.notify();
                return LockStep::Stop;
            }
            Err(_) => return LockStep::Stop,
        };
        let rows = self.table.read(cx).rows(cx);
        let Some(row) = rows.iter().find(|row| row.name == var) else {
            return LockStep::Stop;
        };
        match secret_lock::question(secret, &row.value, stored.as_deref()) {
            Some(question) => {
                let spec = secret_lock::lock_ask(question, var);
                LockStep::Ask(crate::confirm::ask(spec, window, cx))
            }
            None => LockStep::Go,
        }
    }

    // The row's typing goes ahead of the toggle on the same queue, so the toggle moves what was
    // typed; an event would be delivered behind it.
    fn toggle_lock(&mut self, env: String, var: String, secret: bool, cx: &mut Context<Self>) {
        if self.selected.as_deref() != Some(env.as_str()) || self.is_locked(cx) {
            return;
        }
        let taken = self.table.update(cx, |t, cx| {
            t.confirm_typing(&var, cx);
            t.take_typing(&var, cx)
        });
        if let Some(value) = taken {
            let (env, var) = (env.clone(), var.clone());
            let op = EnvOp::SetSecretValue { env, var, value };
            self.send(Command::Env(op), Sent::Plain, cx);
        }
        self.retire_variables(cx);
        let op = EnvOp::SetSecret { env, var, secret };
        self.send(Command::Env(op), Sent::Plain, cx);
        // Queued behind the toggle, so the actor answers from the file it wrote.
        self.list_orphans(cx);
    }

    /// Whether a Save (`saving`) or a release of held typing would have to ask the store first.
    pub fn owes_replace_check(&self, saving: bool, cx: &App) -> bool {
        !self.typing_to_check(saving, cx).is_empty()
    }

    // What the action leaves the table with: a release of held typing takes no draft row, so
    // its prompt must not name one.
    fn typing_to_check(&self, saving: bool, cx: &App) -> Vec<(String, String)> {
        let table = self.table.read(cx);
        match saving {
            true => table.unsaved_secret_typing(cx),
            false => table.held_secret_typing(cx),
        }
    }

    /// Whether a lock or Save check is under way.
    pub fn checking(&self) -> bool {
        self.checking
    }

    /// Starts a check of what a Save (`saving`) or a release of held typing would write: the
    /// task names the rows that would overwrite a stored value, or yields `None` once a failed
    /// read is on the error line. The caller ends it with [`Self::end_replace_check`].
    pub fn begin_replace_check(
        &mut self,
        saving: bool,
        cx: &mut Context<Self>,
    ) -> Task<Option<Vec<String>>> {
        self.set_checking(true, cx);
        let Some(env) = self.selected.clone() else {
            return Task::ready(Some(Vec::new()));
        };
        let typing = self.typing_to_check(saving, cx);
        self.checked.clone_from(&typing);
        let reads: Vec<_> = typing
            .iter()
            .map(|(var, _)| self.read_stored(&env, var, cx))
            .collect();
        cx.spawn(async move |this, cx| {
            let mut stored = Vec::new();
            for read in reads {
                match read.recv().await {
                    Ok(Ok(value)) => stored.push(value),
                    Ok(Err(message)) => {
                        this.update(cx, |e, cx| {
                            e.error = Some(message);
                            cx.notify();
                        })
                        .ok();
                        return None;
                    }
                    Err(_) => return None,
                }
            }
            this.update(cx, |e, cx| {
                e.error = None;
                cx.notify();
            })
            .ok();
            Some(secret_lock::replaced(&typing, &stored))
        })
    }

    /// Ends the check [`Self::begin_replace_check`] started; `agreed` lets the typing it read
    /// out of the table, and anything else keeps it there.
    pub fn end_replace_check(&mut self, agreed: bool, cx: &mut Context<Self>) {
        let checked = std::mem::take(&mut self.checked);
        if agreed {
            self.table
                .update(cx, |t, cx| t.confirm_checked(&checked, cx));
        }
        self.set_checking(false, cx);
    }

    // Runs `then` once no new secret row would overwrite a stored value unasked: at once when
    // no such row has typing, so a plain Save never reads the store.
    fn after_replace_check(
        &mut self,
        saving: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        if self.checking {
            return;
        }
        // Typing no Save can store is dropped by leaving, so the person agrees to that first.
        let twins = self.table.read(cx).twin_typing(cx);
        if !saving && !twins.is_empty() {
            return self.after_twins_dropped(twins, window, cx, move |e, window, cx| {
                e.after_replace_check(saving, window, cx, then)
            });
        }
        if !self.owes_replace_check(saving, cx) {
            return then(self, window, cx);
        }
        let read = self.begin_replace_check(saving, cx);
        cx.spawn_in(window, async move |this, cx| {
            let go = secret_lock::replace_agreed(vec![(String::new(), read)], cx).await;
            this.update_in(cx, |e, window, cx| {
                e.end_replace_check(go, cx);
                if go {
                    then(e, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// The twin rows holding typing; see [`EnvVarTable::twin_typing`].
    pub fn twin_typing(&self, cx: &App) -> Vec<String> {
        self.table.read(cx).twin_typing(cx)
    }

    /// Empties the twin rows' typing once the person agreed to drop it.
    pub fn discard_twin_typing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.table
            .update(cx, |t, cx| t.discard_twin_typing(window, cx));
    }

    // Asks before dropping typing in twin rows `names`; Discard empties them and runs `then`,
    // Cancel keeps them.
    fn after_twins_dropped(
        &mut self,
        names: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        self.set_checking(true, cx);
        cx.spawn_in(window, async move |this, cx| {
            let go = secret_lock::twins_dropped(names, cx).await;
            this.update_in(cx, |e, window, cx| {
                e.set_checking(false, cx);
                if go {
                    e.table
                        .update(cx, |t, cx| t.discard_twin_typing(window, cx));
                    then(e, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Whether the table holds typing only it knows of; see [`EnvVarTable::holds_typing`].
    pub fn holds_typing(&self, cx: &App) -> bool {
        self.table.read(cx).holds_typing(cx)
    }

    // Runs `act` once held typing has left the table through the Save check, so an action that
    // would lose it first stores it, or stops on a Cancel or a failed read.
    fn after_held(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        act: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        if !self.holds_typing(cx) {
            return act(self, window, cx);
        }
        self.after_replace_check(false, window, cx, move |e, window, cx| {
            e.store_pending_secrets(false, cx);
            act(e, window, cx);
        });
    }

    /// [`Self::save`], after asking whether a new secret row may replace a stored value.
    pub fn save_asking(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.after_replace_check(true, window, cx, |e, _, cx| e.save(cx));
    }

    /// [`Self::answer`]; an answer that writes the draft asks as a Save does.
    pub fn answer_asking(
        &mut self,
        banner: Banner,
        choice: Choice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let writes = self.conflicts(cx).is_some_and(|(target, _, held)| {
            let commands = banner::commands(banner, choice, &target, &held);
            commands.iter().any(writes_draft)
        });
        if writes {
            self.after_replace_check(true, window, cx, move |e, _, cx| {
                e.answer(banner, choice, cx)
            });
        } else {
            self.answer(banner, choice, cx);
        }
    }

    // The value stored under `var`'s name on a channel of its own, queued after pending writes.
    fn read_stored(
        &mut self,
        env: &str,
        var: &str,
        cx: &mut Context<Self>,
    ) -> async_channel::Receiver<Result<Option<String>, String>> {
        let (reply, answer) = async_channel::bounded(1);
        let command = Command::StoredSecret {
            env: env.to_owned(),
            var: var.to_owned(),
            reply,
        };
        let root = self.root.clone();
        self.workspace
            .update(cx, |w, cx| w.send(&root, command, cx));
        answer
    }

    // The value comes back on a channel of its own; it is shown only if the same environment
    // is still selected when it arrives.
    fn reveal(&mut self, env: String, var: String, window: &mut Window, cx: &mut Context<Self>) {
        let (reply, answer) = async_channel::bounded(1);
        let command = Command::RevealSecret {
            env: env.clone(),
            var: var.clone(),
            reply,
        };
        let root = self.root.clone();
        self.workspace
            .update(cx, |w, cx| w.send(&root, command, cx));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(answer) = answer.recv().await else {
                return;
            };
            this.update_in(cx, |e, window, cx| {
                if e.selected.as_deref() != Some(env.as_str()) {
                    return;
                }
                match answer {
                    Ok(value) => e
                        .table
                        .update(cx, |t, cx| t.show_secret(&var, value, window, cx)),
                    Err(message) => e.error = Some(message),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // A created environment is selected; a failure shows its message, and a failed rename
    // marks its still open field invalid.
    fn notices(
        &mut self,
        root: &Path,
        notices: &[Notice],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if root != self.root {
            return;
        }
        for notice in notices {
            let Some(sent) = self.sent.remove(&notice.command) else {
                continue;
            };
            edit_track::refused(&mut self.history, notice);
            match (&notice.kind, sent) {
                (NoticeKind::EnvironmentCreated(name), Sent::Create) => {
                    let pending = self.pending_add.take();
                    // Not `choose`: a failure delivered alongside this one must keep its line.
                    self.select(name, window, cx);
                    self.list_orphans(cx);
                    if let Some(var) = pending {
                        self.table.update(cx, |t, cx| t.add_row(&var, window, cx));
                        // Made for an undefined variable: with no active environment, it
                        // becomes the active one so the next send finds the variable.
                        self.workspace.update(cx, |w, cx| {
                            if w.selected_environment(root, cx).is_none() {
                                w.select_environment(root, Some(name.clone()), cx);
                            }
                        });
                    }
                }
                (NoticeKind::Failed { message, .. }, sent) => {
                    if matches!(sent, Sent::Create) {
                        self.pending_add = None;
                    }
                    if let Sent::Rename { from, to } = sent {
                        self.renaming.take_if(|r| r.2 == notice.command);
                        // Only the field still offering this very name is the refused one.
                        let held = self.name_input.read(cx).value().trim() == to;
                        if self.selected.as_ref() == Some(&from)
                            && self.naming == Some(Naming::Rename)
                            && held
                        {
                            self.refused = Some(to);
                        }
                    }
                    self.error = Some(message.clone());
                }
                _ => {}
            }
        }
        cx.notify();
    }

    // Commands up to `acked` had their notices handed over already, so only newer ones stay
    // mapped; a send before that delivery must not drop an outcome still on its way.
    fn delivered(&mut self, root: &Path, acked: u64) {
        if root == self.root {
            self.sent.retain(|&id, _| id > acked);
            self.history.settle(acked);
        }
    }

    // A Revert of the shown environment, sent from anywhere, drops its unsent secret typing
    // now, so no input before the actor answers can store it.
    fn reverted(&mut self, root: &Path, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        if root == self.root && self.selected.as_deref() == Some(name) {
            self.table
                .update(cx, |t, cx| t.discard_secret_typing(window, cx));
        }
    }

    // Typing alone makes no draft, so an outside change replaces a clean environment without a
    // banner. A stamp raised since the last refresh of the same environment means its file was
    // replaced; every secret row is masked again, and dropped typing gets a notice. A missing view
    // keeps the last stamp, so a namesake recreated under a held selection is still compared.
    fn check_outside_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.view(cx) else {
            return;
        };
        let same = |name: &String| *name == view.name;
        let seen = self.seen.as_ref().filter(|(name, ..)| same(name));
        let outside = seen.is_some_and(|(_, stamp, _)| view.reloaded > *stamp);
        // A save or a reload is a point undo stops at, so the next keystroke is a step of its own.
        if outside || seen.is_some_and(|(_, _, dirty)| *dirty && !view.dirty) {
            self.history.end_typing();
        }
        if outside {
            let typed = self.table.read(cx).has_secret_typing(cx);
            self.table
                .update(cx, |t, cx| t.discard_secret_typing(window, cx));
            if typed {
                self.notice = Some(SECRETS_CLEARED);
            }
        }
        self.seen = Some((view.name.clone(), view.reloaded, view.dirty));
    }

    fn sync_lock(&mut self, cx: &mut Context<Self>) {
        // A blur under any disk banner would store typing its Reload or Discard is about to drop.
        let on_disk = self.view(cx).is_some_and(|v| v.disk != DiskState::Present);
        let held = on_disk || self.checking;
        self.table.update(cx, |t, _| t.set_blur_held(held));
        let locked = self.is_locked(cx);
        if locked != self.locked {
            self.locked = locked;
            self.table
                .update(cx, |t, cx| t.set_secret_locked(locked, cx));
        }
        if locked && self.naming == Some(Naming::Rename) {
            self.naming = None;
        }
        cx.notify();
    }

    // A selected name gone from the snapshot falls back to the active environment, else the
    // first, unless a create or rename of ours is still unacknowledged and may yet bring it.
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(snapshot) = self.snapshot(cx) else {
            return;
        };
        let names = self.names(cx);
        self.landed(&names, window, cx);
        // An answered rename the snapshot never showed (its source came back) is over too.
        self.renaming.take_if(|r| snapshot.acked >= r.2);
        // A deleted or renamed environment's record goes, so a later namesake starts afresh.
        self.saved.retain(|env, _| names.contains(env));
        self.saved_secrets.retain(|env, _| names.contains(env));
        self.saved_records.retain(|env, _| names.contains(env));
        let present = self
            .selected
            .as_ref()
            .is_some_and(|s| names.iter().any(|n| n == s));
        if !present && snapshot.acked >= self.awaiting {
            // The environment is gone, so typing still in its secret rows has nowhere to go.
            self.selected = None;
            self.history.clear();
            // A rename prompt names the vanished environment; a create prompt still makes sense.
            if self.naming == Some(Naming::Rename) {
                self.naming = None;
            }
            let active = self
                .workspace
                .read(cx)
                .selected_environment(&self.root, cx)
                .filter(|a| names.contains(a));
            match active.as_ref().or(names.first()) {
                Some(next) => self.select(&next.clone(), window, cx),
                None => {
                    self.table
                        .update(cx, |t, cx| t.load(Vec::new(), window, cx));
                }
            }
        } else {
            self.load(false, window, cx);
        }
        self.check_outside_change(window, cx);
        self.sync_lock(cx);
        if snapshot.acked >= self.awaiting {
            self.apply_intent(window, cx);
        }
    }

    // A rename in flight has landed once its source is gone and its target is there: the new
    // name takes the selection unless the user chose another meanwhile, and closes its field.
    fn landed(&mut self, names: &[String], window: &mut Window, cx: &mut Context<Self>) {
        let Some((from, to, _)) = &self.renaming else {
            return;
        };
        if names.contains(from) || !names.contains(to) {
            return;
        }
        // A selection or field the user moved to since is theirs, so only the source's closes.
        if self.selected.as_ref() == Some(from) {
            self.selected = Some(to.clone());
            // Listed under the old name, where a rename leaves its orphans.
            self.orphans = Orphans::Unknown;
            if self.naming == Some(Naming::Rename) {
                self.naming = None;
                self.focus.focus(window, cx);
            }
        }
        self.renaming = None;
    }

    // Applies an "Add to environment" intent for this collection once: select its target and
    // add the row, or open the naming prompt and add the row when the new environment arrives.
    fn apply_intent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.root.clone();
        let Some(mut intent) = self.workspace.update(cx, |w, _| w.take_env_intent(&root)) else {
            return;
        };
        // A target deleted since the click is reported, and the row goes to the environment
        // now selected, else into a new one.
        let gone = intent.select.take_if(|env| !self.names(cx).contains(env));
        if gone.is_some() {
            intent.create = self.selected.is_none();
        }
        if intent.create {
            self.pending_add = Some(intent.add_var);
            if self.naming != Some(Naming::Create) {
                self.start_naming(Naming::Create, window, cx);
            }
        } else {
            // Reselecting the shown environment would reload it and drop typing in progress; a
            // real switch asks as a chosen one does.
            let add_var = intent.add_var;
            match intent
                .select
                .filter(|env| self.selected.as_ref() != Some(env))
            {
                Some(env) => self.after_save_check(window, cx, move |e, window, cx| {
                    e.select(&env, window, cx);
                    e.table.update(cx, |t, cx| t.add_row(&add_var, window, cx));
                }),
                None => {
                    self.table
                        .update(cx, |t, cx| t.add_row(&add_var, window, cx));
                }
            }
        }
        // Deferred: the row's edit is sent when this update ends, and a send clears the line.
        if let Some(gone) = gone {
            let this = cx.entity();
            cx.defer(move |cx| {
                this.update(cx, |e, cx| {
                    e.error = Some(format!("environment `{gone}` no longer exists"));
                    cx.notify();
                })
            });
        }
    }

    // Shows the selected environment's view; unless `all`, a field with an edit in flight
    // keeps what the user typed.
    fn load(&mut self, all: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(view), Some(name)) = (self.view(cx), self.selected.clone()) else {
            return;
        };
        // A dirty environment first seen now counts all its names as saved.
        if !view.dirty || !self.saved.contains_key(&name) {
            let names = view.variables.iter().map(|v| v.name.clone()).collect();
            self.saved.insert(name.clone(), names);
            let secrets = (view.variables.iter().filter(|v| v.secret))
                .map(|v| v.name.clone())
                .collect();
            self.saved_secrets.insert(name.clone(), secrets);
            (self.saved_records).insert(name.clone(), secret_records(&view.variables));
        }
        let seq = self.snapshot(cx).map_or(0, |s| s.seq);
        for field in [Field::Variables, Field::Extends] {
            if all || !self.busy(field, cx) {
                self.loaded.note((name.clone(), field), seq);
            }
        }
        if all || !self.busy(Field::Variables, cx) {
            self.base = view.variables.clone();
            let mut rows = env_rows(&self.base);
            let saved = self.saved.get(&name).map(Vec::as_slice).unwrap_or_default();
            for row in &mut rows {
                row.unsaved = !saved.contains(&row.name);
            }
            let secrets = self.saved_secrets.get(&name).cloned().unwrap_or_default();
            self.table.update(cx, |t, cx| {
                t.set_saved_secrets(secrets);
                t.load(rows, window, cx)
            });
        }
        if all || !self.busy(Field::Extends, cx) {
            let items = parents(&self.names(cx), &name, view.extends.as_deref());
            let values: Vec<Option<String>> = items.iter().map(|p| p.value.clone()).collect();
            let extends = view.extends.clone();
            self.extends.update(cx, |s, cx| {
                if values != self.parent_names {
                    s.set_items(items, window, cx);
                }
                if s.selected_value() != Some(&extends) {
                    s.set_selected_value(&extends, window, cx);
                }
            });
            self.parent_names = values;
        }
    }

    // A pending variable belongs to the create prompt; closing it or switching to rename
    // forgets the variable.
    fn start_naming(&mut self, naming: Naming, window: &mut Window, cx: &mut Context<Self>) {
        self.drop_refusal(cx);
        if self.naming == Some(naming) || naming == Naming::Rename {
            self.pending_add = None;
        }
        if self.naming == Some(naming) {
            self.naming = None;
        } else {
            let text = match naming {
                Naming::Create => String::new(),
                Naming::Rename => self.selected.clone().unwrap_or_default(),
            };
            // Rename opens on the current name selected, so typing replaces it.
            self.name_input.update(cx, |s, cx| {
                s.set_value(text, window, cx);
                s.focus(window, cx);
                if naming == Naming::Rename {
                    s.select_all(window, cx);
                }
            });
            self.naming = Some(naming);
        }
        cx.notify();
    }

    // Closes the prompt without sending anything; focus moves to the editor because the
    // focused input leaves the tree and editor keys would otherwise stop arriving.
    fn cancel_naming(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.naming = None;
        self.pending_add = None;
        self.drop_refusal(cx);
        self.focus.focus(window, cx);
        cx.notify();
    }

    // A refused name's error lives as long as its field holds that name.
    fn drop_refusal(&mut self, cx: &mut Context<Self>) {
        if self.refused.take().is_some() {
            self.error = None;
            cx.notify();
        }
    }

    fn commit_name(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if text.is_empty() {
            return self.commit_name_now(text, window, cx);
        }
        // Both land by selecting a new name, which would take held typing with it; a create
        // leaves a dirty environment, so a new row's typing goes too.
        let create = self.naming == Some(Naming::Create);
        // A name New refuses is refused before the check, whose Replace saves.
        if create && let Some(reason) = create_refusal(text, &self.names(cx)) {
            self.error = Some(reason);
            self.refused = Some(text.to_owned());
            cx.notify();
            return;
        }
        let text = text.to_owned();
        let commit = move |e: &mut Self, window: &mut Window, cx: &mut Context<Self>| {
            e.commit_name_now(&text, window, cx)
        };
        match create {
            true => self.after_save_check(window, cx, commit),
            false => self.after_held(window, cx, commit),
        }
    }

    // Runs `act`, which drops the table's typing, once a Save would have nothing to ask about it:
    // Replace saves as Save does and then acts; Cancel or a failed read keeps the typing.
    fn after_save_check(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        act: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        // Typing a Save would ask about must not leave while another check holds the prompt.
        let owed = self.owes_replace_check(true, cx);
        if self.checking && owed {
            return;
        }
        if self.checking || !owed {
            return self.after_held(window, cx, act);
        }
        let read = self.begin_replace_check(true, cx);
        cx.spawn_in(window, async move |this, cx| {
            let answer = secret_lock::replace_answer(vec![(String::new(), read)], cx).await;
            this.update_in(cx, |e, window, cx| {
                e.end_replace_check(answer == ReplaceAnswer::Replace, cx);
                match answer {
                    ReplaceAnswer::Replace => {
                        e.save(cx);
                        act(e, window, cx);
                    }
                    ReplaceAnswer::Nothing => e.after_held(window, cx, act),
                    ReplaceAnswer::Stop => {}
                }
            })
            .ok();
        })
        .detach();
    }

    fn commit_name_now(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        // A rename keeps its field, focused, until the outcome says whether the name took.
        let unchanged = self.selected.as_deref() == Some(text);
        if self.naming == Some(Naming::Rename) && !text.is_empty() && !unchanged {
            return self.rename(text, cx);
        }
        if self.naming == Some(Naming::Rename) {
            self.focus.focus(window, cx);
        }
        match self.naming.take() {
            _ if text.is_empty() => self.pending_add = None,
            Some(Naming::Create) => self.create(text, cx),
            Some(Naming::Rename) => self.rename(text, cx),
            None => {}
        }
        cx.notify();
    }

    // The banners of the selected environment, from the current snapshot alone.
    fn conflicts(&self, cx: &App) -> Option<(Target, Vec<Banner>, Vec<Edit>)> {
        let view = self.view(cx)?;
        let target = Target::Environment(view.name.clone());
        let held = self
            .snapshot(cx)
            .and_then(|s| s.stale.get(&target).cloned())
            .unwrap_or_default();
        Some((target, banner::banners(view.disk, held.len()), held))
    }

    /// The open Compare dialog, if one was opened from the banner.
    pub fn compare(&self) -> Option<Entity<crate::compare_dialog::CompareDialog>> {
        self.compare.clone()
    }

    // Opens the read-only diff of disk against draft; the banner's own answer runs from it.
    fn open_compare(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((target, shown, _)) = self.conflicts(cx) else {
            return;
        };
        if !shown.contains(&Banner::ChangedOnDisk) {
            return;
        }
        let this = cx.entity().downgrade();
        let answer: crate::compare_dialog::Answer = std::rc::Rc::new(move |choice, window, cx| {
            let _ = this.update(cx, |e, cx| {
                e.answer_asking(Banner::ChangedOnDisk, choice, window, cx)
            });
        });
        let (workspace, root) = (self.workspace.clone(), self.root.clone());
        self.compare = Some(crate::compare_dialog::open(
            workspace, root, target, answer, window, cx,
        ));
    }

    /// Answers the shown `banner` with `choice`, reading the held edits afresh.
    pub fn answer(&mut self, banner: Banner, choice: Choice, cx: &mut Context<Self>) {
        let Some((target, shown, held)) = self.conflicts(cx) else {
            return;
        };
        // A click from a frame drawn before the banner went away answers nothing.
        let kind = std::mem::discriminant(&banner);
        if !shown.iter().any(|b| std::mem::discriminant(b) == kind) {
            return;
        }
        let commands = banner::commands(banner, choice, &target, &held);
        // A write of the draft is refused or stores the typing first, as Save does.
        if commands.iter().any(writes_draft) {
            if self.refuse_colliding_secrets(cx) {
                return;
            }
            self.store_pending_secrets(true, cx);
        }
        for command in commands {
            self.send(command, Sent::Plain, cx);
        }
    }
}

impl EnvEditor {
    // A swatch button takes no focus on click, so focus moves to the editor when it has
    // none, and its keys (Save) keep working after a pick.
    fn pick(&mut self, color: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.contains_focused(window, cx) {
            window.focus(&self.focus, cx);
        }
        self.set_color(color.unwrap_or_default(), cx);
    }

    // None, the presets, and the file's own colour when it matches no preset; the chosen one
    // carries a ring. The custom swatch only shows the file's text and sends nothing.
    fn swatches(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let color = self.view(cx).and_then(|v| v.color.clone());
        let chosen = |c: Option<&str>| match (color.as_deref(), c) {
            (None, None) => true,
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
            _ => false,
        };
        let custom = color
            .clone()
            .filter(|c| !ENV_SWATCHES.iter().any(|p| p.eq_ignore_ascii_case(c)));
        let none = swatch("env-swatch-none", "None", None, chosen(None), cx)
            .on_click(cx.listener(|e, _, window, cx| e.pick(None, window, cx)));
        let presets = ENV_SWATCHES.iter().enumerate().map(|(ix, hex)| {
            swatch(("env-swatch", ix), hex, Some(hex), chosen(Some(hex)), cx)
                .on_click(cx.listener(move |e, _, window, cx| e.pick(Some(hex), window, cx)))
        });
        let presets: Vec<_> = presets.collect();
        let custom = custom.map(|text| swatch("env-swatch-custom", &text, Some(&text), true, cx));
        // Each swatch reports to the follow, so Tab onto one scrolls it into view.
        let follow = &self.follow;
        let presets = (presets.into_iter().enumerate())
            .map(|(ix, b)| Followed::new(("env-swatch-follow", ix), follow, b));
        h_flex()
            .id("env-swatches")
            .test_support()
            .flex_wrap()
            .gap_1()
            .child(Followed::new("env-swatch-none-follow", follow, none))
            .children(presets)
            .children(custom.map(|b| Followed::new("env-swatch-custom-follow", follow, b)))
    }

    // The pane's header: the name as a title that truncates, then the "…" menu, then Discard
    // changes and Save at the right, live only while there are edits to drop or save.
    fn header(&self, locked: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let name = self.selected.clone().unwrap_or_default();
        let dirty = self.view(cx).is_some_and(|v| v.dirty);
        let this = cx.entity();
        let followed = |id: &'static str, button: Button| Followed::new(id, &self.follow, button);
        let more = Button::new("env-more")
            .ghost()
            .compact()
            .icon(IconName::Ellipsis)
            .tooltip(MORE_ACTIONS)
            .accessibility_label(MORE_ACTIONS)
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                env_menu(menu, this.clone(), locked)
            });
        h_flex()
            .w_full()
            .gap_2()
            .child(
                div()
                    .id("env-title")
                    .test_support()
                    .aria_label(name.clone())
                    // A zero basis keeps the name out of the pane's minimum width, so a long
                    // one truncates instead of squeezing the list or pushing Save out.
                    .flex_1()
                    .w_0()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(name),
            )
            .child(Followed::new("env-more-follow", &self.follow, more))
            .child(followed(
                "env-duplicate-follow",
                Button::new("env-duplicate")
                    .label("Duplicate")
                    .lock(locked, cx)
                    .when(locked, |b| b.tooltip(RENAME_LOCKED_REASON))
                    .on_click(cx.listener(|e, _, window, cx| {
                        e.after_held(window, cx, |e, _, cx| e.duplicate(cx))
                    })),
            ))
            .child(followed(
                "env-discard-follow",
                Button::new("env-discard")
                    .label("Discard changes")
                    .lock(!locked, cx)
                    .on_click(cx.listener(|e, _, window, cx| e.discard_asking(window, cx))),
            ))
            .child(followed(
                "env-save-follow",
                Button::new("env-save")
                    .when(dirty, |b| b.primary())
                    .label("Save")
                    .on_click(cx.listener(|e, _, window, cx| e.save_asking(window, cx))),
            ))
    }
}

// One ghost swatch button labelled `label`: a dot of `fill` (muted when the text is no hex
// colour), ringed when chosen. With no fill the outline is all there is, so it takes the
// muted text colour, which holds 3:1 against the pane in both themes.
fn swatch(
    id: impl Into<ElementId>,
    label: &str,
    fill: Option<&str>,
    chosen: bool,
    cx: &App,
) -> Button {
    let theme = cx.theme();
    let dot = div()
        .id("env-swatch-dot")
        .test_support()
        .size(style::CHECKBOX_SIZE)
        .rounded_full()
        .border_1()
        .border_color(if fill.is_some() {
            theme.border
        } else {
            theme.muted_foreground
        })
        .when_some(fill, |dot, hex| {
            dot.bg(crate::ui::parse_color(hex).unwrap_or(theme.muted))
        });
    let ring = chosen.then(|| {
        div()
            .id("env-swatch-ring")
            .test_support()
            .absolute()
            .inset_0()
            .rounded_full()
            .border_2()
            .border_color(theme.foreground)
    });
    Button::new(id)
        .ghost()
        .compact()
        .tooltip(label.to_string())
        .accessibility_label(label.to_string())
        .child(div().relative().p(style::SPACE_0).child(dot).children(ring))
}

/// The content of the "…" menu's Rename entry while the environment is locked.
pub fn locked_rename_label(cx: &App) -> impl IntoElement + gpui_kit::Styled + use<> {
    ui::menu_label_cue(div(), cx)
        .id("env-rename-locked")
        .test_support()
        .aria_label(RENAME_LOCKED_REASON)
        .tooltip(|window, cx| Tooltip::new(RENAME_LOCKED_REASON).build(window, cx))
        .child("Rename")
}

// The "…" menu's entries, in the order of the `ENV_MENU_*` indices. Locked, Rename is a
// disabled entry whose content names why, since a kit tooltip has no test hook of its own.
fn env_menu(menu: PopupMenu, editor: Entity<EnvEditor>, locked: bool) -> PopupMenu {
    let rename = if locked {
        PopupMenuItem::element(|_, cx| locked_rename_label(cx)).disabled(true)
    } else {
        let editor = editor.clone();
        PopupMenuItem::new("Rename").on_click(move |_, window, cx| {
            editor.update(cx, |e, cx| e.start_naming(Naming::Rename, window, cx))
        })
    };
    let export = editor.clone();
    let export_all = editor.clone();
    let delete = PopupMenuItem::element(|_, cx| {
        div()
            .id("env-menu-delete")
            .test_support()
            .aria_label("Delete")
            .text_color(cx.theme().danger)
            .child("Delete")
    })
    .on_click(move |_, window, cx| editor.update(cx, |e, cx| e.delete(window, cx)));
    menu.item(rename)
        .item(
            PopupMenuItem::new("Export")
                .on_click(move |_, window, cx| export.update(cx, |e, cx| e.export(window, cx))),
        )
        .item(
            PopupMenuItem::new("Export all environments…").on_click(move |_, window, cx| {
                export_all.update(cx, |e, cx| e.export_all(window, cx))
            }),
        )
        .separator()
        .item(delete)
}

impl EnvEditor {
    /// The selected row's mark: the accent while the editor root or the list holds focus,
    /// muted while focus is in the pane's inputs and buttons or outside the editor.
    pub fn mark_color(&self, window: &Window, cx: &App) -> Hsla {
        let in_list = self.focus.is_focused(window) || self.list_focus.contains_focused(window, cx);
        crate::sidebar::selection_edge_color(in_list, cx)
    }
}

// The selected environment's value of the field `edit` writes, as an edit; `None` for a
// kind that is not undoable.
fn view_edit(view: &EnvView, edit: &Edit) -> Option<Edit> {
    Some(match edit {
        Edit::Variables(_) => Edit::Variables(view.variables.clone()),
        Edit::Color(_) => Edit::Color(view.color.clone()),
        Edit::Extends(_) => Edit::Extends(view.extends.clone()),
        _ => return None,
    })
}

impl Focusable for EnvEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for EnvEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let names = self.names(cx);
        // Rows read like the sidebar's: the selected one gets the active fill and an edge mark
        // (accent while the list has focus, as in the tree); the others reserve the mark's width
        // so labels never shift. Each row is a tab stop, so Enter or Space selects it.
        let theme = cx.theme();
        let (active, hover) = (theme.list_active, theme.list_hover);
        let mark_color = self.mark_color(window, cx);
        let mut naming = self.naming_field(window, cx);
        let renaming = self.naming == Some(Naming::Rename);
        let mut items = Vec::with_capacity(names.len() + 1);
        for (ix, name) in names.iter().enumerate() {
            let chosen = self.selected.as_deref() == Some(name.as_str());
            // Rename edits the name in its row's place, as the sidebar's rename does.
            if chosen
                && renaming
                && let Some(field) = naming.take()
            {
                items.push(field);
                continue;
            }
            let mark = div()
                .w(px(2.))
                .h_full()
                .flex_shrink_0()
                .rounded_l(style::RADIUS);
            let mark = if chosen {
                mark.id(("env-item-mark", ix))
                    .test_support()
                    .bg(mark_color)
                    .into_any_element()
            } else {
                mark.into_any_element()
            };
            let focus = window
                .use_keyed_state(("env-item", ix), cx, |_, cx| {
                    cx.focus_handle().tab_stop(true)
                })
                .read(cx)
                .clone();
            let focused = focus.is_focused(window);
            let name = name.clone();
            let item = h_flex()
                .id(("env-item", ix))
                .test_support()
                .track_focus(&focus)
                .w_full()
                .h(style::ROW_HEIGHT)
                .rounded(style::RADIUS)
                .cursor_pointer()
                .when(chosen, |row| row.bg(active))
                .when(!chosen, |row| row.hover(move |s| s.bg(hover)))
                .when(focused, |row| row.focus_ring_style(window, cx))
                .child(mark)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .pl(style::SPACE_2)
                        .truncate()
                        .child(name.clone()),
                )
                .on_click(cx.listener(move |e, _, window, cx| e.choose(&name, window, cx)));
            items.push(
                Followed::new(("env-item-follow", ix), &self.follow, item).into_any_element(),
            );
        }
        // A new environment's name is typed in a new entry at the list's end.
        items.extend(naming);
        // The list gives up width only as far as the pane's minimum content needs it.
        let list = v_flex()
            .id("env-list")
            .test_support()
            .track_focus(&self.list_focus)
            .w(style::ENV_EDITOR_LIST_WIDTH)
            .min_w(style::ENV_EDITOR_LIST_MIN)
            .flex_shrink(1.)
            .gap_1()
            .children(items)
            .child(Followed::new(
                "env-create-follow",
                &self.follow,
                Button::new("env-create").label("New").on_click(
                    cx.listener(|e, _, window, cx| e.start_naming(Naming::Create, window, cx)),
                ),
            ));
        // The editor scrolls as one page, list included, so a short or narrow window still
        // reaches the variable table.
        let root = h_flex()
            .id("env-editor")
            .test_support()
            .size_full()
            .overflow_scroll()
            .track_scroll(&self.scroll)
            .p(PANE_INSET)
            .pl(style::tab_content_inset(window))
            .pt(style::tab_content_inset(window))
            .pr(style::SCROLLBAR_GUTTER)
            .gap(style::SPACE_2)
            .items_start()
            .key_context("EnvEditor")
            .track_focus(&self.focus)
            .on_action(cx.listener(|e, _: &SaveEnvironment, window, cx| e.save_asking(window, cx)))
            .on_action(cx.listener(|e, _: &UndoEdit, window, cx| e.undo_edit(window, cx)))
            .on_action(cx.listener(|e, _: &RedoEdit, window, cx| e.redo_edit(window, cx)))
            .child(list)
            .child(
                div()
                    .id("env-divider")
                    .test_support()
                    .w(px(1.))
                    .flex_shrink_0()
                    .self_stretch()
                    .bg(cx.theme().border),
            );
        let notice = self.notice.map(|text| {
            div()
                .id("env-notice")
                .test_support()
                .aria_label(text)
                .w_full()
                .min_w_0()
                .text_color(cx.theme().muted_foreground)
                .child(text)
        });
        let field_error = self.field_error().is_some();
        let error = self.error().filter(|_| !field_error).map(|message| {
            // Bounded to the pane so a long path wraps instead of widening it.
            div()
                .id("env-error")
                .test_support()
                .text_color(cx.theme().danger)
                .w_full()
                .min_w_0()
                .child(message)
        });
        let editor = cx.entity().downgrade();
        let on_remove = move |env: &str, name: &str, window: &mut Window, cx: &mut App| {
            editor
                .update(cx, |e, cx| e.ask_remove_fileless(env, name, window, cx))
                .ok();
        };
        let fileless =
            orphan_section::fileless_section(&self.fileless, &self.follow, on_remove, cx);
        if self.selected.is_none() {
            // The label and its action hug their content as one group, centred in the pane.
            let empty = v_flex()
                .id("env-empty")
                .test_support()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .id("env-empty-label")
                        .test_support()
                        .text_color(cx.theme().muted_foreground)
                        .child("No environments yet."),
                )
                .child(
                    Button::new("env-empty-create")
                        .label("New environment")
                        .on_click(cx.listener(|e, _, window, cx| {
                            e.start_naming(Naming::Create, window, cx)
                        })),
                );
            return self.scrolled(
                root.child(
                    v_flex()
                        .id("env-empty-pane")
                        .test_support()
                        .flex_1()
                        .size_full()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .child(empty)
                        .children(fileless.map(|f| div().w_full().child(f)))
                        .children(error),
                )
                // The list's controls report even with nothing selected, so the watch stays.
                .child(self.follow.watch(&self.scroll)),
            );
        }
        let locked = self.locked;
        let header = self.header(locked, cx);
        let swatches = self.swatches(cx);
        let this = cx.entity();
        let shown = self
            .conflicts(cx)
            .map_or_else(Vec::new, |(_, shown, _)| shown);
        // Every disk banner offers a Revert, which restores no secret value already stored.
        let on_disk = shown.iter().any(|b| !matches!(b, Banner::HeldEdits(_)));
        let banners: Vec<_> = shown
            .into_iter()
            .map(|b| {
                let this = this.clone();
                banner::banner_row(b, move |b, choice, window, cx| {
                    this.update(cx, |e, cx| match choice {
                        Choice::Tertiary => e.open_compare(window, cx),
                        _ => e.answer_asking(b, choice, window, cx),
                    })
                })
            })
            .collect();
        let has_secret = self
            .view(cx)
            .is_some_and(|v| v.variables.iter().any(|var| var.secret));
        let secrets_kept = (on_disk && has_secret).then(|| {
            div()
                .id("env-secrets-kept")
                .test_support()
                .aria_label(SECRETS_KEPT)
                .text_color(cx.theme().muted_foreground)
                .child(SECRETS_KEPT)
        });
        let orphans = self.orphans.without_drafts(&self.draft_variables(cx));
        let editor = cx.entity().downgrade();
        let on_remove = move |name: &str, window: &mut Window, cx: &mut App| {
            editor
                .update(cx, |e, cx| e.ask_remove_orphan(name, window, cx))
                .ok();
        };
        let orphans = orphan_section::section(&orphans, &self.follow, on_remove, cx);
        self.scrolled(
            root.child(
                v_flex()
                    .id("env-pane")
                    .test_support()
                    .flex_1()
                    .min_w(style::ENV_PANE_MIN)
                    .gap_2()
                    .children(banners)
                    .children(secrets_kept)
                    .child(header)
                    .child(ui::form_row("env-color-row", "Color", swatches))
                    .child(ui::form_row(
                        "env-extends-row",
                        "Extends",
                        div()
                            .w(style::ENV_CONTROL_WIDTH)
                            .max_w_full()
                            .child(Followed::new(
                                "env-extends-follow",
                                &self.follow,
                                Select::new(&self.extends).id("env-extends"),
                            )),
                    ))
                    .child(self.table.clone())
                    .children(orphans)
                    .children(fileless)
                    .children(notice)
                    .children(error),
            )
            .child(self.follow.watch(&self.scroll)),
        )
    }
}

/// Why a new or renamed environment cannot be called `name`: one of `names` already has it.
pub(crate) fn name_taken(name: &str, names: &[String]) -> Option<String> {
    (names.iter().any(|n| n == name))
        .then(|| format!("An environment named `{name}` already exists"))
}

/// Why New refuses `name` among `names`, as Rename would: core would make a taken name a copy
/// and fit an invalid one to a file name, so both are refused instead of changed unseen.
pub(crate) fn create_refusal(name: &str, names: &[String]) -> Option<String> {
    let invalid = || apinata_core::EnvError::InvalidName {
        name: name.to_owned(),
    };
    match apinata_core::check_environment_name(name) {
        Err(err) => Some(err.to_string()),
        Ok(fitted) if fitted != name => Some(invalid().to_string()),
        Ok(_) => name_taken(name, names),
    }
}

/// The name field's Cancel and confirm buttons, confirm reading Create for New and Rename
/// otherwise; they wrap under each other when the list is too narrow for both.
pub(crate) fn naming_buttons(
    (cancel_id, confirm_id): (&'static str, &'static str),
    creating: bool,
    on_cancel: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_confirm: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    h_flex()
        .flex_wrap()
        .justify_end()
        .gap_1()
        .child(
            Button::new(cancel_id)
                .label("Cancel")
                .small()
                .on_click(on_cancel),
        )
        .child(
            Button::new(confirm_id)
                .label(if creating { "Create" } else { "Rename" })
                .small()
                .primary()
                .on_click(on_confirm),
        )
}

fn writes_draft(command: &Command) -> bool {
    matches!(command, Command::Save(_) | Command::Overwrite(_))
}

impl EnvEditor {
    // The name field with its label above it, so New and Rename read differently, and a
    // refusal under it that wraps in the list's width; `None` while no name is being collected.
    fn naming_field(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let label = self.naming_label()?;
        let refusal = self.field_error();
        let field = v_flex()
            .w_full()
            .gap_1()
            // The input lets an idle Escape propagate, so the wrapper dismisses the prompt.
            .on_action(cx.listener(|e, _: &Escape, window, cx| e.cancel_naming(window, cx)))
            .child(
                div()
                    .id("env-name-label")
                    .test_support()
                    .aria_label(label.clone())
                    .text_size(style::TEXT_SMALL)
                    .text_color(cx.theme().muted_foreground)
                    .truncate()
                    .child(label),
            )
            .child(ui::invalid_input(
                Input::new(&self.name_input)
                    .id("env-name-input")
                    .w_full()
                    .bg(ui::field_fill(false, cx)),
                // The frame hugs the input, so the danger ring covers it and no more.
                div().w_full(),
                &self.name_input,
                refusal.is_some(),
                (
                    "env-name-danger-ring".into(),
                    "env-name-invalid-icon".into(),
                ),
                window,
                cx,
            ))
            .when_some(refusal, |d, message| {
                d.child(ui::field_refusal_in_column(
                    "env-name-error",
                    message.into(),
                    cx,
                ))
            })
            .child(naming_buttons(
                ("env-name-cancel", "env-name-confirm"),
                self.naming == Some(Naming::Create),
                cx.listener(|e, _, window, cx| e.cancel_naming(window, cx)),
                cx.listener(|e, _, window, cx| {
                    let text = e.name_input.read(cx).value().trim().to_string();
                    e.commit_name(&text, window, cx);
                }),
            ));
        Some(field.into_any_element())
    }

    /// Wraps the editor's scrolling box so the kit scrollbar overlays its right edge, with the
    /// sideways bar in a row of its own under it, so the page's bottom stays clear of it.
    fn scrolled(&self, scrolling: impl IntoElement) -> Div {
        let page = div()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(scrolling)
            .children(ui::scroll_bar_y("env-editor-scrollbar", &self.scroll));
        v_flex()
            .size_full()
            .child(page)
            .children(ui::scroll_bar_x("env-editor-scrollbar-x", &self.scroll))
    }
}
