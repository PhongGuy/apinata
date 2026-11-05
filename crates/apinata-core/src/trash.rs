//! The seam between a collection and a place deleted files can come back from.

use std::ffi::OsString;
use std::fmt::Debug;
use std::path::{Path, PathBuf};

/// A trash that takes files and gives them back. The platform's implements it in the app;
/// tests use a fake, so they never touch the user's trash.
pub trait Trash: Send + Sync + Debug {
    /// Whether this trash looks usable; a cheap check, not a promise that `trash` succeeds.
    fn available(&self) -> bool;

    /// Move the file or directory at `path` into the trash and return how to find it again.
    /// Fails with [`TrashError::Unavailable`] when there is no trash, leaving `path` in place.
    fn trash(&self, path: &Path) -> Result<TrashHandle, TrashError>;

    /// Move the item behind `handle` back to its original path, or fail with
    /// [`TrashError::RestoreUnsupported`] where the platform cannot. It must not overwrite an
    /// existing file; [`Collection::restore_trashed`](crate::Collection::restore_trashed)
    /// checks the path first.
    fn restore(&self, handle: &TrashHandle) -> Result<(), TrashError>;
}

/// Why the trash refused or failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TrashError {
    /// There is no trash to use.
    #[error("the trash is not available: {0}")]
    Unavailable(String),
    /// The item to restore is no longer in the trash: emptied, or restored elsewhere.
    #[error("it is no longer in the trash")]
    Missing,
    /// This platform's trash cannot give items back; the item stays in the system trash.
    #[error("it is in the system trash, but cannot be restored from here")]
    RestoreUnsupported,
    /// The trash exists but the operation failed.
    #[error("{0}")]
    Failed(String),
}

/// Where a trashed item came from and an opaque id the trash that took it can find it by.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TrashHandle {
    original: PathBuf,
    id: OsString,
}

impl TrashHandle {
    /// A handle for the item that was at `original`, known to its trash as `id`.
    pub fn new(original: PathBuf, id: OsString) -> TrashHandle {
        TrashHandle { original, id }
    }

    /// The absolute path the item had before it was trashed.
    pub fn original(&self) -> &Path {
        &self.original
    }

    /// The trash's own id for the item.
    pub fn id(&self) -> &OsString {
        &self.id
    }
}
