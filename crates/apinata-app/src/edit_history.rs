//! The undo and redo stack of a request editor's structured edits.

use std::collections::HashMap;

use crate::command::{Edit, Field};
use crate::edit_track::same_value;

/// Where a keystroke landed: the cell `(row, column)` and how many rows the table had.
/// Consecutive edits with equal typing merge into one step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Typing {
    /// The row and column of the cell typed in.
    pub key: (usize, u8),
    /// The row count after the edit; the keystroke that creates a row never merges backwards.
    pub rows: usize,
}

#[derive(Clone)]
struct Step {
    id: u64,
    before: Edit,
    after: Edit,
    typing: Option<Typing>,
    /// The send stamp of the value this step currently leaves: `after` while undoable,
    /// `before` while redoable.
    stamp: u64,
}

/// What the last history change did, kept per command so a refused command can be undone
/// exactly: only that change, nothing a later command did.
enum Change {
    /// A new step went on the undo stack, clearing `redo`.
    Pushed { id: u64, redo: Vec<Step> },
    /// A keystroke merged into the step `id`, which held `after` at `stamp` before.
    Merged {
        id: u64,
        after: Edit,
        stamp: u64,
        redo: Vec<Step>,
    },
    /// The step moved from the undo stack to the redo stack.
    Undid(Step),
    /// The step moved from the redo stack to the undo stack.
    Redid(Step),
    /// An edit with no step: only the sent value changed.
    Unrecorded,
}

struct Reversal {
    change: Change,
    field: Field,
    /// The stamp this change gave the field's sent value, and the entry it replaced.
    stamp: u64,
    prior: Option<(Edit, u64)>,
}

/// Steps of structured edits, each a `before` and an `after` value of one field.
///
/// A step is valid only while the field still holds the value the step leaves. The history
/// stamps every value it sends, so an edit it did not record (or one from outside) shows up
/// as a stamp it does not know.
#[derive(Default)]
pub struct EditHistory {
    undo: Vec<Step>,
    redo: Vec<Step>,
    sent: HashMap<Field, (Edit, u64)>,
    clock: u64,
    next_id: u64,
    /// The change of the command being built, until [`Self::bind`] ties it to an id.
    staged: Option<Reversal>,
    pending: HashMap<u64, Reversal>,
}

impl EditHistory {
    /// The value last sent for `field`, whether recorded or not.
    pub fn last_sent(&self, field: Field) -> Option<&Edit> {
        self.sent.get(&field).map(|(edit, _)| edit)
    }

    /// Note an edit sent without a step, so a step on its field stops being valid.
    pub fn note_sent(&mut self, edit: &Edit) -> u64 {
        let (stamp, prior) = self.stamp(edit);
        self.staged = Some(Reversal {
            change: Change::Unrecorded,
            field: edit.field(),
            stamp,
            prior,
        });
        stamp
    }

    // Stamps and stores `edit` as the field's sent value, returning the entry it replaced.
    fn stamp(&mut self, edit: &Edit) -> (u64, Option<(Edit, u64)>) {
        self.clock += 1;
        let prior = self.sent.insert(edit.field(), (edit.clone(), self.clock));
        (self.clock, prior)
    }

    /// Record `before` to `after` as a step and clear redo. With the same `typing` as the
    /// top step the two merge, keeping the older `before`.
    pub fn record(&mut self, before: Edit, after: Edit, typing: Option<Typing>) {
        let redo = std::mem::take(&mut self.redo);
        let field = after.field();
        let (stamp, prior) = self.stamp(&after);
        let merges =
            |top: &Step| typing.is_some() && top.typing == typing && top.after.field() == field;
        let change = match self.undo.last_mut() {
            Some(top) if merges(top) => Change::Merged {
                id: top.id,
                after: std::mem::replace(&mut top.after, after),
                stamp: std::mem::replace(&mut top.stamp, stamp),
                redo,
            },
            _ => {
                self.next_id += 1;
                let id = self.next_id;
                self.undo.push(Step {
                    id,
                    before,
                    after,
                    typing,
                    stamp,
                });
                Change::Pushed { id, redo }
            }
        };
        self.staged = Some(Reversal {
            change,
            field,
            stamp,
            prior,
        });
    }

    /// Ends the open typing run, so the next keystroke starts a step of its own: a save or a
    /// reload is a point undo must stop at.
    pub fn end_typing(&mut self) {
        if let Some(top) = self.undo.last_mut() {
            top.typing = None;
        }
    }

    /// The field the next undo (or redo) would write, so the caller can check it first.
    pub fn next_field(&self, undo: bool) -> Option<Field> {
        let stack = if undo { &self.undo } else { &self.redo };
        stack.last().map(|step| step.after.field())
    }

    /// Ties the change just made to command `id`, so [`Self::refused`] can reverse it. With
    /// no id the command was never sent, and the change is reversed at once.
    pub fn bind(&mut self, id: Option<u64>) {
        match (self.staged.take(), id) {
            (Some(reversal), Some(id)) => {
                self.pending.insert(id, reversal);
            }
            (Some(reversal), None) => self.reverse(reversal),
            (None, _) => {}
        }
    }

    /// Reverses the change command `id` made, after the actor refused it: an undone step
    /// returns to the undo stack, a keystroke leaves the step it merged into, a new step goes.
    pub fn refused(&mut self, id: u64) {
        if let Some(reversal) = self.pending.remove(&id) {
            self.reverse(reversal);
        }
    }

    /// Reverses the change made but not yet bound, for a command that is not going out.
    pub fn revert_staged(&mut self) {
        if let Some(reversal) = self.staged.take() {
            self.reverse(reversal);
        }
    }

    /// Forgets the commands up to `acked`, whose refusals have been delivered already.
    pub fn settle(&mut self, acked: u64) {
        self.pending.retain(|&id, _| id > acked);
    }

    // Each arm touches only what the change itself moved, and only while that is still
    // where the change left it; a later change to the same step wins.
    fn reverse(&mut self, reversal: Reversal) {
        let Reversal {
            change,
            field,
            stamp,
            prior,
        } = reversal;
        match change {
            Change::Pushed { id, redo } => {
                self.undo.retain(|step| step.id != id);
                if self.redo.is_empty() {
                    self.redo = redo;
                }
            }
            Change::Merged {
                id,
                after,
                stamp: earlier,
                redo,
            } => {
                if let Some(step) = self
                    .undo
                    .iter_mut()
                    .find(|s| s.id == id && s.stamp == stamp)
                {
                    step.after = after;
                    step.stamp = earlier;
                }
                if self.redo.is_empty() {
                    self.redo = redo;
                }
            }
            Change::Undid(step) => {
                if let Some(at) = self.redo.iter().position(|s| s.id == step.id) {
                    self.redo.remove(at);
                    self.undo.push(step);
                }
            }
            Change::Redid(step) => {
                if let Some(at) = self.undo.iter().position(|s| s.id == step.id) {
                    self.undo.remove(at);
                    self.redo.push(step);
                }
            }
            Change::Unrecorded => {}
        }
        // The field kept its old value, so the old sent value is current again.
        if self.sent.get(&field).is_some_and(|(_, now)| *now == stamp) {
            match prior {
                Some(prior) => self.sent.insert(field, prior),
                None => self.sent.remove(&field),
            };
        }
    }

    /// Whether there is a step to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether there is a step to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    // A field that moved on retires its own steps only. The URL bar rewrites the params, so
    // a stale params step must not take unrelated steps (headers, vars) down with it.
    fn forget(&mut self, field: Field) {
        self.undo.retain(|s| s.after.field() != field);
        self.redo.retain(|s| s.after.field() != field);
    }

    /// Forgets a field that changed outside the history (a reload): its steps and its sent
    /// value, so the next step starts from what the field holds now.
    pub fn moved_on(&mut self, field: Field) {
        self.forget(field);
        self.sent.remove(&field);
    }

    /// Forget every step and sent value.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.sent.clear();
        self.staged = None;
        self.pending.clear();
    }

    /// The edit that undoes the top step, now owed to the collection. `view_matches` says
    /// whether the field in the view equals an edit; pass `None` while commands are in
    /// flight, when only the last sent value is current. A field that moved on loses its
    /// steps and returns `None`; other fields keep theirs.
    pub fn undo(&mut self, view_matches: Option<&dyn Fn(&Edit) -> bool>) -> Option<Edit> {
        let step = self.undo.pop()?;
        if !self.leaves(&step, &step.after, view_matches) {
            self.forget(step.after.field());
            return None;
        }
        let edit = step.before.clone();
        let (stamp, prior) = self.stamp(&edit);
        self.redo.push(Step {
            typing: None,
            stamp,
            ..step.clone()
        });
        self.staged = Some(Reversal {
            change: Change::Undid(step),
            field: edit.field(),
            stamp,
            prior,
        });
        Some(edit)
    }

    /// The edit that redoes the last undone step; `view_matches` as for [`Self::undo`].
    pub fn redo(&mut self, view_matches: Option<&dyn Fn(&Edit) -> bool>) -> Option<Edit> {
        let step = self.redo.pop()?;
        if !self.leaves(&step, &step.before, view_matches) {
            self.forget(step.before.field());
            return None;
        }
        let edit = step.after.clone();
        let (stamp, prior) = self.stamp(&edit);
        self.undo.push(Step {
            typing: None,
            stamp,
            ..step.clone()
        });
        self.staged = Some(Reversal {
            change: Change::Redid(step),
            field: edit.field(),
            stamp,
            prior,
        });
        Some(edit)
    }

    /// Retires a step the field's own editor undid or redid: when `edit` is the value the top
    /// undo step came from (or the top redo step leads to) and the field still holds the value
    /// that step leaves, the step moves to the other stack without anything to send. False
    /// leaves the history alone.
    pub fn follow_editor(&mut self, edit: &Edit) -> bool {
        let field = edit.field();
        let holds = |this: &Self, value: &Edit| {
            this.sent
                .get(&field)
                .is_some_and(|(sent, _)| same_value(sent, value))
        };
        let undid = self.undo.last().is_some_and(|top| {
            top.after.field() == field && same_value(&top.before, edit) && holds(self, &top.after)
        });
        if undid && let Some(step) = self.undo.pop() {
            self.move_step(step, edit, true);
            return true;
        }
        let redid = self.redo.last().is_some_and(|top| {
            top.after.field() == field && same_value(&top.after, edit) && holds(self, &top.before)
        });
        if redid && let Some(step) = self.redo.pop() {
            self.move_step(step, edit, false);
            return true;
        }
        false
    }

    // Moves a step the editor itself undid (or redid) to the other stack, stamping `edit` as
    // the field's value, with nothing to send.
    fn move_step(&mut self, step: Step, edit: &Edit, undid: bool) {
        let field = edit.field();
        let (stamp, prior) = self.stamp(edit);
        let moved = Step {
            typing: None,
            stamp,
            ..step.clone()
        };
        let change = if undid {
            self.redo.push(moved);
            Change::Undid(step)
        } else {
            self.undo.push(moved);
            Change::Redid(step)
        };
        self.staged = Some(Reversal {
            change,
            field,
            stamp,
            prior,
        });
    }

    // The field must still hold the value `left` that the step leaves.
    fn leaves(
        &self,
        step: &Step,
        left: &Edit,
        view_matches: Option<&dyn Fn(&Edit) -> bool>,
    ) -> bool {
        match view_matches {
            Some(matches) => matches(left),
            None => self
                .sent
                .get(&left.field())
                .is_some_and(|(_, stamp)| *stamp == step.stamp),
        }
    }
}
