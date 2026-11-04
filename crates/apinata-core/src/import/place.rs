//! Placing an imported collection's items into a folder of an existing collection.
//!
//! The bundle is written by [`write_collection`] into a scratch directory and only its items
//! are copied across, so the target's own root file, environments and folder settings are never
//! touched. This is the one routine the CLI and the MCP server share.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::{
    ImportBundle, ImportError, ImportReport, record_spec_baseline, write::write_collection,
};
use crate::collection::Collection;
use crate::item_path::ItemPath;
use crate::secrets::SecretStore;

/// What [`place_items`] wrote.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Placed {
    /// The folder the items went into.
    pub folder: ItemPath,
    /// Core's report; its `items` are relative to `folder`.
    pub report: ImportReport,
    /// How many of the items are requests, at any depth.
    pub requests: usize,
}

/// The folder at `segments` under the root, created where missing. Fails with
/// [`ImportError::Folder`] when a name is invalid, passes through a symlink, is a file, or the
/// result resolves outside the collection.
pub fn ensure_folder(
    collection: &mut Collection,
    segments: &[String],
) -> Result<ItemPath, ImportError> {
    let bad = |problem: String| ImportError::Folder { problem };
    let io_error = |path: &Path, source: io::Error| ImportError::Io {
        path: path.to_path_buf(),
        source,
    };
    let root = collection
        .root()
        .canonicalize()
        .map_err(|e| io_error(collection.root(), e))?;
    let mut folder = ItemPath::root();
    for segment in segments {
        let next = folder
            .join(segment)
            .ok_or_else(|| bad(format!("has an invalid folder name: {segment:?}")))?;
        let path = collection.root().join(next.as_str());
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(bad(format!("passes through a symlink: {segment:?}")));
            }
            Ok(meta) if meta.is_dir() => folder = next,
            Ok(_) => {
                return Err(bad(format!("names a file, not a folder: {segment:?}")));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                folder = collection.create_folder(&folder, segment)?;
            }
            Err(e) => return Err(io_error(&path, e)),
        }
    }
    // The checks above look at each name as written; this one looks at where the result is.
    let dir = collection.root().join(folder.as_str());
    let real = dir.canonicalize().map_err(|e| io_error(&dir, e))?;
    if !real.starts_with(&root) {
        return Err(bad("resolves outside the collection".to_owned()));
    }
    Ok(folder)
}

/// Write `bundle`'s items directly into the existing `folder` of `collection`, then reload.
/// No environment is written and no secret is stored, so `secrets` is only what core asks
/// for. A name already in the folder is refused with [`ImportError::Exists`] and nothing is
/// written. An OpenAPI bundle also records the folder's baseline, which lets a later update find
/// its requests. A failure removes what this call copied and nothing else.
pub fn place_items(
    collection: &mut Collection,
    folder: &ItemPath,
    bundle: &ImportBundle,
    secrets: &dyn SecretStore,
) -> Result<Placed, ImportError> {
    let scratch = tempfile::tempdir().map_err(|source| ImportError::Io {
        path: std::env::temp_dir(),
        source,
    })?;
    let (staged, report) = write_collection(scratch.path(), &bundle.collection, &[], secrets)?;
    let dest = collection.root().join(folder.as_str());
    let mut copied = Vec::new();
    let undo = |copied: &[PathBuf]| {
        for path in copied {
            let _ = if path.is_dir() {
                fs::remove_dir_all(path)
            } else {
                fs::remove_file(path)
            };
        }
    };
    if let Err(source) = copy_items(staged.root(), &dest, &mut copied) {
        undo(&copied);
        collection.reload();
        return Err(match source.kind() {
            io::ErrorKind::AlreadyExists => ImportError::Exists { path: dest },
            _ => ImportError::Io { path: dest, source },
        });
    }
    // The copied files are not in the tree yet.
    collection.reload();
    if bundle.collection.spec_source {
        let placed: Vec<ItemPath> = report
            .items
            .iter()
            .filter_map(|item| {
                item.as_str()
                    .split('/')
                    .try_fold(folder.clone(), |p, segment| p.join(segment))
            })
            .collect();
        if let Err(e) = record_spec_baseline(collection, folder, &placed) {
            undo(&copied);
            collection.reload();
            return Err(e);
        }
    }
    let requests = report
        .items
        .iter()
        .filter(|item| staged.request(item).is_some())
        .count();
    Ok(Placed {
        folder: folder.clone(),
        report,
        requests,
    })
}

/// Copy a staged collection's items into `dest`: not its root file, its environments, its
/// hidden files or a root `folder.yml`, which `dest` already has. Each top-level path made is
/// pushed on `copied` so a failure can undo it.
fn copy_items(staged: &Path, dest: &Path, copied: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(staged)? {
        let entry = entry?;
        let name = entry.file_name();
        let lossy = name.to_string_lossy();
        let own = matches!(
            &*lossy,
            "opencollection.yml" | "environments" | "folder.yml"
        );
        if own || lossy.starts_with('.') {
            continue;
        }
        // A top-level name that exists is refused, so nothing already there is overwritten
        // and the undo list holds only paths this call made.
        let to = dest.join(&name);
        if fs::symlink_metadata(&to).is_ok() {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
        copied.push(to.clone());
        copy_tree(&entry.path(), &to)?;
    }
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    if from.is_dir() {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(from, to).map(|_| ())
    }
}
