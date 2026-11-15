//! The tree's undo and redo: deletes into the trash, moves and renames, newest first. A move
//! or rename is put back exactly, file names and seqs alike, so an undo leaves no diff.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use apinata_core::ItemPath;
use gpui_kit::component::WindowExt as _;
use gpui_kit::{App, Context, Window, actions};

use super::batch::{Batch, Verb};
use super::{Sidebar, UNDO_DELETE_CAP, UndoEntry, ui};
use crate::command::{Command, Notice, NoticeKind, TreeOp};
use crate::snapshot::DiskState;
use crate::ui::ToastKind;

actions!(
    sidebar,
    [
        /// Redoes the newest tree action Ctrl+Z took back.
        RedoInTree
    ]
);

/// Which stack a finished gesture lands on: an action and a redo go on the undo stack, an
/// undo on the redo stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Step {
    Forward,
    Undo,
    Redo,
}

/// One item a command moved or renamed: where it is now, where it was, and the command, whose
/// record of names and seqs puts it back.
#[derive(Clone, Debug)]
pub(super) struct Placed {
    now: ItemPath,
    then: ItemPath,
    command: u64,
}

/// One entry of the undo or redo stack.
pub(super) enum TreeStep {
    /// Items deleted into the trash.
    Delete(UndoEntry),
    /// Items moved or renamed by one gesture, in the order they were sent.
    Placed {
        root: PathBuf,
        generation: u64,
        items: Vec<Placed>,
    },
    /// Items an undo brought back from the trash, which a redo deletes again.
    Retrash {
        root: PathBuf,
        generation: u64,
        items: Vec<ItemPath>,
    },
}

impl TreeStep {
    fn instance(&self) -> (&Path, u64) {
        match self {
            TreeStep::Delete(d) => (&d.root, d.generation),
            TreeStep::Placed {
                root, generation, ..
            }
            | TreeStep::Retrash {
                root, generation, ..
            } => (root, *generation),
        }
    }
}

// One sent command of a gesture: its item, and where it ended once answered (`None` inside
// when it failed).
struct Sent {
    item: ItemPath,
    outcome: Option<Option<ItemPath>>,
}

/// The moves or renames one gesture sent, until every command has answered.
pub(super) struct Gesture {
    root: PathBuf,
    generation: u64,
    step: Step,
    sent: BTreeMap<u64, Sent>,
}

impl Gesture {
    /// The collection instance the gesture was sent to.
    pub(super) fn instance(&self) -> (PathBuf, u64) {
        (self.root.clone(), self.generation)
    }

    fn finished(&self) -> bool {
        self.sent.values().all(|s| s.outcome.is_some())
    }
}

/// The commands whose records put `items` back.
pub(super) fn commands(items: &[Placed]) -> Vec<u64> {
    items.iter().map(|p| p.command).collect()
}

fn bare(item: &ItemPath) -> &str {
    let name = item.file_name();
    name.strip_suffix(".yml").unwrap_or(name)
}

impl Sidebar {
    /// Sends each op of `ops` (its item and the move or rename) to `root` as one gesture;
    /// returns the ids sent with their items. A new action ends the redo stack.
    pub(super) fn send_placing(
        &mut self,
        root: &Path,
        ops: Vec<(ItemPath, TreeOp)>,
        step: Step,
        cx: &mut Context<Self>,
    ) -> Vec<(u64, ItemPath)> {
        if step == Step::Forward {
            self.clear_redo(cx);
        }
        let mut sent = BTreeMap::new();
        let ids: Vec<(u64, ItemPath)> = self.workspace.update(cx, |w, cx| {
            (ops.into_iter())
                .filter_map(|(item, op)| {
                    let id = w.send(root, Command::Tree(op), cx)?;
                    let outcome = None;
                    sent.insert(
                        id,
                        Sent {
                            item: item.clone(),
                            outcome,
                        },
                    );
                    Some((id, item))
                })
                .collect()
        });
        if let (false, Some(generation)) = (sent.is_empty(), self.generation(root, cx)) {
            (self.gestures).push(Gesture {
                root: root.to_path_buf(),
                generation,
                step,
                sent,
            });
        }
        ids
    }

    // Notes each gesture command's answer: a move gives its new path, a refusal ends it.
    pub(super) fn record_gestures(
        &mut self,
        root: &Path,
        generation: u64,
        notices: &[Notice],
        cx: &mut Context<Self>,
    ) {
        for g in (self.gestures.iter_mut()).filter(|g| g.root == root && g.generation == generation)
        {
            for notice in notices {
                let Some(sent) = g.sent.get_mut(&notice.command) else {
                    continue;
                };
                match &notice.kind {
                    NoticeKind::Moved { to, .. } => sent.outcome = Some(Some(to.clone())),
                    NoticeKind::Failed { .. } | NoticeKind::NotFound(_) | NoticeKind::Stale(_) => {
                        sent.outcome.get_or_insert(None);
                    }
                    _ => {}
                }
            }
        }
        self.finish_gestures(cx);
    }

    // A command answered with no notice kept its path: a reorder, or a rename that kept the
    // file name.
    pub(super) fn settle_gestures(
        &mut self,
        root: &Path,
        generation: u64,
        acked: u64,
        cx: &mut Context<Self>,
    ) {
        for g in (self.gestures.iter_mut()).filter(|g| g.root == root && g.generation == generation)
        {
            for (_, sent) in g.sent.range_mut(..=acked) {
                if sent.outcome.is_none() {
                    sent.outcome = Some(Some(sent.item.clone()));
                }
            }
        }
        self.finish_gestures(cx);
    }

    fn finish_gestures(&mut self, cx: &mut Context<Self>) {
        let (done, open): (Vec<_>, Vec<_>) = std::mem::take(&mut self.gestures)
            .into_iter()
            .partition(Gesture::finished);
        self.gestures = open;
        for g in done {
            let failed: Vec<u64> = (g.sent.iter())
                .filter(|(_, s)| s.outcome == Some(None))
                .map(|(id, _)| *id)
                .collect();
            self.forget_records(&g.root, g.generation, failed, cx);
            let items: Vec<Placed> = (g.sent.into_iter())
                .filter_map(|(command, s)| {
                    Some(Placed {
                        now: s.outcome??,
                        then: s.item,
                        command,
                    })
                })
                .collect();
            if items.is_empty() {
                continue;
            }
            let step = TreeStep::Placed {
                root: g.root,
                generation: g.generation,
                items,
            };
            match g.step {
                Step::Undo => self.redo.push(step),
                Step::Forward | Step::Redo => self.push_history(step, cx),
            }
        }
    }

    // Keeps a step undoable, letting the oldest go past the cap. An evicted delete's tickets
    // are forgotten only by the instance that issued them.
    pub(super) fn push_history(&mut self, step: TreeStep, cx: &mut Context<Self>) {
        self.history.push(step);
        while self.history.len() > UNDO_DELETE_CAP {
            let old = match self.history.remove(0) {
                TreeStep::Delete(old) => old,
                TreeStep::Placed {
                    root,
                    generation,
                    items,
                } => {
                    self.forget_records(&root, generation, commands(&items), cx);
                    continue;
                }
                TreeStep::Retrash { .. } => continue,
            };
            if self.generation(&old.root, cx) == Some(old.generation) {
                self.workspace.update(cx, |w, cx| {
                    for (ticket, _) in &old.trashed {
                        let op = TreeOp::ForgetTrash { ticket: *ticket };
                        w.send(&old.root, Command::Tree(op), cx);
                    }
                });
            }
        }
    }

    /// Ends the redo stack, as a new action does, letting its records go.
    pub(super) fn clear_redo(&mut self, cx: &mut Context<Self>) {
        for step in std::mem::take(&mut self.redo) {
            if let TreeStep::Placed {
                root,
                generation,
                items,
            } = step
            {
                self.forget_records(&root, generation, commands(&items), cx);
            }
        }
    }

    // Tells the instance `generation` of `root` that no step needs these commands' records;
    // a closed instance took them with it.
    pub(super) fn forget_records(
        &mut self,
        root: &Path,
        generation: u64,
        commands: Vec<u64>,
        cx: &mut Context<Self>,
    ) {
        if commands.is_empty() || self.generation(root, cx) != Some(generation) {
            return;
        }
        let op = TreeOp::ForgetRecords { commands };
        self.workspace
            .update(cx, |w, cx| w.send(root, Command::Tree(op), cx));
    }

    // Drops the steps of collection instances no longer open.
    pub(super) fn retain_open_steps(&mut self, cx: &App) {
        let ws = self.workspace.read(cx);
        let open = |(root, generation): (&Path, u64)| {
            (ws.collections().iter()).any(|c| c.root() == root && c.generation() == generation)
        };
        self.history.retain(|s| open(s.instance()));
        self.redo.retain(|s| open(s.instance()));
    }

    /// A finished restore came from an undo, so a redo deletes its items again.
    pub(super) fn note_restored(&mut self, batch: &Batch) {
        let items = batch.restored();
        if !items.is_empty() {
            self.redo.push(TreeStep::Retrash {
                root: batch.root().to_path_buf(),
                generation: batch.generation(),
                items,
            });
        }
    }

    /// Ctrl+Shift+Z in the tree: redoes the newest step Ctrl+Z took back; with none it does
    /// nothing.
    pub fn redo_in_tree(&mut self, _: &RedoInTree, window: &mut Window, cx: &mut Context<Self>) {
        if !self.gestures.is_empty() {
            return;
        }
        self.retain_open_steps(cx);
        match self.redo.pop() {
            Some(TreeStep::Placed {
                root,
                generation,
                items,
            }) => {
                let done = commands(&items);
                self.put_back(&root, generation, items, Step::Redo, window, cx);
                self.forget_records(&root, generation, done, cx);
            }
            Some(TreeStep::Retrash { root, items, .. }) => {
                self.send_deletes(&root, items, true, window, cx);
            }
            Some(TreeStep::Delete(_)) | None => {}
        }
    }

    // Puts each item back where it was, last sent first, so each command's record of names and
    // seqs meets the tree as that command found it. When one item is gone, changed on disk, or
    // would land on a path a file now holds, it says so and sends nothing.
    pub(super) fn put_back(
        &mut self,
        root: &Path,
        generation: u64,
        items: Vec<Placed>,
        step: Step,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.generation(root, cx) != Some(generation) {
            return;
        }
        let Some(snap) = self.workspace.read(cx).snapshot(root, cx) else {
            return;
        };
        let row = |p: &ItemPath| snap.content.rows.iter().find(|r| r.path == *p);
        let blocked = items.iter().find_map(|p| match row(&p.now) {
            _ if !p.now.to_path(root).exists() => {
                Some(format!("\"{}\" is no longer there", bare(&p.now)))
            }
            Some(r) if r.disk == DiskState::Present => (p.then != p.now
                && p.then.to_path(root).exists())
            .then(|| format!("\"{}\" already exists", bare(&p.then))),
            _ => Some(format!("\"{}\" changed on disk", bare(&p.now))),
        });
        if let Some(why) = blocked {
            let verb = if step == Step::Undo { "undo" } else { "redo" };
            let text = format!("Could not {verb}: {why}.");
            window.push_notification(ui::kind_toast(ToastKind::Error, text), cx);
            return;
        }
        let ops: Vec<(ItemPath, TreeOp)> = (items.into_iter().rev())
            .map(|p| {
                let op = TreeOp::Place {
                    item: p.now.clone(),
                    to: p.then,
                    before: p.command,
                };
                (p.now, op)
            })
            .collect();
        let sent = self.send_placing(root, ops, step, cx);
        // Several items report once, as the gesture that moved them did.
        if sent.len() > 1 {
            let ids: Vec<u64> = sent.iter().map(|(id, _)| *id).collect();
            self.workspace.update(cx, |w, _| w.track_batch(root, &ids));
            (self.batches).push(Batch::new(Verb::Move, root.to_path_buf(), generation, sent));
        }
    }

    /// Whether the tree has a step to undo: a delete still landing, or a finished step of a
    /// collection instance still open.
    pub fn can_undo_in_tree(&self, cx: &App) -> bool {
        let ws = self.workspace.read(cx);
        let open = |(root, generation): (&Path, u64)| {
            (ws.collections().iter()).any(|c| c.root() == root && c.generation() == generation)
        };
        self.batches.iter().any(|b| b.awaits_undo())
            || self.history.iter().any(|s| open(s.instance()))
    }

    /// Whether the tree has a step Ctrl+Z took back that a redo can apply again.
    pub fn can_redo_in_tree(&self, cx: &App) -> bool {
        let ws = self.workspace.read(cx);
        let open = |(root, generation): (&Path, u64)| {
            (ws.collections().iter()).any(|c| c.root() == root && c.generation() == generation)
        };
        self.redo.iter().any(|s| open(s.instance()))
    }
}
