//! Copy, cut and paste of sidebar items. The clipboard lives in the app; the system clipboard
//! only gets the items' file paths as text.

use std::path::{Path, PathBuf};

use apinata_core::{DropTarget, ItemPath, PasteSource, TreeError};
use gpui_kit::component::WindowExt as _;
use gpui_kit::{ClipboardItem, Context, Window};

use super::batch::{Batch, Verb};
use super::model::{Node, NodeKind};
use super::multi::row_keys;
use super::{CopySelected, CutSelected, PasteSelected, Sidebar, ui};
use crate::command::{Command, Notice, NoticeKind, TreeOp};
use crate::ui::ToastKind;

/// Whether a clipboard holds copies or items to move on paste.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipMode {
    /// Pasting adds a copy and keeps the source.
    Copy,
    /// Pasting moves the source.
    Cut,
}

/// The items last copied or cut, all from one collection.
#[derive(Debug, Clone)]
pub struct Clipboard {
    /// Copy or cut.
    pub mode: ClipMode,
    /// The collection the items live in.
    pub root: PathBuf,
    /// The top-level items, in sidebar order.
    pub items: Vec<ItemPath>,
    /// The open generation of `root` the items were taken from.
    pub generation: u64,
}

/// The folder a paste on `node` goes into: a folder is itself, a request its folder, a
/// collection its root. Other rows take no paste.
pub fn paste_target(node: &Node) -> Option<(PathBuf, ItemPath)> {
    let folder = match node.kind {
        NodeKind::Collection => ItemPath::root(),
        NodeKind::Folder => node.path.clone(),
        NodeKind::Request | NodeKind::Opaque => node.path.parent()?,
        _ => return None,
    };
    Some((node.root.clone(), folder))
}

impl Clipboard {
    /// Whether pasting into `folder` of the collection at `root` would put a copied folder
    /// inside itself.
    pub fn blocks_paste_into(&self, root: &Path, folder: &ItemPath) -> bool {
        self.root == root
            && self.items.iter().any(|item| {
                folder == item || folder.as_str().starts_with(&format!("{}/", item.as_str()))
            })
    }
}

impl Clipboard {
    /// The reason a paste into `folder` is refused, if it is: core's message for a folder
    /// pasted into itself.
    pub fn refusal(&self, root: &Path, folder: &ItemPath) -> Option<String> {
        if !self.blocks_paste_into(root, folder) {
            return None;
        }
        let item = self.items.first().cloned().unwrap_or_else(ItemPath::root);
        Some(TreeError::IntoItself { item }.to_string())
    }
}

impl Sidebar {
    /// Whether a paste on `node` is refused because the clipboard holds a folder that contains
    /// the target.
    pub fn paste_blocked(&self, node: &Node) -> bool {
        let (Some(clip), Some((root, folder))) = (&self.clipboard, paste_target(node)) else {
            return false;
        };
        clip.blocks_paste_into(&root, &folder)
    }

    /// How many items the clipboard holds.
    pub fn clipboard_len(&self) -> usize {
        self.clipboard.as_ref().map_or(0, |c| c.items.len())
    }

    /// Whether the clipboard holds copies or a cut, if it holds anything.
    pub fn clipboard_mode(&self) -> Option<ClipMode> {
        self.clipboard.as_ref().map(|c| c.mode)
    }

    /// Copy the selected requests and folders. Pasting adds copies and keeps the sources.
    pub fn copy_selection(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.take(ClipMode::Copy, cx);
    }

    /// Cut the selected requests and folders. They move only when pasted.
    pub fn cut_selection(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.take(ClipMode::Cut, cx);
    }

    /// Paste into the folder under the selected row (see [`paste_target`]).
    pub fn paste_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.selected_node(cx).and_then(paste_target);
        if let Some((root, folder)) = target {
            self.paste(&root, folder, window, cx);
        }
    }

    /// Action form of [`Self::copy_selection`].
    pub fn copy_action(&mut self, _: &CopySelected, window: &mut Window, cx: &mut Context<Self>) {
        self.copy_selection(window, cx);
    }

    /// Action form of [`Self::cut_selection`].
    pub fn cut_action(&mut self, _: &CutSelected, window: &mut Window, cx: &mut Context<Self>) {
        self.cut_selection(window, cx);
    }

    /// Action form of [`Self::paste_selected`].
    pub fn paste_action(&mut self, _: &PasteSelected, window: &mut Window, cx: &mut Context<Self>) {
        self.paste_selected(window, cx);
    }

    /// Copy or cut the row a context menu opened on, which need not be the focused one.
    pub fn copy_row(&mut self, node: &Node, mode: ClipMode, cx: &mut Context<Self>) {
        let ok = matches!(
            node.kind,
            NodeKind::Request | NodeKind::Folder | NodeKind::Opaque
        );
        if ok {
            self.fill_clipboard(mode, node.root.clone(), vec![node.path.clone()], cx);
        }
    }

    // The selection the clipboard takes: the top-level members of a multi-selection, else the
    // focused row when it is a request, folder or opaque item.
    fn copy_set(&self, cx: &Context<Self>) -> Option<(PathBuf, Vec<ItemPath>)> {
        if self.multi.is_active() {
            let root = self.multi.keys().first()?.root.clone();
            let snapshot = self.workspace.read(cx).snapshot(&root, cx)?;
            let keys = self
                .multi
                .normalised(&row_keys(&root, &snapshot.content.rows));
            let items = keys.into_iter().map(|k| k.path).collect();
            return Some((root, items));
        }
        let node = self.selected_node(cx)?;
        matches!(
            node.kind,
            NodeKind::Request | NodeKind::Folder | NodeKind::Opaque
        )
        .then(|| (node.root.clone(), vec![node.path.clone()]))
    }

    fn take(&mut self, mode: ClipMode, cx: &mut Context<Self>) {
        let Some((root, items)) = self.copy_set(cx) else {
            return;
        };
        self.fill_clipboard(mode, root, items, cx);
    }

    fn fill_clipboard(
        &mut self,
        mode: ClipMode,
        root: PathBuf,
        items: Vec<ItemPath>,
        cx: &mut Context<Self>,
    ) {
        if items.is_empty() {
            return;
        }
        let text: Vec<String> = items
            .iter()
            .map(|i| i.to_path(&root).display().to_string())
            .collect();
        let Some(generation) = self.generation(&root, cx) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text.join("\n")));
        self.clipboard = Some(Clipboard {
            mode,
            root,
            items,
            generation,
        });
        cx.notify();
    }

    /// Paste the clipboard into the folder `folder` of the collection at `root`. A cut inside
    /// one collection moves; every other paste writes copies of the saved files, and a cut
    /// across collections then deletes each source whose copy was written.
    pub fn paste(
        &mut self,
        root: &Path,
        folder: ItemPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(clip) = self.clipboard.clone() else {
            return;
        };
        // A cut whose source instance closed is spent, even if the observer has not run yet.
        if clip.mode == ClipMode::Cut && self.generation(&clip.root, cx) != Some(clip.generation) {
            self.clipboard = None;
            cx.notify();
            return;
        }
        if let Some(message) = clip.refusal(root, &folder) {
            // The same refusal core gives a copy, so a cut says it too instead of failing
            // one move at a time.
            window.push_notification(
                gpui_kit::component::notification::Notification::error(message),
                cx,
            );
            return;
        }
        if clip.mode == ClipMode::Cut && clip.root == root {
            self.move_items(root, &clip.items, folder, cx);
            return;
        }
        let (tx, rx) = async_channel::bounded(1);
        let items = clip.items.clone();
        let command = Command::CopyOut { items, reply: tx };
        if self
            .workspace
            .update(cx, |w, cx| w.send(&clip.root, command, cx))
            .is_none()
        {
            return;
        }
        // A cut deletes only from the instance its copies come from, never a reopened one.
        let (target, cut_from) = (
            root.to_path_buf(),
            (clip.mode == ClipMode::Cut).then_some((clip.root, clip.generation)),
        );
        cx.spawn_in(window, async move |this, cx| {
            let Ok(copies) = rx.recv().await else {
                return;
            };
            this.update_in(cx, |s, window, cx| {
                s.paste_copies(&target, folder, cut_from, copies, window, cx)
            })
            .ok();
        })
        .detach();
    }

    // A cut pasted in its own collection: each item that is not already in `folder` moves there.
    fn move_items(
        &mut self,
        root: &Path,
        items: &[ItemPath],
        folder: ItemPath,
        cx: &mut Context<Self>,
    ) {
        let sent: Vec<(u64, ItemPath)> = self.workspace.update(cx, |w, cx| {
            items
                .iter()
                .filter(|i| i.parent().as_ref() != Some(&folder))
                .filter_map(|item| {
                    let op = TreeOp::Move {
                        item: item.clone(),
                        to: DropTarget::Inside(folder.clone()),
                    };
                    Some((w.send(root, Command::Tree(op), cx)?, item.clone()))
                })
                .collect()
        });
        if let Some(generation) = self.generation(root, cx) {
            for (id, item) in &sent {
                (self.moves).insert((root.to_path_buf(), generation, *id), item.clone());
            }
        }
        self.track(Verb::Move, root, sent, cx);
    }

    fn track(
        &mut self,
        verb: Verb,
        root: &Path,
        sent: Vec<(u64, ItemPath)>,
        cx: &mut Context<Self>,
    ) {
        if sent.is_empty() {
            return;
        }
        let ids: Vec<u64> = sent.iter().map(|(id, _)| *id).collect();
        self.workspace.update(cx, |w, _| w.track_batch(root, &ids));
        let generation = self.generation(root, cx).unwrap_or(u64::MAX);
        (self.batches).push(Batch::new(verb, root.to_path_buf(), generation, sent));
    }

    // The source collection answered: write each copy into `folder`. The batch says once, when
    // it finishes, how many pasted copies left their source's unsaved edits out.
    fn paste_copies(
        &mut self,
        root: &Path,
        folder: ItemPath,
        cut_from: Option<(PathBuf, u64)>,
        copies: Vec<(ItemPath, Result<PasteSource, String>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The cut's source closed while its copies were on the way: writing them now would
        // only duplicate items that were never going to be deleted.
        if let Some((from, source)) = &cut_from
            && self.generation(from, cx) != Some(*source)
        {
            if (self.clipboard.as_ref()).is_some_and(|c| c.mode == ClipMode::Cut && c.root == *from)
            {
                self.clipboard = None;
            }
            cx.notify();
            return;
        }
        let mut dirty = Vec::new();
        let mut failed = Vec::new();
        let mut sent: Vec<(u64, ItemPath)> = Vec::new();
        let generation = self.generation(root, cx).unwrap_or(u64::MAX);
        for (item, copy) in copies {
            let source = match copy {
                Ok(source) => source,
                Err(why) => {
                    failed.push(format!("{} ({why})", item.file_name()));
                    continue;
                }
            };
            let unsaved = source.has_unsaved_edits();
            let op = TreeOp::Paste {
                target: folder.clone(),
                source,
            };
            let id = self
                .workspace
                .update(cx, |w, cx| w.send(root, Command::Tree(op), cx));
            if let Some(id) = id {
                // A source with unsaved edits is never deleted: the edits would be lost.
                if let Some(from) = cut_from.as_ref().filter(|_| !unsaved) {
                    let key = (root.to_path_buf(), generation, id);
                    let (from, source) = from.clone();
                    (self.cuts).insert(key, (from, source, item.clone()));
                }
                if unsaved {
                    dirty.push(id);
                }
                sent.push((id, item));
            }
        }
        if !failed.is_empty() {
            // Nothing sent means the paste failed outright; the rest may still land.
            let kind = if sent.is_empty() {
                ToastKind::Error
            } else {
                ToastKind::Warning
            };
            let text = format!("Could not paste: {}", failed.join(", "));
            window.push_notification(ui::kind_toast(kind, text), cx);
        }
        if let Some((id, _)) = sent.last() {
            let row = super::PendingRow {
                rename: false,
                notice: None,
            };
            (self.pending).insert((root.to_path_buf(), generation, *id), row);
        }
        self.track(Verb::Paste, root, sent, cx);
        if let Some(batch) = self.batches.last_mut().filter(|_| !dirty.is_empty()) {
            batch.note_unsaved(dirty, cut_from.is_some());
        }
    }

    // A cut pasted into another collection: once a copy is written its source is deleted, and
    // the item leaves the clipboard. A copy that failed leaves its source and cut state alone.
    // A cut moved inside its collection leaves the clipboard only once the move is confirmed.
    pub(super) fn finish_cuts(
        &mut self,
        root: &Path,
        generation: u64,
        notices: &[Notice],
        cx: &mut Context<Self>,
    ) {
        if self.cuts.is_empty() && self.moves.is_empty() {
            return;
        }
        for notice in notices {
            let key = (root.to_path_buf(), generation, notice.command);
            let Some(item) = self.moves.remove(&key) else {
                continue;
            };
            if matches!(notice.kind, NoticeKind::Moved { .. })
                && let Some(clip) = &mut self.clipboard
                && clip.root == root
            {
                clip.items.retain(|i| *i != item);
            }
        }
        let mut deletes: Vec<(PathBuf, ItemPath)> = Vec::new();
        for notice in notices {
            let key = (root.to_path_buf(), generation, notice.command);
            match &notice.kind {
                // A source closed since the copy was taken keeps its item, and the clipboard too.
                NoticeKind::Created(_) => {
                    if let Some((from, source, item)) = self.cuts.remove(&key)
                        && self.generation(&from, cx) == Some(source)
                    {
                        deletes.push((from, item));
                    }
                }
                NoticeKind::Failed { .. } | NoticeKind::NotFound(_) | NoticeKind::Stale(_) => {
                    self.cuts.remove(&key);
                }
                _ => {}
            }
        }
        for (from, item) in &deletes {
            if let Some(clip) = &mut self.clipboard
                && clip.root == *from
            {
                clip.items.retain(|i| i != item);
            }
        }
        if self.clipboard.as_ref().is_some_and(|c| c.items.is_empty()) {
            self.clipboard = None;
        }
        let mut roots: Vec<PathBuf> = deletes.iter().map(|(r, _)| r.clone()).collect();
        roots.dedup();
        for from in roots {
            let sent: Vec<(u64, ItemPath)> = self.workspace.update(cx, |w, cx| {
                deletes
                    .iter()
                    .filter(|(r, _)| *r == from)
                    .filter_map(|(_, item)| {
                        let op = TreeOp::Delete { item: item.clone() };
                        Some((w.send(&from, Command::Tree(op), cx)?, item.clone()))
                    })
                    .collect()
            });
            self.track(Verb::Delete, &from, sent, cx);
        }
    }
}
