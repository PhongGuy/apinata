//! Collections, folders and environments, walked the way bruno-cli walks them.

use crate::config_file::{CollectionFile, FolderFile};
use crate::document::{DiskState, Document, hash_bytes, is_special_file, read_regular, rebase};
use crate::error::{FileError, OpenError};
use crate::item_path::ItemPath;
use crate::opaque::OpaqueItem;
use crate::request::RequestFile;
use crate::sequence::sort_by_name_then_seq;
use std::cmp::Ordering;
use std::ffi::OsStr;
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

mod conflict;
pub use conflict::FileTexts;
mod create;
mod dotenv_write;
mod draft;
mod ignore;
mod paste;
mod reload;
pub(crate) mod snapshot;
mod tree;
pub use dotenv_write::{DotenvBackup, dotenv_quotable};
pub(crate) use ignore::Ignore;
pub use ignore::{IgnorePolicy, OpenOptions};
pub use paste::PasteSource;
pub use reload::{Change, ChangeKind, ItemKind};
pub use snapshot::{DraftSnapshot, SnapshotError};
pub(crate) use tree::new_request_root;
pub use tree::{DropTarget, Trashed};

pub(crate) const ROOT_FILE: &str = "opencollection.yml";
/// A workspace folder's file; at a collection root it belongs to the workspace, not the tree.
pub(crate) const WORKSPACE_FILE: &str = "workspace.yml";
pub(crate) const FOLDER_FILE: &str = "folder.yml";
pub(crate) const ENVIRONMENTS_DIR: &str = "environments";

/// An opened collection directory.
#[derive(Debug)]
#[non_exhaustive]
pub struct Collection {
    root: PathBuf,
    file: CollectionFile,
    dir_name: String,
    tree: Folder,
    environments: Vec<Environment>,
    /// Hash of the root `.env` as last read; `None` when there is none. The file itself is
    /// parsed per `VarContext`.
    dotenv_hash: Option<u64>,
    load_errors: Vec<FileError>,
    ignore: Ignore,
    /// Unsaved new requests, kept beside the tree so runs, reloads and renumbering never see
    /// them; each sits at `<folder>/untitled-<n>.draft`.
    drafts: Vec<RequestFile>,
    /// The last `n` handed to a draft name.
    draft_count: u64,
}

/// A directory of requests and sub-folders.
#[derive(Debug)]
#[non_exhaustive]
pub struct Folder {
    path: PathBuf,
    dir_name: String,
    is_root: bool,
    /// `folder.yml`, when present and loadable or created for an edit; `None` for the root.
    file: Option<FolderFile>,
    /// An existing `folder.yml` failed to load, so it is never replaced.
    file_broken: bool,
    folders: Vec<Folder>,
    requests: Vec<RequestFile>,
    opaque: Vec<OpaqueItem>,
    /// The directory is gone and the node stays only for the dirty drafts under it.
    removed: bool,
}

/// One item of a folder: a sub-folder, a request or an opaque item.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum Sibling<'a> {
    /// A sub-folder.
    Folder(&'a Folder),
    /// An `http` request.
    Request(&'a RequestFile),
    /// A request file of a type Apinata does not model.
    Opaque(&'a OpaqueItem),
}

/// An environment file from `environments/`.
#[derive(Debug)]
#[non_exhaustive]
pub struct Environment {
    name: String,
    doc: Document,
}

impl Collection {
    /// Open the collection rooted at `root` under the `BrunoCli` ignore policy.
    ///
    /// Files that fail to load are reported through `load_errors` and do not stop the walk.
    pub fn open(root: impl AsRef<Path>) -> Result<Collection, OpenError> {
        Collection::open_with(root, &OpenOptions::default())
    }

    /// Open the collection rooted at `root` with `options`, which choose the ignore policy.
    ///
    /// Files that fail to load are reported through `load_errors` and do not stop the walk.
    pub fn open_with(
        root: impl AsRef<Path>,
        options: &OpenOptions,
    ) -> Result<Collection, OpenError> {
        let root = root.as_ref();
        let root_file = root.join(ROOT_FILE);
        match fs::metadata(&root_file) {
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {
                if crate::legacy::is_legacy_collection(root) {
                    return Err(OpenError::LegacyFormat {
                        root: root.to_path_buf(),
                    });
                }
                return Err(OpenError::NotACollection {
                    root: root.to_path_buf(),
                });
            }
            Err(source) => {
                return Err(OpenError::Io {
                    path: root_file,
                    source,
                });
            }
        }
        let root_doc = Document::load(&root_file).map_err(|e| OpenError::InvalidRoot {
            path: root_file.clone(),
            message: e.message().to_owned(),
        })?;
        // A path like "." or "x/.." has no usable final component, so ask the filesystem.
        let mut dir_name = dir_name_of(root);
        if (dir_name.is_empty() || dir_name == "..")
            && let Ok(real) = fs::canonicalize(root)
        {
            dir_name = dir_name_of(&real);
        }

        let ignore = Ignore::new(options.ignore, root, &root_doc);
        let mut load_errors = Vec::new();
        let tree = walk(root, &ignore, &mut load_errors)?;
        let environments = load_environments(root, &ignore, &mut load_errors);
        let dotenv_hash = read_regular(&root.join(".env"))
            .ok()
            .map(|b| hash_bytes(&b));
        Ok(Collection {
            root: root.to_path_buf(),
            file: CollectionFile::from_document(root_doc),
            dir_name,
            tree,
            environments,
            dotenv_hash,
            load_errors,
            ignore,
            drafts: Vec::new(),
            draft_count: 0,
        })
    }

    /// The collection directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `info.name` from `opencollection.yml`, else the directory name.
    pub fn name(&self) -> &str {
        self.file.name().unwrap_or(&self.dir_name)
    }

    /// The `opencollection.yml` file.
    pub fn file(&self) -> &CollectionFile {
        &self.file
    }

    /// The `opencollection.yml` file, for editing.
    pub fn file_mut(&mut self) -> &mut CollectionFile {
        &mut self.file
    }

    /// The root folder.
    pub fn tree(&self) -> &Folder {
        &self.tree
    }

    /// The environment whose file stem is `name`.
    pub fn environment(&self, name: &str) -> Option<&Environment> {
        self.environments.iter().find(|e| e.name == name)
    }

    /// The environment whose file stem is `name`, for editing.
    pub fn environment_mut(&mut self, name: &str) -> Option<&mut Environment> {
        self.environments.iter_mut().find(|e| e.name == name)
    }

    pub(crate) fn environments_mut(&mut self) -> &mut Vec<Environment> {
        &mut self.environments
    }

    /// Environments, sorted by name.
    pub fn environments(&self) -> &[Environment] {
        &self.environments
    }

    /// Files that could not be loaded.
    pub fn load_errors(&self) -> &[FileError] {
        &self.load_errors
    }

    /// The parsed `opencollection.yml`.
    pub(crate) fn root_document(&self) -> &Document {
        self.file.document()
    }

    /// Whether this collection's policy hides `path` (`Ignore::hides`): false outside the root,
    /// for the root itself, `opencollection.yml` and `.env`.
    pub fn is_ignored(&self, path: &Path) -> bool {
        self.ignore.rel(path).is_some_and(|rel| {
            !matches!(rel.as_str(), "" | ROOT_FILE | ".env") && self.ignore.hides(&rel)
        })
    }

    /// The directories whose entries core reads, sorted: the root, every folder whose
    /// directory is on disk, and `environments` unless ignored.
    pub fn watch_dirs(&self) -> Vec<PathBuf> {
        // A removed folder is kept only for drafts, and so is everything beneath it.
        fn live(folder: &Folder, out: &mut Vec<PathBuf>) {
            if !folder.removed {
                out.push(folder.path.clone());
                folder.folders.iter().for_each(|f| live(f, out));
            }
        }
        let mut dirs = Vec::new();
        live(&self.tree, &mut dirs);
        let environments = self.root.join(ENVIRONMENTS_DIR);
        if !self.is_ignored(&environments) {
            dirs.push(environments);
        }
        dirs.sort();
        dirs
    }

    /// The item path of `path`; `None` outside the root, with `..`, or for a non-UTF-8 name.
    pub fn item_path(&self, path: &Path) -> Option<ItemPath> {
        let rel = path.strip_prefix(&self.root).ok()?;
        let mut item = ItemPath::root();
        for component in rel.components() {
            match component {
                Component::Normal(name) => item = item.join(name.to_str()?)?,
                Component::CurDir => {}
                _ => return None,
            }
        }
        Some(item)
    }

    /// The folder at `path`; the root folder for the root.
    pub fn folder(&self, path: &ItemPath) -> Option<&Folder> {
        path.segments().try_fold(&self.tree, |f, name| {
            f.folders.iter().find(|sub| sub.dir_name == name)
        })
    }

    /// The folders that have a spec baseline entry of their own and still exist, in tree order
    /// (the sidebar's order, parents before their children), not the order they were imported.
    pub fn baseline_folders(&self) -> Vec<ItemPath> {
        let recorded = self.file.spec_baseline_folders();
        let mut found = Vec::new();
        let mut stack = vec![(ItemPath::root(), &self.tree)];
        // Walk depth first, taking children in display order via a reversed stack.
        while let Some((path, folder)) = stack.pop() {
            if recorded.contains(&path) {
                found.push(path.clone());
            }
            for sub in folder.folders.iter().rev() {
                if let Some(child) = path.join(&sub.dir_name) {
                    stack.push((child, sub));
                }
            }
        }
        found
    }

    /// The request at `path`, a draft included.
    pub fn request(&self, path: &ItemPath) -> Option<&RequestFile> {
        if path.is_draft() {
            let full = path.to_path(&self.root);
            return self.drafts.iter().find(|d| d.path() == full);
        }
        let name = path.file_name();
        self.folder(&path.parent()?)?
            .requests
            .iter()
            .find(|r| r.file_name() == name)
    }

    /// The request at `path`, a draft included, for editing.
    pub fn request_mut(&mut self, path: &ItemPath) -> Option<&mut RequestFile> {
        if path.is_draft() {
            let full = path.to_path(&self.root);
            return self.drafts.iter_mut().find(|d| d.path() == full);
        }
        let name = path.file_name();
        self.folder_mut(&path.parent()?)?
            .requests
            .iter_mut()
            .find(|r| r.file_name() == name)
    }

    /// The opaque item at `path`.
    pub fn opaque_item(&self, path: &ItemPath) -> Option<&OpaqueItem> {
        let name = path.file_name();
        self.folder(&path.parent()?)?
            .opaque
            .iter()
            .find(|o| o.file_name() == name)
    }

    /// The opaque item at `path`, for a `seq` edit.
    pub fn opaque_item_mut(&mut self, path: &ItemPath) -> Option<&mut OpaqueItem> {
        let name = path.file_name();
        self.folder_mut(&path.parent()?)?
            .opaque
            .iter_mut()
            .find(|o| o.file_name() == name)
    }

    /// The `folder.yml` of the folder at `path`, for editing. `None` for the root, an unknown
    /// path, or a `folder.yml` that failed to load. A folder without one gets an unsaved
    /// `{info: {name: <dir name>, type: folder}}`, written on save.
    pub fn folder_file_mut(&mut self, path: &ItemPath) -> Option<&mut FolderFile> {
        let folder = self.folder_mut(path)?;
        if folder.is_root || folder.file_broken {
            return None;
        }
        if folder.file.is_none() {
            folder.file = Some(FolderFile::unsaved(
                folder.path.join(FOLDER_FILE),
                &folder.dir_name,
            ));
        }
        folder.file.as_mut()
    }

    fn folder_mut(&mut self, path: &ItemPath) -> Option<&mut Folder> {
        path.segments().try_fold(&mut self.tree, |f, name| {
            f.folders.iter_mut().find(|sub| sub.dir_name == name)
        })
    }
}

impl Folder {
    /// Follow a rename on disk of this folder or an ancestor: the prefix `from` became `to`.
    pub(crate) fn relocate(&mut self, from: &Path, to: &Path) {
        self.path = rebase(&self.path, from, to);
        self.dir_name = dir_name_of(&self.path);
        if let Some(file) = &mut self.file {
            file.document_mut().relocate(from, to);
        }
        self.requests.iter_mut().for_each(|r| r.relocate(from, to));
        self.opaque.iter_mut().for_each(|o| o.relocate(from, to));
        self.folders.iter_mut().for_each(|f| f.relocate(from, to));
    }

    /// `info.name` from `folder.yml`, else the directory name.
    pub fn name(&self) -> &str {
        self.file
            .as_ref()
            .and_then(FolderFile::name)
            .unwrap_or(&self.dir_name)
    }

    /// The loaded or created `folder.yml`; `None` for the root or without a loadable one.
    pub fn file(&self) -> Option<&FolderFile> {
        self.file.as_ref()
    }

    /// The directory's own name.
    pub fn dir_name(&self) -> &str {
        &self.dir_name
    }

    /// The directory path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `Removed` for a folder kept only for the drafts under its vanished directory, else
    /// `Present`.
    pub fn disk_state(&self) -> DiskState {
        if self.removed {
            DiskState::Removed
        } else {
            DiskState::Present
        }
    }

    /// `info.seq` from `folder.yml`.
    pub fn seq(&self) -> Option<u64> {
        self.file.as_ref().and_then(FolderFile::seq)
    }

    /// Sub-folders in Bruno's display order.
    pub fn folders(&self) -> &[Folder] {
        &self.folders
    }

    /// Requests ordered by `seq`, then file name; requests without `seq` come last.
    pub fn requests(&self) -> &[RequestFile] {
        &self.requests
    }

    /// Request files of a type Apinata does not model, by file name.
    pub fn opaque_items(&self) -> &[OpaqueItem] {
        &self.opaque
    }

    /// Sub-folders in their own order, then requests and opaque items together in `seq` order:
    /// by `seq`, then file name, with the seq-less last.
    pub fn siblings(&self) -> Vec<Sibling<'_>> {
        let mut out: Vec<Sibling<'_>> = self.folders.iter().map(Sibling::Folder).collect();
        let mut items: Vec<Sibling<'_>> = (self.requests.iter().map(Sibling::Request))
            .chain(self.opaque.iter().map(Sibling::Opaque))
            .collect();
        items.sort_by(|a, b| by_seq_then_file(a.seq(), a.path(), b.seq(), b.path()));
        out.extend(items);
        out
    }

    /// The parsed `folder.yml`; `None` for the root or when the file is missing or broken.
    pub(crate) fn document(&self) -> Option<&Document> {
        self.file.as_ref().map(FolderFile::document)
    }
}

impl Sibling<'_> {
    /// The folder's directory or the item's file.
    pub fn path(&self) -> &Path {
        match self {
            Sibling::Folder(f) => f.path(),
            Sibling::Request(r) => r.path(),
            Sibling::Opaque(o) => o.path(),
        }
    }

    /// The display name.
    pub fn name(&self) -> &str {
        match self {
            Sibling::Folder(f) => f.name(),
            Sibling::Request(r) => r.name(),
            Sibling::Opaque(o) => o.name(),
        }
    }

    /// `info.seq`, when present and a non-negative integer.
    pub fn seq(&self) -> Option<u64> {
        match self {
            Sibling::Folder(f) => f.seq(),
            Sibling::Request(r) => r.seq(),
            Sibling::Opaque(o) => o.seq(),
        }
    }
}

impl Environment {
    /// The file stem.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The environment file.
    pub fn path(&self) -> &Path {
        self.doc.path()
    }

    /// The parsed environment file.
    pub(crate) fn document(&self) -> &Document {
        &self.doc
    }

    pub(crate) fn document_mut(&mut self) -> &mut Document {
        &mut self.doc
    }

    pub(crate) fn from_parts(name: String, doc: Document) -> Environment {
        Environment { name, doc }
    }

    /// Take the new stem and path after the file was renamed on disk.
    pub(crate) fn renamed(&mut self, name: String, path: PathBuf) {
        self.name = name;
        self.doc.set_path(path);
    }
}

/// `info.name` from the `opencollection.yml` under `root`, read without opening the collection.
///
/// `None` when the file is missing, does not parse or has no usable (string, non-blank) name, so a caller can
/// fall back to the directory name.
pub fn collection_name(root: &Path) -> Option<String> {
    let doc = Document::load(&root.join(ROOT_FILE)).ok()?;
    let name = doc.get(&["info", "name"])?.as_str()?;
    (!name.trim().is_empty()).then(|| name.to_owned())
}

fn dir_name_of(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

/// Name order for folders and environments. A deliberate approximation of `localeCompare`
/// for non-ASCII names, punctuation and case ties: lowercased first, raw as tie-break.
pub(crate) fn name_order(a: &str, b: &str) -> Ordering {
    a.to_lowercase()
        .cmp(&b.to_lowercase())
        .then_with(|| a.cmp(b))
}

/// Whether `walk` loads a file of this name as a request or opaque item.
fn is_item_file(name: &str) -> bool {
    name.ends_with(".yml") && name != ROOT_FILE && name != FOLDER_FILE
}

/// Walk one directory under `ignore`'s root, skipping what it ignores. Only the root's read
/// failure is fatal; deeper ones become load errors.
fn walk(dir: &Path, ignore: &Ignore, errors: &mut Vec<FileError>) -> Result<Folder, OpenError> {
    let rel = ignore.rel(dir).unwrap_or_default();
    let is_root = rel.is_empty();
    let dir_name = dir_name_of(dir);
    let mut file = None;
    let mut file_broken = false;
    if !is_root && !ignore.hides(&format!("{rel}/{FOLDER_FILE}")) {
        let folder_file = dir.join(FOLDER_FILE);
        if fs::symlink_metadata(&folder_file).is_ok() && !is_special_file(&folder_file) {
            match Document::load(&folder_file) {
                Ok(loaded) => file = Some(FolderFile::from_document(loaded)),
                Err(e) => {
                    file_broken = true;
                    errors.push(e);
                }
            }
        }
    }

    let mut folders = Vec::new();
    let mut requests = Vec::new();
    let mut opaque = Vec::new();
    // bruno-cli reads this directory's folder.yml, then loads nothing else inside it.
    let entries = match fs::read_dir(dir) {
        _ if ignore.skips_children_of(&rel) => None,
        Ok(entries) => Some(entries),
        Err(source) if is_root => {
            return Err(OpenError::Io {
                path: dir.to_path_buf(),
                source,
            });
        }
        Err(e) => {
            errors.push(FileError::new(dir, e.to_string()));
            return Ok(Folder {
                path: dir.to_path_buf(),
                dir_name,
                is_root,
                file,
                file_broken,
                folders,
                requests,
                opaque,
                removed: false,
            });
        }
    };
    for entry in entries.into_iter().flatten() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                errors.push(FileError::new(dir, e.to_string()));
                continue;
            }
        };
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let child_rel = if is_root {
            file_name.clone()
        } else {
            format!("{rel}/{file_name}")
        };
        if ignore.matches(&child_rel) {
            continue;
        }
        // lstat semantics: a symlink is neither a directory nor a file here.
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            // The root's `environments` holds environments, not a folder; that is not an ignore.
            if !(is_root && file_name == ENVIRONMENTS_DIR) {
                folders.push(walk(&path, ignore, errors)?);
            }
        } else if meta.is_file()
            && is_item_file(&file_name)
            && !(is_root && file_name == WORKSPACE_FILE)
        {
            match RequestFile::load(&path) {
                Ok(request) => requests.push(request),
                // A rejected file stays a load error; one of another request type also joins
                // the folder's items so it keeps its place in the ordering.
                Err(e) => {
                    errors.push(e);
                    opaque.extend(OpaqueItem::load(&path));
                }
            }
        }
    }

    let mut folder = Folder {
        path: dir.to_path_buf(),
        dir_name,
        is_root,
        file,
        file_broken,
        folders,
        requests,
        opaque,
        removed: false,
    };
    sort_lists(&mut folder);
    Ok(folder)
}

/// Put one folder's three lists in display order; rerun after any change to them.
fn sort_lists(folder: &mut Folder) {
    folder
        .requests
        .sort_by(|a, b| by_seq_then_file(a.seq(), a.file_name(), b.seq(), b.file_name()));
    folder
        .opaque
        .sort_by(|a, b| by_seq_then_file(a.seq(), a.file_name(), b.seq(), b.file_name()));
    folder.folders = sort_folders(std::mem::take(&mut folder.folders));
}

/// The order of a folder's requests and opaque items: by `seq`, then file name, seq-less last.
fn by_seq_then_file<N: Ord + ?Sized>(a: Option<u64>, an: &N, b: Option<u64>, bn: &N) -> Ordering {
    match (a, b) {
        (Some(x), Some(y)) => x.cmp(&y).then_with(|| an.cmp(bn)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => an.cmp(bn),
    }
}

/// Port of Bruno's `sortByNameThenSequence`: sort by name, then splice folders that have a
/// positive `seq` into that list at index `seq - 1`. Equal seqs share one slot, so a group
/// counts as a single position for every later splice.
fn sort_folders(folders: Vec<Folder>) -> Vec<Folder> {
    sort_by_name_then_seq(folders, |f| f.dir_name.as_str(), Folder::seq)
}

/// Load `environments/*.yml` at the collection root that `ignore` does not hide, sorted by name.
fn load_environments(
    root: &Path,
    ignore: &Ignore,
    errors: &mut Vec<FileError>,
) -> Vec<Environment> {
    let dir = root.join(ENVIRONMENTS_DIR);
    let hides =
        |name: &OsStr| ignore.hides(&format!("{ENVIRONMENTS_DIR}/{}", name.to_string_lossy()));
    load_environment_dir(&dir, hides, errors).unwrap_or_else(|e| {
        errors.push(FileError::new(&dir, e.to_string()));
        Vec::new()
    })
}

/// Load every `*.yml` file in `dir` whose file name `hides` does not reject, sorted by name.
/// A missing `dir` is empty; a `dir` that cannot be read is the error, and a file that fails
/// to load goes to `errors`.
pub(crate) fn load_environment_dir(
    dir: &Path,
    hides: impl Fn(&OsStr) -> bool,
    errors: &mut Vec<FileError>,
) -> std::io::Result<Vec<Environment>> {
    let mut found = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(found),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                errors.push(FileError::new(dir, e.to_string()));
                continue;
            }
        };
        let path = entry.path();
        let is_file = fs::symlink_metadata(&path).is_ok_and(|m| m.is_file());
        if !is_file || hides(&entry.file_name()) || path.extension().is_none_or(|e| e != "yml") {
            continue;
        }
        let name = path
            .file_stem()
            .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
        match Document::load(&path) {
            Ok(doc) => found.push(Environment { name, doc }),
            Err(e) => errors.push(e),
        }
    }
    found.sort_by(|a, b| name_order(&a.name, &b.name));
    Ok(found)
}
