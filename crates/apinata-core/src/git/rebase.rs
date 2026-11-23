//! Pull by rebasing unpushed commits onto the upstream, with a fallback for a rebase that stops:
//! the local bytes of every conflicted path are kept outside the repository before the branch is
//! reset to the remote tip, and the local changes that did not conflict are committed on top.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use super::branch::{self, SwitchError, Untracked, lines};
use super::stream::{CancelFlag, StreamError, git_stream};
use super::sync::{
    UpstreamRef, counts, current_branch, failed, fast_forward, from_fetch, from_git, from_switch,
    upstream_of, verify,
};
use super::write::has_identity;
use super::{CommitError, GitOptions, SyncError, check_root, commit, redact_url, run};

/// The ref that keeps the branch tip from before the last sync, so nothing local is unreachable.
pub(super) const PRE_SYNC: &str = "refs/apinata/pre-sync";

/// The message of the commit that re-applies the local changes after a conflict.
const FALLBACK_MESSAGE: &str = "Keep local changes after a sync conflict";

/// What `rebase_pull` and `squash_unpushed` did.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RebasePulled {
    /// The upstream had nothing the branch lacks.
    UpToDate,
    /// The branch had no unpushed commits and now sits at the upstream tip `to`.
    FastForwarded {
        /// The full id of the new tip.
        to: String,
    },
    /// The unpushed commits were replayed on the upstream tip.
    Rebased {
        /// The full id of the new tip.
        to: String,
        /// How many commits sit on top of the upstream tip.
        unpushed: usize,
    },
    /// The rebase stopped; local bytes went to the copies folder, HEAD is the remote tip
    /// plus at most one fallback commit, and `to` is that HEAD.
    Conflicted {
        /// The full id of the new tip.
        to: String,
        /// The paths both sides changed differently, in path order.
        paths: Vec<ConflictPath>,
    },
}

/// One path the local and remote sides changed differently.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ConflictPath {
    /// The path relative to the repository's top, `/`-separated.
    pub path: String,
    /// How the two sides differ.
    pub kind: ConflictKind,
}

/// How the two sides of a conflicted path differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConflictKind {
    /// Both sides changed the file; the local bytes are kept as a copy.
    BothChanged,
    /// The remote deleted a file the local side changed; the local bytes are kept as a copy.
    DeletedRemotely,
    /// The local side deleted a file the remote changed; the remote file stays.
    DeletedLocally,
}

/// Fetch the upstream of the branch HEAD is on and bring the branch onto it: a fast-forward when
/// nothing is unpushed, otherwise a rebase of the unpushed commits. A rebase that stops falls
/// back to the remote tip, keeping each conflicted path's local bytes under `copies`. A detached
/// HEAD, a missing upstream or any uncommitted change refuses before any ref is touched; the tip
/// from before is kept at `refs/apinata/pre-sync`.
pub fn rebase_pull(
    root: &Path,
    copies: &Path,
    cancel: &CancelFlag,
    opts: &GitOptions,
) -> Result<RebasePulled, SyncError> {
    check_root(root).map_err(from_git)?;
    let branch = current_branch(root, opts)?;
    let full = format!("refs/heads/{branch}");
    let Some(local) = verify(root, &full, opts)? else {
        return Err(failed(format!("{branch} has no commits yet; commit first")));
    };
    let UpstreamRef {
        remote,
        tracking,
        name: upstream,
        ..
    } = upstream_of(root, &branch, opts)?;
    branch::ready(root, opts, Untracked::Refuse).map_err(from_switch)?;
    write_step(&["update-ref", PRE_SYNC, &local], root, opts)?;
    branch::fetch(root, &remote, cancel, opts).map_err(from_fetch)?;
    if cancel.is_cancelled() {
        return Err(SyncError::Cancelled);
    }
    // A save can land while the fetch runs; it reads as a dirty tree the caller retries, not a
    // failure git's own refusal would report.
    branch::ready(root, opts, Untracked::Refuse).map_err(from_switch)?;
    let Some(tip) = verify(root, &format!("{tracking}^{{commit}}"), opts)? else {
        return Err(SyncError::NoUpstream {
            branch,
            gone: Some(upstream),
        });
    };
    match counts(root, &full, &tip, opts)? {
        (_, 0) => Ok(RebasePulled::UpToDate),
        (0, _) => {
            fast_forward(root, &branch, &upstream, &tip, opts)?;
            Ok(RebasePulled::FastForwarded { to: tip })
        }
        _ => replay_onto(root, &local, &tip, copies, opts),
    }
}

/// Replay the commits of `local` onto `remote`; a rebase that stops on a conflict takes the
/// fallback that keeps the local bytes under `copies`. HEAD must be on a branch at `local`.
pub(super) fn replay_onto(
    root: &Path,
    local: &str,
    remote: &str,
    copies: &Path,
    opts: &GitOptions,
) -> Result<RebasePulled, SyncError> {
    // Replayed commits need a committer, and the fallback commit an author too.
    if !has_identity(root, opts).map_err(from_git)? {
        return Err(SyncError::NoIdentity);
    }
    let args = [
        "-c",
        "user.useConfigOnly=true",
        "rebase",
        "--no-autostash",
        "--no-autosquash",
        "--no-update-refs",
        remote,
    ];
    // A fresh flag: a rebase stopped halfway would leave the tree between two histories.
    let done = match git_stream(&args, root, opts, &CancelFlag::new(), &mut |_| {}) {
        Ok(done) => done,
        Err(StreamError::Io(e)) => return Err(SyncError::Io(e)),
        Err(StreamError::Cancelled { .. }) => return Err(failed("rebase cancelled".to_owned())),
    };
    if done.success {
        let range = format!("{remote}..HEAD");
        let count = lines(root, &["rev-list", "--count", &range], opts).map_err(from_git)?;
        let unpushed = count
            .trim()
            .parse()
            .map_err(|_| failed(format!("unexpected rev-list output: {}", count.trim())))?;
        return Ok(RebasePulled::Rebased {
            to: head(root, opts)?,
            unpushed,
        });
    }
    // Only a rebase that left its state folder stopped on a conflict; one refused before it
    // began (a hook, a lock) changed nothing and is reported as it is.
    let stopped = run::git_paths(root, opts, &["rebase-merge", "rebase-apply"])
        .is_some_and(|paths| paths.iter().any(|p| p.exists()));
    if !stopped {
        // A save after the last check makes git refuse over the tree; that is still a dirty tree.
        if let Err(SwitchError::Dirty) = branch::ready(root, opts, Untracked::Refuse) {
            return Err(SyncError::Dirty);
        }
        return Err(match run::refused_lock(&done.stderr, root) {
            Some(lock) => SyncError::Busy { lock },
            None => failed(redact_url(done.stderr.trim())),
        });
    }
    write_step(&["rebase", "--abort"], root, opts)?;
    fall_back(root, local, remote, copies, opts)
}

/// After an aborted rebase: copy out each conflicted path's local bytes, and only once they are
/// all on disk reset to `remote` and re-apply every other local change as one commit.
fn fall_back(
    root: &Path,
    local: &str,
    remote: &str,
    copies: &Path,
    opts: &GitOptions,
) -> Result<RebasePulled, SyncError> {
    let base = lines(root, &["merge-base", local, remote], opts).map_err(from_git)?;
    let base = base.trim();
    let mine = changed(root, base, local, opts)?;
    let theirs = changed(root, base, remote, opts)?;
    let mut paths = Vec::new();
    for path in mine.intersection(&theirs) {
        let (l, r) = (
            blob(root, local, path, opts)?,
            blob(root, remote, path, opts)?,
        );
        let kind = match (&l, &r) {
            _ if l == r => continue,
            (None, _) => ConflictKind::DeletedLocally,
            (_, None) => ConflictKind::DeletedRemotely,
            _ => ConflictKind::BothChanged,
        };
        match &l {
            // The newest conflict on a path wins, so a copy replaces an older delete's mark.
            Some(id) => {
                keep_copy(root, id, &copies.join(path), opts)?;
                match fs::remove_file(deleted_marks(copies).join(path)) {
                    Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
                    _ => {}
                }
            }
            None => mark_deleted(&deleted_marks(copies).join(path))?,
        }
        paths.push(ConflictPath {
            path: path.clone(),
            kind,
        });
    }
    write_step(&["reset", "-q", "--hard", remote], root, opts)?;

    // Every local change outside the conflicts goes back on top of the remote tip.
    let (mut restore, mut remove) = (vec![], vec![]);
    for path in mine.iter().filter(|p| !paths.iter().any(|c| &c.path == *p)) {
        match (
            blob(root, local, path, opts)?,
            blob(root, remote, path, opts)?,
        ) {
            (l, r) if l == r => {}
            (Some(_), _) => restore.push(path.as_str()),
            (None, Some(_)) => remove.push(path.as_str()),
            (None, None) => {}
        }
    }
    if !restore.is_empty() {
        write_step(
            &[&["checkout", local, "--"], &restore[..]].concat(),
            root,
            opts,
        )?;
    }
    if !remove.is_empty() {
        write_step(&[&["rm", "-q", "--"], &remove[..]].concat(), root, opts)?;
    }
    if !restore.is_empty() || !remove.is_empty() {
        commit(root, FALLBACK_MESSAGE, opts).map_err(|e| match e {
            CommitError::Git(e) => from_git(e),
            other => failed(other.to_string()),
        })?;
    }
    Ok(RebasePulled::Conflicted {
        to: head(root, opts)?,
        paths,
    })
}

/// The paths that differ between `from` and `to`, renames counted as a delete and an add.
fn changed(
    root: &Path,
    from: &str,
    to: &str,
    opts: &GitOptions,
) -> Result<BTreeSet<String>, SyncError> {
    let args = ["diff", "--name-only", "--no-renames", "-z", from, to];
    let out = run::git(&args, root, opts).map_err(from_git)?;
    if !out.status.success() {
        return Err(from_git(out.failed(&args)));
    }
    Ok(out
        .stdout
        .split(|&b| b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect())
}

/// The blob id of `path` in `tip`, `None` when the path is not there.
fn blob(
    root: &Path,
    tip: &str,
    path: &str,
    opts: &GitOptions,
) -> Result<Option<String>, SyncError> {
    verify(root, &format!("{tip}:{path}"), opts)
}

/// Write blob `id` to `dest` through a synced temporary file beside it, replacing an older copy.
fn keep_copy(root: &Path, id: &str, dest: &Path, opts: &GitOptions) -> Result<(), SyncError> {
    let out = run::git(&["cat-file", "blob", id], root, opts).map_err(from_git)?;
    if !out.status.success() {
        return Err(from_git(out.failed(&["cat-file", "blob", id])));
    }
    let dir = dest
        .parent()
        .ok_or_else(|| io::Error::other("a copy needs a parent folder"))?;
    fs::create_dir_all(dir)?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(&out.stdout)?;
    tmp.as_file().sync_all()?;
    tmp.persist(dest).map_err(|e| e.error)?;
    Ok(())
}

/// The folder beside `copies` that remembers each conflicted local delete as an empty file at
/// its path, since such a conflict leaves no copy behind to show it after a restart.
pub(crate) fn deleted_marks(copies: &Path) -> PathBuf {
    let mut name = copies.as_os_str().to_owned();
    name.push(".deleted");
    PathBuf::from(name)
}

/// Record one conflicted local delete; the mark is empty, its path is the whole message.
fn mark_deleted(mark: &Path) -> Result<(), SyncError> {
    let dir = mark
        .parent()
        .ok_or_else(|| io::Error::other("a mark needs a parent folder"))?;
    fs::create_dir_all(dir)?;
    fs::File::create(mark)?.sync_all()?;
    Ok(())
}

/// The full id of HEAD.
pub(super) fn head(root: &Path, opts: &GitOptions) -> Result<String, SyncError> {
    verify(root, "HEAD", opts)?.ok_or_else(|| failed("HEAD does not resolve".to_owned()))
}

/// One call that changes the repository; a refusal naming a lock is `Busy`.
pub(super) fn write_step(args: &[&str], root: &Path, opts: &GitOptions) -> Result<(), SyncError> {
    let out = run::git_write(args, &[], root, opts).map_err(from_git)?;
    if out.status.success() {
        Ok(())
    } else {
        Err(from_git(out.failed(args)))
    }
}
