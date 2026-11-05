//! The global environment editor: a dialog listing the global environments, with the
//! selected one's variables in the environment table.
//!
//! Every change is a [`GlobalOp`] the workspace applies off the main thread; the dialog only
//! reads the workspace's published snapshot and its last error.

use apinata_core::EnvVariable;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::banner::{self, Banner, Choice};
use crate::dialog_guard::{self, Guard, GuardRef, SaveOutcome};
use crate::env_table::{
    DUPLICATE_SECRET_NAMES, EnvTableEvent, EnvVarTable, env_rows, env_vars_from, secret_records,
    secrets_collide,
};
use crate::import_report::failure_text;
use crate::orphan_section::{self, Orphans};
use crate::scroll_follow::{FocusFollow, Followed};
use crate::secret_lock::{self, LockStep};
use crate::style;
use crate::ui;
use crate::workspace::{GlobalOp, Workspace};

// Which name the name input is collecting.
#[derive(Clone, Copy, PartialEq)]
enum Naming {
    Create,
    Rename,
}

/// Edits the workspace's global environments; the table follows the published snapshot
/// whenever the selected environment's variables there change.
pub struct GlobalEnvEditor {
    workspace: Entity<Workspace>,
    selected: Option<String>,
    table: Entity<EnvVarTable>,
    base: Vec<EnvVariable>,
    // Whether the watcher brought other variables under plain typing, which the banner shows.
    disk_changed: bool,
    // Secret values the table committed on blur, stored with the next Save.
    secrets: Vec<(String, String)>,
    // Why the last reveal failed; the workspace holds edit failures.
    error: Option<String>,
    orphans: Orphans,
    // Set while a lock or Save check is reading the store or asking; a further one is dropped,
    // and a blur stores nothing, since a dialog taking focus must not commit the typing.
    checking: bool,
    name_input: Entity<InputState>,
    naming: Option<Naming>,
    // The rename in flight as (from, to): the snapshot may publish it before its outcome.
    renaming: Option<(String, String)>,
    // Whether `refresh` moved the selection to the rename's new name and the user has not
    // selected since, so that selection is the rename's own.
    followed: bool,
    // Counts openings of the name field, so an outcome closes only the field it was sent from.
    naming_gen: u64,
    // Whether the name field holds a name the last rename refused.
    refused: bool,
    // Takes focus when the name input goes away, so the dialog keeps its Escape.
    focus: FocusHandle,
    // The dialog body's scroll position and the watch that keeps the focused row in it.
    scroll: ScrollHandle,
    follow: FocusFollow,
    // The dialog's close guard, which a switch to another environment asks through too.
    guard: Option<GuardRef>,
    _subs: Vec<Subscription>,
}

impl GlobalEnvEditor {
    /// An editor over `workspace`'s global environments, with the active one selected, else
    /// the first.
    pub fn new(workspace: Entity<Workspace>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let follow = FocusFollow::default();
        let table = cx.new(|cx| {
            let mut table = EnvVarTable::new(window, cx);
            table.set_follow(follow.clone());
            table
        });
        let name_input = cx.new(|cx| InputState::new(window, cx).placeholder("Environment name"));
        let subs = vec![
            cx.observe_in(&workspace, window, |this, _, window, cx| {
                this.refresh(window, cx)
            }),
            cx.subscribe_in(
                &table,
                window,
                |this, _, event: &EnvTableEvent, window, cx| this.table_event(event, window, cx),
            ),
            cx.subscribe_in(
                &name_input,
                window,
                |this, state, event: &InputEvent, window, cx| match event {
                    InputEvent::PressEnter { .. } => {
                        let text = state.read(cx).value().trim().to_string();
                        this.commit_name(&text, window, cx);
                    }
                    // Editing the name answers the error it drew.
                    InputEvent::Change if this.naming.is_some() => this.clear_error(cx),
                    _ => {}
                },
            ),
        ];
        let mut editor = GlobalEnvEditor {
            workspace,
            selected: None,
            table,
            base: Vec::new(),
            disk_changed: false,
            secrets: Vec::new(),
            error: None,
            orphans: Orphans::default(),
            checking: false,
            name_input,
            naming: None,
            renaming: None,
            followed: false,
            naming_gen: 0,
            refused: false,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            follow,
            guard: None,
            _subs: subs,
        };
        editor.refresh(window, cx);
        editor.list_orphans(cx);
        editor
    }

    /// The selected global environment.
    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// Selects `name` and shows its variables as the snapshot has them; secret typing not
    /// yet saved is dropped.
    pub fn select(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.as_deref() != Some(name) {
            self.orphans = Orphans::Unknown;
        }
        self.selected = Some(name.to_owned());
        self.followed = false;
        self.secrets.clear();
        self.naming = None;
        // Empty first, so no secret row's text carries over by name into another environment.
        self.table
            .update(cx, |t, cx| t.load(Vec::new(), window, cx));
        self.load(true, window, cx);
        cx.notify();
    }

    /// A person's selection of `name`: [`Self::select`], then a list of its orphans. Paths the
    /// editor takes by itself use `select`, so nothing unattended reaches the keychain.
    pub fn choose(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_error(cx);
        self.select(name, window, cx);
        self.list_orphans(cx);
    }

    /// Lists the selected environment's orphans off the main thread; an answer for an
    /// environment no longer selected is dropped.
    pub fn list_orphans(&mut self, cx: &mut Context<Self>) {
        let Some(env) = self.selected.clone() else {
            return;
        };
        let listing = self.workspace.read(cx).orphaned_global_secrets(&env, cx);
        cx.spawn(async move |this, cx| {
            let answer = listing.await;
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
    // may be about to declare it. A failure is in the workspace's error, which the dialog shows.
    fn remove_orphan(&mut self, env: String, name: String, cx: &mut Context<Self>) {
        if self.selected.as_deref() != Some(env.as_str()) {
            return;
        }
        let names = orphan_section::visible(&[name], &self.draft_variables(cx));
        if names.is_empty() {
            return;
        }
        let outcome = self.send(GlobalOp::RemoveOrphans { name: env, names }, cx);
        self.list_after(outcome, cx);
    }

    // Lists again once `outcome` reports the operation landed.
    fn list_after(
        &mut self,
        outcome: async_channel::Receiver<Result<(), String>>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(())) = outcome.recv().await {
                this.update(cx, |e, cx| e.list_orphans(cx)).ok();
            }
        })
        .detach();
    }

    // The rows were listed under the old name, where a rename leaves its orphans.
    fn follow_rename(&mut self, to: String) {
        self.selected = Some(to);
        self.orphans = Orphans::Unknown;
    }

    fn draft_variables(&self, cx: &App) -> Vec<EnvVariable> {
        env_vars_from(&self.base, &self.table.read(cx).rows(cx))
    }

    /// Whether the selected environment has edits Save has not stored: changed rows, committed
    /// secret values, or secret typing not yet sent.
    pub fn has_edits(&self, cx: &App) -> bool {
        let table = self.table.read(cx);
        self.selected.is_some()
            && (table.rows(cx) != env_rows(&self.base)
                || !self.secrets.is_empty()
                || table.has_secret_typing(cx))
    }

    // Leaving the selected environment (another one, New, Duplicate) with edits pending asks
    // first, as closing does; Cancel stays where it is.
    fn after_asking(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        act: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let Some(guard) = self.guard.clone() else {
            return act(self, window, cx);
        };
        if guard.busy() {
            return;
        }
        if !self.has_edits(cx) {
            return act(self, window, cx);
        }
        // The prompt reads this editor, so it is asked for once this update has ended.
        let this = cx.entity().downgrade();
        window.defer(cx, move |window, cx| {
            let then = move |window: &mut Window, cx: &mut App| {
                this.update(cx, |e, cx| act(e, window, cx)).ok();
            };
            guard.ask_then(window, cx, Box::new(then));
        });
    }

    /// The selected environment's variables table.
    pub fn table(&self) -> &Entity<EnvVarTable> {
        &self.table
    }

    /// Creates the global environment `name` and selects it once it is listed, starting
    /// empty: nothing typed for the previous one, secrets included, carries over.
    pub fn create(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = Some(name.to_owned());
        self.orphans = Orphans::Unknown;
        self.secrets.clear();
        self.base.clear();
        self.table
            .update(cx, |t, cx| t.load(Vec::new(), window, cx));
        let outcome = self.send(GlobalOp::Create(name.to_owned()), cx);
        self.list_after(outcome, cx);
    }

    /// Copies the selected environment as saved and selects the copy once it is listed. The
    /// copy's name is the one the snapshot gains, since core picks it.
    pub fn duplicate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(from) = self.selected.clone() else {
            return;
        };
        let before = self.names(cx);
        let outcome = self.send(GlobalOp::Duplicate(from), cx);
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(())) = outcome.recv().await {
                this.update_in(cx, |e, window, cx| {
                    let added = e.names(cx).into_iter().find(|n| !before.contains(n));
                    if let Some(name) = added {
                        e.choose(&name, window, cx);
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    /// Renames the selected environment to `to`, as given; the core refuses an invalid name.
    /// The selection moves and the name field closes only once the rename lands; a refusal
    /// keeps the source selected and the field open on the typed name, marked invalid.
    pub fn rename(&mut self, to: &str, window: &mut Window, cx: &mut Context<Self>) {
        // One rename at a time: a second Enter would rename a source already gone.
        if self.renaming.is_some() {
            return;
        }
        let Some(from) = self.selected.clone() else {
            return;
        };
        let to = to.to_owned();
        self.renaming = Some((from.clone(), to.clone()));
        self.followed = false;
        self.refused = false;
        let field = self.naming_gen;
        let op = GlobalOp::Rename {
            from: from.clone(),
            to: to.clone(),
        };
        let outcome = self.send(op, cx);
        cx.spawn_in(window, async move |this, cx| {
            let result = outcome.recv().await.unwrap_or_else(|e| Err(e.to_string()));
            this.update_in(cx, |e, window, cx| {
                e.renaming = None;
                let followed = std::mem::take(&mut e.followed);
                match result {
                    // A selection or field the user moved to or opened since is theirs, so only
                    // the source's (which `refresh` may already show as `to`) closes.
                    Ok(()) => {
                        if e.selected.as_ref() == Some(&from)
                            || (followed && e.selected.as_ref() == Some(&to))
                        {
                            e.follow_rename(to);
                            if e.naming == Some(Naming::Rename) && e.naming_gen == field {
                                e.naming = None;
                                e.focus.focus(window, cx);
                            }
                        }
                        e.load(false, window, cx);
                    }
                    // Only the field that sent the rename is marked, never one reopened since.
                    Err(_) => {
                        let held = e.name_input.read(cx).value().trim() == to;
                        e.refused = e.selected.as_ref() == Some(&from)
                            && e.naming == Some(Naming::Rename)
                            && e.naming_gen == field
                            && held;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Deletes the selected environment.
    pub fn delete(&mut self, cx: &mut Context<Self>) {
        if let Some(name) = self.selected.clone() {
            self.send(GlobalOp::Delete(name), cx);
        }
    }

    /// Send `GlobalOp::Save` for the selected environment with the table's rows and its
    /// pending secret typing (`take_pending_secrets(true, ..)`); the outcome says when it is
    /// stored or why not.
    pub fn save(&mut self, cx: &mut Context<Self>) -> SaveOutcome {
        let Some(name) = self.selected.clone() else {
            return dialog_guard::settled(Ok(()));
        };
        // Refused before any typing is taken: two secrets of one name would share one value.
        let table = self.table.read(cx);
        let draft = table.secret_records(cx);
        let changed = draft != secret_records(&self.base) || table.has_secret_typing(cx);
        if secrets_collide(&draft) && changed {
            self.error = Some(DUPLICATE_SECRET_NAMES.to_owned());
            cx.notify();
            return dialog_guard::settled(Err(DUPLICATE_SECRET_NAMES.to_owned()));
        }
        let (rows, typed) = self.table.update(cx, |t, cx| {
            let typed = t.take_pending_secrets(true, cx);
            (t.rows(cx), typed)
        });
        let mut secrets = std::mem::take(&mut self.secrets);
        secrets.extend(typed);
        let variables = env_vars_from(&self.base, &rows);
        let kept = secrets.clone();
        let outcome = self.send(
            GlobalOp::Save {
                name: name.clone(),
                variables,
                secrets,
            },
            cx,
        );
        // A failed save stored nothing, so its secrets go back ahead of any newer typing,
        // unless another environment was selected meanwhile.
        let (tx, rx) = async_channel::bounded(1);
        cx.spawn(async move |this, cx| {
            let result = outcome.recv().await.unwrap_or_else(|e| Err(e.to_string()));
            this.update(cx, |e, cx| {
                if e.selected.as_deref() != Some(name.as_str()) {
                    return;
                }
                if result.is_ok() {
                    e.list_orphans(cx);
                } else {
                    let newer = std::mem::replace(&mut e.secrets, kept);
                    e.secrets.extend(newer);
                }
            })
            .ok();
            let _ = tx.try_send(result);
        })
        .detach();
        rx
    }

    /// Shows the stored value of the selected environment's secret `variable` unmasked,
    /// read off the main thread; it is shown only if the same environment is still selected.
    pub fn reveal(&mut self, variable: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.selected.clone() else {
            return;
        };
        let read = self
            .workspace
            .read(cx)
            .reveal_global_secret(&name, variable, cx);
        let variable = variable.to_owned();
        cx.spawn_in(window, async move |this, cx| {
            let answer = read.await;
            this.update_in(cx, |e, window, cx| {
                if e.selected.as_deref() != Some(name.as_str()) {
                    return;
                }
                match answer {
                    Ok(value) => e
                        .table
                        .update(cx, |t, cx| t.show_secret(&variable, value, window, cx)),
                    Err(message) => e.error = Some(message),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn set_checking(&mut self, checking: bool, cx: &mut Context<Self>) {
        self.checking = checking;
        self.table.update(cx, |t, _| t.set_blur_held(checking));
    }

    // Asks the store what a lock click would lose, and only then toggles. Nothing is saved or
    // sent before the answer, so a Cancel leaves the store, the file and the draft as they were.
    fn check_lock(
        &mut self,
        variable: String,
        secret: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(env) = self.selected.clone().filter(|_| !self.checking) else {
            return;
        };
        self.set_checking(true, cx);
        let read = self
            .workspace
            .read(cx)
            .reveal_global_secret(&env, &variable, cx);
        cx.spawn_in(window, async move |this, cx| {
            let read = read.await;
            let step = this.update_in(cx, |e, window, cx| {
                e.lock_step(&env, &variable, secret, read, window, cx)
            });
            let go = step.unwrap_or(LockStep::Stop).confirmed().await;
            this.update_in(cx, |e, window, cx| {
                e.set_checking(false, cx);
                if go {
                    e.toggle_lock(env, variable, secret, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn lock_step(
        &mut self,
        env: &str,
        variable: &str,
        secret: bool,
        read: Result<Option<String>, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> LockStep {
        if self.selected.as_deref() != Some(env) {
            return LockStep::Stop;
        }
        let stored = match read {
            Ok(stored) => {
                self.clear_read_error(cx);
                stored
            }
            Err(message) => {
                self.error = Some(message);
                cx.notify();
                return LockStep::Stop;
            }
        };
        let rows = self.table.read(cx).rows(cx);
        let Some(row) = rows.iter().find(|row| row.name == variable) else {
            return LockStep::Stop;
        };
        match secret_lock::question(secret, &row.value, stored.as_deref()) {
            Some(question) => {
                let spec = secret_lock::lock_ask(question, variable);
                LockStep::Ask(crate::confirm::ask(spec, window, cx))
            }
            None => LockStep::Go,
        }
    }

    // A toggle first saves the table as it stands, so the reload after it keeps other rows'
    // typing and a row not yet in the file exists. The toggled variable's typing is stored with
    // that Save, so the toggle moves it, as the collection editor does.
    fn toggle_lock(
        &mut self,
        env: String,
        variable: String,
        secret: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut typing = self.table.read(cx).unsaved_secret_typing(cx);
        typing.retain(|(name, _)| *name != variable);
        self.after_replace_check(typing, window, cx, move |e, cx| {
            if e.selected.as_deref() != Some(env.as_str()) {
                return;
            }
            let typed = e.table.update(cx, |t, cx| {
                t.confirm_typing(&variable, cx);
                t.take_typing(&variable, cx)
            });
            // The newest typing replaces any the variable committed earlier.
            if let Some(text) = typed {
                e.secrets.retain(|(v, _)| *v != variable);
                e.secrets.push((variable.clone(), text));
            }
            let _ = e.save(cx);
            let outcome = e.send(
                GlobalOp::SetSecret {
                    name: env,
                    variable,
                    secret,
                },
                cx,
            );
            e.list_after(outcome, cx);
        });
    }

    // Runs `then` once none of `typing`, what the action would store, overwrites a stored value
    // unasked: at once when there is none, so a plain Save never reads the store. A name is
    // read as the lock reads, and a failed read lands on the error line.
    fn after_replace_check(
        &mut self,
        typing: Vec<(String, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
    ) {
        if self.checking {
            return;
        }
        let Some(env) = self.selected.clone().filter(|_| !typing.is_empty()) else {
            return then(self, cx);
        };
        self.set_checking(true, cx);
        let checked = typing.clone();
        let workspace = self.workspace.read(cx);
        let reads: Vec<_> = typing
            .iter()
            .map(|(variable, _)| workspace.reveal_global_secret(&env, variable, cx))
            .collect();
        let check = cx.spawn(async move |this, cx| {
            let mut stored = Vec::new();
            for read in reads {
                match read.await {
                    Ok(value) => stored.push(value),
                    Err(message) => {
                        this.update(cx, |e, cx| {
                            e.error = Some(message);
                            cx.notify();
                        })
                        .ok();
                        return None;
                    }
                }
            }
            this.update(cx, |e, cx| e.clear_read_error(cx)).ok();
            Some(secret_lock::replaced(&typing, &stored))
        });
        cx.spawn_in(window, async move |this, cx| {
            let go = secret_lock::replace_agreed(vec![(String::new(), check)], cx).await;
            this.update(cx, |e, cx| {
                e.set_checking(false, cx);
                if go {
                    e.table.update(cx, |t, cx| t.confirm_checked(&checked, cx));
                    then(e, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// [`Self::save`], after asking whether a new secret row may replace a stored value. The
    /// outcome's channel closes unanswered when nothing was saved.
    pub fn save_asking(&mut self, window: &mut Window, cx: &mut Context<Self>) -> SaveOutcome {
        let (tx, rx) = async_channel::bounded(1);
        let typing = self.table.read(cx).unsaved_secret_typing(cx);
        self.after_replace_check(typing, window, cx, move |e, cx| {
            let outcome = e.save(cx);
            cx.spawn(async move |_, _| {
                let result = outcome.recv().await.unwrap_or_else(|e| Err(e.to_string()));
                let _ = tx.try_send(result);
            })
            .detach();
        });
        rx
    }

    // Committed secret typing waits for Save.
    fn table_event(&mut self, event: &EnvTableEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            EnvTableEvent::SecretValue { name, value } => {
                self.secrets.push((name.clone(), value.clone()))
            }
            EnvTableEvent::SetSecret { name, secret } => {
                self.check_lock(name.clone(), *secret, window, cx)
            }
            EnvTableEvent::Reveal(variable) => self.reveal(&variable.clone(), window, cx),
            EnvTableEvent::TypingHeld => {
                let typing = self.table.read(cx).held_secret_typing(cx);
                self.after_replace_check(typing, window, cx, |e, cx| {
                    let typed = e
                        .table
                        .update(cx, |t, cx| t.take_pending_secrets(false, cx));
                    e.secrets.extend(typed);
                    cx.notify();
                })
            }
            EnvTableEvent::Changed(_) => {}
        }
    }

    // Deleting removes the file and its stored secrets, so it asks first.
    fn ask_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.selected.clone() else {
            return;
        };
        // A parent others extend is not offered for deletion; core refuses it anyway.
        let by = self
            .workspace
            .read(cx)
            .globals()
            .map_or_else(Vec::new, |g| g.extended_by(&name));
        if !by.is_empty() {
            let spec = crate::confirm::extended_ask("Global environment", &name, &by);
            crate::confirm::ask(spec, window, cx);
            return;
        }
        let title = format!("Delete global environment \"{name}\"?");
        let answer = crate::confirm::ask(crate::confirm::delete_ask(title, None), window, cx);
        cx.spawn(async move |this, cx| {
            if answer.recv().await == Ok(1) {
                this.update(cx, |e, cx| e.delete(cx)).ok();
            }
        })
        .detach();
    }

    fn send(
        &mut self,
        op: GlobalOp,
        cx: &mut Context<Self>,
    ) -> async_channel::Receiver<Result<(), String>> {
        // A new command's outcome is not the rename's refusal, so the field is unmarked.
        self.refused = false;
        self.error = None;
        let outcome = self.workspace.update(cx, |w, cx| w.edit_globals(op, cx));
        cx.notify();
        outcome
    }

    // A read that answers ends an earlier failed read's error; the duplicate-names refusal is
    // the only other error this editor holds, and it stays while the names still collide.
    fn clear_read_error(&mut self, cx: &mut Context<Self>) {
        if self.error.as_deref() != Some(DUPLICATE_SECRET_NAMES) {
            self.error = None;
            cx.notify();
        }
    }

    // Drops both error sources, the workspace's and this editor's own.
    fn clear_error(&mut self, cx: &mut Context<Self>) {
        self.refused = false;
        self.error = None;
        self.workspace.update(cx, |w, cx| w.clear_global_error(cx));
        cx.notify();
    }

    fn names(&self, cx: &App) -> Vec<String> {
        self.workspace.read(cx).global_environments()
    }

    // A selection gone from the snapshot (deleted, or a rename or create that failed) falls
    // back to the active global environment, else the first; while edits are pending, a
    // created or renamed name may simply not be published yet.
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let names = self.names(cx);
        // A rename published ahead of its outcome follows the new name, never a fallback.
        if let Some((from, to)) = &self.renaming
            && self.selected.as_ref() == Some(from)
            && !names.contains(from)
            && names.contains(to)
        {
            let to = to.clone();
            self.follow_rename(to);
            self.followed = true;
        }
        if self.selected.as_ref().is_some_and(|s| names.contains(s)) {
            self.load(false, window, cx);
            return;
        }
        if self.selected.is_some() && self.workspace.read(cx).global_edits_pending() {
            return;
        }
        let active = self.workspace.read(cx).selected_global_environment();
        match active
            .filter(|a| names.contains(a))
            .or(names.first().cloned())
        {
            Some(next) => self.select(&next, window, cx),
            None => {
                self.selected = None;
                self.orphans = Orphans::Unknown;
                self.base.clear();
                self.table
                    .update(cx, |t, cx| t.load(Vec::new(), window, cx));
            }
        }
    }

    // Reloads the table from the snapshot when forced or when the file's variables changed,
    // so an unrelated publish keeps what the user typed. Another program's change, which the
    // watcher brought, leaves plain typing alone under the changed-on-disk banner.
    fn load(&mut self, all: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.selected.as_deref() else {
            return;
        };
        let workspace = self.workspace.read(cx);
        let watched = workspace.globals_from_watch();
        let variables = (workspace.globals())
            .and_then(|g| g.environment(name).map(|e| e.variables()))
            .unwrap_or_default();
        let rows = env_rows(&variables);
        let base = env_rows(&self.base);
        // A file back at what was loaded has nothing to warn about, and the typing stays.
        if !all && rows == base {
            if std::mem::take(&mut self.disk_changed) {
                cx.notify();
            }
            return;
        }
        let typed = self.table.read(cx).rows(cx);
        if !all && watched && typed != base && typed != rows {
            self.disk_changed = true;
            cx.notify();
            return;
        }
        self.disk_changed = false;
        self.base = variables;
        self.table.update(cx, |t, cx| t.load(rows, window, cx));
    }

    // Reload takes the file's variables; Overwrite saves the typing over them.
    fn answer_disk(&mut self, choice: Choice, window: &mut Window, cx: &mut Context<Self>) {
        if !self.disk_changed {
            return;
        }
        match choice {
            Choice::Primary => self.load(true, window, cx),
            Choice::Secondary => {
                let name = self.selected.clone().unwrap_or_default();
                self.base = (self.workspace.read(cx).globals())
                    .and_then(|g| g.environment(&name).map(|e| e.variables()))
                    .unwrap_or_default();
                self.disk_changed = false;
                let _ = self.save_asking(window, cx);
            }
            Choice::Tertiary => {}
        }
        cx.notify();
    }

    fn start_naming(&mut self, naming: Naming, window: &mut Window, cx: &mut Context<Self>) {
        let text = match naming {
            Naming::Create => String::new(),
            Naming::Rename => self.selected.clone().unwrap_or_default(),
        };
        self.name_input
            .update(cx, |s, cx| s.set_value(text, window, cx));
        self.name_input.read(cx).focus_handle(cx).focus(window, cx);
        self.naming = Some(naming);
        self.naming_gen += 1;
        self.refused = false;
        cx.notify();
    }

    fn commit_name(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        // A rename keeps its field, focused, until the outcome says whether the name took.
        if self.naming == Some(Naming::Rename) {
            return self.rename(text, window, cx);
        }
        if let Some(Naming::Create) = self.naming.take()
            && !text.is_empty()
        {
            let name = text.to_owned();
            self.after_asking(window, cx, move |e, window, cx| e.create(&name, window, cx));
        }
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = cx.theme().list_active;
        let items = self.names(cx).into_iter().enumerate().map(|(ix, name)| {
            let chosen = self.selected.as_deref() == Some(name.as_str());
            h_flex()
                .id(("global-env-item", ix))
                .test_support()
                .w_full()
                .h(style::ROW_HEIGHT)
                .px(style::SPACE_2)
                .rounded(style::RADIUS)
                .cursor_pointer()
                .when(chosen, |row| row.bg(active))
                .child(div().truncate().child(name.clone()))
                .on_click(cx.listener(move |e, _, window, cx| {
                    let name = name.clone();
                    e.after_asking(window, cx, move |e, window, cx| e.choose(&name, window, cx))
                }))
        });
        // The list holds no input, so its New button takes Enter as a press.
        v_flex()
            .key_context(ui::DIALOG_FOOTER_CONTEXT)
            .w(style::ENV_LIST_WIDTH)
            .flex_none()
            .gap_1()
            .children(items)
            .child(Followed::new(
                "global-env-create-follow",
                &self.follow,
                Button::new("global-env-create")
                    .label("New")
                    .on_click(cx.listener(|e, _, window, cx| {
                        // Don't save drops the edits here, so the name commits without asking
                        // a second time.
                        e.after_asking(window, cx, |e, window, cx| {
                            if let Some(name) = e.selected.clone().filter(|_| e.has_edits(cx)) {
                                e.select(&name, window, cx);
                            }
                            e.start_naming(Naming::Create, window, cx)
                        })
                    })),
            ))
    }
}

impl Render for GlobalEnvEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let workspace = self.workspace.read(cx);
        let failed = workspace.global_error().is_some();
        let error = (workspace.global_error())
            .or(self.error.as_deref())
            .map(|message| {
                let line = ui::plain_quotes(&failure_text(message));
                div()
                    .id("global-env-error")
                    .test_support()
                    .aria_label(line.clone())
                    .text_color(cx.theme().danger)
                    .child(line)
            });
        let load_errors: Vec<String> = workspace.globals().map_or_else(Vec::new, |g| {
            g.load_errors()
                .iter()
                .map(|e| {
                    let file = e.path().file_name().unwrap_or_default().to_string_lossy();
                    format!("Could not load {file}: {}", e.message())
                })
                .collect()
        });
        let load_errors = load_errors.into_iter().enumerate().map(|(ix, line)| {
            div()
                .id(("global-env-load-error", ix))
                .test_support()
                .child(line)
        });
        let naming = self.naming.map(|naming| {
            let label = match naming {
                Naming::Create => "New global environment name".to_owned(),
                Naming::Rename => format!(
                    "Rename \"{}\" to",
                    self.selected.as_deref().unwrap_or_default()
                ),
            };
            let invalid = self.refused && failed && naming == Naming::Rename;
            v_flex()
                .gap_1()
                .on_action(cx.listener(|e, _: &Escape, window, cx| {
                    e.naming = None;
                    // The refused name's error goes with its field.
                    if e.refused {
                        e.clear_error(cx);
                    }
                    e.focus.focus(window, cx);
                    cx.notify();
                }))
                .child(label)
                .child(ui::invalid_input(
                    Input::new(&self.name_input)
                        .id("global-env-name-input")
                        .w(style::NAME_INPUT_WIDTH)
                        .bg(ui::field_fill(false, cx)),
                    // The frame hugs the input, so the danger ring covers it and no more.
                    div().w(style::NAME_INPUT_WIDTH).flex_none(),
                    &self.name_input,
                    invalid,
                    (
                        "global-env-name-danger-ring".into(),
                        "global-env-name-invalid-icon".into(),
                    ),
                    window,
                    cx,
                ))
        });
        let selected = self.selected.is_some();
        let this = cx.entity().downgrade();
        let disk_banner = (selected && self.disk_changed).then(|| {
            banner::plain_banner_row(Banner::ChangedOnDisk, move |_, choice, window, cx| {
                this.update(cx, |e, cx| e.answer_disk(choice, window, cx))
                    .ok();
            })
        });
        let orphans = self.orphans.without_drafts(&self.draft_variables(cx));
        let editor = cx.entity().downgrade();
        let on_remove = move |name: &str, window: &mut Window, cx: &mut App| {
            editor
                .update(cx, |e, cx| e.ask_remove_orphan(name, window, cx))
                .ok();
        };
        let orphans = orphan_section::section(&orphans, &self.follow, on_remove, cx);
        // Each header button reports to the follow, so Tab onto it scrolls it into view.
        let follow = self.follow.clone();
        let followed = move |id: &'static str, button: Button| Followed::new(id, &follow, button);
        // A focused header button takes Enter as its press, not the dialog's Confirm.
        let header = h_flex()
            .key_context(ui::DIALOG_FOOTER_CONTEXT)
            .gap_2()
            .child(followed(
                "global-env-rename-follow",
                Button::new("global-env-rename").label("Rename").on_click(
                    cx.listener(|e, _, window, cx| e.start_naming(Naming::Rename, window, cx)),
                ),
            ))
            .child(followed(
                "global-env-duplicate-follow",
                Button::new("global-env-duplicate")
                    .label("Duplicate")
                    .on_click(cx.listener(|e, _, window, cx| {
                        e.after_asking(window, cx, |e, window, cx| e.duplicate(window, cx))
                    })),
            ))
            .child(followed(
                "global-env-delete-follow",
                Button::new("global-env-delete")
                    .label("Delete")
                    .on_click(cx.listener(|e, _, window, cx| e.ask_delete(window, cx))),
            ))
            .child(followed(
                "global-env-save-follow",
                Button::new("global-env-save")
                    .label("Save")
                    .on_click(cx.listener(|e, _, window, cx| {
                        let _ = e.save_asking(window, cx);
                    })),
            ));
        let body = if selected {
            v_flex()
                .flex_1()
                .min_w(style::ENV_PANE_MIN)
                .gap_2()
                .children(disk_banner)
                .child(header)
                .children(naming)
                .child(self.table.clone())
                .children(orphans)
                .into_any_element()
        } else {
            // The message and its action hug their content as one group, centred in the pane
            // as the collection editor's empty state is.
            let empty = v_flex()
                .id("global-env-empty")
                .test_support()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .id("global-env-empty-label")
                        .test_support()
                        .text_color(cx.theme().muted_foreground)
                        .child("No global environments yet."),
                )
                .child(
                    Button::new("global-env-empty-create")
                        .label("New global environment")
                        .on_click(cx.listener(|e, _, window, cx| {
                            e.start_naming(Naming::Create, window, cx)
                        })),
                );
            v_flex()
                .id("global-env-empty-pane")
                .test_support()
                .flex_1()
                .min_w(style::ENV_PANE_MIN)
                .items_center()
                .justify_center()
                .gap_2()
                .children(disk_banner)
                .child(empty)
                .children(naming)
                .children(orphans.map(|o| div().w_full().child(o)))
                .into_any_element()
        };
        v_flex()
            .id("global-env-editor")
            .test_support()
            .track_focus(&self.focus)
            .w_full()
            .min_w(style::GLOBAL_ENV_CONTENT_MIN)
            .gap_2()
            .children(error)
            .children(load_errors)
            .child(
                // The row has a floor, and the empty pane stretches to it, so the message is
                // centred and the dialog keeps its height from the empty state to a small one.
                h_flex()
                    .gap_2()
                    .items_stretch()
                    .min_h(style::GLOBAL_ENV_BODY_MIN)
                    .child(self.render_list(cx))
                    .child(body),
            )
    }
}

/// Reload the directory and open the editor dialog (root id `global-env-editor`) over `window`.
pub fn open_global_env_editor(
    workspace: Entity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<GlobalEnvEditor> {
    // An error belongs to the dialog that showed it, so each opening starts clean.
    workspace.update(cx, |w, cx| {
        w.clear_global_error(cx);
        w.reload_globals(cx)
    });
    let editor = cx.new(|cx| GlobalEnvEditor::new(workspace, window, cx));
    let guard = dialog_guard::guarded(editor_guard(&editor), window, cx);
    editor.update(cx, |e, _| e.guard = Some(guard.downgrade()));
    let body = editor.clone();
    let closed = editor.downgrade();
    let opened = guard.clone();
    window.open_dialog(cx, move |d, window, cx| {
        let (scroll, follow) = {
            let editor = body.read(cx);
            (editor.scroll.clone(), editor.follow.clone())
        };
        // The body scrolls inside the room above the status bar, as the cookie jar's does; the
        // sideways bar shares that room, so the body gives up its height to it.
        let room = crate::dialog_frame::titled_body_room(window);
        // Two panes need more room than the default width, but never more than the window.
        let width = style::resolve(style::GLOBAL_ENV_DIALOG_WIDTH, window)
            .min(window.viewport_size().width * 0.9);
        let d = guard.apply(crate::dialog_frame::place(d, window));
        ui::with_close(d, "Global environments")
            .width(width)
            // A name input's Enter propagates to the dialog's `enter -> Confirm`; refusing it
            // keeps the dialog, and so the editor that commits the name, alive.
            .on_ok(|_, _, _| false)
            // Escape, the x and the backdrop end here once the guard lets the dialog go.
            .on_close({
                let closed = closed.clone();
                move |_, _, cx| {
                    closed.update(cx, |e, cx| e.clear_error(cx)).ok();
                }
            })
            .child(
                v_flex()
                    .max_h(room.max(px(0.)))
                    .child(
                        div()
                            .id("global-env-scroll")
                            .test_support()
                            .min_h_0()
                            .overflow_scroll()
                            .track_scroll(&scroll)
                            .child(body.clone())
                            .child(follow.watch(&scroll)),
                    )
                    .children(ui::scroll_bar_x("global-env-scrollbar-x", &scroll)),
            )
    });
    opened.opened(window, cx);
    editor
}

// Closing asks about edits Save has not stored; the prompt's Save is the editor's Save, and
// Don't save stores nothing, the editor going with the dialog.
fn editor_guard(editor: &Entity<GlobalEnvEditor>) -> Guard {
    let (dirty, title, save) = (editor.downgrade(), editor.downgrade(), editor.downgrade());
    let discard = editor.downgrade();
    Guard::new(
        move |cx| dirty.upgrade().is_some_and(|e| e.read(cx).has_edits(cx)),
        move |cx| {
            let name = title.upgrade().and_then(|e| e.read(cx).selected.clone());
            format!("Save changes to \"{}\"?", name.unwrap_or_default()).into()
        },
        move |window, cx| match save.upgrade() {
            Some(e) => e.update(cx, |e, cx| e.save_asking(window, cx)),
            None => dialog_guard::settled(Ok(())),
        },
        // Don't save always closes the dialog, so its error goes with it.
        move |_, cx| {
            if let Some(e) = discard.upgrade() {
                e.update(cx, |e, cx| e.clear_error(cx));
            }
        },
    )
}
