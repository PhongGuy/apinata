//! One editable name and value table for params, headers, vars and form bodies.

use std::any::Any;
use std::cell::Cell;
use std::fmt;
use std::sync::Arc;

use apinata_core::{
    FileEntry, KeyValue, MultipartField, MultipartValue, Param, ParamKind, PostResponseVar,
    VarValue, Variable, VariableSet,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Editor, EditorState, InputEvent, InputState};
use gpui_kit::component::{Size, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, App, AppContext, ClickEvent, Context, Div, DragMoveEvent, Entity, EventEmitter,
    FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, TestSupportExt, Window, div,
    transparent_black,
};

use crate::header_complete::Hint;
use crate::kv_bulk::{bulk_rows, bulk_text, editable};
use crate::kv_grid::{
    self, ActionShow, KvColumn, Reloaded, action_slot, check_slot, kv_head, kv_row,
};
use crate::row_move::{
    self, DraggedRow, MoveRowDown, MoveRowUp, drop_line, drop_side, grip_slot, real_move_target,
    real_step_target,
};
use crate::scroll_follow::{FocusFollow, Followed};
use crate::style::{self, ROW_HEIGHT, TAG_WIDTH, TEXT_SMALL};
use crate::tabs::Side;
use crate::ui::{self, LockButton as _};
use crate::var_tokens::VarField;

/// The name and value placeholders a table starts with.
const DEFAULT_PLACEHOLDERS: (&str, &str) = ("Key", "Value");
/// The content-type placeholder a table starts with.
const TYPE_HINT: &str = "Content type";

/// The core entry a row was built from, type-erased; always equal, shown as `RowEntry`.
/// [`same`](RowEntry::same) compares the entries themselves.
#[derive(Clone)]
pub struct RowEntry {
    entry: Arc<dyn Any + Send + Sync>,
    eq: Arc<EntryEq>,
}

/// Compares two type-erased entries; false when either is not the wrapped type.
type EntryEq = dyn Fn(&dyn Any, &dyn Any) -> bool + Send + Sync;

impl RowEntry {
    /// Wrap `entry`, compared by its type's `PartialEq`.
    pub fn new<T: Any + Send + Sync + PartialEq>(entry: T) -> RowEntry {
        RowEntry::with_eq(entry, T::eq)
    }

    /// Wrap `entry`, compared by `eq`, for an entry whose `PartialEq` misses what a reload
    /// can change (such as the YAML it was read from).
    pub fn with_eq<T: Any + Send + Sync>(entry: T, eq: fn(&T, &T) -> bool) -> RowEntry {
        RowEntry {
            entry: Arc::new(entry),
            eq: Arc::new(
                move |a, b| match (a.downcast_ref::<T>(), b.downcast_ref::<T>()) {
                    (Some(a), Some(b)) => eq(a, b),
                    _ => false,
                },
            ),
        }
    }

    /// Whether `other` wraps an entry of the same type that is the same by this entry's
    /// comparison.
    pub fn same(&self, other: &RowEntry) -> bool {
        (self.eq)(&*self.entry, &*other.entry)
    }

    /// The entry, when it is a `T`.
    pub fn get<T: Any>(&self) -> Option<&T> {
        self.entry.downcast_ref()
    }
}

// Equality ignores the entry, so rows compare by what the table shows.
impl PartialEq for RowEntry {
    fn eq(&self, _: &RowEntry) -> bool {
        true
    }
}

impl fmt::Debug for RowEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RowEntry")
    }
}

/// Whether a change of `table` carries rows it no longer shows: a load came between the
/// change and its handling in the same update, so the rows predate that load. Entries count
/// too, so a load that changed only a field the table hides is caught.
pub fn lags(table: &Entity<KvTable>, rows: &[KvRow], cx: &App) -> bool {
    let shown = table.read(cx).rows(cx);
    let same_entry = |a: &KvRow, b: &KvRow| match (&a.entry, &b.entry) {
        (Some(a), Some(b)) => a.same(b),
        (None, None) => true,
        _ => false,
    };
    shown != rows || !shown.iter().zip(rows).all(|(a, b)| same_entry(a, b))
}

/// One row as the table shows it.
#[derive(Debug, Clone, Default)]
pub struct KvRow {
    /// The name.
    pub name: String,
    /// The value.
    pub value: String,
    /// Whether the row is on.
    pub enabled: bool,
    /// The index of the core entry this row came from; `None` for a new row.
    pub source: Option<usize>,
    /// A fixed row keeps its name and has no toggle and no delete.
    pub fixed: bool,
    /// A short label shown beside the row, such as `query` or `path`.
    pub tag: Option<SharedString>,
    /// The picked files; `Some` makes the row a file row.
    pub files: Option<Vec<String>>,
    /// The content type, shown only when the content-type column is on.
    pub content_type: String,
    /// A note on the row; empty for none.
    pub description: String,
    /// The core entry this row was built from; it outlives `source` leaving the document.
    pub entry: Option<RowEntry>,
}

// Rows compare by what the table shows; the entry is ignored, present or not.
impl PartialEq for KvRow {
    fn eq(&self, other: &KvRow) -> bool {
        let KvRow {
            name,
            value,
            enabled,
            source,
            fixed,
            tag,
            files,
            content_type,
            description,
            entry: _,
        } = self;
        *name == other.name
            && *value == other.value
            && *enabled == other.enabled
            && *source == other.source
            && *fixed == other.fixed
            && *tag == other.tag
            && *files == other.files
            && *content_type == other.content_type
            && *description == other.description
    }
}

/// Which optional columns the table shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KvColumns {
    /// The name column; without it the checkbox means "selected".
    pub name: bool,
    /// The content-type column.
    pub content_type: bool,
    /// The files column with its pick button.
    pub files: bool,
    /// The description column, last.
    pub description: bool,
}

impl KvColumns {
    /// Name and value only.
    pub const PLAIN: KvColumns = KvColumns {
        name: true,
        content_type: false,
        files: false,
        description: false,
    };
    /// Name and value with a description, for headers, params and form-urlencoded bodies.
    pub const DESCRIBED: KvColumns = KvColumns {
        name: true,
        content_type: false,
        files: false,
        description: true,
    };
    /// A multipart form: each row is text or files, with a content type.
    pub const MULTIPART: KvColumns = KvColumns {
        name: true,
        content_type: true,
        files: true,
        description: true,
    };
    /// A file body: unnamed file rows with a content type.
    pub const FILE_BODY: KvColumns = KvColumns {
        name: false,
        content_type: true,
        files: true,
        description: false,
    };

    /// The shown columns as (heading id, cell id, heading text) in order: name (`labels.0`),
    /// value (`labels.1`, or "Files" without a name column), content type, description.
    fn shown(
        self,
        labels: (&'static str, &'static str),
    ) -> Vec<(&'static str, &'static str, &'static str)> {
        let value = if self.name { labels.1 } else { "Files" };
        [
            (self.name, "kv-head-name", "kv-name", labels.0),
            (true, "kv-head-value", "kv-value", value),
            (
                self.content_type,
                "kv-head-ctype",
                "kv-ctype",
                "Content type",
            ),
            (
                self.description,
                "kv-head-description",
                "kv-description",
                "Description",
            ),
        ]
        .into_iter()
        .filter(|(on, ..)| *on)
        .map(|(_, head, cell, heading)| (head, cell, heading))
        .collect()
    }

    /// The shown columns in order, all flexible: name (`labels.0`), value (`labels.1`, or
    /// "Files" without a name column), content type, description.
    pub fn describe(self, labels: (&'static str, &'static str)) -> Vec<KvColumn> {
        self.shown(labels)
            .into_iter()
            .map(|(head, cell, heading)| KvColumn::flex(head, cell, heading))
            .collect()
    }

    /// The headings of the shown columns, in the same order as `describe`.
    pub fn headings(self, labels: (&'static str, &'static str)) -> Vec<&'static str> {
        self.shown(labels).into_iter().map(|(_, _, h)| h).collect()
    }
}

/// The user asked to pick files for the row at this index; the owner answers with `set_files`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickFiles(pub usize);

/// What the table reports to its owner.
#[derive(Debug, Clone, PartialEq)]
pub enum KvEvent {
    /// The user changed the rows; carries all of them.
    Changed(Vec<KvRow>),
}

/// How many of a table's togglable rows are enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tri {
    /// Every togglable row is enabled.
    On,
    /// None is, or there are none.
    Off,
    /// Some are and some are not.
    Mixed,
}

/// The state of the heading checkbox over `rows`. Fixed rows have no toggle, so they do not count.
pub fn enabled_state(rows: &[KvRow]) -> Tri {
    tri(rows.iter().filter(|r| !r.fixed).map(|r| r.enabled))
}

fn tri(flags: impl Iterator<Item = bool>) -> Tri {
    let (mut on, mut off) = (false, false);
    for f in flags {
        on |= f;
        off |= !f;
    }
    match (on, off) {
        (true, false) => Tri::On,
        (true, true) => Tri::Mixed,
        _ => Tri::Off,
    }
}

/// The row fields that live outside the two inputs.
struct Meta {
    enabled: bool,
    source: Option<usize>,
    fixed: bool,
    tag: Option<SharedString>,
    files: Option<Vec<String>>,
    entry: Option<RowEntry>,
}

impl Meta {
    /// A row the user just started: on, new, and free to edit.
    fn fresh() -> Meta {
        Meta {
            enabled: true,
            source: None,
            fixed: false,
            tag: None,
            files: None,
            entry: None,
        }
    }
}

struct Row {
    name: Entity<InputState>,
    value: Entity<InputState>,
    content_type: Entity<InputState>,
    description: Entity<InputState>,
    // Not a tab stop; it only sees focus inside the row's checkbox.
    check: FocusHandle,
    // Not a tab stop; it sees focus anywhere in the row, the delete button included.
    within: FocusHandle,
    meta: Meta,
    _subs: [Subscription; 4],
}

impl Row {
    fn inputs(&self) -> [&Entity<InputState>; 4] {
        [
            &self.name,
            &self.value,
            &self.content_type,
            &self.description,
        ]
    }
}

/// An editable table of name and value rows, always ending with an empty row that turns
/// into a real one when typed in.
pub struct KvTable {
    rows: Vec<Row>,
    blank: Row,
    error: Option<String>,
    // The row whose name cell the refusal in `error` is about.
    error_name_row: Option<usize>,
    locked: bool,
    columns: KvColumns,
    placeholders: (&'static str, &'static str),
    /// The content-type cells' placeholder.
    type_hint: &'static str,
    labels: (&'static str, &'static str),
    vars: Option<Arc<VariableSet>>,
    /// The bulk text editor and its change subscription, while bulk mode is on.
    bulk: Option<(Entity<EditorState>, Subscription)>,
    /// The rows bulk text is matched against, so a row deleted and retyped keeps its identity.
    bulk_base: Vec<KvRow>,
    /// Takes keyboard focus when bulk edit drops its focused editor, so keys still reach the
    /// table's ancestors (Save among them).
    focus: FocusHandle,
    /// The dropped bulk editor's focus handle, kept until the next render can move focus off it.
    stranded: Option<FocusHandle>,
    /// Wraps the bulk toggle in both modes, so a keyboard toggle can hand focus back to it.
    toggle_slot: FocusHandle,
    /// Set by a keyboard toggle: the next render hands focus from the slot to the toggle.
    refocus_toggle: bool,
    /// Where each row reports its bounds to the scroll area holding the table.
    follow: FocusFollow,
    /// Whether the name and value cells complete header names and values.
    header_hints: bool,
    /// The `(row, column)` of the cell the last edit was typed in; cleared by any other edit.
    typed: Option<(usize, u8)>,
    /// Where a dragged row would land: the row under the pointer and the side of it.
    drop: Option<(usize, Side)>,
    /// The row whose checkbox takes focus on the next draw, after a move carried it there.
    check_focus: Cell<Option<usize>>,
    /// The row whose Remove takes focus on the next draw, after a reload moved its row.
    remove_focus: Cell<Option<usize>>,
    /// The control that held focus when the pending checkbox or Remove focus was left; the
    /// pending focus is the user's only while it still does.
    pending_origin: Option<FocusHandle>,
}

impl EventEmitter<KvEvent> for KvTable {}
impl EventEmitter<PickFiles> for KvTable {}

impl KvTable {
    /// An empty table.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let placeholders = DEFAULT_PLACEHOLDERS;
        KvTable {
            rows: Vec::new(),
            blank: Self::new_row(
                Default::default(),
                Meta::fresh(),
                placeholders,
                TYPE_HINT,
                window,
                cx,
            ),
            error: None,
            error_name_row: None,
            locked: false,
            columns: KvColumns::PLAIN,
            placeholders,
            type_hint: TYPE_HINT,
            labels: ("Key", "Value"),
            vars: None,
            bulk: None,
            bulk_base: Vec::new(),
            focus: cx.focus_handle(),
            stranded: None,
            toggle_slot: cx.focus_handle(),
            refocus_toggle: false,
            follow: FocusFollow::default(),
            header_hints: false,
            check_focus: Cell::new(None),
            remove_focus: Cell::new(None),
            pending_origin: None,
            typed: None,
            drop: None,
        }
    }

    /// The cell the latest emitted edit was typed in, or `None` when another kind of edit
    /// (add, delete, toggle, files, bulk text) made it.
    pub fn last_typed(&self) -> Option<(usize, u8)> {
        self.typed
    }

    /// Make the name and value cells offer header names and values as the user types; for
    /// the tables that hold request headers.
    pub fn set_header_hints(&mut self, on: bool) {
        self.header_hints = on;
    }

    /// Reports each row's bounds to `follow`, the watch of the scroll area holding the table.
    pub fn set_follow(&mut self, follow: FocusFollow) {
        self.follow = follow;
    }

    /// Use `name` and `value` as the placeholders of every row, the empty row included.
    pub fn set_placeholders(
        &mut self,
        name: &'static str,
        value: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.placeholders = (name, value);
        for row in self.rows.iter().chain([&self.blank]) {
            row.name
                .update(cx, |s, cx| s.set_placeholder(name, window, cx));
            row.value
                .update(cx, |s, cx| s.set_placeholder(value, window, cx));
        }
        cx.notify();
    }

    /// Use `hint` as the placeholder of every content-type cell, the empty row's included.
    pub fn set_content_type_placeholder(
        &mut self,
        hint: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.type_hint = hint;
        for row in self.rows.iter().chain([&self.blank]) {
            row.content_type
                .update(cx, |s, cx| s.set_placeholder(hint, window, cx));
        }
        cx.notify();
    }

    /// Head the name and value columns with `name` and `value`.
    pub fn set_headings(
        &mut self,
        name: &'static str,
        value: &'static str,
        cx: &mut Context<Self>,
    ) {
        self.labels = (name, value);
        cx.notify();
    }

    /// The name and value placeholders; ("Key", "Value") by default.
    pub fn placeholders(&self) -> (&'static str, &'static str) {
        self.placeholders
    }

    /// The columns shown.
    pub fn columns(&self) -> KvColumns {
        self.columns
    }

    /// The column headings in column order: the name label ("Key" by default) when names
    /// show, the value label ("Value" by default, or "Files" when rows list files in place
    /// of a value), "Content type" and "Description" when those columns show.
    pub fn headings(&self) -> Vec<&'static str> {
        self.columns.headings(self.labels)
    }

    /// Show `columns` from now on.
    pub fn set_columns(&mut self, columns: KvColumns, cx: &mut Context<Self>) {
        self.columns = columns;
        if !columns.name {
            self.drop_bulk(cx);
        }
        cx.notify();
    }

    /// Give row `ix` these files, or make it a text row again with `None`; emits the rows.
    /// Files for the empty row at `rows.len()` make it a real row; any other index past the
    /// end does nothing.
    pub fn set_files(
        &mut self,
        ix: usize,
        files: Option<Vec<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if ix == self.rows.len() && files.is_some() {
            self.materialise(window, cx);
        }
        if let Some(row) = self.rows.get_mut(ix) {
            row.meta.files = files;
            self.typed = None;
            self.changed(cx);
        }
    }

    /// Put `content_type` in the content-type cell of row `ix`, or of the empty row at
    /// `rows.len()`, when that cell is blank or holds `stale`. Emits nothing; a following
    /// edit carries it.
    pub fn fill_content_type(
        &mut self,
        ix: usize,
        content_type: &str,
        stale: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = self.row(ix) else {
            return;
        };
        let now = row.content_type.read(cx).value().to_string();
        if now.is_empty() || Some(now.as_str()) == stale {
            set_text(&row.content_type, content_type, window, cx);
        }
    }

    /// The rows as they stand, typed text included; the empty row is never one of them.
    pub fn rows(&self, cx: &App) -> Vec<KvRow> {
        self.rows
            .iter()
            .map(|row| KvRow {
                name: row.name.read(cx).value().to_string(),
                value: row.value.read(cx).value().to_string(),
                enabled: row.meta.enabled,
                source: row.meta.source,
                fixed: row.meta.fixed,
                tag: row.meta.tag.clone(),
                files: row.meta.files.clone(),
                content_type: row.content_type.read(cx).value().to_string(),
                description: row.description.read(cx).value().to_string(),
                entry: row.meta.entry.clone(),
            })
            .collect()
    }

    /// Show `rows` without emitting an event. In bulk mode a load that parses back to what the
    /// text already says is an echo of the user's typing: it keeps the text and the base's text
    /// rows, and takes the rows left out of the text from the load. Any other load replaces both.
    pub fn load(&mut self, rows: Vec<KvRow>, window: &mut Window, cx: &mut Context<Self>) {
        // A Remove or checkbox focus an earlier load left for a draw that has not come yet is
        // where focus belongs; the window's focus still names the old row's control. The flag
        // says whether it is the checkbox.
        self.drop_moved_on_focus(window);
        let pending = (self.remove_focus.take().map(|ix| (ix, false)))
            .or_else(|| self.check_focus.take().map(|ix| (ix, true)));
        // An undo or redo of a move shows the rows reordered; focus goes with the row it was on.
        let followed = self.focused_cell(window, cx).and_then(|(ix, col)| {
            let now = self.rows(cx);
            let to = row_move::follow(&now, &rows, ix, |r| r.fixed, same_content)?;
            Some((to, col))
        });
        if let Some((editor, _)) = &self.bulk {
            let text = editor.read(cx).value().to_string();
            let echo = bulk_text(&rows) == bulk_text(&bulk_rows(&text, &rows));
            if echo {
                // Keep the load's order, so a left-out row holds its slot; text rows keep the
                // base's identity.
                let mut text_rows = self.bulk_base.iter().filter(|row| editable(row)).cloned();
                self.bulk_base = rows
                    .iter()
                    .map(|row| match editable(row) {
                        true => text_rows.next().unwrap_or_else(|| row.clone()),
                        false => row.clone(),
                    })
                    .collect();
            } else {
                let fresh = bulk_text(&rows);
                editor.update(cx, |e, cx| e.set_value(fresh, window, cx));
                self.bulk_base = rows.clone();
            }
        }
        let held = (pending.map(|(ix, _)| ix)).or_else(|| self.focused_row(window, cx));
        let in_cell = pending.is_none() && self.focused_cell(window, cx).is_some();
        // Focus on a row's control (its Remove or checkbox) stays with that row if it survives
        // the load, and moves as a delete moves it if not. Focus in a cell keeps the rules below.
        let on_box = pending.map_or_else(
            || held.is_some_and(|ix| self.rows[ix].check.contains_focused(window, cx)),
            |(_, on_box)| on_box,
        );
        let control = held
            .filter(|_| !in_cell)
            .map(|ix| kv_grid::after_reload(&self.rows(cx), &rows, ix, |r| r.fixed, same_content));
        self.show(rows, window, cx);
        let dropped = match control {
            Some(Reloaded::Kept(to)) if Some(to) != held || pending.is_some() => {
                let target = if on_box {
                    &self.check_focus
                } else {
                    &self.remove_focus
                };
                target.set(Some(to));
                self.pending_origin = window.focused(cx);
                None
            }
            Some(Reloaded::Dropped(ix)) => Some(ix),
            _ => held.filter(|&ix| ix >= self.rows.len()),
        };
        if let Some((to, col)) = followed
            && let Some(row) = self.rows.get(to)
        {
            row.inputs()[col].focus_handle(cx).focus(window, cx);
        } else if let Some(ix) = dropped {
            // The load dropped the focused row (an undone add, a redone delete); focus moves as
            // a delete moves it, so the editor's undo and redo keys keep reaching it.
            // Several trailing rows may go at once; the last one left then takes focus.
            let ix = ix.min(self.rows.len());
            let target = kv_grid::after_delete(&self.rows, ix, &self.blank);
            target.name.clone().focus_handle(cx).focus(window, cx);
        }
    }

    // A pending focus waits for a draw the table may not get while hidden; once focus has
    // left the control it came from, the user moved on and the pending focus is dropped.
    fn drop_moved_on_focus(&mut self, window: &Window) {
        if !(self.pending_origin.as_ref()).is_some_and(|h| h.is_focused(window)) {
            self.remove_focus.set(None);
            self.check_focus.set(None);
            self.pending_origin = None;
        }
    }

    /// Put `rows` in the inputs. Inputs are reused by index and only text that differs is
    /// set, since setting a value moves the caret to the end.
    fn show(&mut self, rows: Vec<KvRow>, window: &mut Window, cx: &mut Context<Self>) {
        self.rows.truncate(rows.len());
        for (ix, row) in rows.into_iter().enumerate() {
            let meta = Meta {
                enabled: row.enabled,
                source: row.source,
                fixed: row.fixed,
                tag: row.tag,
                files: row.files,
                entry: row.entry,
            };
            match self.rows.get_mut(ix) {
                Some(existing) => {
                    set_text(&existing.name, &row.name, window, cx);
                    set_text(&existing.value, &row.value, window, cx);
                    set_text(&existing.content_type, &row.content_type, window, cx);
                    set_text(&existing.description, &row.description, window, cx);
                    existing.meta = meta;
                }
                None => {
                    let texts = [row.name, row.value, row.content_type, row.description];
                    let hints = self.placeholders;
                    let new = Self::new_row(texts, meta, hints, self.type_hint, window, cx);
                    self.rows.push(new);
                }
            }
        }
        cx.notify();
    }

    /// The name input of row `ix`, or of the empty row at `rows.len()`.
    pub fn name_input(&self, ix: usize) -> Option<&Entity<InputState>> {
        self.row(ix).map(|row| &row.name)
    }

    /// The value input of row `ix`, or of the empty row at `rows.len()`.
    pub fn value_input(&self, ix: usize) -> Option<&Entity<InputState>> {
        self.row(ix).map(|row| &row.value)
    }

    /// The content-type input of row `ix`, or of the empty row at `rows.len()`.
    pub fn content_type_input(&self, ix: usize) -> Option<&Entity<InputState>> {
        self.row(ix).map(|row| &row.content_type)
    }

    fn row(&self, ix: usize) -> Option<&Row> {
        self.rows
            .get(ix)
            .or((ix == self.rows.len()).then_some(&self.blank))
    }

    /// Show `message` under the table, or clear it. `name_row` marks that row's name cell,
    /// when the refusal names one; a row that does not exist marks nothing.
    pub fn set_error(
        &mut self,
        message: Option<String>,
        name_row: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        self.error = message;
        self.error_name_row = name_row.filter(|&ix| ix < self.rows.len());
        cx.notify();
    }

    /// Tint and explain the placeholders in the name and value cells of enabled rows while
    /// the cell is editable; `None` turns that off.
    pub fn set_variables(&mut self, vars: Option<Arc<VariableSet>>, cx: &mut Context<Self>) {
        self.vars = vars;
        cx.notify();
    }

    /// Lock or unlock the table; a locked table disables every input, toggle and button.
    /// Locking also leaves bulk mode.
    pub fn set_locked(&mut self, locked: bool, cx: &mut Context<Self>) {
        self.locked = locked;
        if locked {
            self.drop_bulk(cx);
        }
        cx.notify();
    }

    /// Leave bulk mode without a window. If the editor held focus, the next render moves it to
    /// the table so keys still reach it.
    fn drop_bulk(&mut self, cx: &mut Context<Self>) {
        if let Some((editor, _)) = self.bulk.take() {
            self.stranded = Some(editor.read(cx).focus_handle(cx));
        }
    }

    /// Whether the table shows its rows as bulk text.
    pub fn bulk(&self) -> bool {
        self.bulk.is_some()
    }

    /// Switch between the rows and the bulk text; turning it on writes the current rows as text.
    /// Ignored while locked or without a name column.
    pub fn set_bulk(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        if on == self.bulk() || (on && !self.bulk_offered()) {
            return;
        }
        if !on {
            self.bulk = None;
            self.bulk_base.clear();
            self.focus.focus(window, cx);
            cx.notify();
            return;
        }
        self.bulk_base = self.rows(cx);
        let text = bulk_text(&self.bulk_base);
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("text")
                .default_value(text)
        });
        // Every text change is parsed against the base, shown behind the text and emitted.
        let sub = cx.subscribe_in(
            &editor,
            window,
            |t: &mut KvTable, editor, event, window, cx| {
                if matches!(event, InputEvent::Change) {
                    let rows = bulk_rows(&editor.read(cx).value(), &t.bulk_base);
                    t.show(rows.clone(), window, cx);
                    t.typed = None;
                    cx.emit(KvEvent::Changed(rows));
                }
            },
        );
        editor.read(cx).focus_handle(cx).focus(window, cx);
        self.bulk = Some((editor, sub));
        cx.notify();
    }

    /// The bulk text editor, while bulk mode is on.
    pub fn bulk_editor(&self) -> Option<&Entity<EditorState>> {
        self.bulk.as_ref().map(|(editor, _)| editor)
    }

    fn bulk_offered(&self) -> bool {
        !self.locked && self.columns.name
    }

    /// Whether the table is locked.
    pub fn locked(&self) -> bool {
        self.locked
    }

    /// Each input answers a change with the whole row list, so no handler needs a row index
    /// that a delete could make stale. The handler only asks whether the input is the empty
    /// row's, so a row keeps its inputs and subscriptions when it stops being the empty one.
    fn new_row(
        [name, value, content_type, description]: [String; 4],
        meta: Meta,
        (name_hint, value_hint): (&'static str, &'static str),
        type_hint: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Row {
        let mut input = |text: String, hint: &'static str| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(hint)
                    .default_value(text)
            })
        };
        let name = input(name, name_hint);
        let value = input(value, value_hint);
        let content_type = input(content_type, type_hint);
        let description = input(description, "Description");
        let subs = [&name, &value, &content_type, &description].map(|input| {
            cx.subscribe_in(
                input,
                window,
                |table: &mut KvTable, input, event, window, cx| {
                    if matches!(event, InputEvent::Change) {
                        if table.blank.inputs().contains(&input) {
                            table.materialise(window, cx);
                        }
                        table.typed = table.cell_of(input);
                        table.changed(cx);
                    }
                },
            )
        });
        Row {
            name,
            value,
            content_type,
            description,
            check: cx.focus_handle(),
            within: cx.focus_handle(),
            meta,
            _subs: subs,
        }
    }

    /// The row and column of `input` among the real rows.
    fn cell_of(&self, input: &Entity<InputState>) -> Option<(usize, u8)> {
        self.rows.iter().enumerate().find_map(|(ix, row)| {
            let col = row.inputs().iter().position(|i| *i == input)?;
            Some((ix, col as u8))
        })
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(KvEvent::Changed(self.rows(cx)));
        cx.notify();
    }

    /// Make the empty row the last real row and start a fresh empty one below it.
    fn materialise(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let fresh = Self::new_row(
            Default::default(),
            Meta::fresh(),
            self.placeholders,
            self.type_hint,
            window,
            cx,
        );
        let row = std::mem::replace(&mut self.blank, fresh);
        self.rows.push(row);
    }

    /// Remove row `ix` and move focus to the next row's name, else the previous row's, else
    /// the empty row's, so the keys of the editor around the table keep working.
    fn delete(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.locked || ix >= self.rows.len() {
            return;
        }
        self.rows.remove(ix);
        let target = kv_grid::after_delete(&self.rows, ix, &self.blank)
            .name
            .clone();
        target.focus_handle(cx).focus(window, cx);
        self.typed = None;
        self.changed(cx);
    }

    /// The row and input column holding focus, when it is in a real row's inputs.
    fn focused_cell(&self, window: &Window, cx: &App) -> Option<(usize, usize)> {
        self.rows.iter().enumerate().find_map(|(ix, row)| {
            let col = row
                .inputs()
                .iter()
                .position(|i| i.focus_handle(cx).contains_focused(window, cx))?;
            Some((ix, col))
        })
    }

    /// The real row holding focus anywhere in it: a cell, its checkbox or its delete button.
    fn focused_row(&self, window: &Window, cx: &App) -> Option<usize> {
        self.rows
            .iter()
            .position(|row| row.within.contains_focused(window, cx))
    }

    /// Which rows are fixed, for the move helpers that leave them in place.
    fn pinned(&self) -> Vec<bool> {
        self.rows.iter().map(|r| r.meta.fixed).collect()
    }

    /// Move row `from` into the place of row `dest` as one edit. Fixed rows never change
    /// place, so neither end may be one; a locked table or an index outside the rows does
    /// nothing. Focus goes to the moved row's name unless it already sat in that row, and a
    /// checkbox that held it is focused again at the new place.
    pub fn move_row(
        &mut self,
        from: usize,
        dest: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let len = self.rows.len();
        let fixed = |ix: usize| self.rows[ix].meta.fixed;
        if self.locked || from >= len || dest >= len || from == dest || fixed(from) || fixed(dest) {
            return;
        }
        // Only focus in a cell or the checkbox travels with the row; anywhere else in it (a
        // grip press, the Remove button) the moved row's name takes focus.
        let row = &self.rows[from];
        let inside = row.check.contains_focused(window, cx)
            || (row.inputs().iter()).any(|i| i.focus_handle(cx).contains_focused(window, cx));
        let on_box = self.rows[from].check.contains_focused(window, cx)
            && !self.rows[from].check.is_focused(window);
        row_move::reorder_pinned(&mut self.rows, |r| r.meta.fixed, from, dest);
        if on_box {
            self.check_focus.set(Some(dest));
            self.pending_origin = window.focused(cx);
        } else if !inside {
            self.rows[dest].name.focus_handle(cx).focus(window, cx);
        }
        self.drop = None;
        self.typed = None;
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
            .filter(|&s| !self.locked && real_move_target(&self.pinned(), from, to, s).is_some())
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
        if let Some(dest) = real_step_target(&self.pinned(), ix, up) {
            self.move_row(ix, dest, window, cx);
        }
    }

    /// A row dropped on the `side` of row `to` lands there when that moves it.
    fn drop_row(&mut self, from: usize, to: usize, window: &mut Window, cx: &mut Context<Self>) {
        let side = self.drop.filter(|&(at, _)| at == to).map(|(_, s)| s);
        self.drop = None;
        cx.notify();
        if let Some(dest) = side.and_then(|s| real_move_target(&self.pinned(), from, to, s)) {
            self.move_row(from, dest, window, cx);
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

    fn pick(&mut self, ix: usize, cx: &mut Context<Self>) {
        if !self.locked && ix <= self.rows.len() {
            cx.emit(PickFiles(ix));
        }
    }

    /// Turn a file row back into an empty text row.
    fn clear_files(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.locked {
            return;
        }
        if let Some(row) = self.rows.get_mut(ix) {
            row.meta.files = None;
            set_text(&row.value, "", window, cx);
            self.typed = None;
            self.changed(cx);
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
        if self.locked {
            return;
        }
        if let Some(row) = self.rows.get_mut(ix) {
            row.meta.enabled = enabled;
            self.typed = None;
            self.changed(cx);
        }
    }

    /// The heading checkbox's state, over the rows that have a toggle (never the empty row).
    pub fn enabled_state(&self) -> Tri {
        tri(self
            .rows
            .iter()
            .filter(|r| !r.meta.fixed)
            .map(|r| r.meta.enabled))
    }

    /// Turn every togglable row on, or all off when all are on already, as one edit.
    fn toggle_all(&mut self, cx: &mut Context<Self>) {
        if self.locked {
            return;
        }
        let enable = self.enabled_state() != Tri::On;
        let mut touched = false;
        for row in self.rows.iter_mut().filter(|r| !r.meta.fixed) {
            touched |= row.meta.enabled != enable;
            row.meta.enabled = enable;
        }
        if touched {
            self.typed = None;
            self.changed(cx);
        }
    }

    /// Draw row `ix`. A slot the row does not use is laid out hidden under a `kv-slot-*` id so
    /// every column lines up: the empty row's checkbox, tag and delete, a fixed row's checkbox
    /// and delete, and a text row's Clear when the table has file rows.
    fn render_row(
        &self,
        ix: usize,
        row: &Row,
        blank: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let locked = self.locked;
        let columns = self.columns;
        let muted = cx.theme().muted_foreground;
        let fixed = row.meta.fixed;
        let off = !row.meta.enabled;
        let slotted = blank || fixed;
        let remove_id = (
            if slotted {
                "kv-slot-delete"
            } else {
                "kv-delete"
            },
            ix,
        );
        let take_remove_focus = !slotted && self.remove_focus.get() == Some(ix);
        if take_remove_focus {
            self.remove_focus.set(None);
        }
        let clears = self.has_file_rows();
        let has_files = columns.name && row.meta.files.is_some();
        // The empty row reserves a hidden tag slot whenever a real row is tagged.
        let tag = if blank {
            self.rows.iter().rev().find_map(|r| r.meta.tag.clone())
        } else {
            row.meta.tag.clone()
        };
        // A row lists its files in place of the value; a file body always does.
        let files = if columns.name {
            row.meta.files.clone()
        } else {
            Some(row.meta.files.clone().unwrap_or_default())
        };
        // Delete stays drawn so it is always reachable, but only shows on the row under the
        // pointer or the row holding focus in a field or its checkbox.
        let focused = row.check.contains_focused(window, cx)
            || row
                .inputs()
                .iter()
                .any(|input| input.focus_handle(cx).contains_focused(window, cx));
        let cols = columns.describe(self.labels);
        let win: &Window = window;
        // A cell explains its placeholders only while the row is on and the cell editable; its
        // edge lights while the pointer is over it and turns to the ring while it is focused.
        let cell = |input: &Entity<InputState>,
                    (name, ix): (&'static str, usize),
                    (wrapper, explains): (&'static str, bool),
                    edge: &'static str,
                    mark: Option<(&'static str, &'static str)>,
                    disabled: bool,
                    hint: Option<Hint>,
                    cx: &App| {
            let live = !off && !disabled;
            let set = self.vars.clone().filter(|_| explains && live);
            let field = ui::table_input(input, disabled, win, cx)
                .id((name, ix))
                .when(off, |i| i.text_color(muted));
            let focused = input.focus_handle(cx).contains_focused(win, cx);
            let shell = match mark {
                Some((danger, icon)) => kv_grid::kv_cell_marked(
                    ((danger, ix).into(), (icon, ix).into()),
                    field,
                    input,
                    focused,
                    cx,
                ),
                None => kv_grid::kv_cell((edge, ix), field, input, focused, cx),
            };
            let field = VarField::new((wrapper, ix), input, shell, set);
            sized(&cols, name, field.hints(hint.filter(|_| live)))
        };
        // What the name and value cells of a headers table complete, from the other rows' names.
        let (name_hint, value_hint) = if self.header_hints {
            let present = (self.rows.iter().enumerate())
                .filter(|&(i, _)| i != ix)
                .map(|(_, r)| r.name.read(cx).value().to_string())
                .collect();
            let name = row.name.read(cx).value().to_string();
            (Some(Hint::Name { present }), Some(Hint::Value { name }))
        } else {
            (None, None)
        };
        // The cell the last refusal named gets the alert icon, and so does an enabled header
        // whose name could never be sent; the latter is a warning, the save still happens.
        let unsendable = self.header_hints
            && !off
            && apinata_core::invalid_header_name(&row.name.read(cx).value()).is_some();
        let name_mark = ((self.error_name_row == Some(ix) || unsendable) && !blank)
            .then_some(("kv-name-danger-edge", "kv-name-invalid-icon"));
        let owner = cx.entity_id();
        let handle = (!blank && !fixed && !locked).then(|| {
            let label = row.name.read(cx).value().to_string();
            DraggedRow::new(owner, ix, row_move::preview_label(&label).into())
        });
        let landing = self
            .drop
            .filter(|&(at, _)| at == ix && !blank)
            .map(|(_, s)| s);
        kv_row(("kv-row", ix))
            .track_focus(&row.within)
            .h(ROW_HEIGHT)
            .relative()
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
                r.child(drop_line("kv-drop-line", side, cx))
            })
            .child(grip_slot(("kv-grip", ix), ("kv-slot-grip", ix), handle, cx))
            .child(check_slot(
                div()
                    .track_focus(&row.check)
                    .when(slotted, |d| d.invisible())
                    .child(
                        ui::checkbox(
                            (
                                if slotted {
                                    "kv-slot-enabled"
                                } else {
                                    "kv-enabled"
                                },
                                ix,
                            ),
                            row.meta.enabled,
                        )
                        .disabled(locked || fixed)
                        .tab_stop(!slotted)
                        .take_focus(
                            self.check_focus.get() == Some(ix) && {
                                self.check_focus.set(None);
                                true
                            },
                        )
                        .on_click(cx.listener(
                            move |t, on: &bool, window, cx| t.toggle(ix, *on, window, cx),
                        )),
                    ),
            ))
            .when(columns.name, |r| {
                r.child(cell(
                    &row.name,
                    ("kv-name", ix),
                    ("kv-name-vars", true),
                    "kv-name-edge",
                    name_mark,
                    locked || fixed,
                    name_hint,
                    cx,
                ))
            })
            .map(|r| match &files {
                // Padded and bordered like a small input: the names start where typed text
                // does, and an outer box shares out the row width as a cell's wrapper does.
                Some(files) => r.child(sized(
                    &cols,
                    "kv-value",
                    div().child(
                        div()
                            .id(("kv-files", ix))
                            .test_support()
                            .w_full()
                            .px(Size::Small.input_px())
                            .border_1()
                            .border_color(transparent_black())
                            .truncate()
                            .child(files.join(", ")),
                    ),
                )),
                None => r.child(cell(
                    &row.value,
                    ("kv-value", ix),
                    ("kv-value-vars", true),
                    "kv-value-edge",
                    None,
                    locked,
                    value_hint,
                    cx,
                )),
            })
            .when(columns.content_type, |r| {
                r.child(cell(
                    &row.content_type,
                    ("kv-ctype", ix),
                    ("kv-ctype-vars", false),
                    "kv-ctype-edge",
                    None,
                    locked,
                    None,
                    cx,
                ))
            })
            .when(columns.description, |r| {
                r.child(cell(
                    &row.description,
                    ("kv-description", ix),
                    ("kv-description-vars", false),
                    "kv-description-edge",
                    None,
                    locked,
                    None,
                    cx,
                ))
            })
            .when(columns.files, |r| {
                r.child(
                    Button::new(("kv-pick", ix))
                        .label("Files")
                        .lock(locked, cx)
                        .tab_stop(false)
                        .on_click(cx.listener(move |t, _, _, cx| t.pick(ix, cx))),
                )
            })
            .when(clears, |r| {
                r.child(
                    div().flex_none().when(!has_files, |d| d.invisible()).child(
                        Button::new((
                            if has_files {
                                "kv-clear-files"
                            } else {
                                "kv-slot-clear"
                            },
                            ix,
                        ))
                        .label("Clear")
                        .lock(locked || !has_files, cx)
                        .tab_stop(false)
                        .on_click(
                            cx.listener(move |t, _, window, cx| t.clear_files(ix, window, cx)),
                        ),
                    ),
                )
            })
            .when_some(tag, |r, tag| {
                r.child(
                    div()
                        .id((if blank { "kv-slot-tag" } else { "kv-tag" }, ix))
                        .test_support()
                        .when(blank, |d| d.invisible())
                        .flex_none()
                        .w(TAG_WIDTH)
                        .truncate()
                        .font_family(cx.theme().mono_font_family.clone())
                        .text_size(TEXT_SMALL)
                        .text_color(muted)
                        .child(tag),
                )
            })
            .child(action_slot(
                kv_grid::button_focusing(
                    remove_id,
                    ui::icon_button(remove_id, IconName::Close, "Remove row", None)
                        .lock(locked || fixed, cx)
                        .tab_stop(!slotted)
                        .on_click(cx.listener(move |t, _, window, cx| t.delete(ix, window, cx))),
                    take_remove_focus,
                ),
                if slotted {
                    ActionShow::Never
                } else if focused {
                    ActionShow::Always
                } else {
                    ActionShow::OnHover
                },
            ))
    }

    /// Whether any row lists files, so every row reserves the file rows' Clear slot.
    fn has_file_rows(&self) -> bool {
        self.columns.name && self.rows.iter().any(|r| r.meta.files.is_some())
    }

    /// Draw the heading row: an empty check slot, the column headings, the rows' hidden pick
    /// and clear slots under `kv-head-slot-*` ids, the tag heading, and the bulk toggle.
    fn render_heading(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let columns = self.columns;
        let tag = self.rows.iter().rev().find_map(|r| r.meta.tag.clone());
        let state = self.enabled_state();
        let locked = self.locked;
        kv_head("kv-head", columns.describe(self.labels))
            .shared_band()
            .grip_column()
            .check_box(
                ui::checkbox("kv-head-enabled", state == Tri::On)
                    .mixed(state == Tri::Mixed)
                    .accessible_name("Enable all rows")
                    .disabled(locked)
                    .on_click(cx.listener(|t, _, _, cx| t.toggle_all(cx))),
            )
            .when(columns.files, |h| {
                h.extra(
                    div().flex_none().invisible().child(
                        Button::new("kv-head-slot-pick")
                            .label("Files")
                            .tab_stop(false),
                    ),
                )
            })
            .when(self.has_file_rows(), |h| {
                h.extra(
                    div().flex_none().invisible().child(
                        Button::new("kv-head-slot-clear")
                            .label("Clear")
                            .tab_stop(false),
                    ),
                )
            })
            // The tag column takes a heading like any other column, at the rows' width.
            .when_some(tag, |h, _| {
                let (_, muted) = kv_grid::head_colors(cx);
                h.extra(
                    div()
                        .id("kv-head-tag")
                        .test_support()
                        .aria_label("Type")
                        .flex_none()
                        .w(TAG_WIDTH)
                        .text_size(TEXT_SMALL)
                        .text_color(muted)
                        .truncate()
                        .child("Type"),
                )
            })
            .action(self.bulk_toggle(cx).map(IntoElement::into_any_element))
    }

    /// The bulk toggle, sized like a row's delete button so it can take the heading's last slot;
    /// `None` when bulk mode is not offered.
    fn bulk_toggle(&self, cx: &mut Context<Self>) -> Option<Div> {
        let on = self.bulk();
        let (icon, tooltip) = if on {
            (IconName::Rows2, "Key/value edit")
        } else {
            (IconName::Menu, "Bulk edit")
        };
        self.bulk_offered().then(|| {
            div().flex_none().track_focus(&self.toggle_slot).child(
                ui::icon_button("kv-bulk-toggle", icon, tooltip, None).on_click(cx.listener(
                    move |t, event: &ClickEvent, window, cx| {
                        t.set_bulk(!on, window, cx);
                        // A click moves on to what it opened; a key press stays on the
                        // toggle, redrawn in the other mode, so the same key switches back.
                        // The slot holds focus until that redraw.
                        if event.is_keyboard() {
                            t.toggle_slot.focus(window, cx);
                            t.refocus_toggle = true;
                        }
                    },
                )),
            )
        })
    }
}

/// `el` sized as the column whose cells carry `cell_id`, flexible when there is none.
fn sized<E: Styled>(cols: &[KvColumn], cell_id: &str, el: E) -> E {
    match cols.iter().find(|c| c.cell_id == cell_id) {
        Some(col) => col.size(el),
        None => el.flex_1().min_w_0(),
    }
}

// Whether two rows show the same content, wherever they came from.
fn same_content(a: &KvRow, b: &KvRow) -> bool {
    (
        &a.name,
        &a.value,
        a.enabled,
        &a.files,
        &a.content_type,
        &a.description,
    ) == (
        &b.name,
        &b.value,
        b.enabled,
        &b.files,
        &b.content_type,
        &b.description,
    )
}

pub(crate) fn set_text(input: &Entity<InputState>, text: &str, window: &mut Window, cx: &mut App) {
    crate::ui::sync_input_text(input, text, window, cx);
}

impl Focusable for KvTable {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for KvTable {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.drop_moved_on_focus(window);
        if let Some(stranded) = self.stranded.take()
            && stranded.is_focused(window)
        {
            self.focus.focus(window, cx);
        }
        // The toggle sits under a different parent in each mode, so its handle is new and
        // only the finished frame's Tab order reaches it: work deferred from a render runs
        // once this frame is drawn.
        if std::mem::take(&mut self.refocus_toggle) {
            let slot = self.toggle_slot.clone();
            window.defer(cx, move |window, cx| {
                if slot.is_focused(window) {
                    window.focus_next(cx);
                }
            });
        }
        let error = self.error.clone().map(|message| {
            div()
                .id("kv-error")
                .test_support()
                .text_color(cx.theme().danger)
                .child(message)
        });
        if let Some(editor) = self.bulk_editor() {
            let editor = editor.clone();
            return v_flex()
                .gap_1()
                .w_full()
                .track_focus(&self.focus)
                .child(
                    h_flex()
                        .id("kv-head")
                        .test_support()
                        .w_full()
                        .justify_end()
                        .children(self.bulk_toggle(cx)),
                )
                .child(
                    div()
                        .id("kv-bulk-text")
                        .test_support()
                        .w_full()
                        .min_h(style::EDITOR_MIN_HEIGHT)
                        .h(style::EDITOR_HEIGHT)
                        .child(ui::ringed_editor(Editor::new(&editor).h_full(), &editor)),
                )
                .children(error);
        }
        let mut rows: Vec<_> = self
            .rows
            .iter()
            .enumerate()
            .map(|(ix, row)| {
                let row = self.render_row(ix, row, false, window, cx);
                Followed::new(("kv-row-wrapper", ix), &self.follow, row).into_any_element()
            })
            .collect();
        let blank = self.render_row(self.rows.len(), &self.blank, true, window, cx);
        // A Remove focus lasts one draw, so one its row could not take never fires later.
        self.remove_focus.set(None);
        rows.push(
            Followed::new(("kv-row-wrapper", self.rows.len()), &self.follow, blank)
                .into_any_element(),
        );
        v_flex()
            .gap_1()
            .w_full()
            .track_focus(&self.focus)
            .key_context(kv_grid::STRUCTURED_ROWS_CONTEXT)
            .on_action(cx.listener(|t, _: &MoveRowUp, window, cx| t.step(true, window, cx)))
            .on_action(cx.listener(|t, _: &MoveRowDown, window, cx| t.step(false, window, cx)))
            .child(self.render_heading(cx))
            .children(rows)
            .children(error)
    }
}

/// Param rows: each tagged with its kind; path params are fixed, as in Bruno.
pub fn param_rows(params: &[Param]) -> Vec<KvRow> {
    params
        .iter()
        .enumerate()
        .map(|(ix, p)| {
            let path = matches!(p.kind, ParamKind::Path);
            KvRow {
                name: p.name.clone(),
                value: p.value.clone(),
                enabled: p.enabled,
                source: Some(ix),
                entry: Some(RowEntry::with_eq(p.clone(), Param::same_source)),
                fixed: path,
                tag: Some(if path { "path" } else { "query" }.into()),
                description: p.description.clone().unwrap_or_default(),
                ..KvRow::default()
            }
        })
        .collect()
}

/// Params from `rows`: a row with a source edits a clone of that entry, so keys the app
/// does not show survive; a new row is a query param.
pub fn params_from(base: &[Param], rows: &[KvRow]) -> Vec<Param> {
    rows.iter()
        .map(|row| {
            let mut p = source(base, row).unwrap_or_else(|| Param::new("", "", ParamKind::Query));
            p.name.clone_from(&row.name);
            p.value.clone_from(&row.value);
            p.enabled = row.enabled;
            p.description = described(row, p.description.take());
            p
        })
        .collect()
}

/// Header rows.
pub fn header_rows(headers: &[KeyValue]) -> Vec<KvRow> {
    headers
        .iter()
        .enumerate()
        .map(|(ix, h)| KvRow {
            name: h.name.clone(),
            value: h.value.clone(),
            enabled: h.enabled,
            source: Some(ix),
            entry: Some(RowEntry::with_eq(h.clone(), KeyValue::same_source)),
            description: h.description.clone().unwrap_or_default(),
            ..KvRow::default()
        })
        .collect()
}

/// Headers from `rows`, editing clones of their source entries.
pub fn headers_from(base: &[KeyValue], rows: &[KvRow]) -> Vec<KeyValue> {
    rows.iter()
        .map(|row| {
            let mut h = source(base, row).unwrap_or_else(|| KeyValue::new("", ""));
            h.name.clone_from(&row.name);
            h.value.clone_from(&row.value);
            h.enabled = row.enabled;
            h.description = described(row, h.description.take());
            h
        })
        .collect()
}

/// Variable rows; a typed value shows its data.
pub fn var_rows(vars: &[Variable]) -> Vec<KvRow> {
    vars.iter()
        .enumerate()
        .map(|(ix, v)| KvRow {
            name: v.name.clone(),
            value: match &v.value {
                VarValue::Text(text) => text.clone(),
                VarValue::Typed { data, .. } => data.clone(),
                _ => String::new(),
            },
            enabled: v.enabled,
            source: Some(ix),
            entry: Some(RowEntry::with_eq(v.clone(), Variable::same_source)),
            ..KvRow::default()
        })
        .collect()
}

/// Variables from `rows`; a typed source keeps its declared type around the new data.
pub fn vars_from(base: &[Variable], rows: &[KvRow]) -> Vec<Variable> {
    rows.iter()
        .map(|row| {
            let mut v = source(base, row)
                .unwrap_or_else(|| Variable::new("", VarValue::Text(String::new())));
            v.name.clone_from(&row.name);
            v.value = match &v.value {
                VarValue::Typed { kind, .. } => VarValue::Typed {
                    kind: kind.clone(),
                    data: row.value.clone(),
                },
                _ => VarValue::Text(row.value.clone()),
            };
            v.enabled = row.enabled;
            v
        })
        .collect()
}

/// Post-response variable rows: the name and the expression.
pub fn post_response_rows(vars: &[PostResponseVar]) -> Vec<KvRow> {
    vars.iter()
        .enumerate()
        .map(|(ix, v)| KvRow {
            name: v.name().to_owned(),
            value: v.expression().to_owned(),
            enabled: v.enabled(),
            source: Some(ix),
            entry: Some(RowEntry::with_eq(v.clone(), PostResponseVar::same_source)),
            ..KvRow::default()
        })
        .collect()
}

/// Post-response variables from `rows`, editing clones of their source entries so each
/// keeps the YAML it was read from.
pub fn post_response_vars_from(base: &[PostResponseVar], rows: &[KvRow]) -> Vec<PostResponseVar> {
    rows.iter()
        .map(|row| {
            let mut v = source(base, row).unwrap_or_else(|| PostResponseVar::new("", ""));
            v.set_name(row.name.as_str());
            v.set_expression(row.value.as_str());
            v.set_enabled(row.enabled);
            v
        })
        .collect()
}

/// The skip hints of the enabled rows with an expression, in row order.
pub fn post_response_hints(base: &[PostResponseVar], rows: &[KvRow]) -> Vec<String> {
    post_response_vars_from(base, rows)
        .iter()
        .filter(|v| v.enabled() && !v.expression().trim().is_empty())
        .filter_map(PostResponseVar::skip_hint)
        .collect()
}

/// The Post-response block under a Vars table: a muted heading with the id `ids.0`, `table`
/// wrapped in `ids.1`, and one muted line per hint with the id `(ids.2, ix)`.
pub fn post_response_block(
    table: &Entity<KvTable>,
    hints: Vec<String>,
    (heading_id, table_id, hint_id): (&'static str, &'static str, &'static str),
    cx: &App,
) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    v_flex()
        .gap_1()
        .w_full()
        .child(vars_heading(heading_id, "Post-response", cx))
        .child(
            div()
                .id(table_id)
                .test_support()
                .w_full()
                .child(table.clone()),
        )
        .children(hints.into_iter().enumerate().map(|(ix, hint)| {
            div()
                .id((hint_id, ix))
                .aria_label(hint.clone())
                .test_support()
                .text_size(TEXT_SMALL)
                .text_color(muted)
                .child(hint)
        }))
        .into_any_element()
}

/// The muted small heading above a Vars table, shared so the Pre-request and Post-response
/// headings cannot drift apart.
pub fn vars_heading(id: &'static str, text: &'static str, cx: &App) -> AnyElement {
    div()
        .id(id)
        .aria_label(text)
        .test_support()
        .text_size(TEXT_SMALL)
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .into_any_element()
}

// A row's own entry wins over `base[source]`, so hidden fields survive the entry leaving
// the document while the row stays.
fn source<T: Any + Clone>(base: &[T], row: &KvRow) -> Option<T> {
    row.entry
        .as_ref()
        .and_then(RowEntry::get::<T>)
        .or_else(|| row.source.and_then(|ix| base.get(ix)))
        .cloned()
}

// The row's description as core stores it: no text means no description. The single-line
// input drops CR and LF, so text matching `original` without them keeps `original` intact.
fn described(row: &KvRow, original: Option<String>) -> Option<String> {
    let flat = |d: &String| {
        d.chars()
            .filter(|c| !matches!(c, '\r' | '\n'))
            .collect::<String>()
    };
    match original {
        Some(original) if flat(&original) == row.description => Some(original),
        _ => (!row.description.is_empty()).then(|| row.description.clone()),
    }
}

/// Multipart rows: a file field shows its paths and an empty value.
pub fn multipart_rows(fields: &[MultipartField]) -> Vec<KvRow> {
    fields
        .iter()
        .enumerate()
        .map(|(ix, f)| {
            let (value, files) = match &f.value {
                MultipartValue::Files(paths) => (String::new(), Some(paths.clone())),
                MultipartValue::Text(text) => (text.clone(), None),
                _ => (String::new(), None),
            };
            KvRow {
                name: f.name.clone(),
                value,
                enabled: f.enabled,
                source: Some(ix),
                entry: Some(RowEntry::with_eq(f.clone(), MultipartField::same_source)),
                files,
                content_type: f.content_type.clone(),
                description: f.description.clone().unwrap_or_default(),
                ..KvRow::default()
            }
        })
        .collect()
}

/// Multipart fields from `rows`, editing clones of their source entries; a row with files
/// is a file field, any other a text field.
pub fn multipart_from(base: &[MultipartField], rows: &[KvRow]) -> Vec<MultipartField> {
    rows.iter()
        .map(|row| {
            let mut f = source(base, row).unwrap_or_else(|| MultipartField::text("", ""));
            f.name.clone_from(&row.name);
            f.value = match &row.files {
                Some(paths) => MultipartValue::Files(paths.clone()),
                None => MultipartValue::Text(row.value.clone()),
            };
            f.content_type.clone_from(&row.content_type);
            f.enabled = row.enabled;
            f.description = described(row, f.description.take());
            f
        })
        .collect()
}

/// File-body rows: the checkbox is the selection and a blank path lists no file.
pub fn file_rows(files: &[FileEntry]) -> Vec<KvRow> {
    files
        .iter()
        .enumerate()
        .map(|(ix, f)| KvRow {
            enabled: f.selected,
            source: Some(ix),
            entry: Some(RowEntry::with_eq(f.clone(), FileEntry::same_source)),
            files: Some(if f.path.is_empty() {
                Vec::new()
            } else {
                vec![f.path.clone()]
            }),
            content_type: f.content_type.clone(),
            ..KvRow::default()
        })
        .collect()
}

/// File entries from `rows`, keeping one selected as Bruno's `addFile` and `updateFile` do:
/// a newly checked entry deselects every other, and a new entry is selected only when no
/// other entry is.
pub fn files_from(base: &[FileEntry], rows: &[KvRow]) -> Vec<FileEntry> {
    let newly = rows
        .iter()
        .position(|row| row.enabled && source(base, row).is_some_and(|f| !f.selected));
    let mut out: Vec<FileEntry> = rows
        .iter()
        .enumerate()
        .map(|(ix, row)| {
            let path = row
                .files
                .as_ref()
                .and_then(|p| p.first())
                .cloned()
                .unwrap_or_default();
            let (mut f, selected) = match source(base, row) {
                Some(f) => (f, newly.map_or(row.enabled, |n| n == ix)),
                None => (FileEntry::new(""), false),
            };
            f.path = path;
            f.content_type.clone_from(&row.content_type);
            f.selected = selected;
            f
        })
        .collect();
    if !out.iter().any(|f| f.selected)
        && let Some(ix) = rows.iter().position(|row| source(base, row).is_none())
    {
        out[ix].selected = true;
    }
    out
}
