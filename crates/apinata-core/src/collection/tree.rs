//! Tree operations: create, clone, rename, delete and move for requests and folders, written
//! to disk at once with Bruno's file names, layout and `seq` values.

use super::{Collection, ENVIRONMENTS_DIR, FOLDER_FILE, Folder, sort_lists, walk};
use crate::config_file::FolderFile;
use crate::document::{DiskState, Document, set_value};
use crate::error::TreeError;
use crate::item_path::ItemPath;
use crate::js::js_trim;
use crate::naming::{
    create_unique, create_unique_numbered, sanitize_name, suffixed, unique_rename_target,
    validate_name,
};
use crate::opaque::OpaqueItem;
use crate::order;
use crate::request::{RequestFile, RequestKind};
use crate::sequence::{
    SeqItem, place_restored, reorder_source, reorder_target, reset_order, return_ranked, sort_names,
};
use crate::trash::{Trash, TrashError, TrashHandle};
use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use yaml_serde::{Mapping, Value};

/// Where a dragged item is dropped: into a folder, or above or below a sibling.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DropTarget {
    /// Into this folder; a numbered item from another folder takes the next seq there.
    Inside(ItemPath),
    /// Above this sibling.
    Before(ItemPath),
    /// Below this sibling.
    After(ItemPath),
}

/// How Bruno's numbering classes a sibling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Class {
    Folder,
    Request,
    App,
}

/// One sibling Bruno numbers: where it is, its class and the name and seq it sorts by.
#[derive(Debug)]
pub(super) struct Numbered {
    pub(super) path: ItemPath,
    pub(super) class: Class,
    /// The folder's `folder.yml` failed to load, so it is counted but never renumbered.
    pub(super) file_broken: bool,
    pub(super) item: SeqItem,
    /// The seq on disk; `item.seq` reads a request or app item without one as 1.
    pub(super) stored: Option<u64>,
}

/// What [`Collection::remove_with`] did: the remover's value, the item's tail (the siblings
/// after it, as (path, old seq, new seq)), the other siblings it changed and whether the
/// renumbering itself succeeded.
struct Removal<T> {
    value: T,
    renumbered: Vec<(ItemPath, u64, u64)>,
    ahead: Vec<(ItemPath, u64, u64)>,
    renumber: Result<(), TreeError>,
}

/// An item [`Collection::trash`] moved to a bin, with what undoing it needs.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Trashed {
    item: ItemPath,
    handle: TrashHandle,
    /// The tail: the numbered siblings after the item at delete time, changed or not.
    renumbered: Vec<(ItemPath, u64, u64)>,
    /// The changed siblings the tail cannot vouch for, as (path, old seq, new seq): those outside
    /// it, and those it only ranks. They never place the item; an old seq of 0 means none stored.
    ahead: Vec<(ItemPath, u64, u64)>,
    renumber_failure: Option<String>,
}

impl Trashed {
    /// Where the item was in the collection.
    pub fn item(&self) -> &ItemPath {
        &self.item
    }

    /// Why a sibling could not be renumbered after the item was trashed, if one could not.
    pub fn renumber_failure(&self) -> Option<&str> {
        self.renumber_failure.as_deref()
    }
    /// The bin's handle and the tail, the siblings after the item at delete time as (path, old
    /// seq, new seq), for the log.
    pub(crate) fn parts(&self) -> (&TrashHandle, &[(ItemPath, u64, u64)]) {
        (&self.handle, &self.renumbered)
    }

    /// The changed siblings the tail cannot vouch for, for the log.
    pub(crate) fn ahead(&self) -> &[(ItemPath, u64, u64)] {
        &self.ahead
    }

    /// A trashed item read back from the log; it carries no renumber failure.
    pub(crate) fn from_parts(
        item: ItemPath,
        handle: TrashHandle,
        renumbered: Vec<(ItemPath, u64, u64)>,
        ahead: Vec<(ItemPath, u64, u64)>,
    ) -> Trashed {
        Trashed {
            item,
            handle,
            renumbered,
            ahead,
            renumber_failure: None,
        }
    }
}

/// A tree node taken out of its folder, to be re-attached after a rename on disk.
enum Node {
    Folder(Folder),
    Request(RequestFile),
    Opaque(OpaqueItem),
}

pub(super) const INFO_ORDER: &[&[&str]] = &[order::TOP, order::INFO];

/// The siblings after `item` in `list`, in order, as (path, old seq, new seq), with `changes`
/// giving the new seqs that differ. A sibling without a seq, or with a seq of 0, which sorts the
/// same, stands at its rank; an item not in `list` has no tail.
fn tail_of(
    list: &[Numbered],
    item: &ItemPath,
    changes: &[(ItemPath, u64)],
) -> Vec<(ItemPath, u64, u64)> {
    let items: Vec<SeqItem> = list.iter().map(|n| n.item.clone()).collect();
    let order = reset_order(&items);
    let at = order.iter().position(|&i| list[i].path == *item);
    // A restore drops tail members numbered below the item, so a rank never falls under what
    // precedes it.
    let mut floor = at.and_then(|at| list[order[at]].item.seq).unwrap_or(0);
    (order.iter().zip(1u64..))
        .skip(at.map_or(order.len(), |at| at + 1))
        .map(|(&i, rank)| {
            let path = &list[i].path;
            let old = list[i].item.seq.filter(|&s| s > 0);
            let old = old.unwrap_or(rank.max(floor));
            floor = floor.max(old);
            let new = changes.iter().find(|(p, _)| p == path);
            (path.clone(), old, new.map_or(old, |(_, seq)| *seq))
        })
        .collect()
}

/// The siblings in `changes` that `tail` cannot vouch for, as (path, old seq, new seq): those
/// outside it, and those it gave a rank for want of a stored seq. An old seq of 0 stands for none
/// stored, so it never reads as a number the sibling held.
fn ahead_of(
    list: &[Numbered],
    tail: &[(ItemPath, u64, u64)],
    changes: &[(ItemPath, u64)],
) -> Vec<(ItemPath, u64, u64)> {
    changes
        .iter()
        .filter_map(|(path, new)| {
            let stored = list.iter().find(|n| n.path == *path)?.stored;
            let stored = stored.filter(|&s| s > 0);
            let vouched = stored.is_some() && tail.iter().any(|(p, ..)| p == path);
            (!vouched).then(|| (path.clone(), stored.unwrap_or(0), *new))
        })
        .collect()
}

/// The siblings in `dir` that Bruno numbers: folders, `http` requests, and opaque items of a
/// request type or `app`. An opaque item of any other kind is not a Bruno item and is skipped,
/// as is a draft kept after its file or directory was removed: it is not on disk to count.
pub(super) fn numbered(dir_path: &ItemPath, dir: &Folder) -> Vec<Numbered> {
    let mut out = Vec::new();
    let mut push = |name: &str, class, file_broken, display: &str, seq, stored| {
        if let Some(path) = dir_path.join(name) {
            out.push(Numbered {
                path,
                class,
                file_broken,
                item: SeqItem {
                    name: display.to_owned(),
                    seq,
                },
                stored,
            });
        }
    };
    for f in dir.folders.iter().filter(|f| !f.removed) {
        let seq = f.seq();
        push(
            &f.dir_name,
            Class::Folder,
            f.file_broken,
            f.name(),
            seq,
            seq,
        );
    }
    let requests = dir.requests.iter();
    for r in requests.filter(|r| r.disk_state() != DiskState::Removed) {
        let seq = Some(r.seq().filter(|&s| s > 0).unwrap_or(1));
        push(r.file_name(), Class::Request, false, r.name(), seq, r.seq());
    }
    for o in &dir.opaque {
        match o.kind() {
            "graphql" | "grpc" | "websocket" => {
                let seq = Some(o.seq().filter(|&s| s > 0).unwrap_or(1));
                push(o.file_name(), Class::Request, false, o.name(), seq, o.seq());
            }
            "app" => push(
                o.file_name(),
                Class::App,
                false,
                o.name(),
                Some(o.seq().unwrap_or(1)),
                o.seq(),
            ),
            _ => {}
        }
    }
    out
}

/// The file a new request named `name` of `kind` starts as: an empty URL, inherited auth and
/// Bruno's default settings, with GET for HTTP and POST for GraphQL. The timeout is `inherit`
/// rather than Bruno's `0`, so the default timeout applies to it, as in
/// `RequestSettings::for_new_file`.
pub(crate) fn new_request_root(name: &str, seq: u64, kind: RequestKind) -> Mapping {
    let (key, method) = match kind {
        RequestKind::Graphql => ("graphql", "POST"),
        RequestKind::Http => ("http", "GET"),
    };
    mapping(vec![
        (
            "info",
            Value::Mapping(mapping(vec![
                ("name", name.into()),
                ("type", key.into()),
                ("seq", Value::Number(seq.into())),
            ])),
        ),
        (
            key,
            Value::Mapping(mapping(vec![
                ("method", method.into()),
                ("url", "".into()),
                ("auth", "inherit".into()),
            ])),
        ),
        (
            "settings",
            Value::Mapping(mapping(vec![
                ("encodeUrl", true.into()),
                ("timeout", "inherit".into()),
                ("followRedirects", true.into()),
                ("maxRedirects", Value::Number(5.into())),
                ("forwardAuthorizationHeader", false.into()),
            ])),
        ),
    ])
}

pub(super) fn mapping(entries: Vec<(&str, Value)>) -> Mapping {
    entries
        .into_iter()
        .map(|(k, v)| (Value::from(k), v))
        .collect()
}

pub(super) fn io_error(path: &Path, source: std::io::Error) -> TreeError {
    TreeError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// `InvalidName` for a name the file system refuses as too long, else `Io`.
pub(super) fn create_error(name: &str, path: &Path, source: std::io::Error) -> TreeError {
    if source.kind() == ErrorKind::InvalidFilename {
        TreeError::InvalidName {
            name: name.to_owned(),
        }
    } else {
        io_error(path, source)
    }
}

/// The checks Bruno's dialogs make, before any I/O: a non-blank name whose sanitised stem is a
/// valid file name. Returns the stem.
fn checked_stem(name: &str) -> Result<String, TreeError> {
    let invalid = || TreeError::InvalidName {
        name: name.to_owned(),
    };
    if js_trim(name).is_empty() {
        return Err(invalid());
    }
    let stem = sanitize_name(name);
    if validate_name(&stem) {
        Ok(stem)
    } else {
        Err(invalid())
    }
}

/// A request's file stem: [`checked_stem`], minus the stems Bruno's dialog reserves.
fn request_stem(name: &str) -> Result<String, TreeError> {
    let stem = checked_stem(name)?;
    if stem == "collection" || stem == "folder" {
        return Err(TreeError::InvalidName {
            name: name.to_owned(),
        });
    }
    Ok(stem)
}

/// A folder's directory name: [`checked_stem`], minus `environments` in the root.
pub(super) fn folder_stem(parent: &ItemPath, name: &str) -> Result<String, TreeError> {
    let stem = checked_stem(name)?;
    if parent.is_root() && stem.to_lowercase().contains(ENVIRONMENTS_DIR) {
        return Err(TreeError::InvalidName {
            name: name.to_owned(),
        });
    }
    Ok(stem)
}

impl Collection {
    /// Create a request named `name` in the folder `parent`, as a GET with an empty URL and the
    /// next `seq` among its siblings. The file is `<sanitised name>.yml`, with Bruno's numeric
    /// suffix when that is taken. Returns the new item's path.
    ///
    /// Fails with `NotFound` for an unknown folder, `InvalidName` for a name that cannot be a
    /// file name (or is `collection` or `folder`, or names a path the ignore rule hides), and
    /// `Io` when the write fails.
    pub fn create_request(&mut self, parent: &ItemPath, name: &str) -> Result<ItemPath, TreeError> {
        self.create_request_of_kind(parent, name, RequestKind::Http)
    }

    /// Create a request of `kind` like [`Collection::create_request`]; a GraphQL request starts
    /// as a POST under the `graphql` key. Fails like [`Collection::create_request`].
    pub fn create_request_of_kind(
        &mut self,
        parent: &ItemPath,
        name: &str,
        kind: RequestKind,
    ) -> Result<ItemPath, TreeError> {
        let (stem, seq) = self.new_request_slot(parent, name)?;
        self.write_new_request(parent, &stem, name, new_request_root(name, seq, kind))
    }

    /// Create a request like [`Collection::create_request_of_kind`], from the collection's
    /// presets: it takes `kind` when given, else the preset kind, and starts with the preset
    /// URL. Fails like [`Collection::create_request`].
    pub fn create_request_from_presets(
        &mut self,
        parent: &ItemPath,
        name: &str,
        kind: Option<RequestKind>,
    ) -> Result<ItemPath, TreeError> {
        let kind = kind.unwrap_or_else(|| self.file.presets().kind());
        let (stem, seq) = self.new_request_slot(parent, name)?;
        let root = self.preset_request_root(name, seq, kind);
        self.write_new_request(parent, &stem, name, root)
    }

    /// The template of [`new_request_root`], with the collection's preset URL when it has one.
    pub(super) fn preset_request_root(&self, name: &str, seq: u64, kind: RequestKind) -> Mapping {
        let mut root = new_request_root(name, seq, kind);
        let url = self.file.presets().request_url;
        if !url.is_empty() {
            let key = match kind {
                RequestKind::Graphql => "graphql",
                _ => "http",
            };
            set_value(&mut root, &[key, "url"], Value::String(url), &[]);
        }
        root
    }

    /// Check a new request named `name` may go in the folder `parent`, before any write, and
    /// return its file stem and the `seq` it gets. Fails like [`Collection::create_request`].
    pub(super) fn new_request_slot(
        &self,
        parent: &ItemPath,
        name: &str,
    ) -> Result<(String, u64), TreeError> {
        let stem = request_stem(name)?;
        let dir = self.folder(parent).ok_or_else(|| TreeError::NotFound {
            item: parent.clone(),
        })?;
        self.refuse_hidden(&dir.path.join(format!("{stem}.yml")), name)?;
        Ok((stem, new_seq(parent, dir)))
    }

    /// Write `root` as `<stem>.yml` in the folder `parent`, with Bruno's numeric suffix when
    /// that is taken, and load it into the tree. `name` is reported in an `InvalidName`.
    pub(super) fn write_new_request(
        &mut self,
        parent: &ItemPath,
        stem: &str,
        name: &str,
        root: Mapping,
    ) -> Result<ItemPath, TreeError> {
        let dir_path = self
            .folder(parent)
            .ok_or_else(|| TreeError::NotFound {
                item: parent.clone(),
            })?
            .path
            .clone();
        let (path, _) = create_unique(&dir_path, stem, Some("yml"), |p| {
            Document::create(p, root.clone())
        })
        .map_err(|e| create_error(name, &dir_path, e))?;
        self.insert_request(parent, &path)
    }

    /// Create a folder named `name` in the folder `parent`, with Bruno's `folder.yml` and the
    /// next `seq` among its siblings. The directory is the sanitised name, with Bruno's numeric
    /// suffix when that is taken. Returns the new folder's path.
    ///
    /// Fails like [`Collection::create_request`]; in the root the name may not contain
    /// `environments`, which Bruno reserves.
    pub fn create_folder(&mut self, parent: &ItemPath, name: &str) -> Result<ItemPath, TreeError> {
        let stem = folder_stem(parent, name)?;
        let dir = self.folder(parent).ok_or_else(|| TreeError::NotFound {
            item: parent.clone(),
        })?;
        let seq = new_seq(parent, dir);
        let dir_path = dir.path.clone();
        self.refuse_hidden(&dir_path.join(&stem), name)?;
        let root = mapping(vec![
            (
                "info",
                Value::Mapping(mapping(vec![
                    ("name", name.into()),
                    ("type", "folder".into()),
                    ("seq", Value::Number(seq.into())),
                ])),
            ),
            (
                "request",
                Value::Mapping(mapping(vec![("auth", "inherit".into())])),
            ),
        ]);
        let (path, ()) = create_unique(&dir_path, &stem, None, |p| fs::create_dir(p))
            .map_err(|e| create_error(name, &dir_path, e))?;
        let folder_file = path.join(FOLDER_FILE);
        if let Err(e) = Document::create(&folder_file, root) {
            let _ = fs::remove_dir(&path);
            return Err(io_error(&folder_file, e));
        }
        let mut errors = Vec::new();
        let folder = walk(&path, &self.ignore, &mut errors)
            .map_err(|e| io_error(&path, std::io::Error::other(e.to_string())))?;
        let dir_name = folder.dir_name.clone();
        self.load_errors.extend(errors);
        let parent_folder = self.folder_mut(parent).expect("parent exists");
        parent_folder.folders.push(folder);
        sort_lists(parent_folder);
        Ok(parent
            .join(&dir_name)
            .expect("a directory name is a valid segment"))
    }
}

impl Collection {
    /// Copy the request at `item`, unsaved edits included, beside it as `name` (trimmed), with
    /// the next `seq` among its siblings. When the file name is taken, the file and the display
    /// name get the same numeric suffix. The source is left as it was. Returns the copy's path.
    ///
    /// Fails with `NotFound` unless `item` is an `http` request, and otherwise like
    /// [`Collection::create_request`].
    pub fn clone_request(&mut self, item: &ItemPath, name: &str) -> Result<ItemPath, TreeError> {
        let not_found = || TreeError::NotFound { item: item.clone() };
        if self.is_draft(item) {
            return Err(not_found());
        }
        let source = self.request(item).ok_or_else(not_found)?;
        let root = source.document().mapping().clone();
        self.clone_file(item, root, name)
    }

    /// Copy the opaque item at `item` beside it as `name` (trimmed), with the next `seq` among
    /// its siblings and every other field as it is. Its file is named as
    /// [`Collection::clone_request`] names one. Returns the copy's path.
    ///
    /// Fails with `NotFound` unless `item` is an opaque item, and otherwise like
    /// [`Collection::clone_request`].
    pub fn clone_opaque(&mut self, item: &ItemPath, name: &str) -> Result<ItemPath, TreeError> {
        let not_found = || TreeError::NotFound { item: item.clone() };
        let source = self.opaque_item(item).ok_or_else(not_found)?;
        let root = source.document().mapping().clone();
        self.clone_file(item, root, name)
    }

    /// Write `root`, renamed to `name` and given the next `seq`, as a new file beside `item`.
    fn clone_file(
        &mut self,
        item: &ItemPath,
        mut root: Mapping,
        name: &str,
    ) -> Result<ItemPath, TreeError> {
        let not_found = || TreeError::NotFound { item: item.clone() };
        let parent = item.parent().ok_or_else(not_found)?;
        let name = js_trim(name);
        let stem = request_stem(name)?;
        let dir = self.folder(&parent).ok_or_else(not_found)?;
        let seq = new_seq(&parent, dir);
        let dir_path = dir.path.clone();
        self.refuse_hidden(&dir_path.join(format!("{stem}.yml")), name)?;
        set_value(
            &mut root,
            &["info", "seq"],
            Value::Number(seq.into()),
            INFO_ORDER,
        );
        // A taken file name gets a numeric suffix, and the display name the same one, so two
        // copies under one name stay apart in the tree.
        let (path, _) = create_unique_numbered(&dir_path, &stem, Some("yml"), |p, n| {
            let mut root = root.clone();
            let shown = suffixed(name, None, n);
            set_value(&mut root, &["info", "name"], Value::from(shown), INFO_ORDER);
            Document::create(p, root)
        })
        .map_err(|e| create_error(name, &dir_path, e))?;
        self.insert_file(&parent, &path)
    }

    /// Set the display name of the request or folder at `item`, writing it at once; unsaved
    /// edits stay unsaved. With `rename_file` the file or directory is then renamed to the
    /// sanitised name, with Bruno's numeric suffix when that is taken. Returns the item's path.
    ///
    /// Fails with `InvalidMove` for the root, `NotFound` for an unknown or opaque item,
    /// `InvalidName` for a blank or unusable name (one the ignore rule hides included), and `Conflict` or `Io` when a write fails.
    pub fn rename(
        &mut self,
        item: &ItemPath,
        name: &str,
        rename_file: bool,
    ) -> Result<ItemPath, TreeError> {
        let parent = item
            .parent()
            .ok_or_else(|| TreeError::InvalidMove { item: item.clone() })?;
        let not_found = || TreeError::NotFound { item: item.clone() };
        if self.is_draft(item) {
            return Err(not_found());
        }
        let is_request = self.request(item).is_some();
        if !is_request && self.folder(item).is_none() {
            return Err(not_found());
        }
        if js_trim(name).is_empty() {
            return Err(TreeError::InvalidName {
                name: name.to_owned(),
            });
        }
        // Every name check runs before the first write.
        let target_name = match (rename_file, is_request) {
            (false, _) => None,
            (true, true) => Some(format!("{}.yml", request_stem(name)?)),
            (true, false) => Some(folder_stem(&parent, name)?),
        };
        let old = item.to_path(&self.root);
        if let Some(target_name) = &target_name {
            self.refuse_hidden(&old.with_file_name(target_name), name)?;
        }
        if is_request {
            let request = self.request_mut(item).ok_or_else(not_found)?;
            let doc = request.document_mut();
            doc.write_through(&["info", "name"], Value::from(name), INFO_ORDER)?;
        } else {
            let folder = self.folder_mut(item).ok_or_else(not_found)?;
            write_folder(folder, Some(name), None)?;
        }
        let Some(target_name) = target_name else {
            return Ok(item.clone());
        };
        let target = unique_rename_target(&old, &old.with_file_name(target_name))
            .map_err(|e| io_error(&old, e))?;
        if target != old {
            fs::rename(&old, &target).map_err(|e| io_error(&old, e))?;
            let node = self.detach(item).ok_or_else(not_found)?;
            self.attach(&parent, node, &old, &target);
        }
        child(&parent, &target)
    }

    /// Delete the request, opaque item or folder (with everything in it) at `item`, then
    /// renumber the remaining folders and request-type siblings as Bruno does.
    ///
    /// Fails with `InvalidMove` for the root, `NotFound` for an unknown item, `Conflict` when a
    /// sibling to renumber changed on disk (before anything is deleted), and `Io` when a file
    /// system call fails before anything is deleted. Once the item is gone, a sibling that
    /// cannot be renumbered fails with `DeletedRenumber` (the delete stands), and the
    /// collection is reloaded so memory matches the disk.
    pub fn delete(&mut self, item: &ItemPath) -> Result<(), TreeError> {
        let removal = self.remove_with(item, |full, is_folder| {
            let removed = if is_folder {
                fs::remove_dir_all(full)
            } else {
                fs::remove_file(full)
            };
            removed.map_err(|e| io_error(full, e))
        })?;
        removal.renumber
    }

    /// Like [`Collection::delete`], but the item goes into `bin` and the returned [`Trashed`]
    /// brings it back through [`Collection::restore_trashed`].
    ///
    /// Fails with `Trash` when the bin refuses (the item stays where it was) and otherwise as
    /// `delete` does. A sibling that could not be renumbered is reported by
    /// [`Trashed::renumber_failure`]: the item is in the bin all the same.
    pub fn trash(&mut self, item: &ItemPath, bin: &dyn Trash) -> Result<Trashed, TreeError> {
        let removal = self.remove_with(item, |full, _| {
            bin.trash(full).map_err(|e| TreeError::Trash {
                message: e.to_string(),
            })
        })?;
        Ok(Trashed {
            item: item.clone(),
            handle: removal.value,
            renumbered: removal.renumbered,
            ahead: removal.ahead,
            renumber_failure: removal.renumber.err().map(|e| e.to_string()),
        })
    }

    /// Put an item back where [`Collection::trash`] took it from. The seqs of its folder end up
    /// 1..n, with the item placed by its tail.
    ///
    /// Fails with `RestoreClash` when something is at the original path now, leaving the item
    /// in the bin: a restore never overwrites. Fails with `Trash` when the bin cannot restore.
    pub fn restore_trashed(&mut self, trashed: &Trashed, bin: &dyn Trash) -> Result<(), TreeError> {
        self.restore_file(trashed, bin)?;
        match trashed.item.parent() {
            Some(parent) => self.renumber_restored(&parent, &[trashed]),
            None => Ok(()),
        }
    }

    /// Take the item back from `bin` and load it, leaving every sibling's number alone. Fails
    /// as [`Collection::restore_trashed`] does.
    pub(crate) fn restore_file(
        &mut self,
        trashed: &Trashed,
        bin: &dyn Trash,
    ) -> Result<(), TreeError> {
        let full = trashed.item.to_path(&self.root);
        if fs::symlink_metadata(&full).is_ok() {
            return Err(TreeError::RestoreClash { path: full });
        }
        bin.restore(&trashed.handle).map_err(|e| match e {
            TrashError::Missing => TreeError::NotInTrash,
            e => TreeError::Trash {
                message: e.to_string(),
            },
        })?;
        self.reload_path(&full);
        Ok(())
    }

    /// Number `folder` 1..n with `items`, which are back on disk, in the places their tails
    /// give; see [`crate::sequence::place_restored`]. Numbers stored by siblings that are never
    /// renumbered, such as app items, are skipped, except by a sibling that held one before the
    /// oldest of these deletes, which keeps its tie. `items` run newest delete first. Fails with
    /// `Conflict` before any write when a file to rewrite is not as loaded.
    pub(crate) fn renumber_restored(
        &mut self,
        folder: &ItemPath,
        items: &[&Trashed],
    ) -> Result<(), TreeError> {
        let Some(dir) = self.folder(folder) else {
            return Ok(());
        };
        let (list, held): (Vec<Numbered>, Vec<Numbered>) = numbered(folder, dir)
            .into_iter()
            .partition(|n| n.class != Class::App && !n.file_broken);
        let held: BTreeSet<u64> = held.iter().filter_map(|n| n.stored).collect();
        let restored = |path: &ItemPath| items.iter().any(|t| t.item == *path);
        let rest: Vec<&Numbered> = list.iter().filter(|n| !restored(&n.path)).collect();
        let rest_items: Vec<SeqItem> = rest.iter().map(|n| n.item.clone()).collect();
        let order = reset_order(&rest_items)
            .into_iter()
            .map(|i| rest[i].path.clone())
            .collect();
        // An item that failed to load is not a sibling, so it takes no number.
        let placed: Vec<(ItemPath, Vec<ItemPath>)> = items
            .iter()
            .filter_map(|t| {
                let own = list.iter().find(|n| n.path == t.item)?.item.seq;
                // Records from before the tail was exact also name siblings ahead of the item.
                let after = |old: &u64| own.is_none_or(|own| *old >= own);
                let tail = t.renumbered.iter().filter(|(_, old, _)| after(old));
                let tail = tail.map(|(p, ..)| p.clone());
                Some((t.item.clone(), tail.collect()))
            })
            .collect();
        // Later records hold numbers the deletes themselves produced, so only a sibling's oldest
        // record, or its file when none names it, says what it held.
        let tied = |path: &ItemPath, seq: u64| {
            let oldest_first = items.iter().rev();
            let mut records = oldest_first.flat_map(|t| t.ahead.iter().chain(&t.renumbered));
            let first = records.find(|(p, ..)| p == path).map(|(_, old, _)| *old);
            first.or_else(|| list.iter().find(|n| n.path == *path)?.stored) == Some(seq)
        };
        // A sibling at another number than these deletes left it was moved on purpose, so it
        // neither moves back nor counts as one to move behind.
        let settled = |path: &ItemPath| {
            let mut records = items
                .iter()
                .flat_map(|t| t.ahead.iter().chain(&t.renumbered));
            let last = records.find(|(p, ..)| p == path).map(|(.., new)| *new);
            let now = list
                .iter()
                .find(|n| n.path == *path)
                .and_then(|n| n.item.seq);
            last.is_some() && last == now
        };
        let mut placed = place_restored(order, &placed);
        for t in items {
            let tail = t.renumbered.iter().map(|(p, ..)| p).filter(|p| settled(p));
            let tail: Vec<&ItemPath> = tail.collect();
            let ranked = |p: &ItemPath| t.ahead.iter().any(|(q, old, _)| q == p && *old == 0);
            return_ranked(&mut placed, &tail, ranked);
        }
        let mut next = 1u64;
        let changes: Vec<(ItemPath, u64)> = placed
            .into_iter()
            .map(|path| {
                while held.contains(&next) && !tied(&path, next) {
                    next += 1;
                }
                next += 1;
                (path, next - 1)
            })
            .filter(|(path, seq)| {
                !list
                    .iter()
                    .any(|n| n.path == *path && n.item.seq == Some(*seq))
            })
            .collect();
        self.check_targets(&changes)?;
        self.write_seqs(changes)
    }

    /// The shared body of `delete` and `trash`: check the renumbering, let `remove` take the
    /// item off the disk (given its path and whether it is a folder), then renumber the
    /// siblings. `remove` runs after every check and before any write. The result's tail is
    /// every numbered sibling after the item, whether or not its number changed.
    fn remove_with<T>(
        &mut self,
        item: &ItemPath,
        remove: impl FnOnce(&Path, bool) -> Result<T, TreeError>,
    ) -> Result<Removal<T>, TreeError> {
        let parent = item
            .parent()
            .ok_or_else(|| TreeError::InvalidMove { item: item.clone() })?;
        let not_found = || TreeError::NotFound { item: item.clone() };
        if self.is_draft(item) {
            return Err(not_found());
        }
        let is_folder = self.folder(item).is_some();
        if !is_folder && self.request(item).is_none() && self.opaque_item(item).is_none() {
            return Err(not_found());
        }
        let dir = self.folder(&parent).ok_or_else(not_found)?;
        // Bruno renumbers folders and request types only; a broken folder.yml is left alone.
        let list: Vec<Numbered> = numbered(&parent, dir)
            .into_iter()
            .filter(|n| n.class != Class::App && !n.file_broken)
            .collect();
        let rest: Vec<&Numbered> = list.iter().filter(|n| n.path != *item).collect();
        let items: Vec<SeqItem> = rest.iter().map(|n| n.item.clone()).collect();
        let changes: Vec<(ItemPath, u64)> = reorder_source(&items)
            .into_iter()
            .map(|(i, seq)| (rest[i].path.clone(), seq))
            .collect();
        self.check_targets(&changes)?;
        let renumbered = tail_of(&list, item, &changes);
        let ahead = ahead_of(&list, &renumbered, &changes);
        let full = item.to_path(&self.root);
        let value = match remove(&full, is_folder) {
            Ok(value) => value,
            Err(error) => {
                // A removal that fails part-way has already deleted some files: reload to
                // match disk.
                self.reload();
                return Err(error);
            }
        };
        self.detach(item);
        self.load_errors.retain(|e| !e.path().starts_with(&full));
        self.drafts.retain(|d| !d.path().starts_with(&full));
        // The removal is done, so a renumber failure must not read as a failed delete.
        let renumber = self
            .write_seqs(changes)
            .map_err(|source| TreeError::DeletedRenumber {
                source: Box::new(source),
            });
        Ok(Removal {
            value,
            renumbered,
            ahead,
            renumber,
        })
    }

    /// Move the item at `item` as a drop in Bruno's sidebar does, renumbering the source and
    /// target directories; a taken file name in the target gets Bruno's numeric suffix. A
    /// numbered item dropped `Inside` another folder takes the next `seq` there. Returns the
    /// item's new path. A drop `Inside` the item's own parent changes nothing.
    ///
    /// Fails with `NotFound` for an unknown item or target, `InvalidMove` for the root, a drop
    /// on the item or under it, `Inside` a non-folder or beside the root, or into a path the
    /// ignore rule hides, and `Conflict` or `Io` like [`Collection::delete`].
    /// `Renumber` means the item moved but a sibling's seq could not be written.
    pub fn move_item(&mut self, item: &ItemPath, to: DropTarget) -> Result<ItemPath, TreeError> {
        let invalid = || TreeError::InvalidMove { item: item.clone() };
        let source_dir = item.parent().ok_or_else(invalid)?;
        let (target, below) = match &to {
            DropTarget::Inside(t) => (t, None),
            DropTarget::Before(t) => (t, Some(false)),
            DropTarget::After(t) => (t, Some(true)),
        };
        for path in [item, target] {
            if !self.exists(path) {
                return Err(TreeError::NotFound { item: path.clone() });
            }
        }
        let under_item = target.as_str().starts_with(&format!("{}/", item.as_str()));
        if target == item || under_item {
            return Err(invalid());
        }
        let dir = match below {
            None if self.folder(target).is_some() => target.clone(),
            None => return Err(invalid()),
            Some(_) => target.parent().ok_or_else(invalid)?,
        };
        if dir != source_dir && self.is_ignored(&dir.to_path(&self.root).join(item.file_name())) {
            return Err(invalid());
        }
        let source_list = self.renumberable(&source_dir);
        let dragged = source_list
            .iter()
            .find(|n| n.path == *item)
            .map(|n| n.item.clone());
        if dir == source_dir {
            let (Some(below), Some(dragged)) = (below, dragged) else {
                return Ok(item.clone());
            };
            let at = |p: &ItemPath| source_list.iter().position(|n| n.path == *p);
            let (Some(t), Some(d)) = (at(target), at(item)) else {
                return Ok(item.clone());
            };
            let changes = reorder_target(&items_of(&source_list), t, d, below, dragged.seq);
            let changes = renumbered(&source_list, changes);
            self.check_targets(&changes)?;
            self.write_seqs(changes)?;
            return Ok(item.clone());
        }

        // Another directory: the source closes the gap; a drop beside a target lists the item
        // last with the target's seq, as Bruno does, but compares against its stored seq.
        let rest: Vec<Numbered> = source_list
            .into_iter()
            .filter(|n| n.path != *item)
            .collect();
        let mut changes = renumbered(&rest, reorder_source(&items_of(&rest)));
        let mut placed = None;
        if let (Some(below), Some(dragged)) = (below, &dragged) {
            let list = self.renumberable(&dir);
            if let Some(t) = list.iter().position(|n| n.path == *target) {
                let mut items = items_of(&list);
                items.push(SeqItem {
                    name: dragged.name.clone(),
                    seq: list[t].item.seq,
                });
                let last = items.len() - 1;
                for (i, seq) in reorder_target(&items, t, last, below, dragged.seq) {
                    match list.get(i) {
                        Some(n) => changes.push((n.path.clone(), seq)),
                        None => placed = Some(seq),
                    }
                }
            }
        }
        // A drop inside takes the target's next seq, so a hand-set sibling never ties with it;
        // Bruno keeps the stored seq here.
        if below.is_none()
            && dragged.is_some()
            && let Some(folder) = self.folder(&dir)
        {
            placed = Some(new_seq(&dir, folder));
        }
        // The dragged item's own write is checked too, while it still sits at its old path.
        let mut checked = changes.clone();
        checked.extend(placed.map(|seq| (item.clone(), seq)));
        self.check_targets(&checked)?;
        let old = item.to_path(&self.root);
        let wanted = dir.to_path(&self.root).join(item.file_name());
        let new = unique_rename_target(&old, &wanted).map_err(|e| io_error(&old, e))?;
        fs::rename(&old, &new).map_err(|e| io_error(&old, e))?;
        let node = self
            .detach(item)
            .ok_or_else(|| TreeError::NotFound { item: item.clone() })?;
        self.attach(&dir, node, &old, &new);
        let moved = child(&dir, &new)?;
        changes.extend(placed.map(|seq| (moved.clone(), seq)));
        // The rename is done, so a renumber failure must still tell the caller where the item is.
        self.write_seqs(changes)
            .map_err(|source| TreeError::Renumber {
                moved: moved.clone(),
                source: Box::new(source),
            })?;
        Ok(moved)
    }

    /// Renumber the items of the folder `dir` 1..n in case-insensitive natural name order, as
    /// one batch checked before any write. Folders and requests share one numbering, as in
    /// a drop. Returns the items whose seq changed; an already sorted folder writes nothing.
    ///
    /// Fails with `NotFound` for an unknown folder, and `Conflict` or `Io` like
    /// [`Collection::delete`].
    pub fn sort_by_name(&mut self, dir: &ItemPath) -> Result<Vec<ItemPath>, TreeError> {
        if self.is_draft(dir) || self.folder(dir).is_none() {
            return Err(TreeError::NotFound { item: dir.clone() });
        }
        let list = self.renumberable(dir);
        let changes = renumbered(&list, sort_names(&items_of(&list)));
        if changes.is_empty() {
            return Ok(Vec::new());
        }
        self.check_targets(&changes)?;
        let paths = changes.iter().map(|(p, _)| p.clone()).collect();
        self.write_seqs(changes)?;
        Ok(paths)
    }

    /// `InvalidName` before any write when the ignore rule hides `path`, which the next
    /// reload would drop again.
    pub(super) fn refuse_hidden(&self, path: &Path, name: &str) -> Result<(), TreeError> {
        if self.is_ignored(path) {
            return Err(TreeError::InvalidName {
                name: name.to_owned(),
            });
        }
        Ok(())
    }

    /// Whether `path` is in the tree; a draft is not.
    fn exists(&self, path: &ItemPath) -> bool {
        !self.is_draft(path)
            && (self.folder(path).is_some()
                || self.request(path).is_some()
                || self.opaque_item(path).is_some())
    }

    /// The siblings in `dir` a move renumbers: every numbered one but a broken folder.
    fn renumberable(&self, dir: &ItemPath) -> Vec<Numbered> {
        let Some(folder) = self.folder(dir) else {
            return Vec::new();
        };
        numbered(dir, folder)
            .into_iter()
            .filter(|n| !n.file_broken)
            .collect()
    }

    /// `Conflict` before any write when a file a renumber will rewrite is not as loaded. A
    /// folder without a `folder.yml` on disk is checked when its file is created.
    fn check_targets(&self, changes: &[(ItemPath, u64)]) -> Result<(), TreeError> {
        for (path, _) in changes {
            let doc = if let Some(r) = self.request(path) {
                Some(r.document())
            } else if let Some(o) = self.opaque_item(path) {
                Some(o.document())
            } else {
                self.folder(path)
                    .and_then(|f| f.file.as_ref())
                    .map(FolderFile::document)
                    .filter(|d| d.loaded_hash().is_some())
            };
            if let Some(doc) = doc {
                doc.check_unchanged()?;
            }
        }
        Ok(())
    }

    /// Write each new seq through to disk, then re-sort the touched folders. On an error the
    /// files already written stay, and the collection is reloaded so memory matches the disk.
    fn write_seqs(&mut self, changes: Vec<(ItemPath, u64)>) -> Result<(), TreeError> {
        for (path, seq) in &changes {
            if let Err(error) = self.write_seq(path, *seq) {
                self.reload();
                return Err(error);
            }
        }
        for (path, _) in &changes {
            if let Some(folder) = path.parent().and_then(|p| self.folder_mut(&p)) {
                sort_lists(folder);
            }
        }
        Ok(())
    }

    fn write_seq(&mut self, path: &ItemPath, seq: u64) -> Result<(), TreeError> {
        let keys = &["info", "seq"];
        if let Some(r) = self.request_mut(path) {
            return Ok(r
                .document_mut()
                .write_through(keys, Value::from(seq), INFO_ORDER)?);
        }
        if let Some(o) = self.opaque_item_mut(path) {
            return Ok(o
                .document_mut()
                .write_through(keys, Value::from(seq), INFO_ORDER)?);
        }
        match self.folder_mut(path) {
            Some(folder) => write_folder(folder, None, Some(seq)),
            None => Ok(()),
        }
    }

    /// Load the request just written at `path` into the folder `parent`.
    pub(super) fn insert_request(
        &mut self,
        parent: &ItemPath,
        path: &Path,
    ) -> Result<ItemPath, TreeError> {
        let request = RequestFile::load(path)
            .map_err(|e| io_error(path, std::io::Error::other(e.message().to_owned())))?;
        let folder = self.folder_mut(parent).ok_or_else(|| TreeError::NotFound {
            item: parent.clone(),
        })?;
        folder.requests.push(request);
        sort_lists(folder);
        child(parent, path)
    }

    /// Remove the request file at `item` from disk and memory without renumbering or checking
    /// its siblings, for undoing a request this process just created. `Io` when removal fails.
    pub(crate) fn discard_request(&mut self, item: &ItemPath) -> Result<(), TreeError> {
        let full = item.to_path(&self.root);
        fs::remove_file(&full).map_err(|e| io_error(&full, e))?;
        self.detach(item);
        Ok(())
    }

    /// Take the node at `item` out of its folder.
    fn detach(&mut self, item: &ItemPath) -> Option<Node> {
        let name = item.file_name().to_owned();
        let folder = self.folder_mut(&item.parent()?)?;
        if let Some(i) = folder.folders.iter().position(|f| f.dir_name == name) {
            return Some(Node::Folder(folder.folders.remove(i)));
        }
        if let Some(i) = folder.requests.iter().position(|r| r.file_name() == name) {
            return Some(Node::Request(folder.requests.remove(i)));
        }
        let i = folder.opaque.iter().position(|o| o.file_name() == name)?;
        Some(Node::Opaque(folder.opaque.remove(i)))
    }

    /// Put a node renamed on disk from `from` to `to` into the folder `dir`, carrying its paths
    /// and the load errors and drafts under it along.
    fn attach(&mut self, dir: &ItemPath, node: Node, from: &Path, to: &Path) {
        for error in &mut self.load_errors {
            error.relocate(from, to);
        }
        self.drafts.iter_mut().for_each(|d| d.relocate(from, to));
        let Some(folder) = self.folder_mut(dir) else {
            return;
        };
        match node {
            Node::Folder(mut f) => {
                f.relocate(from, to);
                folder.folders.push(f);
            }
            Node::Request(mut r) => {
                r.relocate(from, to);
                folder.requests.push(r);
            }
            Node::Opaque(mut o) => {
                o.relocate(from, to);
                folder.opaque.push(o);
            }
        }
        sort_lists(folder);
    }
}

/// Write a folder's name or seq into its `folder.yml`. Without one on disk, Bruno's minimal
/// file is created (`info` plus an empty `request`); an unsaved draft keeps its edits on top.
/// A `folder.yml` that failed to load is never replaced (`Io`, `InvalidData`).
fn write_folder(
    folder: &mut Folder,
    name: Option<&str>,
    seq: Option<u64>,
) -> Result<(), TreeError> {
    let path = folder.path.join(FOLDER_FILE);
    if folder.file_broken {
        let source = std::io::Error::new(ErrorKind::InvalidData, "folder.yml does not load");
        return Err(io_error(&path, source));
    }
    if let Some(file) = folder
        .file
        .as_mut()
        .filter(|f| f.document().loaded_hash().is_some())
    {
        if let Some(name) = name {
            file.write_name(name)?;
        }
        if let Some(seq) = seq {
            file.write_seq(seq)?;
        }
        return Ok(());
    }
    let mut info = vec![
        ("name", Value::from(name.unwrap_or(&folder.dir_name))),
        ("type", Value::from("folder")),
    ];
    info.extend(seq.map(|s| ("seq", Value::Number(s.into()))));
    let root = mapping(vec![
        ("info", Value::Mapping(mapping(info))),
        ("request", Value::Mapping(Mapping::new())),
    ]);
    let created = Document::create(&path, root).map_err(|source| match source.kind() {
        ErrorKind::AlreadyExists => TreeError::Conflict { path: path.clone() },
        _ => io_error(&path, source),
    })?;
    match &mut folder.file {
        Some(draft) => {
            draft.document_mut().adopt_saved(created);
            if let Some(name) = name {
                draft.set_name(name);
            }
            if let Some(seq) = seq {
                draft.set_seq(seq);
            }
        }
        None => folder.file = Some(FolderFile::from_document(created)),
    }
    Ok(())
}

/// The item path of `path`, a file or directory just made in the folder `parent`.
pub(super) fn child(parent: &ItemPath, path: &Path) -> Result<ItemPath, TreeError> {
    path.file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| parent.join(n))
        .ok_or_else(|| {
            io_error(
                path,
                std::io::Error::new(ErrorKind::InvalidData, "not a UTF-8 file name"),
            )
        })
}

fn items_of(list: &[Numbered]) -> Vec<SeqItem> {
    list.iter().map(|n| n.item.clone()).collect()
}

/// Map renumbering results, by index into `list`, to item paths.
fn renumbered(list: &[Numbered], changes: Vec<(usize, u64)>) -> Vec<(ItemPath, u64)> {
    changes
        .into_iter()
        .map(|(i, seq)| (list[i].path.clone(), seq))
        .collect()
}

/// The `seq` a new item in the folder `dir`, at `dir_path`, gets.
pub(super) fn new_seq(dir_path: &ItemPath, dir: &Folder) -> u64 {
    next_seq(&numbered(dir_path, dir))
}

/// The `seq` for a new item: Bruno's count of folders and request-type items plus one, raised
/// past the highest sibling `seq` (`app` items included) so a hand-set number never ties.
fn next_seq(siblings: &[Numbered]) -> u64 {
    let counted = siblings
        .iter()
        .filter(|s| matches!(s.class, Class::Folder | Class::Request))
        .count() as u64;
    let highest = siblings
        .iter()
        .filter_map(|s| s.item.seq)
        .max()
        .unwrap_or(0);
    (counted + 1).max(highest + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn request(name: &str, seq: u64) -> String {
        format!(
            "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: http://localhost/\n"
        )
    }

    /// A collection with these files at its root.
    fn open(files: &[(&str, String)]) -> (tempfile::TempDir, Collection) {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let root = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";
        fs::write(dir.path().join("opencollection.yml"), root).expect("writes");
        for (rel, text) in files {
            fs::write(dir.path().join(rel), text).expect("writes");
        }
        let c = Collection::open(dir.path()).expect("opens");
        (dir, c)
    }

    /// A trashed item read back from a record, with no bin behind it.
    fn trashed(item: &str, tail: &[(&str, u64, u64)]) -> Trashed {
        let path = |rel: &str| ItemPath::new(rel).expect("path");
        let tail = tail.iter().map(|&(p, old, new)| (path(p), old, new));
        let handle = TrashHandle::new(PathBuf::from(item), "id".into());
        Trashed::from_parts(path(item), handle, tail.collect(), Vec::new())
    }

    fn disk_seq(dir: &Path, rel: &str) -> Option<u64> {
        let text = fs::read_to_string(dir.join(rel)).expect("reads");
        let doc: Value = yaml_serde::from_str(&text).expect("yaml");
        doc["info"]["seq"].as_u64()
    }

    #[test]
    fn a_restored_item_is_numbered_into_place_with_its_siblings() {
        // [a, b, c] with b deleted: c closed the gap, and b came back still numbered 2.
        let (dir, mut c) = open(&[
            ("a.yml", request("a", 1)),
            ("b.yml", request("b", 2)),
            ("c.yml", request("c", 2)),
        ]);
        let b = trashed("b.yml", &[("c.yml", 3, 2)]);
        c.renumber_restored(&ItemPath::root(), &[&b])
            .expect("renumbers");
        let seqs = ["a.yml", "b.yml", "c.yml"].map(|f| disk_seq(dir.path(), f));
        assert_eq!(seqs, [1, 2, 3].map(Some));
    }

    #[test]
    fn an_item_that_did_not_load_takes_no_number() {
        let (dir, mut c) = open(&[
            ("a.yml", request("a", 1)),
            ("b.yml", "info: [\n".to_owned()),
            ("c.yml", request("c", 3)),
        ]);
        let b = trashed("b.yml", &[("c.yml", 3, 2)]);
        c.renumber_restored(&ItemPath::root(), &[&b])
            .expect("renumbers");
        assert_eq!(disk_seq(dir.path(), "a.yml"), Some(1));
        assert_eq!(disk_seq(dir.path(), "c.yml"), Some(2));
    }
}
