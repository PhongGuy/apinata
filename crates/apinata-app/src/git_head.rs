//! The git branch label for a collection, read from plain files with no git process; every
//! failure gives `None`.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// The nearest `.git` at or above `start`: a directory as is, or a file's `gitdir:` path
/// resolved against the file's own directory.
pub fn git_dir_for(start: &Path) -> Option<PathBuf> {
    // Only the first `.git` counts: an unreadable one ends the search instead of skipping it.
    let (dir, dot_git) = start
        .ancestors()
        .map(|dir| (dir, dir.join(".git")))
        .find_map(|(dir, dot_git)| match fs::symlink_metadata(&dot_git) {
            Ok(_) => Some(Some((dir, dot_git))),
            Err(e) if e.kind() == ErrorKind::NotFound => None,
            Err(_) => Some(None),
        })??;
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    let text = fs::read_to_string(&dot_git).ok()?;
    let target = text.trim().strip_prefix("gitdir:")?.trim();
    (!target.is_empty()).then(|| dir.join(target))
}

/// `HEAD` in `git_dir`: `ref: refs/heads/<name>` gives `<name>` (None for `.invalid`, the
/// reftable stub); 40 or 64 hex digits give the first 7; anything else None.
pub fn head_label(git_dir: &Path) -> Option<String> {
    let text = fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = text.trim();
    if let Some(name) = head.strip_prefix("ref: refs/heads/") {
        return (!name.is_empty() && name != ".invalid").then(|| name.to_owned());
    }
    let is_hash = matches!(head.len(), 40 | 64) && head.bytes().all(|b| b.is_ascii_hexdigit());
    is_hash.then(|| head[..7].to_owned())
}

/// `git_dir_for(start)` then `head_label`.
pub fn branch_label(start: &Path) -> Option<String> {
    head_label(&git_dir_for(start)?)
}
