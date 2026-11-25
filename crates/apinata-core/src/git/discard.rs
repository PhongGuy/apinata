//! Throwing away a file's changes: what is on disk goes to the trash first, then git restores
//! `HEAD`'s version, and an undo puts the trashed copy back.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::repo_lock::{REPO_LOCK_WAIT, RepoBusy, repo_lock};
use super::write::{check_relative, run_write, under_link};
use sha2::{Digest as _, Sha256};

use super::{
    FileStatus, GitError, GitOptions, StageState, check_root, git_stdout, show_prefix, status,
};
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
    // A hash of what git put at the path, so an undo can tell it was not edited since.
    restored: Option<[u8; 32]>,
    // Something on disk or in the index was dropped and nothing kept it.
    lost: bool,
}

impl Discarded {
    /// Whether [`undo_discard`] can bring back everything the discard dropped: every file on
    /// disk went to the trash, and no staged version differing from it was dropped.
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
    /// A step failed after files went to the trash, and these could not be put back.
    #[error("{cause}; your version of {} is still in the trash", list(paths))]
    Stranded {
        /// The files left in the trash, relative to the collection.
        paths: Vec<PathBuf>,
        /// What failed first.
        cause: String,
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
            return Err(malformed(path, "is behind a symbolic link"));
        }
    }
    let _lock = repo_lock(root, REPO_LOCK_WAIT, options)?;
    let prefix = show_prefix(root, options)?;
    let current = status(root, &prefix, usize::MAX, options)?;

    // Each touched path, whether `HEAD` holds it, whether git knows it, and whether the index
    // holds a version of it nothing else keeps; a rename touches its old path too.
    let mut touched: Vec<Touched> = Vec::new();
    for path in paths {
        let path = path.as_ref().to_path_buf();
        let mut staged_only = matches!(
            current.stage.get(&path),
            Some(StageState::Both | StageState::StagedGone)
        );
        // A rename's staged copy that is still `HEAD`'s file comes back with the old path.
        if let Some(FileStatus::Renamed { from }) = current.files.get(&path) {
            let staged = format!(":0:{prefix}{}", path.to_string_lossy());
            let head = format!("HEAD:{prefix}{}", from.to_string_lossy());
            let blob = |rev: &str| git_stdout(&["rev-parse", rev], root, options);
            staged_only &= blob(&staged).is_none_or(|b| Some(b) != blob(&head));
        }
        let at = |path: &PathBuf, head, git| Touched {
            path: path.clone(),
            head,
            git,
            staged_only,
        };
        match current.files.get(&path) {
            None => return Err(DiscardError::Unchanged(path)),
            Some(FileStatus::Conflicted) => return Err(DiscardError::Conflicted(path)),
            Some(FileStatus::Modified | FileStatus::Deleted) => touched.push(at(&path, true, true)),
            Some(FileStatus::Renamed { from }) => {
                if under_link(root, from) {
                    return Err(malformed(from, "is behind a symbolic link"));
                }
                touched.push(at(&path, false, true));
                touched.push(at(from, true, true));
            }
            Some(FileStatus::Untracked) => touched.push(at(&path, false, false)),
            Some(_) => touched.push(at(&path, false, true)),
        }
    }
    // A folder row is a nested repository, which is more than the one file the caller names.
    if let Some(t) = touched.iter().find(|t| {
        let meta = root.join(&t.path).symlink_metadata();
        meta.is_ok_and(|m| m.is_dir())
    }) {
        return Err(malformed(&t.path, "is a folder"));
    }
    let usable = trash.available();
    if !usable && let Some(t) = touched.iter().find(|t| !t.head && on_disk(root, &t.path)) {
        return Err(DiscardError::NeedsTrash(t.path.clone()));
    }

    // Every file on disk goes to the trash before git runs; a refusal puts back what went.
    let mut files: Vec<DiscardedFile> = Vec::new();
    for Touched {
        path, staged_only, ..
    } in &touched
    {
        let present = on_disk(root, path);
        let trashed = match present && usable {
            true => match trash.trash(&root.join(path)) {
                Ok(handle) => Some(handle),
                Err(e) => return Err(put_back(root, &files, trash, trash_error(path, e))),
            },
            false => None,
        };
        files.push(DiscardedFile {
            path: path.clone(),
            lost: *staged_only || (present && trashed.is_none()),
            trashed,
            restored: None,
        });
    }

    let tracked: Vec<&PathBuf> = (touched.iter())
        .filter(|t| t.git)
        .map(|t| &t.path)
        .collect();
    if !tracked.is_empty() {
        let mut args: Vec<OsString> = ["restore", "--source=HEAD", "--staged", "--worktree", "--"]
            .into_iter()
            .map(OsString::from)
            .collect();
        args.extend(tracked.iter().map(|p| p.as_os_str().to_owned()));
        if let Err(e) = run_write(&args, root, options) {
            return Err(put_back(root, &files, trash, e.into()));
        }
    }
    // A copy that cannot be read cannot be told apart from a later edit, so it ends the undo.
    for file in &mut files {
        match fingerprint(&root.join(&file.path)) {
            Ok(print) => file.restored = print,
            Err(_) => file.lost = true,
        }
    }
    Ok(Discarded { files })
}

/// Undo `discarded` under the repository lock: each trashed version comes back in place of git's
/// restored copy, which is set aside first and returned if the trash cannot give the version
/// back, so no path is left empty. Refused with `Changed`, before anything moves, when a path no
/// longer holds what the discard left there, and with `CannotUndo` when something was dropped
/// that no trash kept.
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
        if fingerprint(&root.join(&file.path)).ok() != Some(file.restored) {
            return Err(DiscardError::Changed(file.path.clone()));
        }
    }
    for file in &discarded.files {
        let Some(handle) = &file.trashed else {
            continue;
        };
        let full = root.join(&file.path);
        let aside = match file.restored {
            Some(_) => {
                let aside = aside_path(&full);
                fs::rename(&full, &aside).map_err(GitError::Io)?;
                Some(aside)
            }
            None => None,
        };
        match trash.restore(handle) {
            Ok(()) => {
                if let Some(aside) = aside {
                    fs::remove_file(aside).map_err(GitError::Io)?;
                }
            }
            Err(e) => {
                if let Some(aside) = aside {
                    fs::rename(aside, &full).map_err(GitError::Io)?;
                }
                return Err(trash_error(&file.path, e));
            }
        }
    }
    // Git's copy of a file that had nothing on disk goes once every trashed version is back.
    for file in &discarded.files {
        if file.trashed.is_none() && file.restored.is_some() {
            fs::remove_file(root.join(&file.path)).map_err(GitError::Io)?;
        }
    }
    Ok(())
}

// A free name beside `full` to hold git's copy while the trash puts the version back.
fn aside_path(full: &Path) -> PathBuf {
    let name = full.file_name().unwrap_or_default().to_string_lossy();
    (0u32..)
        .map(|n| full.with_file_name(format!(".{name}.undo-{}-{n}", std::process::id())))
        .find(|p| p.symlink_metadata().is_err())
        .unwrap_or_else(|| full.with_file_name(format!(".{name}.undo")))
}

// Whether anything, a dangling link included, sits at `rel`.
fn on_disk(root: &Path, rel: &Path) -> bool {
    root.join(rel).symlink_metadata().is_ok()
}

// A hash of the kind and content of what sits at `path`, never following a link; `None` when
// nothing is.
fn fingerprint(path: &Path) -> io::Result<Option<[u8; 32]>> {
    let meta = match path.symlink_metadata() {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut hash = Sha256::new();
    if meta.file_type().is_symlink() {
        hash.update(b"link:");
        hash.update(fs::read_link(path)?.into_os_string().into_encoded_bytes());
    } else if meta.is_file() {
        hash.update(b"file:");
        let mut file = fs::File::open(path)?;
        let mut chunk = vec![0u8; 64 * 1024];
        loop {
            let n = io::Read::read(&mut file, &mut chunk)?;
            if n == 0 {
                break;
            }
            hash.update(&chunk[..n]);
        }
    } else {
        hash.update(b"other:");
    }
    Ok(Some(hash.finalize().into()))
}

// After `cause`, every trashed file whose path is free again comes back; the error names any
// that stayed in the trash.
fn put_back(
    root: &Path,
    files: &[DiscardedFile],
    trash: &dyn Trash,
    cause: DiscardError,
) -> DiscardError {
    let stranded: Vec<PathBuf> = (files.iter())
        .filter_map(|f| Some((f, f.trashed.as_ref()?)))
        .filter(|(f, handle)| on_disk(root, &f.path) || trash.restore(handle).is_err())
        .map(|(f, _)| f.path.clone())
        .collect();
    match stranded.is_empty() {
        true => cause,
        false => DiscardError::Stranded {
            paths: stranded,
            cause: cause.to_string(),
        },
    }
}

fn list(paths: &[PathBuf]) -> String {
    let names: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
    names.join(", ")
}

fn malformed(path: &Path, why: &str) -> DiscardError {
    GitError::Malformed(format!("{} {why}", path.display())).into()
}

struct Touched {
    path: PathBuf,
    head: bool,
    git: bool,
    staged_only: bool,
}

fn trash_error(path: &Path, error: TrashError) -> DiscardError {
    DiscardError::Trash {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}
