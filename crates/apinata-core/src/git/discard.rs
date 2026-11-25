//! Throwing away a file's changes: what is on disk goes to the trash first, then git restores
//! `HEAD`'s version, and an undo puts the trashed copy back.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use super::repo_lock::{REPO_LOCK_WAIT, RepoBusy, repo_lock};
use super::write::{check_relative, run_write, under_link};
use super::{FileStatus, GitError, GitOptions, check_root, show_prefix, status};
use crate::trash::{Trash, TrashError, TrashHandle};

/// What [`discard`] did, which [`undo_discard`] reverses.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Discarded {
    files: Vec<DiscardedFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiscardedFile {
    // Relative to the collection root.
    path: PathBuf,
    // Where the version on disk went; `None` when nothing was there or there was no trash.
    trashed: Option<TrashHandle>,
    // What git put at the path, so an undo can tell it was not edited since.
    restored: Option<Vec<u8>>,
    // Something was on disk and nothing kept it.
    lost: bool,
}

impl Discarded {
    /// Whether [`undo_discard`] can bring back what was on disk: every file there went to the
    /// trash.
    pub fn undoable(&self) -> bool {
        !self.files.iter().any(|f| f.lost)
    }

    /// The collection-relative paths the discard changed, a rename's two among them.
    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.files.iter().map(|f| f.path.as_path())
    }
}

/// Why [`discard`] or [`undo_discard`] did nothing.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DiscardError {
    /// Git lists no change at this path in the collection.
    #[error("{} has no changes to discard", .0.display())]
    Unchanged(PathBuf),
    /// The file has unresolved merge conflicts.
    #[error("resolve the conflicts in {} before discarding it", .0.display())]
    Conflicted(PathBuf),
    /// The file is not in the last commit, so without a trash it would be deleted for good.
    #[error("{} is not in the last commit and there is no trash to keep it in", .0.display())]
    NeedsTrash(PathBuf),
    /// The trash refused a file; nothing was discarded.
    #[error("the trash could not take {}: {message}", path.display())]
    Trash {
        /// The file, relative to the collection.
        path: PathBuf,
        /// The trash's reason.
        message: String,
    },
    /// The file changed after the discard, so an undo would lose that edit.
    #[error("{} changed after the discard, so it was left as it is", .0.display())]
    Changed(PathBuf),
    /// Part of what was discarded never went to a trash, so it cannot come back.
    #[error("this discard cannot be undone")]
    CannotUndo,
    /// Another tool holds the repository lock.
    #[error(transparent)]
    Busy(#[from] RepoBusy),
    /// A git call or file operation failed.
    #[error(transparent)]
    Git(#[from] GitError),
}

/// Discard the changes to `paths` (relative to the collection `root`) under the repository lock.
/// Whatever is on disk at a path goes to `trash` before git touches it; then modified and deleted
/// files are restored from `HEAD` in the index and on disk, added files leave the index, and a
/// rename restores its old path and drops its new one. Untracked files only go to the trash.
/// Refused before any change: a path leaving the collection or under a symlink (`Malformed`),
/// one git lists no change for, a conflicted one, and, without a usable trash, any file not in
/// `HEAD`. Without a trash a file in `HEAD` is overwritten and the result is not undoable.
pub fn discard(
    root: &Path,
    paths: &[impl AsRef<Path>],
    trash: &dyn Trash,
    options: &GitOptions,
) -> Result<Discarded, DiscardError> {
    check_root(root)?;
    for path in paths {
        let path = path.as_ref();
        check_relative(path)?;
        if under_link(root, path) {
            return Err(GitError::Malformed(format!(
                "{} is behind a symbolic link",
                path.display()
            ))
            .into());
        }
    }
    let _lock = repo_lock(root, REPO_LOCK_WAIT, options)?;
    let prefix = show_prefix(root, options)?;
    let current = status(root, &prefix, usize::MAX, options)?;

    // Each touched path, whether `HEAD` holds it and whether git knows it; a rename touches
    // its old path too.
    let mut touched: Vec<(PathBuf, bool, bool)> = Vec::new();
    for path in paths {
        let path = path.as_ref().to_path_buf();
        match current.files.get(&path) {
            None => return Err(DiscardError::Unchanged(path)),
            Some(FileStatus::Conflicted) => return Err(DiscardError::Conflicted(path)),
            Some(FileStatus::Modified | FileStatus::Deleted) => touched.push((path, true, true)),
            Some(FileStatus::Renamed { from }) => {
                let from = from.clone();
                touched.push((path, false, true));
                touched.push((from, true, true));
            }
            Some(FileStatus::Untracked) => touched.push((path, false, false)),
            Some(_) => touched.push((path, false, true)),
        }
    }
    let usable = trash.available();
    if !usable
        && let Some((path, ..)) = touched
            .iter()
            .find(|(p, head, _)| !head && on_disk(root, p))
    {
        return Err(DiscardError::NeedsTrash(path.clone()));
    }

    // Every file on disk goes to the trash before git runs; a refusal puts back what went.
    let mut files: Vec<DiscardedFile> = Vec::new();
    for (path, ..) in &touched {
        let present = on_disk(root, path);
        let trashed = match present && usable {
            true => match trash.trash(&root.join(path)) {
                Ok(handle) => Some(handle),
                Err(e) => {
                    put_back(root, &files, trash);
                    return Err(trash_error(path, e));
                }
            },
            false => None,
        };
        files.push(DiscardedFile {
            path: path.clone(),
            lost: present && trashed.is_none(),
            trashed,
            restored: None,
        });
    }

    let tracked: Vec<&PathBuf> = (touched.iter())
        .filter(|(.., git)| *git)
        .map(|(path, ..)| path)
        .collect();
    if !tracked.is_empty() {
        let mut args: Vec<OsString> = ["restore", "--source=HEAD", "--staged", "--worktree", "--"]
            .into_iter()
            .map(OsString::from)
            .collect();
        args.extend(tracked.iter().map(|p| p.as_os_str().to_owned()));
        if let Err(e) = run_write(&args, root, options) {
            put_back(root, &files, trash);
            return Err(e.into());
        }
    }
    for file in &mut files {
        file.restored = fingerprint(&root.join(&file.path));
    }
    Ok(Discarded { files })
}

/// Undo `discarded` under the repository lock: git's restored copies are removed and the trashed
/// versions come back. Refused with `Changed`, before anything moves, when a path no longer
/// holds what the discard left there, and with `CannotUndo` when a version was never trashed.
pub fn undo_discard(
    root: &Path,
    discarded: &Discarded,
    trash: &dyn Trash,
    options: &GitOptions,
) -> Result<(), DiscardError> {
    check_root(root)?;
    if !discarded.undoable() {
        return Err(DiscardError::CannotUndo);
    }
    let _lock = repo_lock(root, REPO_LOCK_WAIT, options)?;
    for file in &discarded.files {
        if fingerprint(&root.join(&file.path)) != file.restored {
            return Err(DiscardError::Changed(file.path.clone()));
        }
    }
    for file in &discarded.files {
        let full = root.join(&file.path);
        if file.restored.is_some() {
            fs::remove_file(&full).map_err(GitError::Io)?;
        }
    }
    for file in &discarded.files {
        if let Some(handle) = &file.trashed {
            trash
                .restore(handle)
                .map_err(|e| trash_error(&file.path, e))?;
        }
    }
    Ok(())
}

// Whether anything, a dangling link included, sits at `rel`.
fn on_disk(root: &Path, rel: &Path) -> bool {
    root.join(rel).symlink_metadata().is_ok()
}

// The kind and content of what sits at `path`, never following a link; `None` when nothing is.
fn fingerprint(path: &Path) -> Option<Vec<u8>> {
    let meta = path.symlink_metadata().ok()?;
    let (tag, body) = if meta.file_type().is_symlink() {
        let target = fs::read_link(path).ok()?;
        ("link:", target.into_os_string().into_encoded_bytes())
    } else if meta.is_file() {
        ("file:", fs::read(path).unwrap_or_default())
    } else {
        ("other:", Vec::new())
    };
    Some([tag.as_bytes(), &body].concat())
}

// Best effort after a failure: every trashed file whose path is free again comes back.
fn put_back(root: &Path, files: &[DiscardedFile], trash: &dyn Trash) {
    for file in files {
        if let Some(handle) = &file.trashed
            && !on_disk(root, &file.path)
        {
            let _ = trash.restore(handle);
        }
    }
}

fn trash_error(path: &Path, error: TrashError) -> DiscardError {
    DiscardError::Trash {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}
