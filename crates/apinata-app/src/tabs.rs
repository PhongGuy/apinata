//! The open tab list: order, the active tab, the single preview tab and the reopen stack.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use apinata_core::ItemPath;

use crate::name_hints::{Shown, hints, item_place};
use crate::snapshot::Content;
use crate::stale::rebase;

/// A tab's identity; ids come from a counter and are never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(pub u64);

/// Which side of an anchor tab a dragged tab lands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// Left of the anchor.
    Before,
    /// Right of the anchor.
    After,
}

/// What a tab shows.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TabKind {
    /// A request file of the collection.
    Request(ItemPath),
    /// A folder's settings.
    Folder(ItemPath),
    /// The collection's settings.
    Collection,
    /// The collection's environments.
    Environments,
    /// The runner of a folder, or of the whole collection at `ItemPath::root()`.
    Runner(ItemPath),
    /// A run over a multi-selection: sorted, deduplicated, never kept for reopening a closed tab.
    RunSelection(Vec<ItemPath>),
    /// Saved example `index` of the request `item`, shown read-only.
    Example {
        /// The request holding the example.
        item: ItemPath,
        /// The example's position in the request's `examples`.
        index: usize,
    },
}

/// One open tab.
#[derive(Debug, Clone, PartialEq)]
pub struct TabEntry {
    /// The tab's identity.
    pub id: TabId,
    /// The collection root the tab belongs to.
    pub root: PathBuf,
    /// What the tab shows.
    pub kind: TabKind,
    /// Whether this is the preview tab, which the next preview open replaces.
    pub preview: bool,
    /// Whether the tab is pinned. Pinned tabs always form a prefix of the tab list, and the
    /// bulk close commands (Close all, Close other, Close saved) skip them.
    pub pinned: bool,
}

impl TabEntry {
    /// The request item this tab shows; `Some` for a request tab.
    pub fn item(&self) -> Option<&ItemPath> {
        match &self.kind {
            TabKind::Request(path) => Some(path),
            _ => None,
        }
    }
}

/// The muted detail each tab shows after its name, by index: its hint by the shared rule
/// where another open tab shows the same label, `None` otherwise. `labels` and `places` hold
/// each entry's label and place, in order; a request tab's file name tells siblings apart.
pub fn tab_details(
    entries: &[TabEntry],
    labels: &[String],
    places: &[Vec<String>],
) -> Vec<Option<String>> {
    let shown: Vec<Shown> = (entries.iter().zip(labels).zip(places))
        .map(|((entry, label), place)| Shown {
            name: label.clone(),
            place: place.clone(),
            file: match &entry.kind {
                TabKind::Request(item) | TabKind::Example { item, .. } => {
                    Some(item.file_name().to_owned())
                }
                _ => None,
            },
        })
        .collect();
    hints(&shown)
}

/// Where a tab's subject sits, for its hint: the collection's `tag`, then the folders above
/// the request, folder or example it shows, and an example's request; collection-wide tabs
/// and drafts have the tag alone.
pub fn tab_place(entry: &TabEntry, tag: &str, content: Option<&Content>) -> Vec<String> {
    match &entry.kind {
        // A draft lives in no folder until it is saved, whichever folder it was started from.
        TabKind::Request(path) if path.is_draft() => vec![tag.to_owned()],
        TabKind::Request(path) | TabKind::Folder(path) | TabKind::Runner(path) => {
            item_place(tag, content, path)
        }
        TabKind::Example { item, .. } => {
            let mut place = item_place(tag, content, item);
            let request = content.and_then(|c| c.requests.get(item));
            place.push(request.map_or_else(|| item.file_name().to_owned(), |r| r.name.clone()));
            place
        }
        TabKind::Collection | TabKind::Environments | TabKind::RunSelection(_) => {
            vec![tag.to_owned()]
        }
    }
}

/// The ordered tab list with its active tab and a stack of closed tabs for reopening.
#[derive(Debug, Default)]
pub struct Tabs {
    entries: Vec<TabEntry>,
    active: Option<TabId>,
    closed: Vec<(PathBuf, TabKind)>,
    next: u64,
    // Where each sent draft tab's history is kept; dropped with the tab.
    history_keys: HashMap<TabId, ItemPath>,
    // A collection made current by hand; the next explicit activation hands the current
    // collection back to the active tab, while a close never does.
    chosen_root: Option<PathBuf>,
}

impl Tabs {
    /// An empty tab list.
    pub fn new() -> Tabs {
        Tabs::default()
    }

    /// The open tabs, in display order.
    pub fn entries(&self) -> &[TabEntry] {
        &self.entries
    }

    /// The open tab with this id.
    pub fn get(&self, id: TabId) -> Option<&TabEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// The active tab, if any tab is open.
    pub fn active(&self) -> Option<&TabEntry> {
        self.active.and_then(|id| self.get(id))
    }

    /// The collection made current by hand since the last explicit activation, if any.
    pub fn chosen_root(&self) -> Option<&Path> {
        self.chosen_root.as_deref()
    }

    /// Makes `root` current by hand, or with `None` drops the choice.
    pub fn choose_root(&mut self, root: Option<PathBuf>) {
        self.chosen_root = root;
    }

    // An explicit activation, even of the tab already active, ends a hand-made choice.
    fn set_active(&mut self, id: TabId) {
        self.active = Some(id);
        self.chosen_root = None;
    }

    /// Open `kind` in `root` and activate it. An already open item is reused (and pinned by a
    /// permanent open); a new preview replaces the existing preview in place, unremembered;
    /// anything else goes at the end.
    pub fn open(&mut self, root: &Path, kind: TabKind, preview: bool) -> TabId {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|e| e.root == root && e.kind == kind)
        {
            entry.preview &= preview;
            let id = entry.id;
            self.set_active(id);
            return id;
        }
        let entry = TabEntry {
            id: TabId(self.next),
            root: root.to_path_buf(),
            kind,
            preview,
            pinned: false,
        };
        self.next += 1;
        let id = entry.id;
        match self
            .entries
            .iter_mut()
            .find(|e| e.preview)
            .filter(|_| preview)
        {
            Some(slot) => {
                self.history_keys.remove(&slot.id);
                *slot = entry;
            }
            None => self.entries.push(entry),
        }
        self.set_active(id);
        id
    }

    /// Puts back a tab that closed with its target at position `ix` (clamped), without
    /// activating it. It comes back as a preview only while no other preview is open, and
    /// pinned tabs stay a prefix. An already open tab of `kind` is returned as it is.
    pub fn reopen_at(
        &mut self,
        root: &Path,
        kind: TabKind,
        ix: usize,
        preview: bool,
        pinned: bool,
    ) -> TabId {
        if let Some(e) = self
            .entries
            .iter()
            .find(|e| e.root == root && e.kind == kind)
        {
            return e.id;
        }
        let entry = TabEntry {
            id: TabId(self.next),
            root: root.to_path_buf(),
            kind,
            preview: preview && !pinned && !self.entries.iter().any(|e| e.preview),
            pinned,
        };
        self.next += 1;
        let id = entry.id;
        self.entries.insert(ix.min(self.entries.len()), entry);
        self.keep_pinned_first();
        id
    }

    /// The history key of draft tab `id`, once a send has given it one.
    pub fn history_key(&self, id: TabId) -> Option<&ItemPath> {
        self.history_keys.get(&id)
    }

    /// Keep tab `id`'s sends under `key` from now on; ignored when the tab is not open.
    pub fn set_history_key(&mut self, id: TabId, key: ItemPath) {
        if self.get(id).is_some() {
            self.history_keys.insert(id, key);
        }
    }

    /// Removes and returns tab `id`'s history key, as when its draft is saved.
    pub fn take_history_key(&mut self, id: TabId) -> Option<ItemPath> {
        self.history_keys.remove(&id)
    }

    /// Make a preview tab permanent. Not to be confused with [`Tabs::set_pinned`], which
    /// pins a tab to the front of the strip.
    pub fn keep(&mut self, id: TabId) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
            entry.preview = false;
        }
    }

    /// Pin or unpin tab `id`; `false` when it is not open or already in that state.
    ///
    /// Pinned tabs are a prefix of the list. Pinning moves the tab to the end of the pinned
    /// group and makes a preview tab permanent; unpinning moves it to the start of the
    /// unpinned group, right after the tabs that stay pinned. Both land at the same index
    /// once the tab is taken out, which is the number of pinned tabs left.
    pub fn set_pinned(&mut self, id: TabId, pinned: bool) -> bool {
        let Some(from) = self.entries.iter().position(|e| e.id == id) else {
            return false;
        };
        if self.entries[from].pinned == pinned {
            return false;
        }
        let mut entry = self.entries.remove(from);
        entry.pinned = pinned;
        if pinned {
            entry.preview = false;
        }
        let at = self.entries.iter().take_while(|e| e.pinned).count();
        self.entries.insert(at, entry);
        true
    }

    /// Activate the tab with this id; `false` when it is not open.
    pub fn activate(&mut self, id: TabId) -> bool {
        let found = self.get(id).is_some();
        if found {
            self.set_active(id);
        }
        found
    }

    /// Activate the tab at position `ix`; `false` when there is none.
    pub fn activate_index(&mut self, ix: usize) -> bool {
        match self.entries.get(ix) {
            Some(entry) => {
                self.set_active(entry.id);
                true
            }
            None => false,
        }
    }

    /// Activate the last tab; `false` when no tab is open.
    pub fn activate_last(&mut self) -> bool {
        self.entries
            .len()
            .checked_sub(1)
            .is_some_and(|ix| self.activate_index(ix))
    }

    /// Move the active tab `delta` places, wrapping around either end.
    pub fn activate_offset(&mut self, delta: isize) {
        let len = self.entries.len() as isize;
        if let Some(ix) = self.active_index() {
            self.activate_index((ix as isize + delta).rem_euclid(len) as usize);
        }
    }

    /// Move the tab `id` to just after `anchor`; nothing changes when either is not open.
    pub fn move_after(&mut self, id: TabId, anchor: TabId) {
        if id == anchor || self.get(anchor).is_none() {
            return;
        }
        let Some(from) = self.entries.iter().position(|e| e.id == id) else {
            return;
        };
        let entry = self.entries.remove(from);
        let to = self
            .entries
            .iter()
            .position(|e| e.id == anchor)
            .map_or(0, |ix| ix + 1);
        self.entries.insert(to, entry);
        self.keep_pinned_first();
    }

    /// Move the tab `id` to just before `anchor`; nothing changes when either is not open.
    pub fn move_before(&mut self, id: TabId, anchor: TabId) {
        if id == anchor || self.get(anchor).is_none() {
            return;
        }
        let Some(from) = self.entries.iter().position(|e| e.id == id) else {
            return;
        };
        let entry = self.entries.remove(from);
        let to = self
            .entries
            .iter()
            .position(|e| e.id == anchor)
            .unwrap_or(self.entries.len());
        self.entries.insert(to, entry);
        self.keep_pinned_first();
    }

    /// Drag-reorder: move `id` to `side` of `anchor` when both are open in the same
    /// collection and the same group (both pinned or both not). Returns whether the order
    /// changed.
    pub fn move_to(&mut self, id: TabId, anchor: TabId, side: Side) -> bool {
        // A drag never crosses the pinned boundary: such a drop is refused, and the strip
        // shows no landing place for it.
        let same_root = match (self.get(id), self.get(anchor)) {
            (Some(a), Some(b)) => a.root == b.root && a.pinned == b.pinned,
            _ => false,
        };
        if !same_root || id == anchor {
            return false;
        }
        let before: Vec<TabId> = self.entries.iter().map(|e| e.id).collect();
        match side {
            Side::Before => self.move_before(id, anchor),
            Side::After => self.move_after(id, anchor),
        }
        self.entries.iter().map(|e| e.id).ne(before)
    }

    /// Close a tab and remember it for reopening, unless it is a runner or a draft; the right
    /// neighbour, else the left, becomes active when it was the active tab.
    pub fn close(&mut self, id: TabId) -> Option<TabEntry> {
        let ix = self.entries.iter().position(|e| e.id == id)?;
        let entry = self.entries.remove(ix);
        self.history_keys.remove(&id);
        if self.active == Some(id) {
            self.active = self.entries.get(ix).or(self.entries.last()).map(|e| e.id);
        }
        let draft = entry.item().is_some_and(ItemPath::is_draft);
        if !matches!(entry.kind, TabKind::Runner(_) | TabKind::RunSelection(_)) && !draft {
            self.closed.push((entry.root.clone(), entry.kind.clone()));
        }
        Some(entry)
    }

    /// Whether a closed tab is waiting to be reopened.
    pub fn has_closed(&self) -> bool {
        !self.closed.is_empty()
    }

    /// The most recently closed tab that has not been reopened yet.
    pub fn pop_closed(&mut self) -> Option<(PathBuf, TabKind)> {
        self.closed.pop()
    }

    /// Example `index` of `item` in `root` was deleted: close its tab, without remembering it
    /// for reopen, and move the tabs of later examples of that request down one index.
    pub fn forget_example(&mut self, root: &Path, item: &ItemPath, index: usize) {
        let gone = TabKind::Example {
            item: item.clone(),
            index,
        };
        self.retain(|e| e.root != root || e.kind != gone);
        self.closed.retain(|(r, k)| r != root || *k != gone);
        for kind in self.kinds_of(root) {
            if let TabKind::Example { item: of, index: n } = kind
                && of == item
                && *n > index
            {
                *n -= 1;
            }
        }
    }

    /// Rebase open and remembered request, folder, runner and example tabs of `root` at or
    /// under `from` onto `to`, after a move or rename.
    pub fn follow_move(&mut self, root: &Path, from: &ItemPath, to: &ItemPath) {
        for kind in self.kinds_of(root) {
            // A selection rebases every member, then stays sorted and free of duplicates.
            if let TabKind::RunSelection(items) = kind {
                for item in items.iter_mut() {
                    if let Some(moved) = rebase(item, from, to) {
                        *item = moved;
                    }
                }
                items.sort();
                items.dedup();
                continue;
            }
            let (TabKind::Request(path)
            | TabKind::Folder(path)
            | TabKind::Runner(path)
            | TabKind::Example { item: path, .. }) = kind
            else {
                continue;
            };
            if let Some(moved) = rebase(path, from, to) {
                *path = moved;
            }
        }
    }

    /// Keep only the tabs `keep` accepts and return the rest, without remembering them for
    /// reopening. A removed active tab hands over to its nearest kept right neighbour, else left.
    pub fn retain(&mut self, mut keep: impl FnMut(&TabEntry) -> bool) -> Vec<TabEntry> {
        let active_ix = self.active_index();
        let (kept, gone): (Vec<_>, Vec<_>) = self
            .entries
            .drain(..)
            .enumerate()
            .partition(|(_, e)| keep(e));
        if let Some(ix) = active_ix.filter(|_| gone.iter().any(|(_, e)| Some(e.id) == self.active))
        {
            let right = kept.iter().find(|(i, _)| *i > ix);
            self.active = right.or(kept.last()).map(|(_, e)| e.id);
        }
        self.entries = kept.into_iter().map(|(_, e)| e).collect();
        for (_, e) in &gone {
            self.history_keys.remove(&e.id);
        }
        gone.into_iter().map(|(_, e)| e).collect()
    }

    // The kinds of `root`'s open and remembered tabs, for edits that rewrite both alike.
    fn kinds_of<'a>(&'a mut self, root: &'a Path) -> impl Iterator<Item = &'a mut TabKind> {
        self.entries
            .iter_mut()
            .filter(move |e| e.root == root)
            .map(|e| &mut e.kind)
            .chain(
                self.closed
                    .iter_mut()
                    .filter(move |(r, _)| r == root)
                    .map(|(_, k)| k),
            )
    }

    // Restores "pinned tabs come first" after a move that ignores the boundary; the order
    // within each group is kept.
    fn keep_pinned_first(&mut self) {
        self.entries.sort_by_key(|e| !e.pinned);
    }

    fn active_index(&self) -> Option<usize> {
        self.entries.iter().position(|e| Some(e.id) == self.active)
    }
}
