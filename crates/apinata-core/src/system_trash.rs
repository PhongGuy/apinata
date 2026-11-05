//! The platform's trash behind the [`Trash`] seam. Tests use a fake and never reach this.
//!
//! The `trash` crate's delete returns nothing, so where it can list the trash (freedesktop
//! and Windows) the trashed item is found again by listing before and after, and restore goes
//! through its `os_limited` API. macOS has no listing, so the item is looked up by name in
//! `~/.Trash` and put back by a rename. Elsewhere there is no trash at all.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

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

#[cfg_attr(
    not(any(
        windows,
        target_os = "macos",
        all(
            unix,
            not(target_os = "macos"),
            not(target_os = "ios"),
            not(target_os = "android")
        )
    )),
    allow(dead_code)
)]
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

/// Whether a bin that finds items by name can put `handle` back: an item it never named
/// cannot be told apart from others there, and a named one must still be in the bin.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn check_by_name(handle: &TrashHandle, in_bin: impl Fn(&OsStr) -> bool) -> Result<(), TrashError> {
    if handle.id().is_empty() {
        return Err(TrashError::RestoreUnsupported);
    }
    if !in_bin(handle.id()) {
        return Err(TrashError::Missing);
    }
    Ok(())
}

/// `path` without the verbatim prefix `canonicalize` adds on Windows, which is the form the
/// recycle bin reports original locations in. Other paths come back unchanged.
#[cfg_attr(not(windows), allow(dead_code))]
fn shell_form(path: &Path) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path.to_path_buf();
    };
    if let Some(share) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{share}"));
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path.to_path_buf(),
    }
}

/// Freedesktop and Windows: the cfg under which the crate offers `os_limited`.
#[cfg(any(
    windows,
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
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
        // same form, without the verbatim prefix the recycle bin never reports.
        let name = path.file_name().ok_or_else(|| failed("no file name"))?;
        let parent = path.parent().ok_or_else(|| failed("no parent directory"))?;
        let parent = parent.canonicalize().map_err(failed)?;
        let original: PathBuf = super::shell_form(&parent).join(name);
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
        // Finder renames a clash and another volume has its own bin, so an item not found
        // under its own name in `~/.Trash` gets an empty id and restore reports it unsupported.
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
        super::check_by_name(handle, |name| bin.join(name).exists())?;
        std::fs::rename(bin.join(handle.id()), handle.original()).map_err(failed)
    }
}

/// Platforms the crate has no trash for: nothing is trashed, so nothing is restored.
#[cfg(not(any(
    windows,
    target_os = "macos",
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
)))]
mod imp {
    use super::{TrashError, TrashHandle};
    use std::path::Path;

    pub fn available() -> bool {
        false
    }

    pub fn trash(_: &Path) -> Result<TrashHandle, TrashError> {
        Err(TrashError::Unavailable("this platform has no trash".into()))
    }

    pub fn restore(_: &TrashHandle) -> Result<(), TrashError> {
        Err(TrashError::RestoreUnsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn handle(id: &str) -> TrashHandle {
        TrashHandle::new(PathBuf::from("/c/a.yml"), id.into())
    }

    #[test]
    fn an_item_the_bin_never_named_cannot_be_restored_on_this_platform() {
        let got = check_by_name(&handle(""), |_| true);
        assert!(
            matches!(got, Err(TrashError::RestoreUnsupported)),
            "{got:?}"
        );
    }

    #[test]
    fn a_named_item_gone_from_the_bin_is_missing() {
        let got = check_by_name(&handle("a.yml"), |_| false);
        assert!(matches!(got, Err(TrashError::Missing)), "{got:?}");
    }

    #[test]
    fn a_named_item_still_in_the_bin_can_be_put_back() {
        let got = check_by_name(&handle("a.yml"), |name| name == "a.yml");
        assert!(got.is_ok(), "{got:?}");
    }

    #[test]
    fn a_verbatim_drive_path_takes_the_form_the_recycle_bin_reports() {
        assert_eq!(
            shell_form(Path::new(r"\\?\C:\dir\a.yml")),
            PathBuf::from(r"C:\dir\a.yml")
        );
    }

    #[test]
    fn a_verbatim_share_path_takes_the_form_the_recycle_bin_reports() {
        assert_eq!(
            shell_form(Path::new(r"\\?\UNC\host\share\a.yml")),
            PathBuf::from(r"\\host\share\a.yml")
        );
    }

    #[test]
    fn a_plain_path_is_left_alone() {
        for plain in ["/home/u/a.yml", r"C:\dir\a.yml", r"\\?\Volume{x}\a.yml"] {
            assert_eq!(shell_form(Path::new(plain)), PathBuf::from(plain));
        }
    }
}
