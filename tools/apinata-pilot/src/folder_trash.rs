//! A trash that is a plain folder, so a piloted run never reaches the user's real trash.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use apinata_core::{Trash, TrashError, TrashHandle};

/// Trashed items live directly under `root`, each named by a unique id. Items move by rename
/// only, so `root` must be on the same filesystem as what is trashed (the data dir is).
#[derive(Debug)]
pub struct FolderTrash {
    root: PathBuf,
    next: AtomicU64,
}

impl FolderTrash {
    /// A trash rooted at `root`; the folder is created on the first `trash`.
    pub fn new(root: PathBuf) -> FolderTrash {
        FolderTrash {
            root,
            next: AtomicU64::new(0),
        }
    }
}

fn failed(what: &str, path: &Path, e: std::io::Error) -> TrashError {
    TrashError::Failed(format!("{what} {}: {e}", path.display()))
}

impl Trash for FolderTrash {
    fn available(&self) -> bool {
        true
    }

    fn trash(&self, path: &Path) -> Result<TrashHandle, TrashError> {
        std::fs::create_dir_all(&self.root).map_err(|e| failed("creating", &self.root, e))?;
        // The counter restarts each launch, so skip ids a previous launch left in the folder.
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let (id, dest) = loop {
            let n = self.next.fetch_add(1, Ordering::Relaxed);
            let id = OsString::from(format!("{n}-{name}"));
            let dest = self.root.join(&id);
            if dest.symlink_metadata().is_err() {
                break (id, dest);
            }
        };
        std::fs::rename(path, &dest).map_err(|e| failed("trashing", path, e))?;
        Ok(TrashHandle::new(path.to_path_buf(), id))
    }

    fn restore(&self, handle: &TrashHandle) -> Result<(), TrashError> {
        // Only an absent entry is Missing; an unreadable one is a real failure.
        let src = self.root.join(handle.id());
        match src.symlink_metadata() {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(TrashError::Missing),
            Err(e) => return Err(failed("reading", &src, e)),
        }
        let original = handle.original();
        // rename would silently replace a file, so an occupied path is refused first.
        if original.symlink_metadata().is_ok() {
            return Err(TrashError::Failed(format!(
                "{} already exists",
                original.display()
            )));
        }
        std::fs::rename(&src, original).map_err(|e| failed("restoring", original, e))
    }
}
