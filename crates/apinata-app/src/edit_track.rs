//! The bookkeeping an editor shares around its [`EditHistory`]: deciding whether a sent
//! edit becomes an undo step, and applying the next step.
//!
//! An editor passes a `view_edit` reader (the field's value in its latest snapshot, as an
//! edit, or `None` for a kind that is not undoable) and, to step, a `holds` check. Everything
//! that is specific to an editor stays with it.

use std::collections::HashMap;
use std::hash::Hash;

use crate::command::{Edit, Notice, NoticeKind};
use crate::edit_history::{EditHistory, Typing};
use crate::kv_table::{header_rows, param_rows, post_response_rows, var_rows};

// What an edit holds, in a form that can be compared: the core types have no equality, so
// lists compare through the rows they show.
fn shape(edit: &Edit) -> String {
    match edit {
        Edit::Params(v) => format!("{:?}", param_rows(v)),
        Edit::Headers(v) => format!("{:?}", header_rows(v)),
        Edit::Vars(v) => format!("{:?}", var_rows(v)),
        Edit::PostResponseVars(v) => format!("{:?}", post_response_rows(v)),
        Edit::Assertions(v) => format!("{:?}", crate::assertions_section::assertion_rows(v)),
        Edit::Variables(v) => format!("{:?}", crate::env_table::env_rows(v)),
        other => format!("{other:?}"),
    }
}

/// Whether two edits of one field hold the same value.
pub fn same_value(a: &Edit, b: &Edit) -> bool {
    match (a, b) {
        (Edit::Body(a), Edit::Body(b)) => a == b,
        (Edit::Auth(a), Edit::Auth(b)) => a == b,
        // The file keeps a script without its edge whitespace, so the view reads it trimmed.
        (Edit::Script(pa, a), Edit::Script(pb, b)) => {
            pa == pb && apinata_core::script_code(a) == apinata_core::script_code(b)
        }
        _ => shape(a) == shape(b),
    }
}

// The value last sent for the edit's field, if it is the same kind of value.
fn sent_for<'a>(history: &'a EditHistory, edit: &Edit) -> Option<&'a Edit> {
    history
        .last_sent(edit.field())
        .filter(|last| match (last, edit) {
            // A URL edit shares the params' field but is not a params value.
            (Edit::Params(_), Edit::Params(_)) => true,
            (Edit::Params(_), _) | (_, Edit::Params(_)) => false,
            _ => true,
        })
}

/// The value the field holds as far as the editor knows: the last one sent, else the view's.
pub fn prior_of(
    history: &EditHistory,
    view_edit: &dyn Fn(&Edit) -> Option<Edit>,
    edit: &Edit,
) -> Option<Edit> {
    match sent_for(history, edit) {
        Some(last) => Some(last.clone()),
        None => view_edit(edit),
    }
}

/// Rebases the history onto a field that moved on outside it, before `edit` is tracked. Only
/// an edit in flight (`busy`) makes the view lag what was sent; on an idle field a view that
/// differs was reloaded, so the field's steps are dropped and its next step starts from the view.
pub fn rebase(
    history: &mut EditHistory,
    busy: bool,
    view_edit: &dyn Fn(&Edit) -> Option<Edit>,
    edit: &Edit,
) {
    if busy {
        return;
    }
    let Some(sent) = sent_for(history, edit) else {
        return;
    };
    if view_edit(edit).is_some_and(|view| !same_value(sent, &view)) {
        history.moved_on(edit.field());
    }
}

// The rows the edit leaves, for telling a keystroke that creates a row from one that edits it.
fn row_count(edit: &Edit) -> usize {
    use apinata_core::BodySpec;
    match edit {
        Edit::Params(v) => v.len(),
        Edit::Headers(v) => v.len(),
        Edit::Vars(v) => v.len(),
        Edit::PostResponseVars(v) => v.len(),
        Edit::Assertions(v) => v.len(),
        Edit::Variables(v) => v.len(),
        Edit::Body(BodySpec::FormUrlEncoded(v)) => v.len(),
        Edit::Body(BodySpec::Multipart(v)) => v.len(),
        _ => 0,
    }
}

/// Records a structured edit as an undo step, with the value it replaces taken from the
/// history (the last value sent) before the lagging view. An edit that changes nothing
/// leaves the field and its steps alone; one without a known earlier value is only noted, so
/// the history knows the field moved on. `typed` is the cell a keystroke landed in; `busy`
/// says the field has an edit in flight, as for [`rebase`].
pub fn track(
    history: &mut EditHistory,
    edit: &Edit,
    typed: Option<(usize, u8)>,
    busy: bool,
    view_edit: &dyn Fn(&Edit) -> Option<Edit>,
) {
    rebase(history, busy, view_edit, edit);
    record(history, edit, typed, view_edit);
}

/// [`track`] for a caller that already ran [`rebase`] for this edit.
pub fn record(
    history: &mut EditHistory,
    edit: &Edit,
    typed: Option<(usize, u8)>,
    view_edit: &dyn Fn(&Edit) -> Option<Edit>,
) {
    let Some(before) = prior_of(history, view_edit, edit) else {
        history.note_sent(edit);
        return;
    };
    if same_value(&before, edit) {
        return;
    }
    // Changing the auth mode is a step of its own, never part of a typing run.
    let switch = matches!((&before, edit), (Edit::Auth(a), Edit::Auth(b))
        if std::mem::discriminant(a) != std::mem::discriminant(b));
    let typing = typed.filter(|_| !switch).map(|key| Typing {
        key,
        rows: row_count(edit),
    });
    history.record(before, edit.clone(), typing);
}

/// The edit to send for the next undo (or redo) step, if any. While the field has an edit in
/// flight (`busy`) the view lags, so the history trusts its own last sent value; otherwise
/// the view must still hold what the step left, as `holds` says.
pub fn step(
    history: &mut EditHistory,
    undo: bool,
    busy: bool,
    holds: &dyn Fn(&Edit) -> bool,
) -> Option<Edit> {
    let matches: Option<&dyn Fn(&Edit) -> bool> = if busy { None } else { Some(holds) };
    if undo {
        history.undo(matches)
    } else {
        history.redo(matches)
    }
}

/// A refused edit never took effect, so the history forgets what it did.
pub fn refused(history: &mut EditHistory, notice: &Notice) {
    if matches!(notice.kind, NoticeKind::Failed { .. }) {
        history.refused(notice.command);
    }
}

/// The snapshots each field's controls loaded, last and before last, for stamping the edits
/// built from them so the actor can hold back one a reload overtook.
#[derive(Debug)]
pub struct LoadStamps<K> {
    stamps: HashMap<K, (u64, u64)>,
}

impl<K> Default for LoadStamps<K> {
    fn default() -> Self {
        LoadStamps {
            stamps: HashMap::new(),
        }
    }
}

impl<K: Eq + Hash> LoadStamps<K> {
    /// Notes that `key` loaded from snapshot `seq`; another load of the same snapshot keeps
    /// the earlier one as the load before last.
    pub fn note(&mut self, key: K, seq: u64) {
        let loads = self.stamps.entry(key).or_insert((seq, seq));
        if loads.1 != seq {
            *loads = (loads.1, seq);
        }
    }

    /// The snapshot an edit of `key` was built from: its last load, or the one before when
    /// the edit's rows are `lagging` (they predate the last load); `None` before any load.
    pub fn basis(&self, key: &K, lagging: bool) -> Option<u64> {
        let &(prior, last) = self.stamps.get(key)?;
        Some(if lagging { prior } else { last })
    }
}
