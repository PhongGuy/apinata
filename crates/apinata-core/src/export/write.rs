//! The atomic, shareable file write behind every export, and its plain-words error.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// A failed export write: names the target file and says why in plain words.
#[derive(Debug)]
#[non_exhaustive]
pub struct WriteError {
    path: PathBuf,
    error: std::io::Error,
}

impl WriteError {
    /// The file that was not written.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What kind of failure stopped the write.
    pub fn kind(&self) -> ErrorKind {
        self.error.kind()
    }
}

// The reason names the folder or the disk, never the temporary file the write went through.
impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cannot write {}: ", self.path.display())?;
        let folder = folder(&self.path).display();
        match self.kind() {
            ErrorKind::NotFound => write!(f, "the folder {folder} does not exist"),
            ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem => {
                write!(f, "you do not have permission to save files in {folder}")
            }
            ErrorKind::StorageFull | ErrorKind::QuotaExceeded => write!(f, "the disk is full"),
            ErrorKind::IsADirectory => write!(f, "a folder already has that name"),
            ErrorKind::NotADirectory => write!(f, "part of the folder path is a file"),
            other => write!(f, "{other}"),
        }
    }
}

impl std::error::Error for WriteError {}

/// Writes `bytes` to a temporary file beside `path`, then renames it over `path`, so a reader
/// never sees a half-written export. The file is meant to be shared, so on unix a new one gets
/// the umask's ordinary mode and a replaced one keeps its mode. Fails when the parent is missing.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), WriteError> {
    write(path, bytes).map_err(|error| WriteError {
        path: path.to_path_buf(),
        error,
    })
}

/// The folder `path` is written into; a bare file name is written into the current one.
fn folder(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let parent = folder(path);
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut builder = tempfile::Builder::new();
    // tempfile defaults to 0600; 0666 is filtered by the umask like any other new file.
    #[cfg(unix)]
    builder.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o666));
    let mut file = builder.tempfile_in(parent)?;
    file.write_all(bytes)?;
    #[cfg(unix)]
    if let Ok(existing) = std::fs::metadata(path) {
        file.as_file().set_permissions(existing.permissions())?;
    }
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // A full disk cannot be made on demand, so the message is checked from the error kind.
    #[test]
    fn a_full_disk_is_named_in_plain_words() {
        let err = WriteError {
            path: PathBuf::from("/x/c.json"),
            error: std::io::Error::from(ErrorKind::StorageFull),
        };
        assert_eq!(err.to_string(), "cannot write /x/c.json: the disk is full");
    }
}
