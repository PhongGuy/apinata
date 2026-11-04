//! Folding on-disk changes back into an open collection, one watcher path at a time. Bytes
//! that match the last read or write are ignored, so Apinata's own writes never come back as
//! changes; a dirty draft outlives its file.

use super::{
    Collection, ENVIRONMENTS_DIR, Environment, FOLDER_FILE, Folder, Ignore, ROOT_FILE,
    WORKSPACE_FILE, is_item_file, name_order, sort_lists, walk,
};
use crate::config_file::FolderFile;
use crate::document::{DiskState, Document, Refresh, hash_bytes, read_regular};
use crate::error::FileError;
use crate::item_path::ItemPath;
use crate::opaque::OpaqueItem;
use crate::request::RequestFile;
use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// One difference between the open collection and the disk.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Change {
    /// The file or directory that changed.
    pub path: PathBuf,
    /// What happened to it.
    pub kind: ChangeKind,
    /// What it is.
    pub item: ItemKind,
    /// For a directory, the item and `folder.yml` files under it before a removal or after an
    /// addition or restore, sorted; empty for a file.
    pub items: Vec<PathBuf>,
}

/// What happened to a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ChangeKind {
    /// It appeared.
    Added,
    /// A clean item took the new content.
    Modified,
    /// Only `info.seq` changed; the draft has it too.
    SeqOnly,
    /// It disappeared or no longer loads.
    Removed,
    /// A dirty item kept its draft; its file changed, was removed or no longer loads.
    Conflict,
    /// A removed or broken file loads again with the content the draft started from.
    Restored,
}

/// The kind of item a change is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ItemKind {
    /// An HTTP or GraphQL request.
    Request,
    /// An item of another type.
    Opaque,
    /// A folder or its `folder.yml`.
    Folder,
    /// `opencollection.yml`.
    Collection,
    /// A file in `environments/`.
    Environment,
    /// The root `.env`; the shell rebuilds its `VarContext`.
    DotEnv,
}

impl Collection {
    /// Fold one path the watcher reported, under [`Collection::root`], into the collection. A
    /// path outside the collection or naming no item yields nothing; an ignored one
    /// ([`Collection::is_ignored`]) reads as gone from disk, so it only drops what the tree
    /// still holds there. Failures to read or parse become load errors.
    pub fn reload_path(&mut self, path: &Path) -> Vec<Change> {
        let Some(rel) = self.item_path(path) else {
            return Vec::new();
        };
        let Some(parent) = rel.parent() else {
            return Vec::new();
        };
        // The files that are not items: `opencollection.yml`, the root `workspace.yml`, `.env`
        // and `environments/*.yml`.
        let full = rel.to_path(&self.root);
        let hidden = self.is_ignored(&full);
        let segments: Vec<&str> = rel.segments().collect();
        match segments.as_slice() {
            [ROOT_FILE] => return self.reload_root_file(&full),
            [WORKSPACE_FILE] => return Vec::new(),
            [".env"] => return self.reload_dotenv(&full),
            [ENVIRONMENTS_DIR, name] if is_environment_file(name) => {
                return self.reload_environment(&full, hidden);
            }
            _ => {}
        }
        if self.folder(&parent).is_none() {
            return Vec::new();
        }
        // A directory on disk or in the tree is a folder appearing or going, never a file.
        let on_disk_dir = !hidden && fs::symlink_metadata(&full).is_ok_and(|m| m.is_dir());
        if on_disk_dir || self.folder(&rel).is_some() {
            return self.reload_dir(&parent, &full, on_disk_dir);
        }
        let name = rel.file_name();
        if name == FOLDER_FILE {
            return self.reload_folder_file(&parent, &full, hidden);
        }
        if !is_item_file(name) {
            return Vec::new();
        }
        self.reload_item(&parent, &full, hidden)
    }

    /// Re-read the direct entries of `folder` (its items, sub-folders and their `folder.yml`)
    /// from disk, so a delete renumbers the siblings that are there now, not the ones this
    /// collection last saw. A folder not in the tree is left alone.
    pub(crate) fn refresh_folder(&mut self, folder: &ItemPath) {
        let Some(known) = self.folder(folder) else {
            return;
        };
        let mut paths = BTreeSet::new();
        paths.extend(known.requests.iter().map(|r| r.path().to_path_buf()));
        paths.extend(known.opaque.iter().map(|o| o.path().to_path_buf()));
        for sub in &known.folders {
            paths.insert(sub.path.join(FOLDER_FILE));
            paths.insert(sub.path.clone());
        }
        // A directory sorts before its own folder.yml, so a new folder is walked first.
        for entry in fs::read_dir(folder.to_path(&self.root))
            .into_iter()
            .flatten()
        {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                paths.insert(path.join(FOLDER_FILE));
                paths.insert(path);
            } else if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(is_item_file)
            {
                paths.insert(path);
            }
        }
        for path in paths {
            self.reload_path(&path);
        }
    }

    /// Rescan everything: each difference once, sorted by path.
    pub fn reload(&mut self) -> Vec<Change> {
        // Directories first, parents before children, so a folder that appears or goes is
        // reported once and nothing beneath it is visited again.
        let mut dirs = BTreeSet::new();
        known_folders(&self.tree, &mut dirs);
        disk_dirs(&self.root, &self.ignore, &mut dirs);
        let mut changes = Vec::new();
        let mut reported: Vec<PathBuf> = Vec::new();
        for dir in dirs {
            if reported.iter().any(|r| dir.starts_with(r)) {
                continue;
            }
            let found = self.reload_path(&dir);
            if !found.is_empty() {
                reported.push(dir);
            }
            changes.extend(found);
        }

        // Then every file the tree knows or the disk holds, read against the updated tree.
        let mut files = BTreeSet::from([self.root.join(ROOT_FILE), self.root.join(".env")]);
        known_files(&self.tree, &mut files);
        files.extend(self.environments.iter().map(|e| e.path().to_path_buf()));
        files.extend(self.load_errors.iter().map(|e| e.path().to_path_buf()));
        let mut folders = BTreeSet::new();
        known_folders(&self.tree, &mut folders);
        for dir in &folders {
            disk_yml_files(dir, &self.ignore, is_item_file, &mut files);
        }
        disk_yml_files(
            &self.root.join(ENVIRONMENTS_DIR),
            &self.ignore,
            is_environment_file,
            &mut files,
        );
        for file in files {
            if !reported.iter().any(|r| file.starts_with(r)) {
                changes.extend(self.reload_path(&file));
            }
        }
        changes.sort_by(|a, b| a.path.cmp(&b.path));
        changes
    }

    /// `opencollection.yml`. Its removal is ignored, and a file that no longer parses becomes a
    /// load error while the old content stays, as does the ignore list; a dirty one is also
    /// marked broken. A new ignore list applies at once: its rescan's changes follow.
    fn reload_root_file(&mut self, full: &Path) -> Vec<Change> {
        let doc = self.file.document_mut();
        let was = doc.disk_state();
        // Only a missing file is ignored; any other read failure is a load error.
        let (bytes, read_error) = match read_regular(full) {
            Ok(bytes) => (bytes, None),
            Err(e) if e.kind() == ErrorKind::NotFound => return Vec::new(),
            Err(e) => (Vec::new(), Some(FileError::new(full, e.to_string()))),
        };
        // The loaded bytes are back, so an earlier parse failure no longer applies. A broken
        // draft must go through the load to become present again.
        if read_error.is_none()
            && was == DiskState::Present
            && doc.loaded_hash() == Some(hash_bytes(&bytes))
        {
            doc.clear_conflict();
            self.replace_errors(full, Vec::new());
            return Vec::new();
        }
        // The list comes from the bytes on disk, never from a draft.
        let mut ignore = None;
        let loaded = match read_error {
            Some(error) => Err(error),
            None => Document::load(full),
        };
        let (kind, errors) = match loaded {
            Ok(fresh) => {
                ignore = Some(self.ignore.reread(&fresh));
                (
                    restored_kind(Some(was), refreshed_kind(doc.refresh(fresh))),
                    Vec::new(),
                )
            }
            Err(error) if doc.is_dirty() => (mark(doc, Some(was), DiskState::Broken), vec![error]),
            Err(error) => (None, vec![error]),
        };
        self.replace_errors(full, errors);
        let mut changes: Vec<Change> = kind
            .map(|kind| Change {
                path: full.to_path_buf(),
                kind,
                item: ItemKind::Collection,
                items: Vec::new(),
            })
            .into_iter()
            .collect();
        // Rescan under the new list; the root file's own change is already reported above.
        if let Some(ignore) = ignore.filter(|i| *i != self.ignore) {
            self.ignore = ignore;
            changes.extend(self.reload().into_iter().filter(|c| c.path != full));
        }
        changes
    }

    /// `environments/<name>.yml`. A file that is gone or no longer parses (a load error) takes
    /// a clean environment with it; a dirty one is kept and marked removed or broken, and a
    /// draft that was never saved survives a missing file.
    fn reload_environment(&mut self, full: &Path, hidden: bool) -> Vec<Change> {
        let read = read_unless_hidden(full, hidden);
        if fs::symlink_metadata(full).is_ok_and(|m| !m.is_file()) {
            return Vec::new();
        }
        let name = full
            .file_stem()
            .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
        let known = self.environments.iter().position(|e| e.name == name);
        let known_hash = known.and_then(|i| self.environments[i].doc.loaded_hash());
        let was = known.map(|i| self.environments[i].doc.disk_state());
        let dirty = known.is_some_and(|i| self.environments[i].doc.is_dirty());
        // A removed draft has forgotten its hash but still had a file.
        let had_file = known_hash.is_some() || was.is_some_and(|w| w != DiskState::Present);
        // The loaded bytes are back, so an error from a break in between no longer applies. A
        // kept draft that is not `Present` must go through the load to come back.
        if let (Ok(bytes), Some(hash)) = (&read, known_hash)
            && hash_bytes(bytes) == hash
            && was == Some(DiskState::Present)
        {
            if let Some(i) = known {
                self.environments[i].doc.clear_conflict();
            }
            self.replace_errors(full, Vec::new());
            return Vec::new();
        }

        let mut errors = Vec::new();
        let kind = match (&read, Document::load(full)) {
            (Err(e), _) if e.kind() == ErrorKind::NotFound => match known {
                Some(i) if dirty && had_file => {
                    mark(&mut self.environments[i].doc, was, DiskState::Removed)
                }
                Some(i) if had_file => {
                    self.environments.remove(i);
                    Some(ChangeKind::Removed)
                }
                _ => None,
            },
            (_, Ok(fresh)) => match known {
                Some(i) => {
                    restored_kind(was, refreshed_kind(self.environments[i].doc.refresh(fresh)))
                }
                None => {
                    self.environments.push(Environment::from_parts(name, fresh));
                    Some(ChangeKind::Added)
                }
            },
            (_, Err(error)) => {
                errors.push(error);
                match known {
                    Some(i) if dirty => mark(&mut self.environments[i].doc, was, DiskState::Broken),
                    Some(i) if had_file => {
                        self.environments.remove(i);
                        Some(ChangeKind::Removed)
                    }
                    _ => None,
                }
            }
        };
        self.environments
            .sort_by(|a, b| name_order(&a.name, &b.name));
        self.replace_errors(full, errors);
        kind.map(|kind| Change {
            path: full.to_path_buf(),
            kind,
            item: ItemKind::Environment,
            items: Vec::new(),
        })
        .into_iter()
        .collect()
    }

    /// The root `.env`, compared by hash only; absence and unreadable are the same.
    fn reload_dotenv(&mut self, full: &Path) -> Vec<Change> {
        let fresh = read_regular(full).ok().map(|b| hash_bytes(&b));
        let kind = match (self.dotenv_hash, fresh) {
            (old, new) if old == new => return Vec::new(),
            (None, Some(_)) => ChangeKind::Added,
            (Some(_), None) => ChangeKind::Removed,
            _ => ChangeKind::Modified,
        };
        self.dotenv_hash = fresh;
        vec![Change {
            path: full.to_path_buf(),
            kind,
            item: ItemKind::DotEnv,
            items: Vec::new(),
        }]
    }

    /// A request or opaque item file in a folder the tree knows; a hidden one reads as gone.
    fn reload_item(&mut self, parent: &ItemPath, full: &Path, hidden: bool) -> Vec<Change> {
        let read = read_unless_hidden(full, hidden);
        let Some(folder) = self.folder_mut(parent) else {
            return Vec::new();
        };
        let name = full
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let request = folder.requests.iter().position(|r| r.file_name() == name);
        let opaque = folder.opaque.iter().position(|o| o.file_name() == name);
        let was = request.map(|i| folder.requests[i].document().disk_state());
        let known_hash = match (request, opaque) {
            (Some(i), _) => folder.requests[i].document().loaded_hash(),
            (None, Some(i)) => folder.opaque[i].document().loaded_hash(),
            (None, None) => None,
        };
        // Hash first: the same bytes change nothing, not even an opaque item's load error. A
        // request's error can only be from a break in between, which these bytes undo. A kept
        // draft that is not `Present` must go through the load to come back.
        if let (Ok(bytes), Some(known)) = (&read, known_hash)
            && hash_bytes(bytes) == known
            && was.is_none_or(|w| w == DiskState::Present)
        {
            if let Some(i) = request {
                folder.requests[i].document_mut().clear_conflict();
                self.replace_errors(full, Vec::new());
            }
            return Vec::new();
        }

        // Load as `walk` does: a request, else an opaque item that also stays a load error.
        // A file that changes type is the old kind removed and the new one added.
        let change = |kind, item| Change {
            path: full.to_path_buf(),
            kind,
            item,
            items: Vec::new(),
        };
        let mut changes = Vec::new();
        let mut errors = Vec::new();
        let gone = read
            .as_ref()
            .is_err_and(|e| e.kind() == ErrorKind::NotFound);
        let loaded = match hidden {
            true => Err(FileError::new(full, "ignored")),
            false => RequestFile::load(full),
        };
        match loaded {
            Ok(fresh) => {
                if let Some(i) = opaque {
                    folder.opaque.remove(i);
                    changes.push(change(ChangeKind::Removed, ItemKind::Opaque));
                }
                match request {
                    Some(i) => {
                        let refreshed = folder.requests[i]
                            .document_mut()
                            .refresh(fresh.into_document());
                        changes.extend(
                            restored_kind(was, refreshed_kind(refreshed))
                                .map(|k| change(k, ItemKind::Request)),
                        );
                    }
                    None => {
                        folder.requests.push(fresh);
                        changes.push(change(ChangeKind::Added, ItemKind::Request));
                    }
                }
            }
            // A dirty request keeps its draft over a file that is gone or does not load, even as
            // another item type; an opaque item at the same name is left alone.
            Err(error) if request.is_some_and(|i| folder.requests[i].is_dirty()) => {
                let doc = request.map(|i| folder.requests[i].document_mut());
                let now = if gone {
                    DiskState::Removed
                } else {
                    errors.push(error);
                    DiskState::Broken
                };
                let kind = doc.and_then(|doc| mark(doc, was, now));
                changes.extend(kind.map(|k| change(k, ItemKind::Request)));
            }
            Err(error) => {
                errors.push(error);
                if let Some(i) = request {
                    folder.requests.remove(i);
                    changes.push(change(ChangeKind::Removed, ItemKind::Request));
                }
                match (OpaqueItem::load(full).filter(|_| !hidden), opaque) {
                    (Some(fresh), Some(i)) => {
                        let refreshed = folder.opaque[i]
                            .document_mut()
                            .refresh(fresh.into_document());
                        changes
                            .extend(refreshed_kind(refreshed).map(|k| change(k, ItemKind::Opaque)));
                    }
                    (Some(fresh), None) => {
                        folder.opaque.push(fresh);
                        changes.push(change(ChangeKind::Added, ItemKind::Opaque));
                    }
                    (None, Some(i)) if !gone && folder.opaque[i].is_dirty() => {
                        changes.push(change(ChangeKind::Conflict, ItemKind::Opaque));
                    }
                    (None, Some(i)) => {
                        folder.opaque.remove(i);
                        changes.push(change(ChangeKind::Removed, ItemKind::Opaque));
                    }
                    (None, None) => {}
                }
            }
        }
        // A file that is gone is no longer an error either.
        if gone {
            errors.clear();
        }
        sort_lists(folder);
        self.replace_errors(full, errors);
        changes
    }

    /// The `folder.yml` of the folder at `dir`. A broken or vanished file leaves the folder
    /// named by its directory and reads as the folder file removed, unless a dirty draft is
    /// kept and marked broken or removed; a loadable one returning reads as added.
    fn reload_folder_file(&mut self, dir: &ItemPath, full: &Path, hidden: bool) -> Vec<Change> {
        let read = read_unless_hidden(full, hidden);
        let Some(folder) = self.folder_mut(dir) else {
            return Vec::new();
        };
        if folder.is_root {
            return Vec::new();
        }
        let known_hash = folder
            .file
            .as_ref()
            .and_then(|f| f.document().loaded_hash());
        let was = folder.file.as_ref().map(|f| f.document().disk_state());
        // The loaded bytes are back, so an error from a break in between no longer applies. A
        // kept draft that is not `Present` must go through the load to come back.
        if let (Ok(bytes), Some(known)) = (&read, known_hash)
            && hash_bytes(bytes) == known
            && was == Some(DiskState::Present)
        {
            if let Some(file) = folder.file.as_mut() {
                file.document_mut().clear_conflict();
            }
            self.replace_errors(full, Vec::new());
            return Vec::new();
        }

        // A removed draft has forgotten its hash but still had a file.
        let had_file = known_hash.is_some() || was.is_some_and(|w| w != DiskState::Present);
        let dirty = folder.file.as_ref().is_some_and(FolderFile::is_dirty);
        let mut errors = Vec::new();
        let kind = match (&read, Document::load(full)) {
            (Err(e), _) if e.kind() == ErrorKind::NotFound && dirty && had_file => {
                folder.file_broken = false;
                folder
                    .file
                    .as_mut()
                    .and_then(|f| mark(f.document_mut(), was, DiskState::Removed))
            }
            // An unsaved stub from `folder_file_mut` has no file to lose, so its draft stays.
            (Err(e), _) if e.kind() == ErrorKind::NotFound => {
                folder.file_broken = false;
                if had_file {
                    folder.file = None;
                }
                had_file.then_some(ChangeKind::Removed)
            }
            (_, Ok(fresh)) => {
                folder.file_broken = false;
                match folder.file.as_mut() {
                    Some(file) => {
                        restored_kind(was, refreshed_kind(file.document_mut().refresh(fresh)))
                    }
                    None => {
                        folder.file = Some(FolderFile::from_document(fresh));
                        Some(ChangeKind::Added)
                    }
                }
            }
            // A dirty file keeps its draft, marked broken; it is still held, so `file_broken`
            // stays false and `folder_file_mut` still reaches the draft.
            (_, Err(error)) if dirty => {
                errors.push(error);
                folder
                    .file
                    .as_mut()
                    .and_then(|f| mark(f.document_mut(), was, DiskState::Broken))
            }
            (_, Err(error)) => {
                errors.push(error);
                folder.file = None;
                folder.file_broken = true;
                had_file.then_some(ChangeKind::Removed)
            }
        };
        // A new seq or name may move the folder among its siblings.
        if let Some(parent) = dir.parent()
            && let Some(parent) = self.folder_mut(&parent)
        {
            sort_lists(parent);
        }
        self.replace_errors(full, errors);
        kind.map(|kind| Change {
            path: full.to_path_buf(),
            kind,
            item: ItemKind::Folder,
            items: Vec::new(),
        })
        .into_iter()
        .collect()
    }

    /// A directory in a folder the tree knows: walked in when new. A vanished one is dropped
    /// with every load error under it, or kept as a removed folder for the dirty drafts under
    /// it; a kept one that reappears is restored entry by entry.
    fn reload_dir(&mut self, parent: &ItemPath, full: &Path, on_disk: bool) -> Vec<Change> {
        let mut errors = Vec::new();
        let ignore = self.ignore.clone();
        let Some(folder) = self.folder_mut(parent) else {
            return Vec::new();
        };
        let known = folder.folders.iter().position(|f| f.path == full);
        let mut items = BTreeSet::new();
        let kind = match (known, on_disk) {
            (None, true) => {
                // The root's `environments` directory holds environments, not a folder.
                if folder.is_root && full.file_name().is_some_and(|n| n == ENVIRONMENTS_DIR) {
                    return Vec::new();
                }
                // `walk` fails only for the root; any deeper failure is a load error.
                let Ok(new) = walk(full, &ignore, &mut errors) else {
                    return Vec::new();
                };
                item_files(&new, &mut items);
                folder.folders.push(new);
                ChangeKind::Added
            }
            (Some(i), false) if !folder.folders[i].removed => {
                item_files(&folder.folders[i], &mut items);
                if keep_drafts(&mut folder.folders[i]) {
                    folder.folders[i].removed = true;
                } else {
                    folder.folders.remove(i);
                }
                ChangeKind::Removed
            }
            (Some(i), true) if folder.folders[i].removed => {
                folder.folders[i].removed = false;
                return self.restore_dir(full);
            }
            // Still there: a read error recorded for the directory itself is stale once it
            // reads again.
            (Some(_), true) => {
                if fs::read_dir(full).is_ok() {
                    self.replace_errors(full, Vec::new());
                }
                return Vec::new();
            }
            // Unknown and absent, or kept and still gone: nothing new to report.
            (_, false) => return Vec::new(),
        };
        sort_lists(folder);
        self.load_errors.retain(|e| !e.path().starts_with(full));
        self.load_errors.extend(errors);
        vec![Change {
            path: full.to_path_buf(),
            kind,
            item: ItemKind::Folder,
            items: items.into_iter().collect(),
        }]
    }

    /// Bring a kept folder's reappeared directory back: reload each entry on disk in name
    /// order, a kept sub-folder recursing through `reload_dir`, and report the folder as
    /// `Restored` ahead of its entries' own changes.
    fn restore_dir(&mut self, full: &Path) -> Vec<Change> {
        let mut entries: Vec<PathBuf> = fs::read_dir(full)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            // As `walk` reads them: a symlink is neither a directory nor a file.
            .filter(|p| fs::symlink_metadata(p).is_ok_and(|m| m.is_dir() || m.is_file()))
            .collect();
        entries.sort();
        let inner: Vec<Change> = entries.iter().flat_map(|p| self.reload_path(p)).collect();
        let mut items = BTreeSet::new();
        if let Some(folder) = self.item_path(full).and_then(|rel| self.folder(&rel)) {
            item_files(folder, &mut items);
        }
        let mut changes = vec![Change {
            path: full.to_path_buf(),
            kind: ChangeKind::Restored,
            item: ItemKind::Folder,
            items: items.into_iter().collect(),
        }];
        changes.extend(inner);
        changes
    }

    /// Make `errors` the only load errors that name `path`.
    pub(super) fn replace_errors(&mut self, path: &Path, errors: Vec<FileError>) {
        self.load_errors.retain(|e| e.path() != path);
        self.load_errors.extend(errors);
    }
}

/// The directory of every folder under `folder`, the root's own included.
fn known_folders(folder: &Folder, out: &mut BTreeSet<PathBuf>) {
    out.insert(folder.path.clone());
    for sub in &folder.folders {
        known_folders(sub, out);
    }
}

/// Every item file and `folder.yml` path the tree knows below `folder`.
fn known_files(folder: &Folder, out: &mut BTreeSet<PathBuf>) {
    if !folder.is_root {
        out.insert(folder.path.join(FOLDER_FILE));
    }
    out.extend(folder.requests.iter().map(|r| r.path().to_path_buf()));
    out.extend(folder.opaque.iter().map(|o| o.path().to_path_buf()));
    for sub in &folder.folders {
        known_files(sub, out);
    }
}

/// Every item file and `folder.yml` the tree holds a document or load error for below
/// `folder`, unlike `known_files`, which names each folder's `folder.yml` whether or not it
/// exists.
fn item_files(folder: &Folder, out: &mut BTreeSet<PathBuf>) {
    if folder.file.is_some() || folder.file_broken {
        out.insert(folder.path.join(FOLDER_FILE));
    }
    out.extend(folder.requests.iter().map(|r| r.path().to_path_buf()));
    out.extend(folder.opaque.iter().map(|o| o.path().to_path_buf()));
    for sub in &folder.folders {
        item_files(sub, out);
    }
}

/// Keep only the dirty drafts under a removed directory, each marked removed; whether any
/// remain. Opaque items and clean documents go, as does every sub-folder left empty.
fn keep_drafts(folder: &mut Folder) -> bool {
    folder.requests.retain(RequestFile::is_dirty);
    folder
        .requests
        .iter_mut()
        .for_each(|r| r.document_mut().mark_removed());
    folder.opaque.clear();
    folder.file_broken = false;
    match &mut folder.file {
        Some(file) if file.is_dirty() => file.document_mut().mark_removed(),
        _ => folder.file = None,
    }
    folder.folders.retain_mut(|child| {
        child.removed = true;
        keep_drafts(child)
    });
    !folder.requests.is_empty() || folder.file.is_some() || !folder.folders.is_empty()
}

/// The directories below `dir` that `walk` would enter.
fn disk_dirs(dir: &Path, ignore: &Ignore, out: &mut BTreeSet<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let is_root = ignore.rel(dir).is_some_and(|r| r.is_empty());
    for path in entries.flatten().map(|e| e.path()) {
        let name = path.file_name().unwrap_or_default();
        let skipped = (is_root && name == ENVIRONMENTS_DIR) || hidden(ignore, &path);
        if !skipped && fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir()) {
            out.insert(path.clone());
            disk_dirs(&path, ignore, out);
        }
    }
}

/// The regular files directly in `dir` whose names pass `keep` and that `ignore` does not hide.
fn disk_yml_files(
    dir: &Path,
    ignore: &Ignore,
    keep: fn(&str) -> bool,
    out: &mut BTreeSet<PathBuf>,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for path in entries.flatten().map(|e| e.path()) {
        let is_file = fs::symlink_metadata(&path).is_ok_and(|m| m.is_file());
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if is_file && keep(&name) && !hidden(ignore, &path) {
            out.insert(path);
        }
    }
}

/// Whether `ignore` hides `path` below the root.
fn hidden(ignore: &Ignore, path: &Path) -> bool {
    ignore.rel(path).is_some_and(|rel| ignore.hides(&rel))
}

/// The file's bytes, or `NotFound` for a hidden one, which reloads as if gone from disk.
fn read_unless_hidden(full: &Path, hidden: bool) -> std::io::Result<Vec<u8>> {
    match hidden {
        true => Err(ErrorKind::NotFound.into()),
        false => read_regular(full),
    }
}

/// Whether a file of this name in `environments/` is an environment, as `open` decides.
fn is_environment_file(name: &str) -> bool {
    Path::new(name).extension().is_some_and(|e| e == "yml")
}

fn refreshed_kind(refreshed: Refresh) -> Option<ChangeKind> {
    match refreshed {
        Refresh::Unchanged => None,
        Refresh::Replaced => Some(ChangeKind::Modified),
        Refresh::SeqOnly => Some(ChangeKind::SeqOnly),
        Refresh::Conflict => Some(ChangeKind::Conflict),
    }
}

/// Move a kept draft to `now`; `Conflict` only when its state changes, so a repeat reload of
/// the same removal or break reports nothing.
fn mark(doc: &mut Document, was: Option<DiskState>, now: DiskState) -> Option<ChangeKind> {
    match now {
        DiskState::Removed => doc.mark_removed(),
        DiskState::Broken => doc.mark_broken(),
        DiskState::Present => {}
    }
    (was != Some(now)).then_some(ChangeKind::Conflict)
}

/// A removed or broken document whose fresh load merged without a conflict is `Restored`; a
/// conflict, or any merge of a document that was `Present`, keeps its own kind.
fn restored_kind(was: Option<DiskState>, kind: Option<ChangeKind>) -> Option<ChangeKind> {
    let returned = was.is_some_and(|w| w != DiskState::Present);
    match kind {
        None | Some(ChangeKind::SeqOnly) if returned => Some(ChangeKind::Restored),
        kind => kind,
    }
}
