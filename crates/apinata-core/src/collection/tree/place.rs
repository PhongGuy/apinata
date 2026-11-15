//! Exact put-back of a move or rename: the item returns to a given path, and every numbered
//! sibling in the recorded folders gets the seq it had, so an undo leaves nothing in a diff.

use super::{INFO_ORDER, child, io_error, numbered, write_folder};
use crate::collection::{Collection, sort_lists};
use crate::error::TreeError;
use crate::item_path::ItemPath;
use std::fs;
use yaml_serde::Value;

/// Each numbered item of some folders, with the seq it stores (if any) and its display name,
/// read before a move or rename so that [`Collection::place`] can put them back.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SeqRecord {
    items: Vec<Recorded>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Recorded {
    path: ItemPath,
    seq: Option<u64>,
    name: String,
}

impl Collection {
    /// The numbered items of each folder in `dirs` as they stand now; an unknown folder adds
    /// nothing.
    pub fn seq_record(&self, dirs: &[ItemPath]) -> SeqRecord {
        let mut items: Vec<Recorded> = Vec::new();
        for dir in dirs {
            let Some(folder) = self.folder(dir) else {
                continue;
            };
            for n in numbered(dir, folder) {
                if !items.iter().any(|r| r.path == n.path) {
                    items.push(Recorded {
                        path: n.path,
                        seq: n.stored,
                        name: n.item.name,
                    });
                }
            }
        }
        SeqRecord { items }
    }

    /// Put the item at `item` back at the exact path `to`, with the display name and the seqs
    /// `record` holds for it and its siblings. `record` names items by where they were, so the
    /// entry at `to` is the item's own. Returns the item's new path.
    ///
    /// Fails before any write with `NotFound` for an unknown item or folder, `InvalidMove` for
    /// the root or a path under the item, `Taken` when anything is at `to`, and `Conflict` when
    /// a file to rewrite changed on disk. `Renumber` means the item moved but a seq could not be
    /// written.
    pub fn place(
        &mut self,
        item: &ItemPath,
        to: &ItemPath,
        record: &SeqRecord,
    ) -> Result<ItemPath, TreeError> {
        let invalid = || TreeError::InvalidMove { item: item.clone() };
        if !self.exists(item) {
            return Err(TreeError::NotFound { item: item.clone() });
        }
        let dir = to.parent().ok_or_else(invalid)?;
        let under = to.as_str().starts_with(&format!("{}/", item.as_str()));
        if item.parent().is_none() || under {
            return Err(invalid());
        }
        if self.folder(&dir).is_none() {
            return Err(TreeError::NotFound { item: dir });
        }
        let new = to.to_path(&self.root);
        let old = item.to_path(&self.root);
        if to != item && fs::symlink_metadata(&new).is_ok() && !same_file(&old, &new) {
            return Err(TreeError::Taken { path: new });
        }
        // Each recorded item by where it is now: the placed item is still at `item`.
        let now = |p: &ItemPath| if p == to { item.clone() } else { p.clone() };
        let name = (record.items.iter())
            .find(|r| r.path == *to)
            .map(|r| r.name.clone())
            .filter(|n| self.name_of(item).is_some_and(|cur| cur != n));
        let seqs: Vec<(ItemPath, Option<u64>)> = (record.items.iter())
            .filter(|r| self.stored_seq(&now(&r.path)).is_some_and(|s| s != r.seq))
            .map(|r| (r.path.clone(), r.seq))
            .collect();
        let mut checked: Vec<(ItemPath, u64)> = seqs.iter().map(|(p, _)| (now(p), 0)).collect();
        if name.is_some() {
            checked.push((item.clone(), 0));
        }
        self.check_targets(&checked)?;
        // The rename goes first, so a refused one leaves every file as it was.
        let placed = if to == item {
            item.clone()
        } else {
            fs::rename(&old, &new).map_err(|e| io_error(&old, e))?;
            let node = self
                .detach(item)
                .ok_or_else(|| TreeError::NotFound { item: item.clone() })?;
            self.attach(&dir, node, &old, &new);
            child(&dir, &new)?
        };
        let named = name.map_or(Ok(()), |name| self.write_name(&placed, &name));
        let written = named
            .and_then(|()| (seqs.iter()).try_for_each(|(path, seq)| self.write_stored(path, *seq)));
        if let Err(source) = written {
            self.reload();
            return Err(TreeError::Renumber {
                moved: placed,
                source: Box::new(source),
            });
        }
        for (path, _) in &seqs {
            if let Some(folder) = path.parent().and_then(|p| self.folder_mut(&p)) {
                sort_lists(folder);
            }
        }
        Ok(placed)
    }

    // The seq the item at `path` stores, or `None` inside when it stores none; `None` when
    // there is no such item.
    fn stored_seq(&self, path: &ItemPath) -> Option<Option<u64>> {
        if let Some(r) = self.request(path) {
            return Some(r.seq());
        }
        if let Some(o) = self.opaque_item(path) {
            return Some(o.seq());
        }
        self.folder(path).map(|f| f.seq())
    }

    fn name_of(&self, path: &ItemPath) -> Option<&str> {
        if let Some(r) = self.request(path) {
            return Some(r.name());
        }
        self.folder(path).map(|f| f.name())
    }

    fn write_name(&mut self, path: &ItemPath, name: &str) -> Result<(), TreeError> {
        if let Some(r) = self.request_mut(path) {
            let doc = r.document_mut();
            return Ok(doc.write_through(&["info", "name"], Value::from(name), INFO_ORDER)?);
        }
        match self.folder_mut(path) {
            Some(folder) => write_folder(folder, Some(name), None),
            None => Ok(()),
        }
    }

    // Writes `seq`, or takes the stored seq out when the item had none.
    fn write_stored(&mut self, path: &ItemPath, seq: Option<u64>) -> Result<(), TreeError> {
        if let Some(seq) = seq {
            return self.write_seq(path, seq);
        }
        let keys = &["info", "seq"];
        if let Some(r) = self.request_mut(path) {
            return Ok(r.document_mut().remove_through(keys)?);
        }
        if let Some(o) = self.opaque_item_mut(path) {
            return Ok(o.document_mut().remove_through(keys)?);
        }
        match self.folder_mut(path).and_then(|f| f.file.as_mut()) {
            Some(file) => Ok(file.document_mut().remove_through(keys)?),
            None => Ok(()),
        }
    }
}

// Whether `a` and `b` name one file, as a path differing only in letter case does on a
// case-insensitive file system.
fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if let (Ok(x), Ok(y)) = (fs::symlink_metadata(a), fs::symlink_metadata(b)) {
            return x.dev() == y.dev() && x.ino() == y.ino();
        }
    }
    matches!((fs::canonicalize(a), fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
}
