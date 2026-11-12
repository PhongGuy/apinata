//! The variables table of one environment, with masked secret rows.
//!
//! Plain rows edit the draft like `KvTable` does. A secret row's value never enters the draft:
//! it is typed into a masked input and handed to the owner as `SecretValue` on Enter or blur,
//! or, for a row not yet saved, only on Save through `take_pending_secrets`.

use apinata_core::{EnvVariable, VarValue};
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, App, AppContext, Context, DragMoveEvent, Entity, EventEmitter, FocusHandle,
    Focusable, InteractiveElement, IntoElement, ParentElement, Rems, Render, Styled, Subscription,
    TestSupportExt, Window, div,
};
use std::cell::Cell;

use crate::kv_grid::{self, Reloaded};
use crate::kv_table::set_text;
use crate::row_move::{
    self, DraggedRow, MoveRowDown, MoveRowUp, drop_line, drop_side, grip_slot, move_target,
    step_target,
};
use crate::scroll_follow::{FocusFollow, Followed};
use crate::style::{
    self, CHECKBOX_SIZE, ENV_CELL_MIN, KV_GRIP_WIDTH, KV_HEAD_HEIGHT, RADIUS, TEXT_SMALL,
};
use crate::tabs::Side;
use crate::ui;

/// Fixed width for the show or hide eye button, so secret rows keep the columns of plain rows.
pub(crate) const ACTION_SLOT_WIDTH: Rems = style::ui(26.);

/// The key context around an environment table's rows, where the row-move keys bind.
pub const ENV_TABLE_CONTEXT: &str = "EnvTable";

/// The column headings, the pair every variables table shares.
pub const ENV_HEADINGS: [&str; 2] = [
    crate::kv_table::VAR_HEADINGS.0,
    crate::kv_table::VAR_HEADINGS.1,
];

/// A masked secret row's value placeholder: the value is hidden, whether or not one is stored.
pub const SECRET_HIDDEN: &str = "••••••••";

/// A revealed secret row's value placeholder when nothing is stored.
pub const SECRET_UNSET: &str = "Not set";

/// The note under a secret row whose name an earlier secret row already holds.
pub const DUPLICATE_SECRET_NAME: &str = "Secret names must be unique";

/// Why a Save that would write two secrets of one name is refused.
pub const DUPLICATE_SECRET_NAMES: &str = "Duplicate secret names are not allowed";

/// A secret as the file keeps it: its name and whether it is set.
pub type SecretRecord = (String, bool);

/// The secrets among `vars`, in file order.
pub fn secret_records(vars: &[EnvVariable]) -> Vec<SecretRecord> {
    (vars.iter().filter(|v| v.secret))
        .map(|v| (v.name.clone(), v.enabled))
        .collect()
}

/// Whether two of `secrets` share a name. Stored values are keyed by name, so they would
/// share one value.
pub fn secrets_collide(secrets: &[SecretRecord]) -> bool {
    let mut seen = std::collections::HashSet::new();
    (secrets.iter()).any(|(name, _)| !name.is_empty() && !seen.insert(name))
}

/// Whether saving `draft` over a file holding `saved` would write a collision. A collision
/// the file already holds does not block a save that leaves every secret as it was.
pub fn writes_colliding_secrets(draft: &[SecretRecord], saved: &[SecretRecord]) -> bool {
    secrets_collide(draft) && draft != saved
}

/// One variable as the table shows it; a secret row's value is always empty.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EnvRow {
    /// The name.
    pub name: String,
    /// The value; empty for a secret row.
    pub value: String,
    /// Whether the variable is set.
    pub enabled: bool,
    /// Whether the value lives outside the file.
    pub secret: bool,
    /// The index of the core entry this row came from; `None` for a new row.
    pub source: Option<usize>,
    /// Whether the saved environment file has no variable of this name.
    pub unsaved: bool,
}

/// What the table reports to its owner; its `Debug` never prints a secret value.
#[derive(Clone, PartialEq)]
pub enum EnvTableEvent {
    /// The user changed the rows; carries all of them.
    Changed(Vec<EnvRow>),
    /// The user asked to make the named variable secret or plain.
    SetSecret {
        /// The variable.
        name: String,
        /// The requested state.
        secret: bool,
    },
    /// The user committed a value for the named secret variable.
    SecretValue {
        /// The variable.
        name: String,
        /// The value to store.
        value: String,
    },
    /// The user asked to see the named secret variable's stored value.
    Reveal(String),
    /// The user asked to store typing the table holds until the store has been checked.
    TypingHeld,
}

impl std::fmt::Debug for EnvTableEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnvTableEvent::Changed(rows) => f.debug_tuple("Changed").field(rows).finish(),
            EnvTableEvent::SetSecret { name, secret } => f
                .debug_struct("SetSecret")
                .field("name", name)
                .field("secret", secret)
                .finish(),
            EnvTableEvent::SecretValue { name, .. } => f
                .debug_struct("SecretValue")
                .field("name", name)
                .finish_non_exhaustive(),
            EnvTableEvent::Reveal(name) => f.debug_tuple("Reveal").field(name).finish(),
            EnvTableEvent::TypingHeld => f.write_str("TypingHeld"),
        }
    }
}

#[derive(Clone, Copy)]
struct Meta {
    enabled: bool,
    secret: bool,
    source: Option<usize>,
    unsaved: bool,
}

struct Row {
    name: Entity<InputState>,
    value: Entity<InputState>,
    meta: Meta,
    /// A secret row's last shown or committed text, so an unchanged blur sends nothing.
    committed: String,
    /// Whether a secret row's value is shown unmasked.
    shown: bool,
    /// Whether typing began before the name reached the file, so the name may already hold a
    /// stored value later typing would overwrite. Only a release or a discard ends it.
    undeclared: bool,
    /// The (name, text) the owner last checked against the store, for a guarded row.
    confirmed: Option<(String, String)>,
    /// The name as last accepted, which a refused rename goes back to.
    named: String,
    // Not a tab stop; it only sees focus inside the row's checkbox.
    check: FocusHandle,
    // Not a tab stop; it only sees focus inside the row's Remove.
    remove: FocusHandle,
    // Tracks the whole row, so focus on any control in it (cell, checkbox, eye, delete)
    // counts as focus in the row.
    within: FocusHandle,
    _subs: [Subscription; 2],
}

impl Row {
    fn guarded(&self) -> bool {
        self.meta.secret && (self.meta.unsaved || self.undeclared)
    }

    // The one rule every route out of the table obeys: a guarded row's typing leaves only as
    // the name and text the owner confirmed, so no save, event or blur can skip the check.
    fn releases(&self, text: &str, cx: &App) -> bool {
        let name = self.name.read(cx).value();
        let agreed = |(n, t): &(String, String)| n.as_str() == name.as_ref() && t == text;
        !self.guarded() || self.confirmed.as_ref().is_some_and(agreed)
    }

    // Typing the file has caught up with and no release has taken: a row still unsaved is the
    // draft's to report instead, and an emptied row has nothing to hold.
    fn held(&self, cx: &App) -> bool {
        let text = self.value.read(cx).value();
        let typing = !text.is_empty() && text != self.committed.as_str();
        self.guarded() && !self.meta.unsaved && typing
    }

    fn released(&mut self, text: String) {
        self.committed = text;
        self.forget_typing();
    }

    fn forget_typing(&mut self) {
        self.undeclared = false;
        self.confirmed = None;
    }
}

/// A row control that keeps focus through a reload that keeps its row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Control {
    Check,
    Lock,
    Remove,
}

// What a row typed into the empty row starts as: plain, enabled, new and not yet saved.
const FRESH: Meta = Meta {
    enabled: true,
    secret: false,
    source: None,
    unsaved: true,
};

/// An editable table of environment variables, always ending with an empty row that turns
/// into a real one when typed in.
pub struct EnvVarTable {
    rows: Vec<Row>,
    blank: Row,
    secret_locked: bool,
    /// While set, a blur leaves secret typing unsent; Enter and Save still commit it.
    blur_held: bool,
    /// Where focus goes when Hide masks a focused value, so the next key cannot replace it.
    focus: FocusHandle,
    /// Where each row reports its bounds to the scroll area holding the table.
    follow: FocusFollow,
    /// The `(row, column)` of the cell the latest emitted change was typed in; `None` for
    /// any other kind of change.
    typed: Option<(usize, u8)>,
    /// Where a dragged row would land: the row under the pointer and the side of it.
    drop: Option<(usize, Side)>,
    /// The row whose control takes focus on the next draw, after a reload moved its row.
    control_focus: Cell<Option<(usize, Control)>>,
    /// The control that held focus when the pending control focus was left; the pending
    /// focus is the user's only while it still does.
    pending_origin: Option<FocusHandle>,
    /// The row control that held focus at the last draw, and its handle: a dirty environment
    /// disables the locks, which drops a focused one from the focus tree before the load that
    /// follows can see it there.
    last_control: Option<(usize, Control, FocusHandle)>,
    /// The secrets the saved file declares, names a locked new row may not take.
    saved_secrets: Vec<String>,
    /// The name input of the row whose last name was refused, and why.
    name_note: Option<(Entity<InputState>, String)>,
}

impl EventEmitter<EnvTableEvent> for EnvVarTable {}

impl EnvVarTable {
    /// An empty table.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        EnvVarTable {
            rows: Vec::new(),
            blank: Self::new_row("", FRESH, window, cx),
            secret_locked: false,
            blur_held: false,
            focus: cx.focus_handle(),
            follow: FocusFollow::default(),
            typed: None,
            drop: None,
            control_focus: Cell::new(None),
            pending_origin: None,
            last_control: None,
            saved_secrets: Vec::new(),
            name_note: None,
        }
    }

    /// The cell the latest emitted change was typed in, or `None` when another kind of change
    /// (add, delete, toggle) made it.
    pub fn last_typed(&self) -> Option<(usize, u8)> {
        self.typed
    }

    /// Reports each row control's bounds, and each row's, to `follow`, the watch of the scroll
    /// area holding the table.
    pub fn set_follow(&mut self, follow: FocusFollow) {
        self.follow = follow;
    }

    /// The rows as they stand; a secret row's value is always empty.
    pub fn rows(&self, cx: &App) -> Vec<EnvRow> {
        self.rows
            .iter()
            .map(|row| EnvRow {
                name: row.name.read(cx).value().to_string(),
                value: if row.meta.secret {
                    String::new()
                } else {
                    row.value.read(cx).value().to_string()
                },
                enabled: row.meta.enabled,
                secret: row.meta.secret,
                source: row.meta.source,
                unsaved: row.meta.unsaved,
            })
            .collect()
    }

    // A pending focus waits for a draw the table may not get while hidden; once focus has
    // left the control it came from, the user moved on and the pending focus is dropped.
    fn drop_moved_on_focus(&mut self, window: &Window) {
        if !(self.pending_origin.as_ref()).is_some_and(|h| h.is_focused(window)) {
            self.control_focus.set(None);
            self.pending_origin = None;
        }
    }

    /// Show `rows` without emitting an event. A secret row keeps its own inputs, matched by
    /// name, so its typing, focus and mask move with it; another row never inherits a secret
    /// row's inputs, so focused typing cannot land in a plain value. Other rows reuse by index.
    /// The empty row is never part of the reuse pool and keeps its inputs and focus.
    pub fn load(&mut self, rows: Vec<EnvRow>, window: &mut Window, cx: &mut Context<Self>) {
        // A control focus an earlier load left for a draw that has not come yet is where focus
        // belongs; the window's focus still names the old row's control.
        self.drop_moved_on_focus(window);
        let pending = self.control_focus.take();
        let in_cell = pending.is_none() && self.focused_cell(window, cx).is_some();
        let remembered = (self.last_control.as_ref())
            .filter(|(ix, _, handle)| *ix < self.rows.len() && handle.is_focused(window));
        let held = (pending.map(|(ix, _)| ix))
            .or_else(|| self.focused_row(window, cx))
            .or(remembered.map(|(ix, _, _)| *ix));
        // An undo or redo of a move shows the rows reordered; focus goes with the row it was on.
        let followed = self.focused_cell(window, cx).and_then(|(ix, col)| {
            let to = row_move::follow(&self.rows(cx), &rows, ix, |_| false, same_content)?;
            Some((to, col))
        });
        // Focus on a row's control (its lock, checkbox or Remove) stays with that row if it
        // survives the load, and moves as a delete moves it if not.
        let kind = (pending.map(|(_, kind)| kind))
            .or(remembered.map(|(_, kind, _)| *kind))
            .or_else(|| held.map(|ix| self.focused_control(ix, window, cx)));
        let control = held
            .filter(|_| !in_cell)
            .map(|ix| kv_grid::after_reload(&self.rows(cx), &rows, ix, |_| false, same_content));
        let mut old: Vec<Option<Row>> = std::mem::take(&mut self.rows)
            .into_iter()
            .map(Some)
            .collect();
        let mut kept: Vec<Option<Row>> = Vec::with_capacity(rows.len());
        for row in &rows {
            let pos = old.iter().position(|o| {
                o.as_ref().is_some_and(|o| {
                    row.secret && o.meta.secret && o.name.read(cx).value() == row.name
                })
            });
            kept.push(pos.and_then(|pos| old[pos].take()));
        }
        for (ix, (row, kept)) in rows.into_iter().zip(kept).enumerate() {
            let meta = Meta {
                enabled: row.enabled,
                secret: row.secret,
                source: row.source,
                unsaved: row.unsaved,
            };
            if let Some(mut existing) = kept {
                // A save from outside the table declares the name but stores none of the typing.
                let typing = existing.value.read(cx).value() != existing.committed.as_str();
                existing.undeclared |= typing && existing.meta.unsaved;
                existing.meta = meta;
                existing.named = row.name;
                if !existing.guarded() {
                    existing.confirmed = None;
                }
                self.rows.push(existing);
                continue;
            }
            let mut existing = match old.get_mut(ix).and_then(|o| o.take_if(|o| !o.meta.secret)) {
                Some(reused) => reused,
                None => Self::new_row(&row.name, meta, window, cx),
            };
            let text = if row.secret { String::new() } else { row.value };
            // A note belongs to the row it was raised on; a reload that puts another name in
            // these inputs drops it.
            if (self.name_note.as_ref()).is_some_and(|(note, _)| note == &existing.name)
                && existing.name.read(cx).value() != row.name
            {
                self.name_note = None;
            }
            set_text(&existing.name, &row.name, window, cx);
            set_text(&existing.value, &text, window, cx);
            existing.named = row.name;
            existing.value.update(cx, |state, cx| {
                state.set_masked(row.secret, window, cx);
                state.set_placeholder(value_placeholder(row.secret, true), window, cx);
            });
            existing.meta = meta;
            existing.committed = String::new();
            existing.shown = false;
            existing.forget_typing();
            self.rows.push(existing);
        }
        if let Some((to, col)) = followed
            && let Some(row) = self.rows.get(to)
        {
            [&row.name, &row.value][col]
                .focus_handle(cx)
                .focus(window, cx);
        } else if let Some(Reloaded::Kept(to)) = control {
            // A locked lock cannot hold focus, so the row's name takes it instead.
            let row = &self.rows[to];
            let lock_off = self.secret_locked && !row.meta.unsaved;
            let kind = kind.unwrap_or(Control::Lock);
            if kind == Control::Lock && (lock_off || row.name.read(cx).value().is_empty()) {
                row.name.clone().focus_handle(cx).focus(window, cx);
            } else if Some(to) != held || pending.is_some() {
                self.control_focus.set(Some((to, kind)));
                self.pending_origin = window.focused(cx);
            }
        } else if let Some(Reloaded::Dropped(ix)) = control {
            let target = kv_grid::after_delete(&self.rows, ix, &self.blank);
            target.name.clone().focus_handle(cx).focus(window, cx);
        } else if held.is_some() && self.focused_row(window, cx).is_none() {
            // The load dropped the focused row (an undone add). The table itself takes focus, so
            // the editor's undo and redo keys still reach it while typing lands in no other row.
            self.focus.focus(window, cx);
        }
        cx.notify();
    }

    /// Empty and mask every secret row as Hide does, so typing a Reload threw away is neither
    /// shown nor stored by a later Save. Text already sent stays in the store. Focus leaves an
    /// emptied value input, so a later key or blur cannot overwrite the stored secret.
    pub fn discard_secret_typing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut focused = false;
        for row in self.rows.iter_mut().filter(|row| row.meta.secret) {
            focused |= row.value.read(cx).focus_handle(cx).is_focused(window);
            set_text(&row.value, "", window, cx);
            row.value.update(cx, |state, cx| {
                state.set_masked(true, window, cx);
                state.set_placeholder(value_placeholder(true, true), window, cx);
            });
            row.committed = String::new();
            row.shown = false;
            row.forget_typing();
        }
        if focused {
            self.focus.focus(window, cx);
        }
        cx.notify();
    }

    /// Show the named secret's stored value unmasked; `None` means nothing is stored.
    pub fn show_secret(
        &mut self,
        name: &str,
        value: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = value.unwrap_or_default();
        self.set_secret_text(name, text, false, window, cx);
    }

    /// Mask the named secret again and drop its shown value.
    pub fn hide_secret(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.set_secret_text(name, String::new(), true, window, cx);
    }

    /// Why a name typed into a locked row was refused, while that row shows it.
    pub fn name_note(&self) -> Option<&str> {
        let (input, note) = self.name_note.as_ref()?;
        let shown = self.rows.iter().any(|row| &row.name == input);
        shown.then_some(note.as_str())
    }

    /// The secret rows as the file would keep them, in row order.
    pub fn secret_records(&self, cx: &App) -> Vec<SecretRecord> {
        (self.rows.iter())
            .filter(|row| row.meta.secret)
            .map(|row| (row.name.read(cx).value().to_string(), row.meta.enabled))
            .collect()
    }

    /// [`DUPLICATE_SECRET_NAME`] when row `ix` is a secret whose name another secret row
    /// also holds; every row of the name is marked.
    pub fn duplicate_note(&self, ix: usize, cx: &App) -> Option<&'static str> {
        self.has_twin(ix, cx).then_some(DUPLICATE_SECRET_NAME)
    }

    fn has_twin(&self, ix: usize, cx: &App) -> bool {
        let Some(row) = self.rows.get(ix).filter(|row| row.meta.secret) else {
            return false;
        };
        let name = row.name.read(cx).value();
        let twin =
            |(j, r): (usize, &Row)| j != ix && r.meta.secret && r.name.read(cx).value() == name;
        !name.is_empty() && self.rows.iter().enumerate().any(twin)
    }

    /// Sets the secrets the saved file declares. A locked row not yet saved cannot be renamed
    /// onto one, so a blur or Save never writes over that secret's stored value unasked.
    pub fn set_saved_secrets(&mut self, names: Vec<String>) {
        self.saved_secrets = names;
    }

    /// Whether row `ix` is a secret row whose value is shown.
    pub fn revealed(&self, ix: usize) -> bool {
        self.rows
            .get(ix)
            .is_some_and(|row| row.meta.secret && row.shown)
    }

    /// Disable or enable every secret checkbox.
    pub fn set_secret_locked(&mut self, locked: bool, cx: &mut Context<Self>) {
        self.secret_locked = locked;
        cx.notify();
    }

    /// Hold or release blur commits. The owner holds them while the file changed on disk, so
    /// focus leaving for Compare or another window cannot store typing a Reload would drop.
    pub fn set_blur_held(&mut self, held: bool) {
        self.blur_held = held;
    }

    /// Whether a row holds typing its file has caught up with: unsaved work that nothing else
    /// shows, so every decision about leaving or closing reads this.
    pub fn holds_typing(&self, cx: &App) -> bool {
        self.rows.iter().any(|row| row.held(cx)) || !self.twin_typing(cx).is_empty()
    }

    /// The names of secret rows holding typing that can never be stored, since another secret
    /// row shares the name; leaving drops it, so it counts as held.
    pub fn twin_typing(&self, cx: &App) -> Vec<String> {
        let typed = |row: &Row| row.value.read(cx).value() != row.committed.as_str();
        let mut names: Vec<String> = (0..self.rows.len())
            .filter(|&ix| self.has_twin(ix, cx) && typed(&self.rows[ix]))
            .map(|ix| self.rows[ix].name.read(cx).value().to_string())
            .collect();
        names.dedup();
        names
    }

    /// Put back what each twin row showed before it was typed in, once the person agreed.
    pub fn discard_twin_typing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let twins: Vec<usize> = (0..self.rows.len())
            .filter(|&ix| self.has_twin(ix, cx))
            .collect();
        for ix in twins {
            let row = &self.rows[ix];
            set_text(&row.value, &row.committed.clone(), window, cx);
        }
        cx.notify();
    }

    /// Whether a secret row holds typing not yet sent.
    pub fn has_secret_typing(&self, cx: &App) -> bool {
        self.rows
            .iter()
            .filter(|row| row.meta.secret)
            .any(|row| row.value.read(cx).value() != row.committed.as_str())
    }

    /// The secret rows' typing not yet committed, as (name, value), marked committed without an
    /// event. Rows not yet saved are included only when `unsaved` is set, for a Save of the
    /// file; otherwise their typing stays pending, so a discard never reaches the store.
    pub fn take_pending_secrets(&mut self, unsaved: bool, cx: &App) -> Vec<(String, String)> {
        let mut pending = Vec::new();
        // A twin's typing never leaves: it would land on the value both names share.
        let twins: Vec<bool> = (0..self.rows.len())
            .map(|ix| self.has_twin(ix, cx))
            .collect();
        let rows = self.rows.iter_mut().zip(twins).filter(|(_, twin)| !twin);
        for (row, _) in rows.filter(|(row, _)| row.meta.secret && (unsaved || !row.meta.unsaved)) {
            let text = row.value.read(cx).value().to_string();
            if text != row.committed && row.releases(&text, cx) {
                row.released(text.clone());
                pending.push((row.name.read(cx).value().to_string(), text));
            }
        }
        pending
    }

    /// The typing not yet committed in saved secret row `name`, marked committed without an
    /// event, so an owner can act on it ahead of anything an event would queue behind.
    pub fn take_typing(&mut self, name: &str, cx: &App) -> Option<String> {
        let ix = (self.rows.iter()).position(|row| row.name.read(cx).value() == name)?;
        // A twin's value would land on the one both names share.
        if self.has_twin(ix, cx) {
            return None;
        }
        let row = &mut self.rows[ix];
        let text = row.value.read(cx).value().to_string();
        let held = !row.meta.secret || row.meta.unsaved || text == row.committed;
        if held || !row.releases(&text, cx) {
            return None;
        }
        row.released(text.clone());
        Some(text)
    }

    /// The typing that needs a check of the store first, as (name, value): what a Save would
    /// store under a name that may already hold a value.
    pub fn unsaved_secret_typing(&self, cx: &App) -> Vec<(String, String)> {
        let uncommitted = |row: &&Row| row.value.read(cx).value() != row.committed.as_str();
        (self.rows.iter().filter(|row| row.guarded()))
            .filter(uncommitted)
            .map(|row| {
                let name = row.name.read(cx).value().to_string();
                (name, row.value.read(cx).value().to_string())
            })
            .filter(|(name, text)| !name.is_empty() && !text.is_empty())
            .collect()
    }

    /// The part of [`Self::unsaved_secret_typing`] that leaves without a Save of the file: the
    /// rows the file already has. A draft row is asked about at its own Save.
    pub fn held_secret_typing(&self, cx: &App) -> Vec<(String, String)> {
        (self.rows.iter().filter(|row| row.held(cx)))
            .map(|row| {
                let name = row.name.read(cx).value().to_string();
                (name, row.value.read(cx).value().to_string())
            })
            .collect()
    }

    /// Records that the store was asked about row `name`'s typing as it stands now, which lets
    /// that name and text out once. Any edit voids it.
    pub fn confirm_typing(&mut self, name: &str, cx: &App) {
        for row in (self.rows.iter_mut()).filter(|row| row.guarded()) {
            if row.name.read(cx).value() == name {
                let text = row.value.read(cx).value().to_string();
                row.confirmed = Some((name.to_owned(), text));
            }
        }
    }

    /// [`Self::confirm_typing`] for the (name, text) pairs a check read; a row typed in since
    /// stays unconfirmed.
    pub fn confirm_checked(&mut self, checked: &[(String, String)], cx: &App) {
        for row in (self.rows.iter_mut()).filter(|row| row.guarded()) {
            let name = row.name.read(cx).value().to_string();
            let text = row.value.read(cx).value().to_string();
            if checked.iter().any(|(n, t)| *n == name && *t == text) {
                row.confirmed = Some((name, text));
            }
        }
    }

    /// Appends an enabled, plain, unsaved row `name` with an empty value and focuses its value,
    /// or focuses the value of an existing row with that name. Returns the row's index.
    pub fn add_row(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) -> usize {
        let existing = self
            .rows
            .iter()
            .position(|row| row.name.read(cx).value() == name);
        let ix = existing.unwrap_or_else(|| {
            // Built as typing into the empty row would build it, so it stays unsaved until Save.
            self.rows.push(Self::new_row(name, FRESH, window, cx));
            self.changed(cx);
            self.rows.len() - 1
        });
        self.rows[ix]
            .value
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
        ix
    }

    /// The name input of row `ix`, or of the empty row at `rows.len()`.
    pub fn name_input(&self, ix: usize) -> Option<&Entity<InputState>> {
        self.row(ix).map(|row| &row.name)
    }

    /// The value input of row `ix`, or of the empty row at `rows.len()`.
    pub fn value_input(&self, ix: usize) -> Option<&Entity<InputState>> {
        self.row(ix).map(|row| &row.value)
    }

    fn row(&self, ix: usize) -> Option<&Row> {
        self.rows
            .get(ix)
            .or((ix == self.rows.len()).then_some(&self.blank))
    }

    fn set_secret_text(
        &mut self,
        name: &str,
        text: String,
        masked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for row in &mut self.rows {
            if row.meta.secret && row.name.read(cx).value() == name {
                set_text(&row.value, &text, window, cx);
                row.value.update(cx, |state, cx| {
                    state.set_masked(masked, window, cx);
                    state.set_placeholder(value_placeholder(true, masked), window, cx);
                });
                row.committed.clone_from(&text);
                row.shown = !masked;
                row.forget_typing();
            }
        }
        cx.notify();
    }

    /// Handlers find their row by input entity, so no index goes stale after a delete.
    fn new_row(name: &str, meta: Meta, window: &mut Window, cx: &mut Context<Self>) -> Row {
        let named = name.to_string();
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Name")
                .default_value(name.to_string())
        });
        let value = cx.new(|cx| InputState::new(window, cx).placeholder("Value"));
        let subs = [&name, &value].map(|input| {
            cx.subscribe_in(
                input,
                window,
                |table: &mut EnvVarTable, input, event: &InputEvent, window, cx| {
                    table.input_event(input, event, window, cx)
                },
            )
        });
        Row {
            name,
            value,
            meta,
            committed: String::new(),
            shown: false,
            undeclared: false,
            confirmed: None,
            named,
            check: cx.focus_handle(),
            remove: cx.focus_handle(),
            within: cx.focus_handle(),
            _subs: subs,
        }
    }

    /// Plain edits redraft the list; a secret value waits for Enter or blur and is sent only
    /// when its text differs from what was last shown or committed. A change in the empty row
    /// first makes it a real plain row, keeping its inputs and focus.
    fn input_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let in_blank = &self.blank.name == input || &self.blank.value == input;
        if in_blank && matches!(event, InputEvent::Change) {
            let fresh = Self::new_row("", FRESH, window, cx);
            let row = std::mem::replace(&mut self.blank, fresh);
            self.rows.push(row);
        }
        let Some(ix) = self
            .rows
            .iter()
            .position(|row| &row.name == input || &row.value == input)
        else {
            return;
        };
        let row = &self.rows[ix];
        let secret_value = row.meta.secret && &row.value == input;
        if matches!(event, InputEvent::Change) && &row.name == input {
            let name = row.name.read(cx).value().to_string();
            let locked_new = row.meta.secret && row.meta.unsaved;
            if locked_new && self.saved_secrets.contains(&name) {
                return self.refuse_name(ix, &name, window, cx);
            }
            // A refused lock's note outlives the late edit that brought the name it names.
            let still_refused = self.saved_secrets.contains(&name);
            // Keys typed in one batch queue a Change each, read after the refusal put the old
            // name back; only a name that differs from the accepted one is a new edit.
            let renamed = name != self.rows[ix].named;
            let noted = renamed && (self.name_note.as_ref()).is_some_and(|(note, _)| note == input);
            self.rows[ix].named = name.clone();
            // The note follows the name the row holds: another saved secret rewrites it.
            if noted && still_refused {
                self.note_saved_secret(ix, &name, cx);
            } else if noted {
                self.name_note = None;
            }
        }
        match event {
            InputEvent::Change if !secret_value => {
                let column = u8::from(&self.rows[ix].value == input);
                self.typed = Some((ix, column));
                self.emit_changed(cx)
            }
            // The guard outlives the text: only the confirmation depends on it.
            InputEvent::Change => {
                let row = &mut self.rows[ix];
                row.undeclared |= row.meta.unsaved;
                if row.value.read(cx).value().is_empty() {
                    row.confirmed = None;
                }
            }
            InputEvent::PressEnter { .. } if secret_value && self.rows[ix].held(cx) => {
                cx.emit(EnvTableEvent::TypingHeld)
            }
            InputEvent::PressEnter { .. } if secret_value => self.commit_secret(ix, cx),
            InputEvent::Blur if secret_value && !self.blur_held => self.commit_secret(ix, cx),
            _ => {}
        }
    }

    // A saved secret's value is keyed by its name, so a locked new row taking that name would
    // store over it; the name goes back and the row says why.
    fn refuse_name(&mut self, ix: usize, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let row = &self.rows[ix];
        set_text(&row.name, &row.named.clone(), window, cx);
        self.note_saved_secret(ix, name, cx);
    }

    fn note_saved_secret(&mut self, ix: usize, name: &str, cx: &mut Context<Self>) {
        let note = format!(
            "\"{name}\" is a saved secret. Choose another name, or remove it and save first."
        );
        self.name_note = Some((self.rows[ix].name.clone(), note));
        cx.notify();
    }

    /// Send row `ix`'s secret text when it differs from what was last shown or committed, so
    /// each value is sent exactly once. Requests call this first: clicking a button does not
    /// blur the input, and a reveal answer would overwrite typing never sent. An unsaved row
    /// may still be unticked, so its value waits for Save (`take_pending_secrets`).
    fn commit_secret(&mut self, ix: usize, cx: &mut Context<Self>) {
        // A twin's value would land on the one both names share, so its typing waits.
        if self.has_twin(ix, cx) {
            return;
        }
        let secret_saved = |row: &&mut Row| row.meta.secret && !row.meta.unsaved;
        let Some(row) = self.rows.get_mut(ix).filter(secret_saved) else {
            return;
        };
        let text = row.value.read(cx).value().to_string();
        if text == row.committed || !row.releases(&text, cx) {
            return;
        }
        row.released(text.clone());
        let name = row.name.read(cx).value().to_string();
        cx.emit(EnvTableEvent::SecretValue { name, value: text });
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        self.typed = None;
        self.emit_changed(cx)
    }

    fn emit_changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(EnvTableEvent::Changed(self.rows(cx)));
        cx.notify();
    }

    /// Remove row `ix` and move focus to the next row's name, else the previous row's, else
    /// the empty row's, so the keys of the editor around the table keep working.
    fn delete(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix < self.rows.len() {
            self.rows.remove(ix);
            let target = kv_grid::after_delete(&self.rows, ix, &self.blank)
                .name
                .clone();
            target.focus_handle(cx).focus(window, cx);
            self.changed(cx);
        }
    }

    /// The row and input column (name 0, value 1) holding focus in a real row.
    fn focused_cell(&self, window: &Window, cx: &App) -> Option<(usize, usize)> {
        self.rows.iter().enumerate().find_map(|(ix, row)| {
            let col = [&row.name, &row.value]
                .iter()
                .position(|i| i.focus_handle(cx).contains_focused(window, cx))?;
            Some((ix, col))
        })
    }

    /// The real row holding focus anywhere in it: a cell, its checkbox, eye or delete.
    fn focused_row(&self, window: &Window, cx: &App) -> Option<usize> {
        (self.rows.iter()).position(|row| row.within.contains_focused(window, cx))
    }

    /// Which control of row `ix` holds focus, when it is not a cell; the eye counts as the lock.
    fn focused_control(&self, ix: usize, window: &Window, cx: &App) -> Control {
        let row = &self.rows[ix];
        if row.check.contains_focused(window, cx) {
            Control::Check
        } else if row.remove.contains_focused(window, cx) {
            Control::Remove
        } else {
            Control::Lock
        }
    }

    /// Move row `from` to index `dest` as one edit. The Row moves whole, so a secret row keeps
    /// its typing, mask and reveal. Focus goes to the moved row's name unless it already sits
    /// in that row.
    pub fn move_row(
        &mut self,
        from: usize,
        dest: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let len = self.rows.len();
        if from >= len || dest >= len || from == dest {
            return;
        }
        // Focus on the row element itself (a grip press) is not focus on a control in it.
        let inside = self.focused_row(window, cx) == Some(from)
            && !self.rows[from].within.is_focused(window);
        row_move::reorder(&mut self.rows, from, dest);
        if !inside {
            self.rows[dest].name.focus_handle(cx).focus(window, cx);
        }
        self.drop = None;
        self.changed(cx);
    }

    /// Mark where dragging row `from` over row `to` would land, or clear the mark with `None`.
    /// A side where the drop would leave the row in place shows nothing.
    pub fn hover_drop(
        &mut self,
        from: usize,
        to: usize,
        side: Option<Side>,
        cx: &mut Context<Self>,
    ) {
        let next = side
            .filter(|&s| move_target(self.rows.len(), from, to, s).is_some())
            .map(|s| (to, s));
        let next = match (next, self.drop) {
            (None, Some((at, _))) if at != to => self.drop,
            _ => next,
        };
        if next != self.drop {
            self.drop = next;
            cx.notify();
        }
    }

    /// Alt+Up or Alt+Down: swap the row holding focus with its neighbour.
    fn step(&mut self, up: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.focused_row(window, cx) else {
            return;
        };
        if let Some(dest) = step_target(self.rows.len(), ix, up) {
            self.move_row(ix, dest, window, cx);
        }
    }

    /// A row dropped on the empty row moves nowhere; focus stays with the dragged row.
    fn drop_on_empty(&mut self, from: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.drop = None;
        if let Some(row) = self.rows.get(from) {
            row.name.focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }

    /// A row dropped on the `side` of row `to` lands there when that moves it.
    fn drop_row(&mut self, from: usize, to: usize, window: &mut Window, cx: &mut Context<Self>) {
        let side = self.drop.filter(|&(at, _)| at == to).map(|(_, s)| s);
        self.drop = None;
        cx.notify();
        if let Some(dest) = side.and_then(|s| move_target(self.rows.len(), from, to, s)) {
            self.move_row(from, dest, window, cx);
        }
    }

    fn toggle(&mut self, ix: usize, enabled: bool, window: &mut Window, cx: &mut Context<Self>) {
        // A click lands focus in the row unless the checkbox already holds it, so Alt+Up and
        // Alt+Down move the row.
        if let Some(row) = self.rows.get(ix)
            && !row.check.contains_focused(window, cx)
        {
            row.check.focus(window, cx);
        }
        if let Some(row) = self.rows.get_mut(ix) {
            row.meta.enabled = enabled;
            self.changed(cx);
        }
    }

    /// A saved variable's secret flag is the actor's to change, so the checkbox only asks, and
    /// the owner decides what becomes of the row's typing. A row not yet in the file flips
    /// locally and redrafts; Save then stores its typed value.
    fn request_secret(
        &mut self,
        ix: usize,
        secret: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(row) = self.rows.get_mut(ix).filter(|r| r.meta.unsaved) {
            let name = row.name.read(cx).value().to_string();
            if name.is_empty() {
                return;
            }
            // Locking under a saved secret's name would make the row that secret, so a blur
            // would store over it; the input's text counts, as its last edit may not have landed.
            if secret && self.saved_secrets.contains(&name) {
                return self.note_saved_secret(ix, &name, cx);
            }
            let row = &mut self.rows[ix];
            row.meta.secret = secret;
            row.committed = String::new();
            row.shown = false;
            row.confirmed = None;
            row.value.update(cx, |state, cx| {
                state.set_masked(secret, window, cx);
                state.set_placeholder(value_placeholder(secret, true), window, cx);
            });
            self.changed(cx);
            return;
        }
        if let Some(row) = self.rows.get(ix) {
            let name = row.name.read(cx).value().to_string();
            // A plain row named like a saved secret is not that variable; locking it would act
            // on the saved one, so the row refuses and says why.
            if secret && !row.meta.secret && self.saved_secrets.contains(&name) {
                return self.note_saved_secret(ix, &name, cx);
            }
            if !self.secret_locked && !name.is_empty() {
                cx.emit(EnvTableEvent::SetSecret { name, secret });
            }
        }
    }

    fn reveal(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.mask_unsaved(ix, false, window, cx) {
            return;
        }
        self.commit_secret(ix, cx);
        if let Some(row) = self.rows.get(ix).filter(|row| row.meta.secret) {
            cx.emit(EnvTableEvent::Reveal(row.name.read(cx).value().to_string()));
        }
    }

    /// Store typing not yet sent before masking, as `reveal` does, so Hide never loses it.
    fn hide(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.mask_unsaved(ix, true, window, cx) {
            return;
        }
        self.commit_secret(ix, cx);
        if let Some(row) = self.rows.get(ix).filter(|row| row.meta.secret) {
            let name = row.name.read(cx).value().to_string();
            if row.value.read(cx).focus_handle(cx).is_focused(window) {
                self.focus.focus(window, cx);
            }
            self.hide_secret(&name, window, cx);
        }
    }

    /// The store holds nothing to show for a guarded row, so Reveal and Hide only unmask
    /// or mask its typing; returns whether row `ix` was such a row.
    fn mask_unsaved(
        &mut self,
        ix: usize,
        masked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        // An emptied row the file has holds no typing to show, so its Reveal reads the store.
        let emptied = |r: &Row| !r.meta.unsaved && r.value.read(cx).value().is_empty();
        let Some(row) = (self.rows.get_mut(ix)).filter(|r| r.guarded() && (masked || !emptied(r)))
        else {
            return false;
        };
        row.value.update(cx, |state, cx| {
            state.set_masked(masked, window, cx);
            state.set_placeholder(value_placeholder(true, masked), window, cx);
        });
        row.shown = !masked;
        cx.notify();
        true
    }

    /// Draw row `ix`, or the empty row at `rows.len()`. The empty row has nothing to toggle,
    /// make secret or delete yet, so its toggle and delete are laid out hidden, under their own
    /// ids, and its lock is bare space, to keep its columns in line.
    fn render_row(&self, ix: usize, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let blank = ix == self.rows.len();
        let row = if blank { &self.blank } else { &self.rows[ix] };
        let slot =
            |real: &'static str, hidden: &'static str| (if blank { hidden } else { real }, ix);
        let name_locked = row.meta.secret && !row.meta.unsaved;
        // Each Tab stop reports its own bounds: the row is wider than a narrow area, and
        // following the row keeps only its left edge in view.
        let follow = &self.follow;
        let followed = |id: &'static str, child: AnyElement| Followed::new((id, ix), follow, child);
        // Each input sits borderless in a shared-look cell whose edge shows on hover or focus;
        // the minimum sits on the cell, so the lock and eye never eat into it.
        let cell = |input: &Entity<InputState>,
                    field,
                    (edge, id): (&'static str, &'static str),
                    cx: &App| {
            let focused = input.focus_handle(cx).contains_focused(window, cx);
            let cell = kv_grid::kv_cell((edge, ix), field, input, focused, cx);
            div()
                .flex_1()
                .min_w(ENV_CELL_MIN)
                .child(followed(id, cell.into_any_element()))
        };
        let handle = (!blank).then(|| {
            DraggedRow::new(
                cx.entity_id(),
                ix,
                row_move::preview_label(&row.name.read(cx).value()).into(),
            )
        });
        let landing = self
            .drop
            .filter(|&(at, _)| at == ix && !blank)
            .map(|(_, s)| s);
        let row = h_flex()
            .id(("env-row", ix))
            .test_support()
            .track_focus(&row.within)
            .relative()
            .gap_2()
            .w_full()
            .when(!blank, |r| {
                r.on_drag_move(cx.listener(move |t, e: &DragMoveEvent<DraggedRow>, _, cx| {
                    let d = e.drag(cx).clone();
                    if d.owner != cx.entity_id() {
                        return;
                    }
                    let side = e
                        .bounds
                        .contains(&e.event.position)
                        .then(|| drop_side(e.event.position.y, e.bounds));
                    t.hover_drop(d.from, ix, side, cx);
                }))
                .on_drop(cx.listener(move |t, d: &DraggedRow, window, cx| {
                    if d.owner == cx.entity_id() {
                        t.drop_row(d.from, ix, window, cx);
                    }
                }))
            })
            .when(blank, |r| {
                r.on_drop(cx.listener(move |t, d: &DraggedRow, window, cx| {
                    if d.owner == cx.entity_id() {
                        t.drop_on_empty(d.from, window, cx);
                    }
                }))
            })
            .when_some(landing, |r, side| {
                r.child(drop_line("env-drop-line", side, cx))
            })
            .child(grip_slot(
                ("env-grip", ix),
                ("env-slot-grip", ix),
                handle,
                cx,
            ))
            .child(
                div()
                    .flex_none()
                    .track_focus(&row.check)
                    .when(blank, |d| d.invisible())
                    .child(
                        ui::checkbox(slot("env-enabled", "env-slot-enabled"), row.meta.enabled)
                            .tab_stop(false)
                            .take_focus(!blank && self.take_control_focus(ix, Control::Check))
                            .on_click(cx.listener(move |t, on: &bool, window, cx| {
                                t.toggle(ix, *on, window, cx)
                            })),
                    ),
            )
            // A saved secret's stored value is keyed by the name, so it is read-only but still
            // reads normally. A row not yet saved stores nothing until Save asks, so it renames.
            .child(cell(
                &row.name,
                ui::readonly_input(
                    ui::table_input(&row.name, false, window, cx),
                    &row.name,
                    name_locked,
                    window,
                    cx,
                )
                .id(("env-name", ix)),
                ("env-name-edge", "env-name-follow"),
                cx,
            ))
            .child(
                h_flex()
                    .id(("env-value-cell", ix))
                    .test_support()
                    .flex_1()
                    .gap_2()
                    .child(cell(
                        &row.value,
                        ui::table_input(&row.value, false, window, cx).id(("env-value", ix)),
                        ("env-value-edge", "env-value-follow"),
                        cx,
                    ))
                    .child(followed("env-secret-follow", self.render_lock(ix, cx)))
                    .child(self.render_action_slot(ix, cx)),
            )
            .child(
                div()
                    .flex_none()
                    .track_focus(&row.remove)
                    .when(blank, |d| d.invisible())
                    .child(followed(
                        "env-delete-follow",
                        kv_grid::button_focusing(
                            slot("env-delete", "env-slot-delete"),
                            ui::icon_button(
                                slot("env-delete", "env-slot-delete"),
                                IconName::Close,
                                "Remove variable",
                                None,
                            )
                            .tab_stop(!blank)
                            .on_click(
                                cx.listener(move |t, _, window, cx| t.delete(ix, window, cx)),
                            ),
                            !blank && self.take_control_focus(ix, Control::Remove),
                        )
                        .into_any_element(),
                    )),
            );
        let refused = (self.name_note.as_ref())
            .filter(|(input, _)| !blank && *input == self.rows[ix].name)
            .map(|(_, note)| note.clone());
        let duplicate = self.duplicate_note(ix, cx);
        if refused.is_none() && duplicate.is_none() {
            return Followed::new(("env-row-wrapper", ix), &self.follow, row).into_any_element();
        }
        // Each note sits under the row whose name it is about.
        let danger = cx.theme().danger;
        let note = |id: gpui_kit::ElementId, text: String| {
            div()
                .id(id)
                .test_support()
                .pl(KV_GRIP_WIDTH + CHECKBOX_SIZE)
                .text_size(TEXT_SMALL)
                .text_color(danger)
                .child(text)
        };
        let row = v_flex()
            .w_full()
            .child(row)
            .children(refused.map(|text| note("env-name-refused".into(), text)))
            .children(duplicate.map(|text| note(("env-name-duplicate", ix).into(), text.into())));
        Followed::new(("env-row-wrapper", ix), &self.follow, row).into_any_element()
    }

    /// Whether row `ix`'s `control` takes the focus a reload left for it, clearing that once
    /// it does.
    fn take_control_focus(&self, ix: usize, control: Control) -> bool {
        let take = self.control_focus.get() == Some((ix, control));
        if take {
            self.control_focus.set(None);
        }
        take
    }

    /// Row `ix`'s secrecy lock, a Tab stop while enabled; the empty row holds bare space of its
    /// width instead.
    fn render_lock(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        if ix == self.rows.len() {
            return div().flex_none().w(CHECKBOX_SIZE).into_any_element();
        }
        // Rows are drawn for indices up to `rows.len()`, the blank row, and `row` answers all of them.
        #[allow(clippy::expect_used)]
        let row = self
            .row(ix)
            .expect("indices up to rows.len() all have a row");
        // A row not yet saved owns its secret flag, so a dirty or busy environment never locks it.
        let secret = row.meta.secret;
        let name = row.name.read(cx).value().to_string();
        let unnamed = name.is_empty();
        // A plain row named like a saved secret stays clickable, so the click can say why it is
        // refused instead of the lock going dead without a word.
        let clash = !secret && self.saved_secrets.contains(&name);
        let locked = self.secret_locked && !row.meta.unsaved && !clash;
        let tip = match (locked, secret) {
            (true, _) => crate::env_editor::RENAME_LOCKED_REASON,
            (false, false) => "Make secret",
            (false, true) => "Make plain",
        };
        div()
            .flex_none()
            .flex()
            .items_center()
            .child(
                ui::lock_toggle(("env-secret", ix), secret)
                    .disabled(locked || unnamed)
                    .take_focus(self.take_control_focus(ix, Control::Lock))
                    .tooltip(tip)
                    .on_click(
                        cx.listener(move |t, on: &bool, w, cx| t.request_secret(ix, *on, w, cx)),
                    ),
            )
            .into_any_element()
    }

    /// Row `ix`'s show or hide eye, in a fixed slot so plain rows keep the same columns.
    fn render_action_slot(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        // Rows are drawn for indices up to `rows.len()`, the blank row, and `row` answers all of them.
        #[allow(clippy::expect_used)]
        let row = self
            .row(ix)
            .expect("indices up to rows.len() all have a row");
        let secret = row.meta.secret;
        let revealed = secret && row.shown;
        h_flex()
            .id(("env-action-slot", ix))
            .flex_none()
            .w(ACTION_SLOT_WIDTH)
            .when(secret, |d| {
                let (face, icon, words) = if revealed {
                    ("env-hide", IconName::EyeOff, "Hide value")
                } else {
                    ("env-reveal", IconName::Eye, "Show value")
                };
                // One button across the flip keeps its element state, like every other eye.
                d.child(
                    ui::swap_icon_button(("env-eye", ix), (face, ix), icon, words)
                        .tab_stop(false)
                        .on_click(cx.listener(move |t, _, w, cx| {
                            if revealed {
                                t.hide(ix, w, cx)
                            } else {
                                t.reveal(ix, w, cx)
                            }
                        })),
                )
            })
            .test_support()
            .into_any_element()
    }

    /// Draw the heading row. Bare spacers the width of the rows' controls stand in for them,
    /// so each heading starts where its column does and the heading holds no control.
    fn render_heading(&self, cx: &mut Context<Self>) -> AnyElement {
        let (fill, muted) = kv_grid::head_colors(cx);
        let text = |text: &'static str| {
            div()
                .text_size(TEXT_SMALL)
                .text_color(muted)
                .flex_1()
                .min_w(ENV_CELL_MIN)
                .truncate()
                .child(text)
        };
        let [name, value] = ENV_HEADINGS;
        h_flex()
            .id("env-head")
            .test_support()
            .gap_2()
            .w_full()
            .h(KV_HEAD_HEIGHT)
            .rounded(RADIUS)
            .bg(fill)
            .child(div().flex_none().w(KV_GRIP_WIDTH))
            .child(div().flex_none().w(CHECKBOX_SIZE))
            .child(text(name).id("env-head-name").test_support())
            // The value heading spans the cell's lock and eye slots, so it shrinks no further
            // than the value cell and keeps over it down to the row's minimum.
            .child(
                h_flex()
                    .id("env-head-value")
                    .test_support()
                    .flex_1()
                    .gap_2()
                    .child(text(value))
                    .child(div().flex_none().w(CHECKBOX_SIZE))
                    .child(div().flex_none().w(ACTION_SLOT_WIDTH)),
            )
            // Delete is an icon button, as wide as the eye's slot.
            .child(div().flex_none().w(ACTION_SLOT_WIDTH))
            .into_any_element()
    }
}

impl Render for EnvVarTable {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.drop_moved_on_focus(window);
        if let Some(ix) = self.focused_row(window, cx)
            && self.focused_cell(window, cx).is_none()
        {
            let kind = self.focused_control(ix, window, cx);
            self.last_control = window.focused(cx).map(|handle| (ix, kind, handle));
        }
        let rows: Vec<_> = std::iter::once(self.render_heading(cx))
            .chain((0..=self.rows.len()).map(|ix| self.render_row(ix, window, cx)))
            .collect();
        v_flex()
            .id("env-table")
            .test_support()
            .track_focus(&self.focus)
            .key_context(ENV_TABLE_CONTEXT)
            .on_action(cx.listener(|t, _: &MoveRowUp, window, cx| t.step(true, window, cx)))
            .on_action(cx.listener(|t, _: &MoveRowDown, window, cx| t.step(false, window, cx)))
            .gap_1()
            .w_full()
            .children(rows)
    }
}

// Whether two rows show the same content, wherever they came from.
fn same_content(a: &EnvRow, b: &EnvRow) -> bool {
    (&a.name, &a.value, a.enabled, a.secret) == (&b.name, &b.value, b.enabled, b.secret)
}

/// The value placeholder: dots while a secret is masked, since the table cannot see whether
/// the store holds a value; "Not set" for a revealed empty secret.
fn value_placeholder(secret: bool, masked: bool) -> &'static str {
    match (secret, masked) {
        (false, _) => "Value",
        (true, true) => SECRET_HIDDEN,
        (true, false) => SECRET_UNSET,
    }
}

/// Rows for `vars`; a typed value shows its data and a secret shows nothing.
pub fn env_rows(vars: &[EnvVariable]) -> Vec<EnvRow> {
    vars.iter()
        .enumerate()
        .map(|(ix, v)| EnvRow {
            name: v.name.clone(),
            value: match (&v.value, v.secret) {
                (_, true) => String::new(),
                (VarValue::Text(text), _) => text.clone(),
                (VarValue::Typed { data, .. }, _) => data.clone(),
                _ => String::new(),
            },
            enabled: v.enabled,
            secret: v.secret,
            source: Some(ix),
            unsaved: false,
        })
        .collect()
}

/// Variables from `rows`, editing clones of their source entries so unseen keys survive. A
/// typed source keeps its declared type; a secret keeps its base value. `secret` is set only
/// from a row not yet saved, the one row whose flag the table owns.
pub fn env_vars_from(base: &[EnvVariable], rows: &[EnvRow]) -> Vec<EnvVariable> {
    rows.iter()
        .map(|row| {
            let mut v = row
                .source
                .and_then(|ix| base.get(ix))
                .cloned()
                .unwrap_or_else(|| EnvVariable::new("", VarValue::Text(String::new())));
            v.name.clone_from(&row.name);
            v.enabled = row.enabled;
            if row.unsaved && row.secret != v.secret {
                v.secret = row.secret;
                v.value = VarValue::Text(String::new());
            }
            if !v.secret {
                v.value = match &v.value {
                    VarValue::Typed { kind, .. } => VarValue::Typed {
                        kind: kind.clone(),
                        data: row.value.clone(),
                    },
                    _ => VarValue::Text(row.value.clone()),
                };
            }
            v
        })
        .collect()
}
