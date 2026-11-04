//! What to watch in a repository so a commit, stage or checkout made elsewhere is noticed:
//! the git directory itself (for `HEAD`, `index` and `packed-refs`) and the branch heads.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use crate::git_head::git_dir_for;

/// The directories to watch for the collection at `root`; empty outside a repository.
///
/// Always the git directory, never its objects. A linked worktree's `.git` file points at its
/// own git directory, which holds its `HEAD` and `index`; its branches live in the shared
/// directory named by `commondir`, so that one's `refs/heads` is watched too.
pub fn dirs(root: &Path) -> BTreeSet<PathBuf> {
    let Some(git_dir) = git_dir_for(root).filter(|d| d.is_dir()) else {
        return BTreeSet::new();
    };
    let common = std::fs::read_to_string(git_dir.join("commondir"))
        .ok()
        .map(|text| lexical(&git_dir.join(text.trim())))
        .unwrap_or_else(|| git_dir.clone());
    let heads = common.join("refs").join("heads");
    let mut out = BTreeSet::from([git_dir]);
    if heads.is_dir() {
        out.insert(heads);
    }
    out
}

/// Whether a change to `path` can alter the status or the branch: `HEAD`, `index`,
/// `packed-refs` or a branch head, but never a lock file, which git renames over the real one.
pub fn relevant(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if name.ends_with(".lock") {
        return false;
    }
    if matches!(name, "HEAD" | "index" | "packed-refs") {
        return true;
    }
    let mut up = path.ancestors().skip(1);
    let heads = up.next().and_then(Path::file_name);
    let refs = up.next().and_then(Path::file_name);
    heads == Some("heads".as_ref()) && refs == Some("refs".as_ref())
}

// `commondir` is usually `../..`; the dots are folded away so the path is the one a watch
// reports and tests compare, without resolving symlinks.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}
