//! Cloning a repository: the request, progress and error types and `clone_repo`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use super::GitOptions;
use super::stream::{CancelFlag, StreamError, Streamed, git_stream};
use super::url::{redact_url, validate_clone_url, validate_folder_name};

/// What to clone and where: `url` into `parent/name`.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct CloneRequest {
    /// The repository URL or absolute path.
    pub url: String,
    /// The existing directory that will hold the clone.
    pub parent: PathBuf,
    /// The folder created inside `parent`.
    pub name: String,
}

impl CloneRequest {
    /// A request to clone `url` into `parent/name`.
    pub fn new(
        url: impl Into<String>,
        parent: impl Into<PathBuf>,
        name: impl Into<String>,
    ) -> CloneRequest {
        CloneRequest {
            url: url.into(),
            parent: parent.into(),
            name: name.into(),
        }
    }
}

/// One progress line from git: a phase and, when git gave one, its percentage.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CloneProgress {
    /// The phase text, such as `Receiving objects`.
    pub phase: String,
    /// How far along the phase is, 0 to 100.
    pub percent: Option<u8>,
}

/// Split a git stderr line into a phase and a percentage. A `remote: ` prefix is dropped; a line
/// without a `%` is all phase.
fn parse_progress(line: &str) -> CloneProgress {
    let line = line.strip_prefix("remote: ").unwrap_or(line);
    let percent_at = line.find('%');
    let parsed = percent_at.and_then(|at| {
        let (phase, _) = line.split_once(':')?;
        let digits: String = line[..at]
            .chars()
            .rev()
            .take_while(char::is_ascii_digit)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        Some((phase.trim().to_owned(), digits.parse::<u8>().ok()?))
    });
    match parsed {
        Some((phase, percent)) => CloneProgress {
            phase,
            percent: Some(percent.min(100)),
        },
        None => CloneProgress {
            phase: line.trim().to_owned(),
            percent: None,
        },
    }
}

/// Clone `req.url` into `req.parent/req.name`, reporting git's progress and stopping on `cancel`.
/// A failed or cancelled clone removes what it created (an existing empty folder is emptied, not
/// deleted). Returns the new folder. Credentials in the URL never appear in errors or progress.
pub fn clone_repo(
    req: &CloneRequest,
    cancel: &CancelFlag,
    opts: &GitOptions,
    on_progress: &mut dyn FnMut(CloneProgress),
) -> Result<PathBuf, CloneError> {
    validate_clone_url(&req.url)?;
    validate_folder_name(&req.name)?;
    // An explicit check: a spawn in a missing cwd reports NotFound, which would read as GitMissing.
    if !fs::metadata(&req.parent)?.is_dir() {
        return Err(
            io::Error::new(io::ErrorKind::NotADirectory, "parent is not a directory").into(),
        );
    }
    let dest = req.parent.join(&req.name);
    let existed = match fs::read_dir(&dest) {
        Ok(mut entries) => {
            if entries.next().is_some() {
                return Err(CloneError::Exists(dest));
            }
            true
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => false,
        Err(e) => return Err(CloneError::Io(e)),
    };
    if cancel.is_cancelled() {
        return Err(CloneError::Cancelled);
    }

    // Git echoes the URL in several variants, so every URL-shaped word is redacted, not just ours.
    let scrub = redact_url;
    // Everything after `--` is a positional, so a folder name starting with '-' is not an option.
    let args = [
        "clone",
        "--progress",
        "--no-recurse-submodules",
        "--",
        req.url.as_str(),
        req.name.as_str(),
    ];
    let result = git_stream(&args, &req.parent, opts, cancel, &mut |line| {
        on_progress(parse_progress(&scrub(line)));
    });
    settle(result, dest, existed, scrub)
}

/// Turn the streamed outcome into the clone result, removing what the clone created on every
/// failure except a missing git binary (which created nothing).
fn settle(
    result: Result<Streamed, StreamError>,
    dest: PathBuf,
    existed: bool,
    scrub: fn(&str) -> String,
) -> Result<PathBuf, CloneError> {
    match result {
        Ok(done) if done.success => Ok(dest),
        Ok(done) => {
            remove_created(&dest, existed);
            Err(CloneError::Failed {
                stderr: scrub(&done.stderr),
            })
        }
        Err(StreamError::Cancelled { .. }) => {
            remove_created(&dest, existed);
            Err(CloneError::Cancelled)
        }
        Err(StreamError::Io(e)) if e.kind() == io::ErrorKind::NotFound => {
            Err(CloneError::GitMissing)
        }
        Err(StreamError::Io(e)) => {
            remove_created(&dest, existed);
            Err(CloneError::Io(e))
        }
    }
}

/// Undo a clone: delete `dest`, or only empty it when it was there before. One retry after a
/// short pause covers a killed git still releasing files; a missing path is already clean.
fn remove_created(dest: &Path, existed: bool) {
    for attempt in 0..2 {
        if attempt > 0 {
            thread::sleep(Duration::from_millis(50));
        }
        let outcome = if existed {
            empty_dir(dest)
        } else {
            fs::remove_dir_all(dest)
        };
        match outcome {
            Ok(()) => return,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return,
            Err(_) => {}
        }
    }
}

fn empty_dir(dir: &Path) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() && !path.is_symlink() {
            fs::remove_dir_all(&path)?;
        } else {
            fs::remove_file(&path)?;
        }
    }
    Ok(())
}

/// Why a repository could not be cloned.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CloneError {
    /// The repository URL was refused before git ran.
    #[error("invalid repository URL: {reason}")]
    InvalidUrl {
        /// What is wrong with the URL.
        reason: String,
    },
    /// The folder name is not one plain path component.
    #[error("invalid folder name: {reason}")]
    InvalidName {
        /// What is wrong with the name.
        reason: String,
    },
    /// The target folder exists and is not empty.
    #[error("{} already exists", .0.display())]
    Exists(PathBuf),
    /// The clone was cancelled.
    #[error("the clone was cancelled")]
    Cancelled,
    /// No `git` binary was found on `PATH`.
    #[error("git was not found on PATH")]
    GitMissing,
    /// Git exited unsuccessfully.
    #[error("git clone failed: {stderr}")]
    Failed {
        /// Git's trimmed error output.
        stderr: String,
    },
    /// The target folder or git could not be read or started.
    #[error(transparent)]
    Io(#[from] io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stream I/O error can strike after git created files in the destination, so it must
    /// clean up like the other failures; a real git cannot be made to fail that way on demand.
    #[test]
    fn a_stream_io_error_removes_what_the_clone_created() {
        let tmp = tempfile::tempdir().unwrap();
        let fresh = tmp.path().join("fresh");
        fs::create_dir_all(fresh.join(".git")).unwrap();
        let kept = tmp.path().join("kept");
        fs::create_dir_all(&kept).unwrap();
        fs::write(kept.join("half"), "x").unwrap();

        let err = || StreamError::Io(io::Error::from(io::ErrorKind::PermissionDenied));
        let r = settle(Err(err()), fresh.clone(), false, redact_url);
        assert!(matches!(r, Err(CloneError::Io(_))), "{r:?}");
        assert!(!fresh.exists());

        let r = settle(Err(err()), kept.clone(), true, redact_url);
        assert!(matches!(r, Err(CloneError::Io(_))), "{r:?}");
        assert!(kept.is_dir() && fs::read_dir(&kept).unwrap().next().is_none());
    }
}
