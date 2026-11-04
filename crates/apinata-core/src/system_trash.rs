//! The platform's trash behind the [`Trash`] seam. Tests use a fake and never reach this.
//!
//! The `trash` crate's delete returns nothing, so on Linux and Windows the trashed item is
//! found again by listing the trash before and after; restore goes through its `os_limited`
//! API. macOS has no listing, so the item is looked up by name in `~/.Trash` and put back by a
//! rename.

use std::path::Path;

use crate::trash::{Trash, TrashError, TrashHandle};

/// The user's real trash.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct SystemTrash;

impl SystemTrash {
    /// The platform trash.
    pub fn new() -> SystemTrash {
        SystemTrash
    }
}

fn failed(error: impl std::fmt::Display) -> TrashError {
    TrashError::Failed(error.to_string())
}

impl Trash for SystemTrash {
    fn available(&self) -> bool {
        imp::available()
    }

    fn trash(&self, path: &Path) -> Result<TrashHandle, TrashError> {
        imp::trash(path)
    }

    fn restore(&self, handle: &TrashHandle) -> Result<(), TrashError> {
        imp::restore(handle)
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::{TrashError, TrashHandle, failed};
    use std::collections::HashSet;
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use trash::os_limited;

    /// Makes the home trash (`$XDG_DATA_HOME/Trash` with `files` and `info`) if it is missing,
    /// owner-only as the freedesktop spec asks. The crate would create it on delete but with
    /// the umask's mode, and only after `available` has already said no.
    #[cfg(not(windows))]
    fn ensure_home_trash() -> Result<(), String> {
        use std::os::unix::fs::DirBuilderExt;
        let home = dirs::data_dir()
            .ok_or_else(|| "no data directory".to_owned())?
            .join("Trash");
        for sub in ["files", "info"] {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(home.join(sub))
                .map_err(|e| format!("{}: {e}", home.join(sub).display()))?;
        }
        Ok(())
    }

    /// Windows always has a recycle bin; elsewhere the home trash must exist or be creatable.
    pub fn available() -> bool {
        #[cfg(windows)]
        return true;
        #[cfg(not(windows))]
        ensure_home_trash().is_ok()
    }

    pub fn trash(path: &Path) -> Result<TrashHandle, TrashError> {
        #[cfg(not(windows))]
        ensure_home_trash().map_err(TrashError::Unavailable)?;
        // The crate canonicalises the parent directory only, so the listing is matched on the
        // same form.
        let name = path.file_name().ok_or_else(|| failed("no file name"))?;
        let parent = path.parent().ok_or_else(|| failed("no parent directory"))?;
        let original: PathBuf = parent.canonicalize().map_err(failed)?.join(name);
        let known: HashSet<OsString> = listed(&original)?.into_iter().map(|i| i.id).collect();
        trash::delete(&original).map_err(|e| TrashError::Unavailable(describe(&e)))?;
        // The newest entry for this path that was not there before is the one just made.
        listed(&original)?
            .into_iter()
            .filter(|i| !known.contains(&i.id))
            .max_by_key(|i| i.time_deleted)
            .map(|i| TrashHandle::new(original.clone(), i.id))
            .ok_or_else(|| failed("the item was trashed but could not be found in the trash"))
    }

    pub fn restore(handle: &TrashHandle) -> Result<(), TrashError> {
        let item = listed(handle.original())?
            .into_iter()
            .find(|i| &i.id == handle.id())
            .ok_or(TrashError::Missing)?;
        os_limited::restore_all([item]).map_err(|e| failed(describe(&e)))
    }

    fn listed(original: &Path) -> Result<Vec<trash::TrashItem>, TrashError> {
        let all = os_limited::list().map_err(|e| TrashError::Unavailable(describe(&e)))?;
        Ok(all
            .into_iter()
            .filter(|i| i.original_path() == original)
            .collect())
    }

    /// The crate's `Display` is its debug dump; name only the cases a user can act on.
    fn describe(error: &trash::Error) -> String {
        match error {
            trash::Error::RestoreCollision { path, .. } => {
                format!("{} already exists", path.display())
            }
            trash::Error::CouldNotAccess { target } => format!("cannot access {target}"),
            trash::Error::Os { description, .. } | trash::Error::Unknown { description } => {
                description.clone()
            }
            #[cfg(not(windows))]
            trash::Error::FileSystem { path, source } => format!("{}: {source}", path.display()),
            _ => "the trash refused it".to_owned(),
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::{TrashError, TrashHandle, failed};
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    fn bin() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".Trash"))
    }

    pub fn available() -> bool {
        bin().is_some_and(|b| b.is_dir())
    }

    pub fn trash(path: &Path) -> Result<TrashHandle, TrashError> {
        let name = path.file_name().ok_or_else(|| failed("no file name"))?;
        let bin = bin().ok_or_else(|| TrashError::Unavailable("no home directory".into()))?;
        // Finder renames a clash, so a name already in the bin cannot be told from the new
        // item; such an item is trashed without a way back, and restore says so.
        let free = !bin.join(name).exists();
        trash::delete(path).map_err(|e| TrashError::Unavailable(format!("{e:?}")))?;
        let id = if free && bin.join(name).exists() {
            name.to_owned()
        } else {
            OsString::new()
        };
        Ok(TrashHandle::new(path.to_path_buf(), id))
    }

    pub fn restore(handle: &TrashHandle) -> Result<(), TrashError> {
        let bin = bin().ok_or_else(|| TrashError::Unavailable("no home directory".into()))?;
        // An item that went in under another name cannot be told apart, so it counts as gone.
        if handle.id().is_empty() || !bin.join(handle.id()).exists() {
            return Err(TrashError::Missing);
        }
        std::fs::rename(bin.join(handle.id()), handle.original()).map_err(failed)
    }
}
