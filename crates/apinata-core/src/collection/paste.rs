//! Pasting a request or a folder subtree, copied from the saved files of a source collection,
//! and duplicating a folder beside itself.

use super::tree::{
    Class, INFO_ORDER, create_error, folder_stem, io_error, mapping, new_seq, numbered,
};
use super::{Collection, FOLDER_FILE, Folder, sort_lists, walk};
use crate::document::{Document, set_value};
use crate::error::TreeError;
use crate::item_path::ItemPath;
use crate::js::js_trim;
use crate::naming::{create_unique, create_unique_numbered, suffixed};
use crate::opaque::OpaqueItem;
use std::fs;
use std::path::{Path, PathBuf};
use yaml_serde::{Mapping, Value};

/// What was copied from the source: the saved content of one request or folder subtree.
#[derive(Debug)]
#[non_exhaustive]
pub struct PasteSource {
    collection_root: PathBuf,
    item: ItemPath,
    name: String,
    unsaved: bool,
    entry: Entry,
}

#[derive(Debug)]
enum Entry {
    File {
        file_name: String,
        content: Mapping,
    },
    Dir {
        dir_name: String,
        folder_file: Option<Mapping>,
        children: Vec<Entry>,
    },
}

impl PasteSource {
    /// The display name of the copied item.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether the copied item is a folder.
    pub fn is_folder(&self) -> bool {
        matches!(self.entry, Entry::Dir { .. })
    }

    /// Whether the source item, or anything under it, had edits that were not saved. The copy
    /// holds the saved content only.
    pub fn has_unsaved_edits(&self) -> bool {
        self.unsaved
    }
}

/// Which content of each file a snapshot takes: what was last saved (paste) or what the user
/// sees now, unsaved edits included (duplicate).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Content {
    Saved,
    Current,
}

impl Content {
    fn of(self, doc: &Document) -> &Mapping {
        match self {
            Content::Saved => doc.saved(),
            Content::Current => doc.mapping(),
        }
    }
}

/// The content of `folder` and everything under it, and whether any of it is unsaved.
fn snapshot_dir(folder: &Folder, drafts_below: bool, content: Content) -> (Entry, bool) {
    let mut unsaved = drafts_below || folder.file.as_ref().is_some_and(|f| f.is_dirty());
    let mut children = Vec::new();
    for f in folder.folders.iter().filter(|f| !f.removed) {
        let (entry, dirty) = snapshot_dir(f, false, content);
        unsaved |= dirty;
        children.push(entry);
    }
    for r in folder
        .requests
        .iter()
        .filter(|r| r.disk_state() == crate::document::DiskState::Present)
    {
        unsaved |= r.is_dirty();
        children.push(file_entry(r.file_name(), content.of(r.document())));
    }
    for o in &folder.opaque {
        unsaved |= o.is_dirty();
        children.push(file_entry(o.file_name(), content.of(o.document())));
    }
    // A folder.yml that was never written is not part of the saved content, but one edited in
    // memory is part of what the user sees.
    let folder_file = folder
        .file
        .as_ref()
        .filter(|f| {
            f.document().loaded_hash().is_some() || (content == Content::Current && f.is_dirty())
        })
        .map(|f| content.of(f.document()).clone());
    let entry = Entry::Dir {
        dir_name: folder.dir_name.clone(),
        folder_file,
        children,
    };
    (entry, unsaved)
}

fn file_entry(file_name: &str, content: &Mapping) -> Entry {
    Entry::File {
        file_name: file_name.to_owned(),
        content: content.clone(),
    }
}

/// The top `folder.yml` of a written copy: the source's, or a bare folder file when it had
/// none, with the copy's `name` and `seq`.
fn top_folder_root(folder_file: Option<&Mapping>, name: &str, seq: u64) -> Mapping {
    let mut root = folder_file.cloned().unwrap_or_else(|| {
        mapping(vec![
            (
                "info",
                Value::Mapping(mapping(vec![("type", "folder".into())])),
            ),
            ("request", Value::Mapping(Mapping::new())),
        ])
    });
    set_value(&mut root, &["info", "name"], Value::from(name), INFO_ORDER);
    set_value(
        &mut root,
        &["info", "seq"],
        Value::Number(seq.into()),
        INFO_ORDER,
    );
    root
}

/// `base` with Bruno-style copy suffix number `n`: none for 0, ` (copy)`, ` (copy 2)`...
fn copy_name(base: &str, n: u32) -> String {
    match n {
        0 => base.to_owned(),
        1 => format!("{base} (copy)"),
        n => format!("{base} (copy {n})"),
    }
}

impl Collection {
    /// Take the saved content of the request or folder at `item` to paste with
    /// [`Collection::paste`], here or into another collection. Unsaved edits are left out and
    /// reported by [`PasteSource::has_unsaved_edits`].
    ///
    /// Fails with `InvalidMove` for the collection root and `NotFound` for an unknown item or a
    /// draft.
    pub fn paste_source(&self, item: &ItemPath) -> Result<PasteSource, TreeError> {
        if item.is_root() {
            return Err(TreeError::InvalidMove { item: item.clone() });
        }
        let not_found = || TreeError::NotFound { item: item.clone() };
        if self.is_draft(item) {
            return Err(not_found());
        }
        let (entry, name, unsaved) = if let Some(folder) = self.folder(item) {
            let full = item.to_path(&self.root);
            let drafts = self.drafts.iter().any(|d| d.path().starts_with(&full));
            let (entry, unsaved) = snapshot_dir(folder, drafts, Content::Saved);
            (entry, folder.name().to_owned(), unsaved)
        } else if let Some(r) = self.request(item) {
            let entry = file_entry(r.file_name(), r.document().saved());
            (entry, r.name().to_owned(), r.is_dirty())
        } else if let Some(o) = self.opaque_item(item) {
            let entry = file_entry(o.file_name(), o.document().saved());
            (entry, o.name().to_owned(), o.is_dirty())
        } else {
            return Err(not_found());
        };
        Ok(PasteSource {
            collection_root: self.root.clone(),
            item: item.clone(),
            name,
            unsaved,
            entry,
        })
    }

    /// Write a copy of `source` into the folder `target` (the root is `ItemPath::root()`),
    /// through the writer. A taken file or display name gets ` (copy)`, then ` (copy 2)` and so
    /// on, on both the file and `info.name`. The copy takes the next `seq` among its siblings.
    /// Returns the new item's path.
    ///
    /// Fails with `NotFound` for an unknown target folder, `IntoItself` for a folder pasted into
    /// itself or below, `InvalidName` for a path the ignore rule hides, and `Io` when a write
    /// fails (a half-written folder is removed).
    pub fn paste(
        &mut self,
        target: &ItemPath,
        source: &PasteSource,
    ) -> Result<ItemPath, TreeError> {
        let dir = self.folder(target).ok_or_else(|| TreeError::NotFound {
            item: target.clone(),
        })?;
        let own = source.collection_root == self.root;
        let below = target
            .as_str()
            .starts_with(&format!("{}/", source.item.as_str()));
        if own && source.is_folder() && (*target == source.item || below) {
            return Err(TreeError::IntoItself {
                item: source.item.clone(),
            });
        }
        let dir_path = dir.path.clone();
        let siblings = numbered(target, dir);
        let seq = new_seq(target, dir);
        match &source.entry {
            Entry::File { file_name, content } => {
                let stem = file_name.strip_suffix(".yml").unwrap_or(file_name);
                let names: Vec<&str> = siblings
                    .iter()
                    .filter(|s| s.class != Class::Folder)
                    .map(|s| s.item.name.as_str())
                    .collect();
                // The candidate sequence never ends, so `find` returns only once a name is free.
                #[allow(clippy::expect_used)]
                let n = (0..)
                    .find(|&n| {
                        let file = format!("{}.yml", copy_name(stem, n));
                        !dir_path.join(file).exists()
                            && !names.contains(&copy_name(&source.name, n).as_str())
                    })
                    .expect("an unbounded range has a free name");
                let name = copy_name(&source.name, n);
                let mut root = content.clone();
                set_value(
                    &mut root,
                    &["info", "name"],
                    Value::from(name.as_str()),
                    INFO_ORDER,
                );
                set_value(
                    &mut root,
                    &["info", "seq"],
                    Value::Number(seq.into()),
                    INFO_ORDER,
                );
                let stem = copy_name(stem, n);
                self.refuse_hidden(&dir_path.join(format!("{stem}.yml")), &name)?;
                let (path, _) = create_unique(&dir_path, &stem, Some("yml"), |p| {
                    Document::create(p, root.clone())
                })
                .map_err(|e| create_error(&name, &dir_path, e))?;
                self.insert_file(target, &path)
            }
            Entry::Dir {
                dir_name,
                folder_file,
                children,
            } => {
                let names: Vec<&str> = siblings
                    .iter()
                    .filter(|s| s.class == Class::Folder)
                    .map(|s| s.item.name.as_str())
                    .collect();
                // The candidate sequence never ends, so `find` returns only once a name is free.
                #[allow(clippy::expect_used)]
                let n = (0..)
                    .find(|&n| {
                        !dir_path.join(copy_name(dir_name, n)).exists()
                            && !names.contains(&copy_name(&source.name, n).as_str())
                    })
                    .expect("an unbounded range has a free name");
                let name = copy_name(&source.name, n);
                let stem = copy_name(dir_name, n);
                self.refuse_hidden(&dir_path.join(&stem), &name)?;
                let root = top_folder_root(folder_file.as_ref(), &name, seq);
                self.write_copy(target, &dir_path, &stem, &name, &root, children)
            }
        }
    }

    /// Copy the folder at `item` with everything in it beside itself as `name` (trimmed), unsaved
    /// edits included and drafts left out. Children keep their file names, names and `seq`s;
    /// the copy takes the next `seq` among its siblings, and its directory is named as
    /// [`Collection::create_folder`] names one; a numeric suffix on it goes on the display name
    /// too. The source is left as it was. Returns the copy's path.
    ///
    /// Fails with `InvalidMove` for the root, `NotFound` unless `item` is a folder,
    /// `InvalidName` for a name `create_folder` refuses, and `Io` when a write fails (the
    /// half-written copy is removed and the tree is untouched).
    pub fn clone_folder(&mut self, item: &ItemPath, name: &str) -> Result<ItemPath, TreeError> {
        let parent = item
            .parent()
            .ok_or_else(|| TreeError::InvalidMove { item: item.clone() })?;
        let not_found = || TreeError::NotFound { item: item.clone() };
        if self.is_draft(item) {
            return Err(not_found());
        }
        let folder = self.folder(item).ok_or_else(not_found)?;
        let (
            Entry::Dir {
                folder_file,
                children,
                ..
            },
            _,
        ) = snapshot_dir(folder, false, Content::Current)
        else {
            unreachable!("a folder snapshots as a directory");
        };
        let name = js_trim(name);
        let stem = folder_stem(&parent, name)?;
        let dir = self.folder(&parent).ok_or_else(not_found)?;
        let seq = new_seq(&parent, dir);
        let dir_path = dir.path.clone();
        self.refuse_hidden(&dir_path.join(&stem), name)?;
        let root = top_folder_root(folder_file.as_ref(), name, seq);
        self.write_copy(&parent, &dir_path, &stem, name, &root, &children)
    }

    /// Write a folder copy as a new directory `stem` (numbered when taken) in `parent`, whose
    /// directory is `dir_path`, and load it. A failed write removes the whole new directory.
    fn write_copy(
        &mut self,
        parent: &ItemPath,
        dir_path: &Path,
        stem: &str,
        name: &str,
        root: &Mapping,
        children: &[Entry],
    ) -> Result<ItemPath, TreeError> {
        let (path, n) =
            create_unique_numbered(dir_path, stem, None, |p, n| fs::create_dir(p).map(|()| n))
                .map_err(|e| create_error(name, dir_path, e))?;
        // A numbered directory numbers the copy's display name the same way.
        let mut root = root.clone();
        if n > 0 {
            let shown = suffixed(name, None, n);
            set_value(&mut root, &["info", "name"], Value::from(shown), INFO_ORDER);
        }
        if let Err(e) = write_dir(&path, &root, children) {
            let _ = fs::remove_dir_all(&path);
            return Err(e);
        }
        self.insert_dir(parent, &path)
    }

    /// Load the file just written at `path` into the folder `parent`.
    pub(super) fn insert_file(
        &mut self,
        parent: &ItemPath,
        path: &Path,
    ) -> Result<ItemPath, TreeError> {
        if let Some(opaque) = OpaqueItem::load(path) {
            let folder = self.folder_mut(parent).ok_or_else(|| TreeError::NotFound {
                item: parent.clone(),
            })?;
            folder.opaque.push(opaque);
            sort_lists(folder);
            return super::tree::child(parent, path);
        }
        self.insert_request(parent, path)
    }

    /// Load the directory just written at `path` into the folder `parent`.
    // The caller found `parent` before writing, and a directory name is a single path segment.
    #[allow(clippy::expect_used)]
    fn insert_dir(&mut self, parent: &ItemPath, path: &Path) -> Result<ItemPath, TreeError> {
        let mut errors = Vec::new();
        let folder = walk(path, &self.ignore, &mut errors)
            .map_err(|e| io_error(path, std::io::Error::other(e.to_string())))?;
        let dir_name = folder.dir_name.clone();
        self.load_errors.extend(errors);
        let parent_folder = self
            .folder_mut(parent)
            .expect("the parent folder was found before the write");
        parent_folder.folders.push(folder);
        sort_lists(parent_folder);
        Ok(parent
            .join(&dir_name)
            .expect("a directory name is a valid segment"))
    }
}

/// Write `folder.yml` and every child into the existing directory `dir`.
fn write_dir(dir: &Path, folder_root: &Mapping, children: &[Entry]) -> Result<(), TreeError> {
    let folder_file = dir.join(FOLDER_FILE);
    Document::create(&folder_file, folder_root.clone()).map_err(|e| io_error(&folder_file, e))?;
    write_children(dir, children)
}

fn write_children(dir: &Path, children: &[Entry]) -> Result<(), TreeError> {
    for child in children {
        match child {
            Entry::File { file_name, content } => {
                let path = dir.join(file_name);
                Document::create(&path, content.clone()).map_err(|e| io_error(&path, e))?;
            }
            Entry::Dir {
                dir_name,
                folder_file,
                children,
            } => {
                let path = dir.join(dir_name);
                fs::create_dir(&path).map_err(|e| io_error(&path, e))?;
                if let Some(root) = folder_file {
                    let file = path.join(FOLDER_FILE);
                    Document::create(&file, root.clone()).map_err(|e| io_error(&file, e))?;
                }
                write_children(&path, children)?;
            }
        }
    }
    Ok(())
}
