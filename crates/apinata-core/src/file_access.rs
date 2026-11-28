//! Which files a collection may name, and the one reader every such file goes through.

use std::io;
use std::path::{Component, Path, PathBuf};

use crate::document::{is_special_file, read_regular};

/// Where the files a collection names (body files, multipart parts, key files and client
/// certificates) may be read from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FileAccess {
    /// Only beneath the collection root: absolute paths, `..` above the root and links that
    /// leave it are refused.
    #[default]
    Collection,
    /// Anywhere the process can read; relative paths are joined to the collection root.
    Anywhere,
}

/// Whether `given` is absolute (a drive, UNC or root prefix included) or has a `..` that climbs
/// above the root it is joined to. Purely lexical: it never touches the disk.
pub(crate) fn escapes_root(given: &Path) -> bool {
    let mut depth = 0usize;
    for part in given.components() {
        match part {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir if depth > 0 => depth -= 1,
            _ => return true,
        }
    }
    false
}

/// Why a collection file was not read.
#[derive(Debug)]
pub(crate) enum ReadFailure {
    /// The path leaves the collection root, so nothing was opened.
    Outside,
    /// The file could not be read.
    Io(io::Error),
}

/// A collection root and the access its files are read under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileScope {
    root: PathBuf,
    access: FileAccess,
}

impl FileScope {
    pub(crate) fn new(root: impl Into<PathBuf>, access: FileAccess) -> FileScope {
        FileScope {
            root: root.into(),
            access,
        }
    }

    /// The same root read under `access`.
    pub(crate) fn with_access(&self, access: FileAccess) -> FileScope {
        FileScope::new(self.root.clone(), access)
    }

    /// Whether `given` passes the lexical check; links are judged only when it is opened.
    pub(crate) fn admits(&self, given: &Path) -> bool {
        self.access == FileAccess::Anywhere || !escapes_root(given)
    }

    /// Whether `given` names something a read could find, judged by the same lookup beneath the
    /// root as a read, so a link leading out is [`ReadFailure::Outside`]. Only metadata is read.
    pub(crate) fn probe(&self, given: &Path) -> Result<(), ReadFailure> {
        if self.access == FileAccess::Anywhere {
            return std::fs::metadata(self.root.join(given))
                .map(drop)
                .map_err(ReadFailure::Io);
        }
        if escapes_root(given) {
            return Err(ReadFailure::Outside);
        }
        let ambient = cap_std::ambient_authority();
        let found = cap_std::fs::Dir::open_ambient_dir(&self.root, ambient)
            .and_then(|dir| dir.metadata(or_root(given)));
        match found {
            Ok(_) => Ok(()),
            Err(e) if self.leaves_root(given, &e) => Err(ReadFailure::Outside),
            Err(e) => Err(ReadFailure::Io(e)),
        }
    }

    /// The bytes of the regular file `given` names, relative to the root.
    pub(crate) fn read(&self, given: &Path) -> Result<Vec<u8>, ReadFailure> {
        self.read_as(given, Kind::Regular)
    }

    fn read_as(&self, given: &Path, kind: Kind) -> Result<Vec<u8>, ReadFailure> {
        let joined = self.root.join(given);
        if self.access == FileAccess::Anywhere {
            let read = match kind {
                Kind::Regular => read_regular(&joined),
                Kind::Body => std::fs::read(&joined),
            };
            return read.map_err(ReadFailure::Io);
        }
        if escapes_root(given) {
            return Err(ReadFailure::Outside);
        }
        // A body may stream from a named pipe, opened so it waits for a writer; any other
        // special file seen before the open is never opened, since opening some devices acts.
        let pipe = kind == Kind::Body && is_fifo(&joined);
        if !pipe && is_special_file(&joined) {
            return Err(not_regular());
        }
        // One open resolves every component beneath the root, so a link swapped in after any
        // earlier look still cannot lead out of it.
        let mut file = match open_beneath(&self.root, given, !pipe) {
            Ok(file) => file,
            Err(e) if self.leaves_root(given, &e) => return Err(ReadFailure::Outside),
            Err(e) => return Err(ReadFailure::Io(e)),
        };
        let opened = file.metadata().map_err(ReadFailure::Io)?.file_type();
        if !opened.is_file() && !opened.is_dir() && !(pipe && is_fifo_type(opened)) {
            return Err(not_regular());
        }
        let mut bytes = Vec::new();
        io::Read::read_to_end(&mut file, &mut bytes).map_err(ReadFailure::Io)?;
        Ok(bytes)
    }

    /// [`FileScope::read`] on a blocking thread.
    pub(crate) fn read_async(&self, given: &Path) -> ReadTask {
        self.spawn_read(Some(given.to_path_buf()), Kind::Regular)
    }

    /// A body file at `joined`, a path already joined to the root, read on a blocking thread; a
    /// named pipe is read too. Under [`FileAccess::Collection`] the caller has checked the path
    /// as written with `admits`.
    pub(crate) fn read_body_async(&self, joined: &Path) -> ReadTask {
        let given = match joined.strip_prefix(&self.root) {
            Ok(rel) => Some(rel.to_path_buf()),
            Err(_) if self.access == FileAccess::Anywhere => Some(joined.to_path_buf()),
            Err(_) => None,
        };
        self.spawn_read(given, Kind::Body)
    }

    /// Starts reading `given` at once; `None` is a path already known to leave the root.
    fn spawn_read(&self, given: Option<PathBuf>, kind: Kind) -> ReadTask {
        let scope = self.clone();
        ReadTask(
            given.map(|given| tokio::task::spawn_blocking(move || scope.read_as(&given, kind))),
        )
    }

    /// Whether a failed open was a link leading out of the root rather than a missing or
    /// unreadable file. Only metadata is read.
    fn leaves_root(&self, given: &Path, err: &io::Error) -> bool {
        let real = |p: &Path| std::fs::canonicalize(p).ok();
        match (real(&self.root), real(&self.root.join(given))) {
            (Some(root), Some(file)) => !file.starts_with(root),
            // A dangling link out of the root: the open refused the escape itself.
            _ => err.kind() == io::ErrorKind::PermissionDenied,
        }
    }
}

/// A read running on a blocking thread; `None` refuses a path already known to leave the root.
pub(crate) struct ReadTask(Option<tokio::task::JoinHandle<Result<Vec<u8>, ReadFailure>>>);

impl Future for ReadTask {
    type Output = Result<Vec<u8>, ReadFailure>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        match self.0.as_mut() {
            None => std::task::Poll::Ready(Err(ReadFailure::Outside)),
            Some(task) => std::pin::Pin::new(task)
                .poll(cx)
                .map(|joined| joined.unwrap_or_else(|e| Err(ReadFailure::Io(io::Error::other(e))))),
        }
    }
}

/// What a read accepts besides a regular file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Regular files only, as keys and certificates are read.
    Regular,
    /// A named pipe as well, as a body file may stream from one.
    Body,
}

fn not_regular() -> ReadFailure {
    ReadFailure::Io(io::Error::new(
        io::ErrorKind::NotFound,
        "not a regular file",
    ))
}

fn is_fifo(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| is_fifo_type(m.file_type()))
}

#[cfg(unix)]
fn is_fifo_type(kind: std::fs::FileType) -> bool {
    std::os::unix::fs::FileTypeExt::is_fifo(&kind)
}

#[cfg(not(unix))]
fn is_fifo_type(_: std::fs::FileType) -> bool {
    false
}

/// `given` opened for reading beneath `root`, no component (links included) resolving outside
/// it; with `nonblocking` the open never waits on a pipe.
fn open_beneath(root: &Path, given: &Path, nonblocking: bool) -> io::Result<std::fs::File> {
    use cap_std::fs::{Dir, OpenOptions};
    let dir = Dir::open_ambient_dir(root, cap_std::ambient_authority())?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        let wait = if nonblocking { libc::O_NONBLOCK } else { 0 };
        cap_std::fs::OpenOptionsExt::custom_flags(&mut options, wait | libc::O_NOCTTY);
    }
    #[cfg(not(unix))]
    let _ = nonblocking;
    Ok(dir.open_with(or_root(given), &options)?.into_std())
}

/// `given`, with an empty path naming the root itself as `.`.
fn or_root(given: &Path) -> &Path {
    if given.as_os_str().is_empty() {
        Path::new(".")
    } else {
        given
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_parent_drive_and_unc_paths_escape_lexically() {
        for inside in ["a.txt", "./a/../b.txt", "a/b/../../c"] {
            assert!(!escapes_root(Path::new(inside)), "{inside}");
        }
        for outside in ["../a", "a/../../b", "/etc/passwd"] {
            assert!(escapes_root(Path::new(outside)), "{outside}");
        }
        #[cfg(windows)]
        for outside in [r"C:\x", r"C:x", r"\x", r"\\server\share\x", r"\\?\C:\x"] {
            assert!(escapes_root(Path::new(outside)), "{outside}");
        }
    }

    #[test]
    fn anywhere_admits_every_path() {
        let scope = FileScope::new("/r", FileAccess::Anywhere);
        assert!(scope.admits(Path::new("../a")));
        assert!(!FileScope::new("/r", FileAccess::Collection).admits(Path::new("../a")));
    }
}
