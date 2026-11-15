//! The tree's undo and redo: deletes into the trash, moves and renames, newest first. Each
//! step is put back through the same tree commands the action itself sent.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use apinata_core::{DropTarget, ItemPath};
use gpui_kit::component::WindowExt as _;
use gpui_kit::{App, Context, Window, actions};

use super::batch::{Batch, Verb};
use super::{Sidebar, UNDO_DELETE_CAP, UndoEntry, ui};
use crate::command::{Command, Notice, NoticeKind, TreeOp};
use crate::snapshot::{DiskState, Row};
use crate::ui::ToastKind;

actions!(
    sidebar,
    [
        /// Redoes the newest tree action Ctrl+Z took back.
        RedoInTree
    ]
);

/// How a move or rename is put back.
#[derive(Clone, Debug)]
pub(super) enum Back {
    /// Move the item to this spot.
    Move(DropTarget),
    /// Rename the item, and its file, to this name.
    Rename(String),
}

/// Which stack a finished gesture lands on: an action and a redo go on the undo stack, an
/// undo on the redo stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Step {
    Forward,
    Undo,
    Redo,
}

/// One item a gesture moved or renamed: where it is now, where it was, and how to put it back.
#[derive(Clone, Debug)]
pub(super) struct Placed {
    at: ItemPath,
    was: ItemPath,
    back: Back,
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

// One sent command of a gesture: its item, how to put it back, and where it ended once
// answered (`None` inside when it failed).
struct Sent {
    item: ItemPath,
    back: Back,
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

// Where to drop each of `items` to put it back where it is now, when they are put back one by
// one in this order: after the sibling above it that is already in place, else before the
// first sibling below that never moved, else into its folder.
fn back_targets(rows: &[Row], items: &[ItemPath]) -> Vec<Back> {
    items
        .iter()
        .enumerate()
        .map(|(ix, item)| {
            let parent = item.parent().unwrap_or_else(ItemPath::root);
            let siblings: Vec<&ItemPath> = (rows.iter())
                .map(|r| &r.path)
                .filter(|p| p.parent().as_ref() == Some(&parent))
                .collect();
            let placed = |p: &ItemPath| !items.contains(p) || items[..ix].contains(p);
            let at = siblings.iter().position(|p| *p == item).unwrap_or(0);
            let above = siblings[..at].iter().rev().find(|p| placed(p));
            let below = (siblings.iter().skip(at + 1)).find(|p| !items.contains(p));
            Back::Move(match (above, below) {
                (Some(p), _) => DropTarget::After((*p).clone()),
                (None, Some(p)) => DropTarget::Before((*p).clone()),
                (None, None) => DropTarget::Inside(parent),
            })
        })
        .collect()
}

fn bare(item: &ItemPath) -> &str {
    let name = item.file_name();
    name.strip_suffix(".yml").unwrap_or(name)
}

impl Sidebar {
    /// Where each of `items` of `root` would be put back to, as a move, read before it moves.
    pub(super) fn move_backs(
        &self,
        root: &Path,
        items: &[ItemPath],
        cx: &Context<Self>,
    ) -> Vec<Back> {
        match self.workspace.read(cx).snapshot(root, cx) {
            Some(snap) => back_targets(&snap.content.rows, items),
            None => (items.iter())
                .map(|i| {
                    Back::Move(DropTarget::Inside(
                        i.parent().unwrap_or_else(ItemPath::root),
                    ))
                })
                .collect(),
        }
    }

    /// Sends each op of `ops` (its item, the move or rename, and how to put it back) to `root`
    /// as one gesture; returns the ids sent with their items. A new action ends the redo stack.
    pub(super) fn send_placing(
        &mut self,
        root: &Path,
        ops: Vec<(ItemPath, TreeOp, Back)>,
        step: Step,
        cx: &mut Context<Self>,
    ) -> Vec<(u64, ItemPath)> {
        if step == Step::Forward {
            self.redo.clear();
        }
        let mut sent = BTreeMap::new();
        let ids: Vec<(u64, ItemPath)> = self.workspace.update(cx, |w, cx| {
            (ops.into_iter())
                .filter_map(|(item, op, back)| {
                    let id = w.send(root, Command::Tree(op), cx)?;
                    sent.insert(
                        id,
                        Sent {
                            item: item.clone(),
                            back,
                            outcome: None,
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
            let items: Vec<Placed> = (g.sent.into_values())
                .filter_map(|s| {
                    Some(Placed {
                        at: s.outcome??,
                        was: s.item,
                        back: s.back,
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
            let TreeStep::Delete(old) = self.history.remove(0) else {
                continue;
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
            }) => self.put_back(&root, generation, items, Step::Redo, window, cx),
            Some(TreeStep::Retrash { root, items, .. }) => {
                self.send_deletes(&root, items, true, window, cx);
            }
            Some(TreeStep::Delete(_)) | None => {}
        }
    }

    // Puts each item back as its step says, in the order they were sent, or, when one of them
    // is gone, changed on disk, or would land on a path a file now holds, says so and sends
    // nothing.
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
        let rows = &snap.content.rows;
        let verb = if step == Step::Undo { "undo" } else { "redo" };
        let row = |p: &ItemPath| rows.iter().find(|r| r.path == *p);
        let blocked = items.iter().find_map(|p| match row(&p.at) {
            _ if !p.at.to_path(root).exists() => {
                Some(format!("\"{}\" is no longer there", bare(&p.at)))
            }
            Some(r) if r.disk == DiskState::Present => (p.was != p.at
                && p.was.to_path(root).exists())
            .then(|| format!("\"{}\" already exists", bare(&p.was))),
            _ => Some(format!("\"{}\" changed on disk", bare(&p.at))),
        });
        if let Some(why) = blocked {
            let text = format!("Could not {verb}: {why}.");
            window.push_notification(ui::kind_toast(ToastKind::Error, text), cx);
            return;
        }
        let ats: Vec<ItemPath> = items.iter().map(|p| p.at.clone()).collect();
        let moves = back_targets(rows, &ats);
        let ops: Vec<(ItemPath, TreeOp, Back)> = (items.into_iter().zip(moves))
            .map(|(p, again)| {
                let (op, again) = match p.back {
                    Back::Move(to) => (
                        TreeOp::Move {
                            item: p.at.clone(),
                            to,
                        },
                        again,
                    ),
                    Back::Rename(name) => {
                        let now = row(&p.at).map_or_else(String::new, |r| r.name.clone());
                        let op = TreeOp::Rename {
                            item: p.at.clone(),
                            name,
                            rename_file: true,
                        };
                        (op, Back::Rename(now))
                    }
                };
                (p.at, op, again)
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
}
