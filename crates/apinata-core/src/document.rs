//! An order-preserving YAML document backing a typed file view.

use crate::error::{FileError, SaveError};
use crate::import::yaml_prepass::{Refusal, prepass};
use crate::order;
use crate::yaml::emit::to_bruno_yaml;
use std::hash::{DefaultHasher, Hasher};
use std::path::{Path, PathBuf};
use yaml_serde::{Mapping, Value};

/// Whether `path`, following links, is a pipe, socket or device, where an open can wait forever.
pub(crate) fn is_special_file(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| !meta.is_file() && !meta.is_dir())
}

/// The bytes at `path`; an [`is_special_file`] is `NotFound`, without being opened.
pub(crate) fn read_regular(path: &Path) -> std::io::Result<Vec<u8>> {
    if is_special_file(path) {
        let kind = std::io::ErrorKind::NotFound;
        return Err(std::io::Error::new(kind, "not a regular file"));
    }
    std::fs::read(path)
}

/// [`read_regular`] as UTF-8 text; other bytes are `InvalidData`.
pub(crate) fn read_regular_text(path: &Path) -> std::io::Result<String> {
    String::from_utf8(read_regular(path)?)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// A parsed file whose root is always a mapping.
#[derive(Clone)]
pub(crate) struct Document {
    path: PathBuf,
    root: Mapping,
    /// The tree as loaded or last saved; dirtiness is a difference from it.
    saved: Mapping,
    /// Hash of the bytes last read from or written to disk, for conflict detection; `None` for
    /// a file not yet on disk.
    loaded_hash: Option<u64>,
    /// Hash of the foreign content last reported as a conflict, so a repeat event stays quiet.
    conflict_hash: Option<u64>,
    disk: DiskState,
    /// A new-request draft with no file of its own: every write is refused.
    in_memory: bool,
    /// Dirty whatever the tree holds, until a successful save; a restored draft sets it.
    unsaved: bool,
}

/// What the disk holds for an open document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DiskState {
    /// The file loads as this document.
    Present,
    /// The file is gone; a save recreates it and its missing directories.
    Removed,
    /// The file no longer loads; a save conflicts until it is overwritten.
    Broken,
}

/// Shows the path and dirty state only, so a wrapper type never prints request content.
impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document")
            .field("path", &self.path)
            .field("dirty", &self.is_dirty())
            .finish_non_exhaustive()
    }
}

/// The most YAML events a collection file may expand to, counting each alias as the node it names.
const MAX_EXPANDED_EVENTS: u64 = 2_000_000;

/// Refuse a YAML text whose alias expansion would exhaust memory or time, before `yaml_serde`
/// builds anything from it. The pre-pass can reject text `yaml_serde` accepts, so past such an
/// error the aliases are uncounted; text with both an anchor and an alias marker is then refused
/// rather than trusted. Any other syntax or nesting error passes through to `yaml_serde`.
pub(crate) fn bound_expansion(text: &str) -> Result<(), String> {
    let too_large =
        || format!("file is too large: more than {MAX_EXPANDED_EVENTS} values once aliases expand");
    match prepass(text, MAX_EXPANDED_EVENTS) {
        Err(Refusal::TooManyEvents) => Err(too_large()),
        Err(Refusal::At { .. }) if text.contains('&') && text.contains('*') => {
            Err("file has aliases and a syntax the safety check cannot follow".to_owned())
        }
        _ => Ok(()),
    }
}

/// The 1-based line of the first git conflict marker in `text`, if any. Only consulted
/// after a parse failure, so marker-like lines inside a block scalar never matter.
fn conflict_marker_line(text: &str) -> Option<usize> {
    text.lines()
        .position(|l| {
            let l = l.trim_end();
            l == "<<<<<<<"
                || l == ">>>>>>>"
                || l == "======="
                || l.starts_with("<<<<<<< ")
                || l.starts_with(">>>>>>> ")
        })
        .map(|i| i + 1)
}

impl Document {
    /// Read and parse `path`, rejecting anything that is not a tag-free mapping.
    pub(crate) fn load(path: &Path) -> Result<Document, FileError> {
        let bytes = read_regular(path).map_err(|e| FileError::new(path, e.to_string()))?;
        let loaded_hash = hash_bytes(&bytes);
        let text = String::from_utf8(bytes)
            .map_err(|_| FileError::new(path, "file is not valid UTF-8"))?;
        // yaml_serde reports a BOM as a second document, so drop it first.
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        bound_expansion(text).map_err(|reason| FileError::new(path, reason))?;
        let value: Value = yaml_serde::from_str(text).map_err(|e| {
            // A file git left conflicted is better named than described by the parser.
            match conflict_marker_line(text) {
                Some(n) => FileError::new(
                    path,
                    format!("merge conflict markers at line {n}; resolve them in git"),
                ),
                None => FileError::new(path, e.to_string()),
            }
        })?;
        // Tags first, so a tagged root is reported as a tag rather than a non-mapping.
        if has_tag(&value) {
            return Err(FileError::new(path, "YAML tags are not supported"));
        }
        let Value::Mapping(root) = value else {
            return Err(FileError::new(path, "the file root must be a mapping"));
        };
        Ok(Document {
            path: path.to_path_buf(),
            saved: root.clone(),
            root,
            loaded_hash: Some(loaded_hash),
            conflict_hash: None,
            disk: DiskState::Present,
            in_memory: false,
            unsaved: false,
        })
    }

    /// A file not yet on disk: dirty until saved; the save refuses a file that appeared at `path`.
    pub(crate) fn new(path: PathBuf, root: Mapping) -> Document {
        Document {
            path,
            root,
            saved: Mapping::new(),
            loaded_hash: None,
            conflict_hash: None,
            disk: DiskState::Present,
            in_memory: false,
            unsaved: false,
        }
    }

    /// A file kept only in memory whose saved baseline is `root`, so it starts clean. Saving
    /// it fails with `Io` (`NotFound`).
    pub(crate) fn draft(path: PathBuf, root: Mapping) -> Document {
        Document {
            path,
            saved: root.clone(),
            root,
            loaded_hash: None,
            conflict_hash: None,
            disk: DiskState::Present,
            in_memory: true,
            unsaved: false,
        }
    }

    /// Write `root` in Bruno's format to `path`, which must not exist (`create_new`), and
    /// return the document as saved. A failed write leaves no partial file behind.
    pub(crate) fn create(path: &Path, root: Mapping) -> std::io::Result<Document> {
        use std::io::Write;
        let text = to_bruno_yaml(&root);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        if let Err(e) = file.write_all(text.as_bytes()) {
            let _ = std::fs::remove_file(path);
            return Err(e);
        }
        Ok(Document {
            path: path.to_path_buf(),
            saved: root.clone(),
            root,
            loaded_hash: Some(hash_bytes(text.as_bytes())),
            conflict_hash: None,
            disk: DiskState::Present,
            in_memory: false,
            unsaved: false,
        })
    }

    /// Point the document at `path` after the file was renamed on disk.
    pub(crate) fn set_path(&mut self, path: PathBuf) {
        self.path = path;
    }

    /// Follow a rename on disk: the path prefix `from` became `to`.
    pub(crate) fn relocate(&mut self, from: &Path, to: &Path) {
        self.path = rebase(&self.path, from, to);
    }

    /// The path the document was loaded from.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Follow `keys` through nested mappings.
    pub(crate) fn get(&self, keys: &[&str]) -> Option<&Value> {
        let (first, rest) = keys.split_first()?;
        rest.iter()
            .try_fold(self.root.get(*first)?, |v, k| v.get(*k))
    }

    /// The value at `keys` in the snapshot last loaded or saved.
    pub(crate) fn get_saved(&self, keys: &[&str]) -> Option<&Value> {
        let (first, rest) = keys.split_first()?;
        rest.iter()
            .try_fold(self.saved.get(*first)?, |v, k| v.get(*k))
    }

    /// The tree as last loaded or saved, without unsaved edits.
    pub(crate) fn saved(&self) -> &Mapping {
        &self.saved
    }

    /// Whether an edit has changed the document since it was loaded or last saved.
    pub(crate) fn is_dirty(&self) -> bool {
        self.unsaved || !same_tree(&self.root, &self.saved)
    }

    /// Keep the document dirty until it is saved, whatever its tree holds.
    pub(crate) fn mark_unsaved(&mut self) {
        self.unsaved = true;
    }

    /// Set the value at `keys`, creating missing mappings on the way. A new key is placed by
    /// Bruno's writer order; `order[i]` is that order for the mapping at depth `i`. Setting a
    /// value equal to the current one leaves the document clean.
    pub(crate) fn set(&mut self, keys: &[&str], value: Value, order: &[&[&str]]) {
        set_in(&mut self.root, keys, value, order, 0);
    }

    /// Remove the key at `keys`, leaving its parents and its neighbours' order alone. Returns
    /// whether anything was there.
    pub(crate) fn remove(&mut self, keys: &[&str]) -> bool {
        remove_in(&mut self.root, keys)
    }

    /// Remove the mapping at `keys` only when it is empty, so a drained section disappears.
    pub(crate) fn remove_if_empty(&mut self, keys: &[&str]) -> bool {
        let empty = self
            .get(keys)
            .and_then(Value::as_mapping)
            .is_some_and(Mapping::is_empty);
        empty && self.remove(keys)
    }

    /// Set the list at `keys` like `set`; an empty list removes the key, as Bruno omits it.
    pub(crate) fn set_list(&mut self, keys: &[&str], items: Vec<Value>, order: &[&[&str]]) {
        if items.is_empty() {
            self.remove(keys);
        } else {
            self.set(keys, Value::Sequence(items), order);
        }
    }

    /// Write the document back in Bruno's format, unless it is clean or the file changed on
    /// disk since it was read. The write goes through a hidden temp file and an atomic rename;
    /// the conflict check is not atomic with it, so a small race window remains.
    pub(crate) fn save(&mut self) -> Result<(), SaveError> {
        if !self.is_dirty() {
            return Ok(());
        }
        let root = self.root.clone();
        self.write_tree(&root, false)?;
        self.saved = root;
        self.unsaved = false;
        Ok(())
    }

    /// Write the draft past the conflict check, even when clean, replacing whatever the file
    /// holds now or recreating it.
    pub(crate) fn save_overwrite(&mut self) -> Result<(), SaveError> {
        let root = self.root.clone();
        self.write_tree(&root, true)?;
        self.saved = root;
        self.unsaved = false;
        Ok(())
    }

    /// Drop the unsaved edits, so the draft is the snapshot last loaded or saved.
    pub(crate) fn discard_draft(&mut self) {
        self.root = self.saved.clone();
    }

    /// What the disk holds for this document.
    pub(crate) fn disk_state(&self) -> DiskState {
        self.disk
    }

    /// The file is gone. Forgetting its hash makes a plain save recreate it, but never over a
    /// file that reappeared.
    pub(crate) fn mark_removed(&mut self) {
        self.disk = DiskState::Removed;
        self.loaded_hash = None;
    }

    /// The file no longer loads. The hash is kept, so a plain save conflicts.
    pub(crate) fn mark_broken(&mut self) {
        self.disk = DiskState::Broken;
    }

    /// Set `keys` in the saved snapshot and write it; only once that succeeds is the draft set
    /// too. Unsaved edits stay unsaved. For a document already on disk: a removed one fails
    /// with `Io` (`NotFound`) rather than recreating its file from the snapshot.
    pub(crate) fn write_through(
        &mut self,
        keys: &[&str],
        value: Value,
        order: &[&[&str]],
    ) -> Result<(), SaveError> {
        if self.disk == DiskState::Removed {
            return Err(removed_error(&self.path));
        }
        let mut snapshot = self.saved.clone();
        if set_in(&mut snapshot, keys, value.clone(), order, 0) {
            self.write_tree(&snapshot, false)?;
            self.saved = snapshot;
        }
        set_in(&mut self.root, keys, value, order, 0);
        Ok(())
    }

    /// `write_through` for a list; an empty list removes the key from snapshot and draft alike.
    pub(crate) fn write_through_list(
        &mut self,
        keys: &[&str],
        items: Vec<Value>,
        order: &[&[&str]],
    ) -> Result<(), SaveError> {
        if !items.is_empty() {
            return self.write_through(keys, Value::Sequence(items), order);
        }
        if self.disk == DiskState::Removed {
            return Err(removed_error(&self.path));
        }
        let mut snapshot = self.saved.clone();
        if remove_in(&mut snapshot, keys) {
            self.write_tree(&snapshot, false)?;
            self.saved = snapshot;
        }
        remove_in(&mut self.root, keys);
        Ok(())
    }

    /// `Conflict` when the file no longer holds the bytes last read or written; for a document
    /// never on disk, when anything has appeared at its path. A pipe, socket or device over the
    /// file is `Conflict`, unopened. `Io` when the file cannot be read.
    pub(crate) fn check_unchanged(&self) -> Result<(), SaveError> {
        let unchanged = match self.loaded_hash {
            None => std::fs::symlink_metadata(&self.path).is_err(),
            Some(loaded) => match read_regular(&self.path) {
                Ok(bytes) => hash_bytes(&bytes) == loaded,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
                Err(source) => {
                    return Err(SaveError::Io {
                        path: self.path.clone(),
                        source,
                    });
                }
            },
        };
        if unchanged {
            Ok(())
        } else {
            Err(SaveError::Conflict {
                path: self.path.clone(),
            })
        }
    }

    /// Write `tree` to the file after the conflict check, which `force` skips, recording the
    /// new bytes' hash. The caller decides which tree becomes the saved snapshot. An in-memory
    /// draft is never written.
    fn write_tree(&mut self, tree: &Mapping, force: bool) -> Result<(), SaveError> {
        let io_err = |source| SaveError::Io {
            path: self.path.clone(),
            source,
        };
        if self.in_memory {
            let source = std::io::Error::new(std::io::ErrorKind::NotFound, "a draft has no file");
            return Err(io_err(source));
        }
        // A removed file may have taken its directories with it.
        if self.disk == DiskState::Removed
            && let Some(parent) = self.path.parent()
        {
            std::fs::create_dir_all(parent).map_err(io_err)?;
        }
        let (target, permissions) = if force {
            // No conflict check, but an existing file is still written through any symlink and
            // keeps its permissions.
            if std::fs::symlink_metadata(&self.path).is_ok() {
                let target = std::fs::canonicalize(&self.path).map_err(io_err)?;
                let permissions = std::fs::metadata(&target).map_err(io_err)?.permissions();
                (target, Some(permissions))
            } else {
                (self.path.clone(), None)
            }
        } else {
            match self.loaded_hash {
                // A new file may only be created where nothing exists, not even a dangling link.
                None => {
                    if std::fs::symlink_metadata(&self.path).is_ok() {
                        return Err(SaveError::Conflict {
                            path: self.path.clone(),
                        });
                    }
                    (self.path.clone(), None)
                }
                Some(_) => {
                    // Write through a symlink to its target, so the link itself survives the rename.
                    let target = std::fs::canonicalize(&self.path).map_err(io_err)?;
                    self.check_unchanged()?;
                    let permissions = std::fs::metadata(&target).map_err(io_err)?.permissions();
                    (target, Some(permissions))
                }
            }
        };
        let text = to_bruno_yaml(tree);
        let temp = temp_path(&target);
        // A fresh file, so whatever sits at the temp name, a pipe included, is never opened.
        let _ = std::fs::remove_file(&temp);
        let written = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .and_then(|mut file| std::io::Write::write_all(&mut file, text.as_bytes()))
            .and_then(|()| match permissions {
                Some(permissions) => std::fs::set_permissions(&temp, permissions),
                None => Ok(()),
            })
            .and_then(|()| std::fs::rename(&temp, &target));
        if let Err(source) = written {
            // Best effort: the write error is the one worth reporting.
            let _ = std::fs::remove_file(&temp);
            return Err(io_err(source));
        }
        self.loaded_hash = Some(hash_bytes(text.as_bytes()));
        self.conflict_hash = None;
        self.disk = DiskState::Present;
        Ok(())
    }

    /// Take a just-created file as this draft's saved state, keeping the draft's edits.
    pub(crate) fn adopt_saved(&mut self, written: Document) {
        self.saved = written.saved;
        self.loaded_hash = written.loaded_hash;
        self.disk = written.disk;
    }

    /// Forget the conflicting content last reported, once the file matches the saved state again.
    pub(crate) fn clear_conflict(&mut self) {
        self.conflict_hash = None;
    }

    /// The draft tree.
    pub(crate) fn mapping(&self) -> &Mapping {
        &self.root
    }

    /// Hash of the bytes last read from or written to disk; `None` for a file not yet on disk.
    pub(crate) fn loaded_hash(&self) -> Option<u64> {
        self.loaded_hash
    }

    /// Take what a fresh load of the same file means for this one. The checks run from the
    /// cheapest no-op to the real change, so a format-only rewrite never reads as seq-only and
    /// a seq-only change never conflicts with a draft.
    pub(crate) fn refresh(&mut self, fresh: Document) -> Refresh {
        // A fresh load means the file is back, whatever happens to the draft.
        self.disk = DiskState::Present;
        // Any load that matches the saved state ends the conflict, so its content can conflict again.
        if self.loaded_hash == fresh.loaded_hash {
            self.conflict_hash = None;
            return Refresh::Unchanged;
        }
        if same_tree(&self.saved, &fresh.saved) {
            self.conflict_hash = None;
            self.loaded_hash = fresh.loaded_hash;
            return Refresh::Unchanged;
        }
        // Seq is sidebar order the app writes itself, so a seq-only change merges silently by design.
        if same_tree(&without_seq(&self.saved), &without_seq(&fresh.saved)) {
            // The saved copy always takes the fresh seq. The draft follows only when its seq was
            // not edited, so an unsaved seq edit survives and its save still lands.
            let draft_follows = self.root.get("info").and_then(|i| i.get("seq"))
                == self.saved.get("info").and_then(|i| i.get("seq"));
            let fresh_seq = fresh.get(&["info", "seq"]);
            let mut trees = vec![&mut self.saved];
            if draft_follows {
                trees.push(&mut self.root);
            }
            for tree in trees {
                match fresh_seq {
                    Some(seq) => {
                        let info_order: &[&[&str]] = &[order::TOP, order::INFO];
                        set_in(tree, &["info", "seq"], seq.clone(), info_order, 0);
                    }
                    None => {
                        if let Some(Value::Mapping(info)) = tree.get_mut("info") {
                            info.shift_remove("seq");
                        }
                    }
                }
            }
            self.conflict_hash = None;
            self.loaded_hash = fresh.loaded_hash;
            return Refresh::SeqOnly;
        }
        if self.is_dirty() {
            if self.conflict_hash == fresh.loaded_hash {
                return Refresh::Unchanged;
            }
            self.conflict_hash = fresh.loaded_hash;
            return Refresh::Conflict;
        }
        *self = fresh;
        Refresh::Replaced
    }
}

/// What [`Document::refresh`] did with a fresh load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refresh {
    /// Same bytes, or the same content in another format.
    Unchanged,
    /// A clean document took the fresh content.
    Replaced,
    /// Only `info.seq` differed; the snapshot and the draft both took it.
    SeqOnly,
    /// A dirty document kept its draft; its next save conflicts.
    Conflict,
}

fn without_seq(tree: &Mapping) -> Mapping {
    let mut tree = tree.clone();
    if let Some(Value::Mapping(info)) = tree.get_mut("info") {
        info.shift_remove("seq");
    }
    tree
}

/// A list entry's known keys in Bruno's form and writer order; `None` marks a key Bruno omits.
pub(crate) type Fields = Vec<(&'static str, Option<Value>)>;

/// The YAML an entry was read from and the fields it parsed to, so an entry that was not
/// changed is written back exactly as read.
#[derive(Clone)]
pub(crate) struct Origin {
    raw: Value,
    read: Fields,
}

impl Origin {
    pub(crate) fn new(raw: Value, read: Fields) -> Origin {
        Origin { raw, read }
    }

    /// Whether two entries were read from the same YAML, key order included; two entries
    /// built in code (no origin) count as the same.
    pub(crate) fn same(a: Option<&Origin>, b: Option<&Origin>) -> bool {
        match (a, b) {
            (Some(a), Some(b)) => same_value(&a.raw, &b.raw),
            (None, None) => true,
            _ => false,
        }
    }
}

impl std::fmt::Debug for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Origin").finish_non_exhaustive()
    }
}

/// The YAML for one list entry. Unchanged fields (`read == fields`) return `raw` as is.
/// Otherwise start from `raw` when it is a mapping (else an empty one): `Some` values are put in
/// place or by `order`, `None` keys removed, so unknown keys stay where they were.
pub(crate) fn write_entry(origin: Option<&Origin>, fields: Fields, order: &[&str]) -> Value {
    if let Some(origin) = origin
        && origin.read == fields
    {
        return origin.raw.clone();
    }
    let mut map = match origin.map(|o| &o.raw) {
        Some(Value::Mapping(map)) => map.clone(),
        _ => Mapping::new(),
    };
    for (key, value) in fields {
        match value {
            Some(value) => put(&mut map, key, value, order),
            None => {
                map.shift_remove(key);
            }
        }
    }
    Value::Mapping(map)
}

/// Structural equality that also respects mapping key order, which `Mapping`'s own `==` ignores
/// but the written file does not.
fn same_tree(a: &Mapping, b: &Mapping) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .all(|((ka, va), (kb, vb))| same_value(ka, kb) && same_value(va, vb))
}

fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Mapping(a), Value::Mapping(b)) => same_tree(a, b),
        (Value::Sequence(a), Value::Sequence(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_value(a, b))
        }
        _ => a == b,
    }
}

/// `.<file>.apinata-tmp` beside `path`; never ends in `.yml`, so a directory walk skips it.
pub(crate) fn temp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    path.with_file_name(format!(".{name}.apinata-tmp"))
}

/// SipHash with fixed keys: stable within a process, which is all conflict detection needs.
pub(crate) fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    hasher.write(bytes);
    hasher.finish()
}

/// `path` with its prefix `from` replaced by `to`; `from` itself maps to `to` exactly, since
/// joining an empty rest would add a trailing separator. Other paths are returned as they are.
pub(crate) fn rebase(path: &Path, from: &Path, to: &Path) -> PathBuf {
    match path.strip_prefix(from) {
        Ok(rest) if rest.as_os_str().is_empty() => to.to_path_buf(),
        Ok(rest) => to.join(rest),
        Err(_) => path.to_path_buf(),
    }
}

/// The refusal of a write-through to a document whose file is gone.
fn removed_error(path: &Path) -> SaveError {
    SaveError::Io {
        path: path.to_path_buf(),
        source: std::io::Error::new(std::io::ErrorKind::NotFound, "the file was removed"),
    }
}

/// Remove the key at `keys` under `map`, leaving its parents and its neighbours' order alone.
/// Returns whether anything was there.
fn remove_in(mut map: &mut Mapping, keys: &[&str]) -> bool {
    let Some((last, parents)) = keys.split_last() else {
        return false;
    };
    for key in parents {
        match map.get_mut(*key) {
            Some(Value::Mapping(child)) => map = child,
            _ => return false,
        }
    }
    // `Mapping::remove` swap-removes and would reorder the file.
    map.shift_remove(*last).is_some()
}

/// [`Document::set`] for a bare tree, such as a copy about to be written as a new file.
pub(crate) fn set_value(map: &mut Mapping, keys: &[&str], value: Value, order: &[&[&str]]) {
    set_in(map, keys, value, order, 0);
}

/// Set `keys` under `map`, returning whether anything changed.
fn set_in(map: &mut Mapping, keys: &[&str], value: Value, order: &[&[&str]], depth: usize) -> bool {
    let Some((key, rest)) = keys.split_first() else {
        return false;
    };
    let here = order.get(depth).copied().unwrap_or(&[]);
    if rest.is_empty() {
        if map.get(*key) == Some(&value) {
            return false;
        }
        put(map, key, value, here);
        return true;
    }
    if !map.get(*key).is_some_and(Value::is_mapping) {
        put(map, key, Value::Mapping(Mapping::new()), here);
    }
    match map.get_mut(*key) {
        Some(Value::Mapping(child)) => set_in(child, rest, value, order, depth + 1),
        _ => false,
    }
}

/// Replace `key` in place, or insert it where Bruno's writer order puts it: right after the last
/// present key that precedes it, else right before the first present key that follows it, else
/// at the end. Keys outside `order` keep their place.
fn put(map: &mut Mapping, key: &str, value: Value, order: &[&str]) {
    if map.contains_key(key) {
        map.insert(Value::from(key), value);
        return;
    }
    let rank = |k: &Value| k.as_str().and_then(|k| order.iter().position(|o| *o == k));
    let mut entries: Vec<(Value, Value)> = std::mem::take(map).into_iter().collect();
    let index = match order.iter().position(|o| *o == key) {
        None => entries.len(),
        Some(own) => {
            let before = entries
                .iter()
                .rposition(|(k, _)| rank(k).is_some_and(|r| r < own));
            let after = entries
                .iter()
                .position(|(k, _)| rank(k).is_some_and(|r| r > own));
            match (before, after) {
                (Some(i), _) => i + 1,
                (None, Some(i)) => i,
                (None, None) => entries.len(),
            }
        }
    };
    entries.insert(index, (Value::from(key), value));
    *map = entries.into_iter().collect();
}

/// A non-negative whole number, accepting `2.0` as JavaScript's `Number.isInteger` does.
pub(crate) fn as_seq(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        let f = value.as_f64()?;
        // 2^53 is the largest range where every integer is exact in an f64.
        (f >= 0.0 && f.fract() == 0.0 && f <= 9_007_199_254_740_992.0).then_some(f as u64)
    })
}

/// Whether a tagged value appears anywhere under `value`.
fn has_tag(value: &Value) -> bool {
    match value {
        Value::Tagged(_) => true,
        Value::Sequence(items) => items.iter().any(has_tag),
        Value::Mapping(map) => map.iter().any(|(k, v)| has_tag(k) || has_tag(v)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::order;
    use tempfile::TempDir;

    const RUNTIME: &str = "info:\n  name: r\nhttp:\n  method: GET\n  url: u\nruntime:\n  variables:\n    - name: a\n      value: \"1\"\n  scripts:\n    - type: tests\n      code: x\nsettings:\n  timeout: 0\n";

    fn load(text: &str) -> (TempDir, Document) {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("r.yml");
        std::fs::write(&path, text).expect("write");
        let doc = Document::load(&path).expect("loads");
        (dir, doc)
    }

    fn keys(map: &Mapping) -> Vec<&str> {
        map.keys()
            .map(|k| k.as_str().expect("string key"))
            .collect()
    }

    fn yaml(text: &str) -> Value {
        yaml_serde::from_str(text).expect("yaml")
    }

    fn fields(name: &str, disabled: bool) -> Fields {
        vec![
            ("name", Some(Value::from(name))),
            ("value", Some(Value::from("v"))),
            ("disabled", disabled.then(|| Value::Bool(true))),
        ]
    }

    const ENTRY: &[&str] = &["name", "value", "description", "disabled"];

    #[test]
    fn alias_expansion_bomb_is_refused_quickly() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("r.yml");
        std::fs::write(
            &path,
            format!(
                "info:\n  name: r\n{}",
                crate::import::yaml_prepass::hostile_yaml()
            ),
        )
        .expect("write");
        let started = std::time::Instant::now();
        let err = Document::load(&path).expect_err("refused");
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        assert!(err.to_string().contains("too large"), "{err}");
    }

    #[test]
    fn bound_expansion_fails_closed_past_a_prepass_error() {
        // A tab-led quoted continuation is a syntax the pre-pass rejects but yaml_serde loads, so
        // the bomb after it would go uncounted.
        let tabbed = "k: \"a\n\tb\"\n";
        assert!(yaml_serde::from_str::<Value>(tabbed).is_ok());
        assert!(super::bound_expansion(tabbed).is_ok());
        let bomb = format!("{tabbed}{}", crate::import::yaml_prepass::hostile_yaml());
        assert!(super::bound_expansion(&bomb).is_err());
    }

    #[test]
    fn debug_never_shows_the_content() {
        let (_dir, doc) = load(
            "info:\n  name: r\nhttp:\n  headers:\n    - name: Authorization\n      value: Bearer s3cret\n",
        );
        let shown = format!("{doc:?}");
        assert!(shown.contains("r.yml"), "{shown}");
        assert!(
            !shown.contains("s3cret") && !shown.contains("Authorization"),
            "{shown}"
        );
    }

    #[test]
    fn remove_deletes_one_key_and_keeps_the_order_of_the_rest() {
        let (_dir, mut doc) = load(RUNTIME);
        assert!(doc.remove(&["runtime", "variables"]));
        assert!(doc.is_dirty());
        let runtime = doc
            .get(&["runtime"])
            .and_then(Value::as_mapping)
            .expect("runtime");
        assert_eq!(keys(runtime), ["scripts"]);
        assert_eq!(keys(&doc.root), ["info", "http", "runtime", "settings"]);
    }

    #[test]
    fn removing_a_missing_key_changes_nothing() {
        let (_dir, mut doc) = load(RUNTIME);
        assert!(!doc.remove(&["runtime", "assertions"]));
        assert!(!doc.remove(&["nothing", "here"]));
        assert!(!doc.remove(&[]));
        assert!(!doc.is_dirty());
    }

    #[test]
    fn removing_a_middle_key_keeps_its_neighbours_in_place() {
        let (_dir, mut doc) = load(RUNTIME);
        assert!(doc.remove(&["http"]));
        assert_eq!(keys(&doc.root), ["info", "runtime", "settings"]);
    }

    #[test]
    fn remove_if_empty_removes_only_an_empty_mapping() {
        let (_dir, mut doc) = load(RUNTIME);
        assert!(!doc.remove_if_empty(&["runtime"]));
        doc.remove(&["runtime", "variables"]);
        doc.remove(&["runtime", "scripts"]);
        assert!(doc.remove_if_empty(&["runtime"]));
        assert_eq!(keys(&doc.root), ["info", "http", "settings"]);
        assert!(!doc.remove_if_empty(&["settings"]));
    }

    #[test]
    fn set_list_writes_a_new_list_where_bruno_puts_it() {
        let (_dir, mut doc) = load(RUNTIME);
        doc.set_list(
            &["runtime", "assertions"],
            vec![yaml("expression: res.status\noperator: eq\nvalue: \"200\"")],
            &[order::TOP, order::children_of("runtime")],
        );
        let runtime = doc
            .get(&["runtime"])
            .and_then(Value::as_mapping)
            .expect("runtime");
        assert_eq!(keys(runtime), ["variables", "scripts", "assertions"]);
    }

    #[test]
    fn set_list_with_no_items_removes_the_key() {
        let (_dir, mut doc) = load(RUNTIME);
        doc.set_list(
            &["runtime", "variables"],
            Vec::new(),
            &[order::TOP, order::children_of("runtime")],
        );
        assert!(doc.get(&["runtime", "variables"]).is_none());
        doc.set_list(
            &["http", "headers"],
            Vec::new(),
            &[order::TOP, order::children_of("http")],
        );
        assert!(doc.get(&["http", "headers"]).is_none());
        assert_eq!(
            keys(
                doc.get(&["http"])
                    .and_then(Value::as_mapping)
                    .expect("http")
            ),
            ["method", "url"]
        );
    }

    #[test]
    fn setting_the_same_list_leaves_the_document_clean() {
        let (_dir, mut doc) = load(RUNTIME);
        let same = doc
            .get(&["runtime", "variables"])
            .and_then(Value::as_sequence)
            .expect("list")
            .clone();
        doc.set_list(
            &["runtime", "variables"],
            same,
            &[order::TOP, order::children_of("runtime")],
        );
        assert!(!doc.is_dirty());
    }

    #[test]
    fn an_unchanged_entry_writes_its_original_value() {
        let raw = yaml("value: v\nx-extra: 1\nname: n\n");
        let origin = Origin::new(raw.clone(), fields("n", false));
        assert_eq!(write_entry(Some(&origin), fields("n", false), ENTRY), raw);
        // Even a value that is not a mapping survives when nothing changed.
        let odd = Value::from("just text");
        let origin = Origin::new(odd.clone(), fields("", false));
        assert_eq!(write_entry(Some(&origin), fields("", false), ENTRY), odd);
    }

    #[test]
    fn a_changed_entry_keeps_unknown_keys_and_places_new_ones_in_bruno_order() {
        let raw = yaml("name: n\nx-extra: 1\nvalue: v\n");
        let origin = Origin::new(raw, fields("n", false));
        let written = write_entry(Some(&origin), fields("m", true), ENTRY);
        let map = written.as_mapping().expect("mapping");
        assert_eq!(keys(map), ["name", "x-extra", "value", "disabled"]);
        assert_eq!(map.get("name"), Some(&Value::from("m")));
        assert_eq!(map.get("disabled"), Some(&Value::Bool(true)));
    }

    #[test]
    fn a_field_set_to_none_is_removed() {
        let raw = yaml("name: n\nvalue: v\ndisabled: true\n");
        let origin = Origin::new(raw, fields("n", true));
        let written = write_entry(Some(&origin), fields("n", false), ENTRY);
        assert_eq!(
            keys(written.as_mapping().expect("mapping")),
            ["name", "value"]
        );
    }

    #[test]
    fn a_new_entry_is_built_in_field_order_without_absent_keys() {
        let written = write_entry(None, fields("n", false), ENTRY);
        assert_eq!(written, yaml("name: n\nvalue: v\n"));
        assert_eq!(
            keys(written.as_mapping().expect("mapping")),
            ["name", "value"]
        );
    }

    #[test]
    fn a_changed_entry_that_was_not_a_mapping_becomes_a_fresh_one() {
        let origin = Origin::new(Value::from("just text"), fields("", false));
        let written = write_entry(Some(&origin), fields("n", false), ENTRY);
        assert_eq!(written, yaml("name: n\nvalue: v\n"));
    }
}

#[cfg(test)]
mod new_file_tests {
    use super::*;
    use tempfile::TempDir;

    fn named(name: &str) -> Mapping {
        let mut root = Mapping::new();
        root.insert(Value::from("name"), Value::from(name));
        root
    }

    #[test]
    fn a_new_document_is_dirty_and_written_on_save() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("e.yml");
        let mut doc = Document::new(path.clone(), named("e"));
        assert!(doc.is_dirty());
        assert!(!path.exists(), "nothing is written before save");
        doc.save().expect("saves");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "name: e\n");
        assert!(!doc.is_dirty());
        doc.set(&["name"], Value::from("f"), &[&["name"]]);
        doc.save().expect("a second save checks the bytes it wrote");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "name: f\n");
    }

    #[test]
    fn a_new_document_never_replaces_a_file_that_appeared() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("e.yml");
        let mut doc = Document::new(path.clone(), named("e"));
        std::fs::write(&path, "name: theirs\n").expect("write");
        assert!(matches!(doc.save(), Err(SaveError::Conflict { .. })));
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "name: theirs\n"
        );
    }

    #[test]
    fn a_renamed_file_is_saved_at_its_new_path() {
        let dir = TempDir::new().expect("tempdir");
        let old = dir.path().join("a.yml");
        let new = dir.path().join("b.yml");
        std::fs::write(&old, "name: a\n").expect("write");
        let mut doc = Document::load(&old).expect("loads");
        std::fs::rename(&old, &new).expect("rename");
        doc.set_path(new.clone());
        doc.set(&["name"], Value::from("b"), &[&["name"]]);
        doc.save().expect("saves");
        assert_eq!(doc.path(), new.as_path());
        assert_eq!(std::fs::read_to_string(&new).expect("read"), "name: b\n");
        assert!(!old.exists());
    }
}

#[cfg(test)]
mod disk_state_tests {
    use super::*;
    use tempfile::TempDir;

    const NAME: &[&[&str]] = &[&["name"]];

    /// A document loaded from `e.yml` holding `name: a`, then edited to `name: b`.
    fn edited(dir: &TempDir, rel: &str) -> Document {
        let path = dir.path().join(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, "name: a\n").expect("write");
        let mut doc = Document::load(&path).expect("loads");
        doc.set(&["name"], Value::from("b"), NAME);
        doc
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).expect("read")
    }

    #[test]
    fn a_loaded_document_is_present() {
        let dir = TempDir::new().expect("tempdir");
        assert_eq!(edited(&dir, "e.yml").disk_state(), DiskState::Present);
    }

    #[test]
    fn a_removed_document_saves_by_recreating_its_file_and_directories() {
        let dir = TempDir::new().expect("tempdir");
        let mut doc = edited(&dir, "f/g/e.yml");
        std::fs::remove_dir_all(dir.path().join("f")).expect("rm");
        doc.mark_removed();
        assert_eq!(doc.disk_state(), DiskState::Removed);
        doc.save().expect("recreates");
        assert_eq!(read(&dir.path().join("f/g/e.yml")), "name: b\n");
        assert_eq!(doc.disk_state(), DiskState::Present);
        assert!(!doc.is_dirty());
    }

    #[test]
    fn a_removed_document_never_replaces_a_file_that_reappeared() {
        let dir = TempDir::new().expect("tempdir");
        let mut doc = edited(&dir, "e.yml");
        doc.mark_removed();
        std::fs::write(dir.path().join("e.yml"), "name: theirs\n").expect("write");
        assert!(matches!(doc.save(), Err(SaveError::Conflict { .. })));
        assert_eq!(read(&dir.path().join("e.yml")), "name: theirs\n");
    }

    #[test]
    fn a_broken_file_conflicts_on_save_and_is_replaced_by_an_overwrite() {
        let dir = TempDir::new().expect("tempdir");
        let mut doc = edited(&dir, "e.yml");
        let path = dir.path().join("e.yml");
        std::fs::write(&path, "name: [").expect("write");
        doc.mark_broken();
        assert_eq!(doc.disk_state(), DiskState::Broken);
        assert!(matches!(doc.save(), Err(SaveError::Conflict { .. })));
        assert_eq!(read(&path), "name: [");
        doc.save_overwrite().expect("overwrites");
        assert_eq!(read(&path), "name: b\n");
        assert_eq!(doc.disk_state(), DiskState::Present);
        assert!(!doc.is_dirty());
        doc.set(&["name"], Value::from("c"), NAME);
        doc.save().expect("a plain save works again");
        assert_eq!(read(&path), "name: c\n");
    }

    #[test]
    fn an_overwrite_replaces_a_changed_file_and_writes_even_when_clean() {
        let dir = TempDir::new().expect("tempdir");
        let mut doc = edited(&dir, "e.yml");
        let path = dir.path().join("e.yml");
        std::fs::write(&path, "name: theirs\n").expect("write");
        doc.save_overwrite().expect("overwrites");
        assert_eq!(read(&path), "name: b\n");
        std::fs::write(&path, "name: theirs\n").expect("write");
        assert!(!doc.is_dirty());
        doc.save_overwrite()
            .expect("overwrites a clean document too");
        assert_eq!(read(&path), "name: b\n");
    }

    #[test]
    fn an_overwrite_recreates_a_removed_file() {
        let dir = TempDir::new().expect("tempdir");
        let mut doc = edited(&dir, "f/e.yml");
        std::fs::remove_dir_all(dir.path().join("f")).expect("rm");
        doc.mark_removed();
        doc.save_overwrite().expect("recreates");
        assert_eq!(read(&dir.path().join("f/e.yml")), "name: b\n");
    }

    #[cfg(unix)]
    #[test]
    fn an_overwrite_writes_through_a_symlink() {
        let dir = TempDir::new().expect("tempdir");
        let target = dir.path().join("target.yml");
        std::fs::write(&target, "name: a\n").expect("write");
        let link = dir.path().join("e.yml");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");
        let mut doc = Document::load(&link).expect("loads");
        doc.set(&["name"], Value::from("b"), NAME);
        std::fs::write(&target, "name: theirs\n").expect("write");
        doc.save_overwrite().expect("overwrites");
        assert!(
            std::fs::symlink_metadata(&link)
                .expect("lstat")
                .is_symlink()
        );
        assert_eq!(read(&target), "name: b\n");
    }

    #[test]
    fn discarding_the_draft_leaves_the_document_clean() {
        let dir = TempDir::new().expect("tempdir");
        let mut doc = edited(&dir, "e.yml");
        doc.discard_draft();
        assert!(!doc.is_dirty());
        assert_eq!(doc.get(&["name"]), Some(&Value::from("a")));
    }

    #[test]
    fn a_fresh_load_makes_a_removed_or_broken_document_present_again() {
        let dir = TempDir::new().expect("tempdir");
        let mut doc = edited(&dir, "e.yml");
        let path = dir.path().join("e.yml");
        doc.mark_broken();
        assert_eq!(
            doc.refresh(Document::load(&path).expect("loads")),
            Refresh::Unchanged
        );
        assert_eq!(doc.disk_state(), DiskState::Present);
        doc.mark_removed();
        assert_eq!(
            doc.refresh(Document::load(&path).expect("loads")),
            Refresh::Unchanged
        );
        assert_eq!(doc.disk_state(), DiskState::Present);
        assert!(doc.is_dirty(), "the draft survives");
        doc.save().expect("the reloaded hash lets a save through");
    }
}
