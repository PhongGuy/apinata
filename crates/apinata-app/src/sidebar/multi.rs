//! The sidebar's multi-selection as pure state: which rows are selected, the range anchor, and
//! the rules that keep a selection inside one collection.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use apinata_core::ItemPath;

use crate::sidebar::model::{Node, NodeKind};
use crate::snapshot::{Row, RowKind};
use crate::stale::{is_within, rebase};

/// One selectable item: a collection root plus the item's path in it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SelKey {
    /// The collection's root directory.
    pub root: PathBuf,
    /// The item's path inside the collection.
    pub path: ItemPath,
}

impl SelKey {
    /// The key of the item at `path` in the collection at `root`.
    pub fn new(root: impl Into<PathBuf>, path: ItemPath) -> SelKey {
        SelKey {
            root: root.into(),
            path,
        }
    }

    /// The key of a tree node, or None when the node is not a selectable row.
    pub fn of(node: &Node) -> Option<SelKey> {
        selectable(node.kind).then(|| SelKey::new(node.root.clone(), node.path.clone()))
    }
}

/// Whether rows of this kind can join a selection.
pub fn selectable(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Request | NodeKind::Folder | NodeKind::Opaque
    )
}

/// The visible selectable rows in tree order across all roots; collapsed folders hide children.
pub fn visible_keys(nodes: &[Node]) -> Vec<SelKey> {
    fn walk(nodes: &[Node], out: &mut Vec<SelKey>) {
        for node in nodes {
            out.extend(SelKey::of(node));
            if node.expanded {
                walk(&node.children, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(nodes, &mut out);
    out
}

/// Every item of one snapshot, in sidebar order.
pub fn row_keys(root: &Path, rows: &[Row]) -> Vec<SelKey> {
    rows.iter()
        .map(|r| SelKey::new(root, r.path.clone()))
        .collect()
}

/// The items a run of `keys` plans, in sidebar order. The collection root (which core reads as
/// "plan everything") and opaque rows never appear.
pub fn run_items(rows: &[Row], keys: &[SelKey]) -> Vec<ItemPath> {
    rows.iter()
        .filter(|r| r.kind != RowKind::Opaque && !r.path.is_root())
        .filter(|r| keys.iter().any(|k| k.path == r.path))
        .map(|r| r.path.clone())
        .collect()
}

/// How many requests a run of `items` executes: a request counts 1, a folder the requests below it.
pub fn run_count(rows: &[Row], items: &[SelKey]) -> usize {
    rows.iter()
        .filter(|r| r.kind == RowKind::Request)
        .filter(|r| items.iter().any(|k| is_within(&r.path, &k.path)))
        .count()
}

/// The selected set and its range anchor. Every key shares one collection root.
#[derive(Debug, Default, Clone)]
pub struct Multi {
    keys: BTreeSet<SelKey>,
    anchor: Option<SelKey>,
}

impl Multi {
    /// The selected keys.
    pub fn keys(&self) -> &BTreeSet<SelKey> {
        &self.keys
    }

    /// The range anchor, if one is set.
    pub fn anchor(&self) -> Option<&SelKey> {
        self.anchor.as_ref()
    }

    /// Whether nothing is selected.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Two or more keys; one key is just the single selection.
    pub fn is_active(&self) -> bool {
        self.keys.len() >= 2
    }

    /// Whether `k` is selected.
    pub fn contains(&self, k: &SelKey) -> bool {
        self.keys.contains(k)
    }

    /// Drops the keys and the anchor.
    pub fn clear(&mut self) {
        self.keys.clear();
        self.anchor = None;
    }

    fn root(&self) -> Option<&Path> {
        self.keys.first().map(|k| k.root.as_path())
    }

    fn replace_with(&mut self, key: SelKey) {
        self.keys.clear();
        self.anchor = Some(key.clone());
        self.keys.insert(key);
    }

    /// Secondary-click. Returns the row focus moves to, if it moves.
    pub fn toggle(
        &mut self,
        key: SelKey,
        focus: Option<&SelKey>,
        order: &[SelKey],
    ) -> Option<SelKey> {
        if self.root().is_some_and(|r| r != key.root) {
            self.replace_with(key.clone());
            return Some(key);
        }
        // A first toggle keeps the row that was already focused as part of the set.
        if self.keys.is_empty()
            && let Some(f) = focus.filter(|f| f.root == key.root && **f != key)
        {
            self.keys.insert(f.clone());
        }
        if !self.keys.remove(&key) {
            self.keys.insert(key.clone());
            self.anchor = Some(key.clone());
            return Some(key);
        }
        if focus != Some(&key) {
            return None;
        }
        // The focus row left the set: land on the next member below it, else the one above.
        let at = order.iter().position(|k| *k == key).unwrap_or(order.len());
        let after = order.iter().skip(at + 1).find(|k| self.keys.contains(*k));
        let before = order[..at.min(order.len())]
            .iter()
            .rev()
            .find(|k| self.keys.contains(*k));
        after.or(before).cloned()
    }

    /// Shift-click. Returns the new focus (always `target`).
    pub fn select_range(
        &mut self,
        target: SelKey,
        focus: Option<&SelKey>,
        order: &[SelKey],
    ) -> SelKey {
        let anchor = self
            .anchor
            .clone()
            .filter(|a| self.keys.contains(a))
            .or_else(|| focus.cloned());
        let pos = |k: &SelKey| order.iter().position(|o| o == k);
        let span = anchor
            .as_ref()
            .filter(|a| a.root == target.root)
            .and_then(|a| Some((a, pos(a)?, pos(&target)?)));
        let Some((anchor, a, t)) = span else {
            self.replace_with(target.clone());
            return target;
        };
        let (lo, hi) = (a.min(t), a.max(t));
        self.keys = order[lo..=hi]
            .iter()
            .filter(|k| k.root == target.root)
            .cloned()
            .collect();
        self.anchor = Some(anchor.clone());
        target
    }

    /// Shift+Up or Shift+Down. None at the end of the root's rows.
    pub fn extend(&mut self, down: bool, focus: &SelKey, order: &[SelKey]) -> Option<SelKey> {
        let at = order.iter().position(|k| k == focus)?;
        let next = if down {
            order.get(at + 1)?
        } else {
            order.get(at.checked_sub(1)?)?
        };
        if next.root != focus.root {
            return None;
        }
        Some(self.select_range(next.clone(), Some(focus), order))
    }

    /// Members without any whose ancestor folder is also a member, in `rows` order.
    pub fn normalised(&self, rows: &[SelKey]) -> Vec<SelKey> {
        rows.iter()
            .filter(|k| self.keys.contains(*k))
            .filter(|k| {
                !self
                    .keys
                    .iter()
                    .any(|o| o.root == k.root && o.path != k.path && is_within(&k.path, &o.path))
            })
            .cloned()
            .collect()
    }

    /// Follows a move of `from` to `to` in one root, for keys at or under `from`.
    pub fn rebase(&mut self, root: &Path, from: &ItemPath, to: &ItemPath) {
        let moved = |k: &SelKey| {
            if k.root != root {
                return k.clone();
            }
            let path = rebase(&k.path, from, to).unwrap_or_else(|| k.path.clone());
            SelKey::new(k.root.clone(), path)
        };
        self.keys = self.keys.iter().map(moved).collect();
        self.anchor = self.anchor.as_ref().map(moved);
    }

    /// Drops keys (and the anchor) `live` rejects and returns them.
    pub fn retain(&mut self, live: impl Fn(&SelKey) -> bool) -> Vec<SelKey> {
        let (keep, dropped): (BTreeSet<SelKey>, BTreeSet<SelKey>) = std::mem::take(&mut self.keys)
            .into_iter()
            .partition(|k| live(k));
        self.keys = keep;
        if self.anchor.as_ref().is_some_and(|a| !live(a)) {
            self.anchor = None;
        }
        dropped.into_iter().collect()
    }

    /// Adds keys of the set's own root (any root when empty).
    pub fn admit(&mut self, keys: impl IntoIterator<Item = SelKey>) {
        for key in keys {
            if self.root().is_none_or(|r| r == key.root) {
                self.keys.insert(key);
            }
        }
    }
}
