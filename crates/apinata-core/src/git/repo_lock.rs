//! A per-repository advisory lock, `apinata.lock` in the git dir, that the app holds around a
//! pull or branch switch and MCP holds around its writes, so neither changes files mid-way
//! through the other. A linked worktree has its own git dir and so its own lock.

use std::fs::{File, TryLockError};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{GitOptions, run};

/// How long [`repo_lock`] callers usually wait for another holder before reporting busy.
pub const REPO_LOCK_WAIT: Duration = Duration::from_secs(2);

/// A held repository lock, released when dropped or when the process exits.
#[derive(Debug)]
#[non_exhaustive]
pub struct RepoLock {
    file: Option<File>,
}

impl RepoLock {
    /// Whether a lock file is actually held: false outside a repository, or when the lock file
    /// could not be opened and the caller goes ahead unlocked.
    pub fn is_held(&self) -> bool {
        self.file.is_some()
    }
}

/// Another holder kept the repository lock for the whole wait.
#[derive(Debug, Clone, thiserror::Error)]
#[error("another tool is changing this collection; try again")]
#[non_exhaustive]
pub struct RepoBusy {
    /// The lock file.
    pub lock: PathBuf,
}

/// Take the lock of the repository holding `root`, retrying for up to `wait`. Outside a
/// repository, or when the lock file cannot be opened or locked for any reason but another
/// holder, it returns an unheld guard and the caller goes ahead unlocked.
pub fn repo_lock(root: &Path, wait: Duration, options: &GitOptions) -> Result<RepoLock, RepoBusy> {
    let unheld = RepoLock { file: None };
    let Some(dir) = git_dir(root, options) else {
        return Ok(unheld);
    };
    let lock = dir.join("apinata.lock");
    let Ok(file) = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock)
    else {
        return Ok(unheld);
    };
    match poll_lock(&file, wait, Duration::from_millis(50)) {
        Polled::Held => Ok(RepoLock { file: Some(file) }),
        Polled::Unsupported(_) => Ok(unheld),
        Polled::Busy => Err(RepoBusy { lock }),
    }
}

/// How a bounded wait for an advisory file lock ended.
pub(crate) enum Polled {
    /// The lock is held until the file is dropped.
    Held,
    /// The filesystem cannot lock this file at all.
    Unsupported(std::io::Error),
    /// Another holder kept it for the whole wait.
    Busy,
}

/// Tries to lock `file` every `step` until `wait` runs out. It polls rather than blocks, so
/// the wait stays bounded on every platform.
pub(crate) fn poll_lock(file: &File, wait: Duration, step: Duration) -> Polled {
    let deadline = Instant::now() + wait;
    loop {
        match file.try_lock() {
            Ok(()) => return Polled::Held,
            Err(TryLockError::Error(e)) => return Polled::Unsupported(e),
            Err(TryLockError::WouldBlock) => {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Polled::Busy;
                }
                std::thread::sleep(left.min(step));
            }
        }
    }
}

/// The absolute git dir of the repository holding `root`, `None` outside one or without git.
fn git_dir(root: &Path, options: &GitOptions) -> Option<PathBuf> {
    let out = run::git(&["rev-parse", "--absolute-git-dir"], root, options).ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    Some(PathBuf::from(text.trim_end_matches(['\n', '\r'])))
}
