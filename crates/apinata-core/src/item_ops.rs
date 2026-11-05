//! Tree operations over raw collection-relative paths, as a CLI argument or an MCP tool call
//! gives them. Each parses its paths, applies the checks that must run before a file changes,
//! and makes one tree call, so every shell creates, moves and removes items the same way.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::collection::{Collection, DropTarget, Trashed};
use crate::config_file::{CollectionFile, FolderFile};
use crate::edit::{AuthSpec, KeyValue, Variable};
use crate::error::{EditError, FileError, SaveError, TreeError};
use crate::item_path::ItemPath;
use crate::method::{InvalidMethod, normalize_method};
use crate::request::RequestKind;
use crate::trash::Trash;
use crate::trash_log::{Entry, Locked, TrashLog};

/// Why an item operation was refused or failed. The messages are the ones users see.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ItemError {
    /// A path that is not a valid collection-relative item path.
    #[error("invalid item path: {raw}")]
    InvalidPath {
        /// The path as given.
        raw: String,
    },
    /// The method asked for is not an HTTP token; nothing was created.
    #[error(transparent)]
    Method(#[from] InvalidMethod),
    /// An item of a type this version only carries through cannot be renamed.
    #[error("`{path}` is a {kind} item; it cannot be renamed yet, so move or delete it instead")]
    Opaque {
        /// The path as given.
        path: String,
        /// Its `info.type`.
        kind: String,
    },
    /// A move named no target or more than one.
    #[error("give exactly one of `inside`, `before` and `after`")]
    Target,
    /// The request was created, but its method or URL could not be saved.
    #[error("created {created} but could not set its method or URL: {message}")]
    NotSet {
        /// The request that now exists.
        created: ItemPath,
        /// Why the save failed.
        message: String,
        /// The file that changed on disk again after the one retry, when that is why.
        conflict: Option<PathBuf>,
    },
    /// The tree call itself failed.
    #[error(transparent)]
    Tree(#[from] TreeError),
    /// No folder exists at the path.
    #[error("no folder at {raw}")]
    NoFolder {
        /// The path as given.
        raw: String,
    },
    /// The folder's `folder.yml` failed to load, so it cannot be edited.
    #[error(transparent)]
    BrokenFolder(FileError),
    /// A section was refused; the folder is named unless it is the collection.
    #[error("{}", edit_message(raw, source))]
    Edit {
        /// The folder path as given, empty for the collection.
        raw: String,
        /// Why the section was refused.
        source: EditError,
    },
    /// Saving the edited file failed; nothing was written and no draft is left.
    #[error(transparent)]
    Save(#[from] SaveError),
    /// The trash record holds nothing for this collection.
    #[error("nothing deleted from this collection is left to restore")]
    NothingToRestore,
    /// Something is at the trashed item's path now; nothing changed and the record is kept.
    #[error(
        "cannot restore {item}: something is at that path now, so nothing was changed; \
         move or rename it, then restore again"
    )]
    RestoreClash {
        /// The collection-relative path.
        item: String,
    },
    /// The folder the item was in is gone; nothing changed and the record is kept.
    #[error(
        "cannot restore {item}: its folder {folder} no longer exists, so nothing was changed; \
         create the folder, then restore again"
    )]
    RestoreNoFolder {
        /// The collection-relative path.
        item: String,
        /// The missing folder.
        folder: String,
    },
    /// The item left the trash some other way, so its record was dropped.
    #[error(
        "{item} is no longer in the trash, so it cannot be restored; \
         the next restore goes to the item deleted before it"
    )]
    NoLongerInTrash {
        /// The collection-relative path.
        item: String,
    },
    /// The platform's trash cannot give items back, so the record was dropped; the item stays
    /// in the system trash.
    #[error(
        "{item} is in the system trash, but cannot be restored from here, so its \
         record was dropped; put it back from the file manager. The next restore goes to the \
         item deleted before it"
    )]
    RestoreUnsupported {
        /// The collection-relative path.
        item: String,
    },
    /// The newest record does not describe an item of this collection, so it was dropped.
    #[error("the trash record for {item} was invalid, so it was dropped and nothing was restored")]
    InvalidTrashRecord {
        /// The path the record named.
        item: String,
    },
    /// The newest delete was a single one, not an import prune; nothing changed.
    #[error(
        "the newest delete was not an import prune; `apinata item restore` (MCP \
         `restore_item`) puts it back{}",
        recorded_prunes_hint(prunes)
    )]
    NotAPrune {
        /// The batch ids in the record, newest first.
        prunes: Vec<String>,
    },
    /// No entry of the record belongs to the batch asked for; nothing changed.
    #[error("no prune {batch} in the trash record{}", unknown_prune_tail(prunes))]
    UnknownPrune {
        /// The batch id as given.
        batch: String,
        /// The batch ids in the record, newest first.
        prunes: Vec<String>,
    },
    /// Something stands in the way of part of a prune, so none of it was restored.
    #[error(
        "cannot restore this prune; nothing in the collection was changed:{}",
        problem_lines(problems)
    )]
    PruneRefused {
        /// Each item in the way and why, one per line of the message.
        problems: Vec<(String, String)>,
    },
    /// The trash record file could not be read; it is left as it is.
    #[error("cannot use the trash record {message}")]
    TrashLog {
        /// The file and why.
        message: String,
    },
    /// The trash refused to give the item back; the record is kept.
    #[error("could not restore {item} from the trash: {message}")]
    RestoreFailed {
        /// The collection-relative path.
        item: String,
        /// Why, for display.
        message: String,
    },
}

/// The record's prunes as `NotAPrune` offers them; nothing when it holds none.
fn recorded_prunes_hint(prunes: &[String]) -> String {
    if prunes.is_empty() {
        return String::new();
    }
    format!(
        "; recorded prunes, newest first: {}; restore one with `--pruned --batch <ID>` (MCP \
         `batch`)",
        prunes.join(", ")
    )
}

fn unknown_prune_tail(prunes: &[String]) -> String {
    if prunes.is_empty() {
        return ", which holds no prunes".to_owned();
    }
    format!("; recorded prunes, newest first: {}", prunes.join(", "))
}

fn problem_lines(problems: &[(String, String)]) -> String {
    problems
        .iter()
        .map(|(item, reason)| format!("\n{item}: {reason}"))
        .collect()
}

fn edit_message(raw: &str, source: &EditError) -> String {
    if raw.is_empty() {
        source.to_string()
    } else {
        format!("{raw}: {source}")
    }
}

impl ItemError {
    /// The file that changed on disk since it was loaded, when that is why the call failed,
    /// so a long-lived caller can drop its stale copy and retry.
    pub fn conflict_path(&self) -> Option<&Path> {
        match self {
            ItemError::Tree(TreeError::Conflict { path })
            | ItemError::Save(SaveError::Conflict { path }) => Some(path),
            _ => None,
        }
    }

    /// For a `NotSet` whose save kept conflicting: the request that was created and the file
    /// that another program changed, so a caller can word its own advice.
    pub fn unsaved_conflict(&self) -> Option<(&ItemPath, &Path)> {
        match self {
            ItemError::NotSet {
                created,
                conflict: Some(path),
                ..
            } => Some((created, path)),
            _ => None,
        }
    }
}

/// What a new request starts with beyond the collection's presets.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct NewRequest {
    kind: Option<RequestKind>,
    method: Option<String>,
    url: Option<String>,
}

impl NewRequest {
    /// A request made from the presets alone.
    pub fn new() -> NewRequest {
        NewRequest::default()
    }

    /// Make a request of `kind` instead of the preset kind.
    pub fn kind(mut self, kind: RequestKind) -> NewRequest {
        self.kind = Some(kind);
        self
    }

    /// Set the method; it is checked and uppercased before any file is created.
    pub fn method(mut self, method: impl Into<String>) -> NewRequest {
        self.method = Some(method.into());
        self
    }

    /// Set the URL; its query becomes params.
    pub fn url(mut self, url: impl Into<String>) -> NewRequest {
        self.url = Some(url.into());
        self
    }
}

/// Parse a raw item path, naming it when it is not a valid relative path.
pub fn parse_path(raw: &str) -> Result<ItemPath, ItemError> {
    ItemPath::new(raw).ok_or_else(|| ItemError::InvalidPath {
        raw: raw.to_owned(),
    })
}

/// Create a request named `name` in folder `parent` (`""` for the root) from the presets,
/// then set `new`'s method and URL through the setters. A bad method is refused before any
/// file exists; once the file exists, a failed save is `NotSet` and leaves no draft behind.
pub fn create_request(
    c: &mut Collection,
    parent: &str,
    name: &str,
    new: &NewRequest,
) -> Result<ItemPath, ItemError> {
    create_request_with(c, parent, name, new, &mut |_| {})
}

/// [`create_request`] with `before_save` called with the file before each save, the seam a
/// test uses to play another writer.
fn create_request_with(
    c: &mut Collection,
    parent: &str,
    name: &str,
    new: &NewRequest,
    before_save: &mut dyn FnMut(&Path),
) -> Result<ItemPath, ItemError> {
    let parent = parse_path(parent)?;
    let method = new.method.as_deref().map(normalize_method).transpose()?;
    let created = c.create_request_from_presets(&parent, name, new.kind)?;
    if method.is_some() || new.url.is_some() {
        set_method_and_url(
            c,
            &created,
            method.as_deref(),
            new.url.as_deref(),
            before_save,
        )
        .map_err(|(message, conflict)| ItemError::NotSet {
            created: created.clone(),
            message,
            conflict,
        })?;
    }
    Ok(created)
}

/// Apply the method and URL to the just-created request and save it. A conflict is retried
/// once against the bytes on disk; any failure drops the draft and returns its message, with
/// the file when a second conflict is why.
fn set_method_and_url(
    c: &mut Collection,
    created: &ItemPath,
    method: Option<&str>,
    url: Option<&str>,
    before_save: &mut dyn FnMut(&Path),
) -> Result<(), (String, Option<PathBuf>)> {
    let mut retried = false;
    loop {
        let r = c
            .request_mut(created)
            .ok_or_else(|| (format!("no request at {created}"), None))?;
        if let Some(method) = method {
            r.set_method(method);
        }
        if let Some(url) = url {
            r.set_url(url);
        }
        let file = r.path().to_owned();
        before_save(&file);
        match r.save() {
            Ok(()) => return Ok(()),
            Err(SaveError::Conflict { .. }) if !retried => retried = true,
            Err(e) => {
                let _ = c.revert(&file);
                let conflict = match &e {
                    SaveError::Conflict { path } => Some(path.clone()),
                    _ => None,
                };
                return Err((e.to_string(), conflict));
            }
        }
        let _ = c.revert(&file);
    }
}

/// Create a folder named `name` in folder `parent` (`""` for the root) and return its path.
pub fn create_folder(c: &mut Collection, parent: &str, name: &str) -> Result<ItemPath, ItemError> {
    let parent = parse_path(parent)?;
    Ok(c.create_folder(&parent, name)?)
}

/// Copy the request or opaque item at `path` beside it under `name` and return the copy's
/// path.
pub fn clone_request(c: &mut Collection, path: &str, name: &str) -> Result<ItemPath, ItemError> {
    let item = parse_path(path)?;
    if c.opaque_item(&item).is_some() {
        return Ok(c.clone_opaque(&item, name)?);
    }
    Ok(c.clone_request(&item, name)?)
}

/// Copy the folder at `path` with everything in it beside it under `name`; returns its path.
pub fn clone_folder(c: &mut Collection, path: &str, name: &str) -> Result<ItemPath, ItemError> {
    let item = parse_path(path)?;
    Ok(c.clone_folder(&item, name)?)
}

/// Copy the request, opaque item or folder at `path` beside it under `name`, whichever is
/// there; returns the copy's path.
pub fn clone_item(c: &mut Collection, path: &str, name: &str) -> Result<ItemPath, ItemError> {
    let item = parse_path(path)?;
    if c.folder(&item).is_some() {
        return Ok(c.clone_folder(&item, name)?);
    }
    clone_request(c, path, name)
}

/// Rename a request or folder and its file or directory; an opaque item is refused by kind.
pub fn rename_item(c: &mut Collection, path: &str, name: &str) -> Result<ItemPath, ItemError> {
    let item = parse_path(path)?;
    if let Some(other) = c.opaque_item(&item) {
        return Err(ItemError::Opaque {
            path: path.to_owned(),
            kind: other.kind().to_owned(),
        });
    }
    Ok(c.rename(&item, name, true)?)
}

/// Move the item at `path` into folder `inside` (`""` for the root), or above `before` or
/// below `after`; exactly one target must be given. Returns the item's new path.
pub fn move_item(
    c: &mut Collection,
    path: &str,
    inside: Option<&str>,
    before: Option<&str>,
    after: Option<&str>,
) -> Result<ItemPath, ItemError> {
    let item = parse_path(path)?;
    let target = match (inside, before, after) {
        (Some(p), None, None) => DropTarget::Inside(parse_path(p)?),
        (None, Some(p), None) => DropTarget::Before(parse_path(p)?),
        (None, None, Some(p)) => DropTarget::After(parse_path(p)?),
        _ => return Err(ItemError::Target),
    };
    Ok(c.move_item(&item, target)?)
}

/// Delete the item at `path` for good (a folder with everything in it) and renumber its
/// siblings. A sibling that could not be renumbered is a returned warning, not a failure.
pub fn delete_item(c: &mut Collection, path: &str) -> Result<Vec<String>, ItemError> {
    let item = parse_path(path)?;
    match c.delete(&item) {
        Ok(()) => Ok(Vec::new()),
        Err(TreeError::DeletedRenumber { source }) => Ok(vec![renumber_warning(c, &source)]),
        Err(e) => Err(e.into()),
    }
}

/// Like [`delete_item`], but the item goes into `bin`, with the same warnings.
pub fn trash_item(
    c: &mut Collection,
    path: &str,
    bin: &dyn Trash,
) -> Result<Vec<String>, ItemError> {
    let item = parse_path(path)?;
    let trashed = c.trash(&item, bin)?;
    // Trashed carries the failure as text, so strip the root from it the same way.
    Ok(trashed
        .renumber_failure()
        .map(|text| text.replace(&root_prefix(c), ""))
        .into_iter()
        .collect())
}

/// Like [`trash_item`], and record the delete in `log` so [`restore_last_trashed`] can undo
/// it. A record that cannot be written, or no `log` at all, is a warning: the item is in the
/// bin either way.
///
/// Deletes and restores of one collection take turns on the log's lock, and the item's folder
/// is read again from disk once the lock is held, so another process's delete in the same
/// folder never leaves this one renumbering stale siblings.
pub fn trash_item_logged(
    c: &mut Collection,
    path: &str,
    bin: &dyn Trash,
    log: Option<&TrashLog>,
) -> Result<Vec<String>, ItemError> {
    let item = parse_path(path)?;
    let record = log.map(|log| log.locked(c.root()));
    if let Some(parent) = item.parent() {
        c.refresh_folder(&parent);
    }
    let trashed = c.trash(&item, bin)?;
    let mut warnings: Vec<String> = trashed
        .renumber_failure()
        .map(|text| text.replace(&root_prefix(c), ""))
        .into_iter()
        .collect();
    let unrestorable =
        |why: &str| format!("moved to the trash, but it cannot be restored from here: {why}");
    match record.map(|record| record.record(&trashed, None)) {
        None => warnings.push(unrestorable("no data directory")),
        Some(Ok(replaced)) => warnings.extend(replaced),
        Some(Err(why)) => warnings.push(unrestorable(&why)),
    }
    Ok(warnings)
}

/// What [`restore_last_trashed`] put back.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Restored {
    item: ItemPath,
    warnings: Vec<String>,
}

impl Restored {
    /// The restored item's collection-relative path.
    pub fn item(&self) -> &ItemPath {
        &self.item
    }

    /// What went wrong after the item was back, such as a sibling left with its new seq.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

/// Put back the newest item [`trash_item_logged`] recorded for this collection and number its
/// folder 1..n, the item placed by its record's tail. Only the handle in the record reaches
/// `bin`, and only when it names a path inside the collection; the trash itself is never searched.
///
/// A clash or a missing folder refuses and keeps the record; an item gone from the bin or an
/// invalid record drops it. A sibling that cannot be renumbered once the item is back is a
/// warning.
pub fn restore_last_trashed(
    c: &mut Collection,
    bin: &dyn Trash,
    log: &TrashLog,
) -> Result<Restored, ItemError> {
    // Held to the end, so a delete recorded meanwhile waits and two restores never share an entry.
    let record = log.locked(c.root());
    let root = c.root().to_path_buf();
    let mut entries = read_entries(&record)?;
    let Some(entry) = entries.last() else {
        return Err(ItemError::NothingToRestore);
    };
    let item = entry.item_text().to_owned();
    // Dropping is best effort: the error already explains, and a stale entry fails the same way.
    let drop_newest = |mut entries: Vec<Entry>| {
        entries.pop();
        let _ = record.store(entries);
    };
    let valid = entry.to_trashed().filter(|t| belongs(&root, t));
    let Some((parent, trashed)) = valid.and_then(|t| Some((t.item().parent()?, t))) else {
        drop_newest(entries);
        return Err(ItemError::InvalidTrashRecord { item });
    };
    if !parent.to_path(&root).is_dir() {
        return Err(ItemError::RestoreNoFolder {
            item,
            folder: parent.to_string(),
        });
    }
    // Number what is on disk, not what a long-lived collection last saw.
    c.refresh_folder(&parent);
    let mut warnings = Vec::new();
    match c.restore_trashed(&trashed, bin) {
        Ok(()) => {}
        Err(TreeError::RestoreClash { .. }) => return Err(ItemError::RestoreClash { item }),
        Err(TreeError::NotInTrash) => {
            drop_newest(entries);
            return Err(ItemError::NoLongerInTrash { item });
        }
        Err(TreeError::RestoreUnsupported) => {
            drop_newest(entries);
            return Err(ItemError::RestoreUnsupported { item });
        }
        Err(TreeError::Trash { message }) => {
            return Err(ItemError::RestoreFailed { item, message });
        }
        // Every other failure comes from renumbering, after the item is back.
        Err(e) => warnings.push(restored_renumber_warning(c, &e)),
    }
    entries.pop();
    if let Err(e) = record.store(entries) {
        warnings.push(format!(
            "restored, but the trash record could not be updated: {e}"
        ));
    }
    Ok(Restored {
        item: trashed.item().clone(),
        warnings,
    })
}

fn read_entries(record: &Locked<'_>) -> Result<Vec<Entry>, ItemError> {
    record.entries().map_err(|e| ItemError::TrashLog {
        message: e.to_string(),
    })
}

/// What [`restore_prune`] or [`restore_last_prune`] put back.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RestoredAll {
    items: Vec<ItemPath>,
    warnings: Vec<String>,
}

impl RestoredAll {
    /// The restored items, newest delete first.
    pub fn items(&self) -> &[ItemPath] {
        &self.items
    }

    /// What went wrong along the way, such as the item that stopped the batch.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

/// Put back every item of the prune that `batch` names, wherever its entries sit in the record,
/// and number the folders they return to 1..n. Every other entry keeps its place.
///
/// Refused with `UnknownPrune` when no entry has that id, with `PruneRefused` when any item
/// cannot go back, and with `TrashLog` when the record cannot be written; in each case nothing in
/// the collection changes, and the record changes only by dropping invalid entries.
///
/// An item that turns out to be gone from the bin, or a bin failure or new clash, stops the batch:
/// with nothing restored yet it is the error; otherwise what is back stays and is renumbered, and
/// a warning says how many remain. A failed renumber or final record update is also a warning.
pub fn restore_prune(
    c: &mut Collection,
    bin: &dyn Trash,
    log: &TrashLog,
    batch: &str,
) -> Result<RestoredAll, ItemError> {
    // One lock for the whole batch, so no delete lands between its entries.
    let record = log.locked(c.root());
    let entries = read_entries(&record)?;
    restore_batch(c, bin, &record, entries, batch)
}

/// [`restore_prune`] for the batch of the newest entry. Refused with `NotAPrune`, changing
/// nothing, when that entry is a single delete.
pub fn restore_last_prune(
    c: &mut Collection,
    bin: &dyn Trash,
    log: &TrashLog,
) -> Result<RestoredAll, ItemError> {
    let record = log.locked(c.root());
    let entries = read_entries(&record)?;
    let newest = entries.last().ok_or(ItemError::NothingToRestore)?;
    let Some(batch) = newest.batch().map(str::to_owned) else {
        return Err(ItemError::NotAPrune {
            prunes: recorded_prunes(&entries),
        });
    };
    restore_batch(c, bin, &record, entries, &batch)
}

/// The batch ids in the record, newest first.
fn recorded_prunes(entries: &[Entry]) -> Vec<String> {
    let mut prunes: Vec<String> = Vec::new();
    for batch in entries.iter().rev().filter_map(Entry::batch) {
        if !prunes.iter().any(|p| p == batch) {
            prunes.push(batch.to_owned());
        }
    }
    prunes
}

/// One batch entry ready to restore: its index in the record, the item and its folder.
struct BatchItem {
    index: usize,
    trashed: Trashed,
    parent: ItemPath,
}

/// What [`check_batch`] found: the valid items newest first, each item in the way with why, and
/// the record indices of the invalid entries.
struct BatchCheck {
    items: Vec<BatchItem>,
    problems: Vec<(String, String)>,
    invalid: BTreeSet<usize>,
}

/// Parse the entries of `batch`, refresh their folders from disk and collect everything that
/// stands in the way of restoring them, changing nothing on disk.
fn check_batch(c: &mut Collection, entries: &[Entry], batch: &str) -> BatchCheck {
    let root = c.root().to_path_buf();
    let mut items: Vec<BatchItem> = Vec::new();
    let mut problems = Vec::new();
    let mut invalid = BTreeSet::new();
    let selected = entries.iter().enumerate().rev();
    for (index, entry) in selected.filter(|(_, e)| e.batch() == Some(batch)) {
        let valid = entry.to_trashed().filter(|t| belongs(&root, t));
        let Some((parent, trashed)) = valid.and_then(|t| Some((t.item().parent()?, t))) else {
            invalid.insert(index);
            let reason = "its trash record was invalid, so it was dropped".to_owned();
            problems.push((entry.item_text().to_owned(), reason));
            continue;
        };
        items.push(BatchItem {
            index,
            trashed,
            parent,
        });
    }
    let parents: BTreeSet<&ItemPath> = items.iter().map(|i| &i.parent).collect();
    for parent in parents {
        c.refresh_folder(parent);
    }
    for (n, item) in items.iter().enumerate() {
        let path = item.trashed.item();
        let same_path = |other: &BatchItem| other.trashed.item() == path;
        let reason = if items[..n].iter().any(same_path) {
            continue;
        } else if items[n + 1..].iter().any(same_path) {
            "two entries of this prune name this path".to_owned()
        } else if !item.parent.to_path(&root).is_dir() {
            format!(
                "its folder {} no longer exists; create the folder, then restore again",
                item.parent
            )
        } else if fs::symlink_metadata(path.to_path(&root)).is_ok() {
            "something is at that path now; move or rename it, then restore again".to_owned()
        } else {
            continue;
        };
        problems.push((path.to_string(), reason));
    }
    BatchCheck {
        items,
        problems,
        invalid,
    }
}

/// The shared body of [`restore_prune`] and [`restore_last_prune`], with the record locked.
fn restore_batch(
    c: &mut Collection,
    bin: &dyn Trash,
    record: &Locked<'_>,
    entries: Vec<Entry>,
    batch: &str,
) -> Result<RestoredAll, ItemError> {
    if !entries.iter().any(|e| e.batch() == Some(batch)) {
        return Err(ItemError::UnknownPrune {
            batch: batch.to_owned(),
            prunes: recorded_prunes(&entries),
        });
    }
    let without = |gone: &BTreeSet<usize>| -> Vec<Entry> {
        let kept = entries.iter().enumerate();
        kept.filter(|(i, _)| !gone.contains(i))
            .map(|(_, e)| e.clone())
            .collect()
    };
    let BatchCheck {
        items,
        problems,
        invalid,
    } = check_batch(c, &entries, batch);
    if !problems.is_empty() {
        // Best effort, as for a single restore; a batch is never stuck on an invalid entry.
        if !invalid.is_empty() {
            let _ = record.store(without(&invalid));
        }
        return Err(ItemError::PruneRefused { problems });
    }
    // Storing the record unchanged proves it can be written before any item moves, so a batch
    // is not left back on disk with its entries still recorded.
    record
        .store(entries.clone())
        .map_err(|e| ItemError::TrashLog {
            message: e.to_string(),
        })?;

    let mut gone = BTreeSet::new();
    let mut back: Vec<&BatchItem> = Vec::new();
    let mut stopped = None;
    for item in &items {
        let text = item.trashed.item().to_string();
        match c.restore_file(&item.trashed, bin) {
            Ok(()) => {
                gone.insert(item.index);
                back.push(item);
            }
            Err(TreeError::NotInTrash) => {
                gone.insert(item.index);
                stopped = Some(ItemError::NoLongerInTrash { item: text });
                break;
            }
            Err(TreeError::RestoreUnsupported) => {
                gone.insert(item.index);
                stopped = Some(ItemError::RestoreUnsupported { item: text });
                break;
            }
            Err(TreeError::RestoreClash { .. }) => {
                stopped = Some(ItemError::RestoreClash { item: text });
                break;
            }
            Err(e) => {
                let message = match e {
                    TreeError::Trash { message } => message,
                    e => e.to_string(),
                };
                stopped = Some(ItemError::RestoreFailed {
                    item: text,
                    message,
                });
                break;
            }
        }
    }
    let mut warnings = Vec::new();
    let parents: BTreeSet<&ItemPath> = back.iter().map(|i| &i.parent).collect();
    for parent in parents {
        let here: Vec<&Trashed> = back
            .iter()
            .filter(|i| i.parent == *parent)
            .map(|i| &i.trashed)
            .collect();
        if let Err(e) = c.renumber_restored(parent, &here) {
            warnings.push(restored_renumber_warning(c, &e));
        }
    }
    let stored = record.store(without(&gone));
    match stopped {
        // With nothing back the stop is the failure itself, not a note on a partial restore.
        Some(e) if back.is_empty() => return Err(e),
        Some(e) => {
            let left = items.len() - gone.len();
            warnings.push(format!(
                "stopped: {e}; {left} more from this prune are still in the trash, so restore \
                 the prune again to continue"
            ));
        }
        None => {}
    }
    if let Err(e) = stored {
        warnings.push(format!(
            "restored, but the trash record could not be updated: {e}; a rerun refuses on the \
             items now present until they fall off the record"
        ));
    }
    Ok(RestoredAll {
        items: back.iter().map(|i| i.trashed.item().clone()).collect(),
        warnings,
    })
}

/// Whether the record's handle names the item's own path in this collection, as given or
/// canonical (the platform trash records the canonical form).
fn belongs(root: &Path, trashed: &crate::collection::Trashed) -> bool {
    let original = trashed.parts().0.original();
    let item = trashed.item();
    original == item.to_path(root)
        || root
            .canonicalize()
            .is_ok_and(|canon| original == item.to_path(&canon))
}

/// A failed renumber after a restore, with paths relative to the collection.
fn restored_renumber_warning(c: &Collection, source: &TreeError) -> String {
    format!(
        "restored, but could not renumber the siblings: {}",
        source.to_string().replace(&root_prefix(c), "")
    )
}

/// The renumber failure as users see it, with paths relative to the collection.
fn renumber_warning(c: &Collection, source: &TreeError) -> String {
    format!(
        "deleted, but could not renumber the siblings: {}",
        source.to_string().replace(&root_prefix(c), "")
    )
}

fn root_prefix(c: &Collection) -> String {
    format!("{}/", c.root().display())
}

/// The sections a folder's `folder.yml` or the collection's `opencollection.yml` shares with
/// every request below it.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct FolderSections {
    /// Headers in file order, disabled ones included.
    pub headers: Vec<KeyValue>,
    /// The authentication requests set to inherit use.
    pub auth: AuthSpec,
    /// Variables in file order, disabled ones included.
    pub vars: Vec<Variable>,
    /// Markdown docs; empty when none.
    pub docs: String,
}

/// Replacement sections for [`update_folder`]; a section left out is not touched.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct FolderPatch {
    headers: Option<Vec<KeyValue>>,
    auth: Option<AuthSpec>,
    vars: Option<Vec<Variable>>,
    docs: Option<String>,
}

impl FolderPatch {
    /// A patch that changes nothing.
    pub fn new() -> FolderPatch {
        FolderPatch::default()
    }

    /// Replace the headers, in this order.
    pub fn headers(mut self, headers: Vec<KeyValue>) -> FolderPatch {
        self.headers = Some(headers);
        self
    }

    /// Replace the authentication.
    pub fn auth(mut self, auth: AuthSpec) -> FolderPatch {
        self.auth = Some(auth);
        self
    }

    /// Replace the variables, in this order.
    pub fn vars(mut self, vars: Vec<Variable>) -> FolderPatch {
        self.vars = Some(vars);
        self
    }

    /// Replace the docs; blank text removes them.
    pub fn docs(mut self, docs: impl Into<String>) -> FolderPatch {
        self.docs = Some(docs.into());
        self
    }
}

/// What [`update_folder`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FolderUpdate {
    file: PathBuf,
    changed: bool,
}

impl FolderUpdate {
    /// The file the sections live in, written only when [`FolderUpdate::changed`].
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// Whether the patch differed from the file, so the file was saved.
    pub fn changed(&self) -> bool {
        self.changed
    }
}

/// Read the shared sections of the folder at `raw` (`""` for the collection). A folder
/// without `folder.yml` reads as empty; a missing folder or a broken `folder.yml` is refused.
pub fn read_folder(c: &Collection, raw: &str) -> Result<FolderSections, ItemError> {
    let path = parse_path(raw)?;
    if path.is_root() {
        return Ok(sections(c.file()));
    }
    let folder = c.folder(&path).ok_or_else(|| ItemError::NoFolder {
        raw: raw.to_owned(),
    })?;
    if let Some(f) = folder.file() {
        return Ok(sections(f));
    }
    // Core keeps no broken flag on a folder, so a load error for its file is the only signal.
    let file = folder.path().join("folder.yml");
    match c.load_errors().iter().find(|e| e.path() == file) {
        Some(error) => Err(ItemError::BrokenFolder(error.clone())),
        None => Ok(FolderSections {
            headers: Vec::new(),
            auth: AuthSpec::None,
            vars: Vec::new(),
            docs: String::new(),
        }),
    }
}

/// Apply `patch` to the folder at `raw` (`""` edits opencollection.yml; a folder edits
/// folder.yml, created on first save) and save it when anything changed. A section equal to
/// the file is skipped, and a list entry reuses the file's entry of the same name, so what the
/// patch does not carry (a description, unknown keys) survives. A failure leaves no draft.
pub fn update_folder(
    c: &mut Collection,
    raw: &str,
    patch: &FolderPatch,
) -> Result<FolderUpdate, ItemError> {
    read_folder(c, raw)?;
    let path = parse_path(raw)?;
    let f: &mut dyn Sections = if path.is_root() {
        c.file_mut()
    } else {
        c.folder_file_mut(&path)
            .ok_or_else(|| ItemError::NoFolder {
                raw: raw.to_owned(),
            })?
    };
    let file = f.path().to_owned();
    let outcome = apply_and_save(f, patch).map_err(|e| match e {
        Ok(source) => ItemError::Edit {
            raw: raw.to_owned(),
            source,
        },
        Err(save) => ItemError::Save(save),
    });
    match outcome {
        Ok(changed) => Ok(FolderUpdate { file, changed }),
        Err(e) => {
            let _ = c.revert(&file);
            Err(e)
        }
    }
}

/// Apply every present section, then save when the draft differs from the disk. The error is
/// a refused section (`Ok`) or a failed save (`Err`).
fn apply_and_save(
    f: &mut dyn Sections,
    p: &FolderPatch,
) -> Result<bool, Result<EditError, SaveError>> {
    // A never-saved folder.yml reads as dirty, so only a section that differed counts.
    let mut touched = false;
    if let Some(headers) = &p.headers {
        let current = f.headers();
        if !same_list(&current, headers, |a, b| {
            (&a.name, &a.value, a.enabled) == (&b.name, &b.value, b.enabled)
        }) {
            let merged = merge_by_name(
                current,
                headers,
                |h| &h.name,
                |c, d| {
                    c.value = d.value.clone();
                    c.enabled = d.enabled;
                },
            );
            f.set_headers(merged).map_err(Ok)?;
            touched = true;
        }
    }
    if let Some(auth) = &p.auth
        && f.auth() != *auth
    {
        f.set_auth(auth.clone());
        touched = true;
    }
    if let Some(vars) = &p.vars {
        let current = f.vars();
        if !same_list(&current, vars, |a, b| {
            (&a.name, &a.value, a.enabled) == (&b.name, &b.value, b.enabled)
        }) {
            let merged = merge_by_name(
                current,
                vars,
                |v| &v.name,
                |c, d| {
                    c.value = d.value.clone();
                    c.enabled = d.enabled;
                },
            );
            f.set_vars(merged);
            touched = true;
        }
    }
    // A config file's docs setter rewrites even unchanged text into the mapping form.
    if let Some(docs) = &p.docs
        && f.docs() != *docs
    {
        f.set_docs(docs.clone());
        touched = true;
    }
    let changed = touched && f.is_dirty();
    if changed {
        f.save().map_err(Err)?;
    }
    Ok(changed)
}

/// Whether `current` already holds `wanted`, compared on the fields a patch carries.
fn same_list<T>(current: &[T], wanted: &[T], eq: impl Fn(&T, &T) -> bool) -> bool {
    current.len() == wanted.len() && current.iter().zip(wanted).all(|(a, b)| eq(a, b))
}

/// Build the new list in `wanted` order. Each entry starts from the first unused current entry
/// of its name, keeping what the file remembers about it, or else is taken as given.
fn merge_by_name<T: Clone>(
    current: Vec<T>,
    wanted: &[T],
    name: fn(&T) -> &String,
    update: impl Fn(&mut T, &T),
) -> Vec<T> {
    let mut used = vec![false; current.len()];
    wanted
        .iter()
        .map(
            |d| match (0..current.len()).find(|&i| !used[i] && name(&current[i]) == name(d)) {
                Some(i) => {
                    used[i] = true;
                    let mut entry = current[i].clone();
                    update(&mut entry, d);
                    entry
                }
                None => d.clone(),
            },
        )
        .collect()
}

fn sections(f: &dyn Sections) -> FolderSections {
    FolderSections {
        headers: f.headers(),
        auth: f.auth(),
        vars: f.vars(),
        docs: f.docs(),
    }
}

/// What `folder.yml` and `opencollection.yml` share, so one edit path serves both.
trait Sections {
    fn path(&self) -> &Path;
    fn headers(&self) -> Vec<KeyValue>;
    fn set_headers(&mut self, headers: Vec<KeyValue>) -> Result<(), EditError>;
    fn auth(&self) -> AuthSpec;
    fn set_auth(&mut self, auth: AuthSpec);
    fn vars(&self) -> Vec<Variable>;
    fn set_vars(&mut self, vars: Vec<Variable>);
    fn docs(&self) -> String;
    fn set_docs(&mut self, docs: String);
    fn is_dirty(&self) -> bool;
    fn save(&mut self) -> Result<(), SaveError>;
}

macro_rules! sections {
    ($file:ty) => {
        impl Sections for $file {
            fn path(&self) -> &Path {
                <$file>::path(self)
            }
            fn headers(&self) -> Vec<KeyValue> {
                <$file>::headers(self)
            }
            fn set_headers(&mut self, headers: Vec<KeyValue>) -> Result<(), EditError> {
                <$file>::set_headers(self, headers)
            }
            fn auth(&self) -> AuthSpec {
                <$file>::auth(self)
            }
            fn set_auth(&mut self, auth: AuthSpec) {
                <$file>::set_auth(self, auth)
            }
            fn vars(&self) -> Vec<Variable> {
                <$file>::vars(self)
            }
            fn set_vars(&mut self, vars: Vec<Variable>) {
                <$file>::set_vars(self, vars)
            }
            fn docs(&self) -> String {
                <$file>::docs(self)
            }
            fn set_docs(&mut self, docs: String) {
                <$file>::set_docs(self, docs)
            }
            fn is_dirty(&self) -> bool {
                <$file>::is_dirty(self)
            }
            fn save(&mut self) -> Result<(), SaveError> {
                <$file>::save(self)
            }
        }
    };
}
sections!(FolderFile);
sections!(CollectionFile);

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: c\n";

    fn open() -> (tempfile::TempDir, Collection) {
        let dir = tempfile::TempDir::new().expect("tempdir");
        fs::write(dir.path().join("opencollection.yml"), ROOT).expect("write");
        let c = Collection::open(dir.path()).expect("opens");
        (dir, c)
    }

    /// Another program rewrites the new file before the first `times` saves.
    fn interfering(times: u32) -> impl FnMut(&Path) {
        let mut n = 0;
        move |file| {
            n += 1;
            if n <= times {
                let text = fs::read_to_string(file).expect("read");
                fs::write(file, format!("{text}# other writer {n}\n")).expect("write");
            }
        }
    }

    #[test]
    fn one_conflict_is_retried_against_the_other_writers_bytes() {
        let (dir, mut c) = open();
        let new = NewRequest::new().method("POST");
        let made = create_request_with(&mut c, "", "A", &new, &mut interfering(1)).expect("set");
        let text = fs::read_to_string(made.to_path(dir.path())).expect("read");
        assert!(text.contains("method: POST"), "{text}");
    }

    #[test]
    fn a_second_conflict_names_the_created_request_and_the_file() {
        let (dir, mut c) = open();
        let new = NewRequest::new().method("POST");
        let err = create_request_with(&mut c, "", "A", &new, &mut interfering(2)).unwrap_err();
        let file = dir.path().join("A.yml");
        let (created, path) = err.unsaved_conflict().expect("a conflict");
        assert_eq!(
            (created.to_string().as_str(), path),
            ("A.yml", file.as_path())
        );
        assert!(
            err.to_string()
                .starts_with("created A.yml but could not set its method or URL: ")
        );
        let r = c.request(created).expect("still loaded");
        assert!(!r.is_dirty() && r.method() == "GET", "the draft is dropped");
    }
}
