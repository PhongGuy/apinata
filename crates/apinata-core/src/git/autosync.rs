//! The unattended git calls of team sync: commit everything, squash the unpushed range so a
//! refused blob leaves history, read the server's and the last fetch's tip, count unpushed
//! commits, and check or throw away the working tree.

use std::path::Path;

use super::branch::{self, lines};
use super::rebase::{PRE_SYNC, head, replay_onto, write_step};
use super::stream::CancelFlag;
use super::sync::{
    UpstreamRef, counts, current_branch, failed, fast_forward, from_fetch, from_git, from_switch,
    upstream_of, verify,
};
use super::{
    CommitError, CommitOutcome, GitError, GitOptions, RebasePulled, SyncError, check_root, commit,
    redact_url, run,
};

/// Stage every change in the repository (`add -A`, so ignored files stay out) and commit it with
/// git's own identity and hooks. `None` when nothing was left to commit.
pub fn commit_all(
    root: &Path,
    message: &str,
    opts: &GitOptions,
) -> Result<Option<CommitOutcome>, CommitError> {
    check_root(root)?;
    write(&["add", "-A"], root, opts)?;
    // Exit 1 from a quiet cached diff is the only sign that something is staged.
    let args = ["diff", "--cached", "--quiet"];
    let out = run::git(&args, root, opts)?;
    match out.status.code() {
        Some(0) => Ok(None),
        Some(1) => commit(root, message, opts).map(Some),
        _ => Err(out.failed(&args).into()),
    }
}

/// Fetch, then fold every unpushed commit into one commit of the branch's current tree and
/// replay that onto the upstream tip, so blobs only the dropped commits held are no longer
/// pushed. The tip from before is kept at `refs/apinata/pre-sync`, and a dirty tree refuses.
pub fn squash_unpushed(
    root: &Path,
    message: &str,
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
    branch::ready(root, opts).map_err(from_switch)?;
    branch::fetch(root, &remote, cancel, opts).map_err(from_fetch)?;
    if cancel.is_cancelled() {
        return Err(SyncError::Cancelled);
    }
    let Some(tip) = verify(root, &format!("{tracking}^{{commit}}"), opts)? else {
        return Err(SyncError::NoUpstream {
            branch,
            gone: Some(upstream),
        });
    };
    write_step(&["update-ref", PRE_SYNC, &local], root, opts)?;
    let (ahead, behind) = counts(root, &full, &tip, opts)?;
    if ahead == 0 {
        if behind == 0 {
            return Ok(RebasePulled::UpToDate);
        }
        fast_forward(root, &branch, &upstream, &tip, opts)?;
        return Ok(RebasePulled::FastForwarded { to: tip });
    }

    let base = lines(root, &["merge-base", &local, &tip], opts).map_err(from_git)?;
    let base = base.trim();
    write_step(&["reset", "-q", "--soft", base], root, opts)?;
    match commit(root, message, opts) {
        Ok(_) => {}
        // The unpushed commits cancel out: what is left is the upstream's own history.
        Err(CommitError::NothingStaged) if base == tip => return Ok(RebasePulled::UpToDate),
        Err(CommitError::NothingStaged) => {
            fast_forward(root, &branch, &upstream, &tip, opts)?;
            return Ok(RebasePulled::FastForwarded { to: tip });
        }
        // A refused commit puts the branch back where it was, so nothing is left half squashed.
        Err(e) => {
            write_step(&["reset", "-q", "--soft", &local], root, opts)?;
            return Err(match e {
                CommitError::Git(e) => from_git(e),
                other => failed(other.to_string()),
            });
        }
    }
    let squashed = head(root, opts)?;
    if base == tip {
        return Ok(RebasePulled::Rebased {
            to: squashed,
            unpushed: 1,
        });
    }
    replay_onto(root, &squashed, &tip, copies, opts)
}

/// The server's tip of the upstream branch, read with `ls-remote` under the call's time limit;
/// `None` when the server has no such branch.
pub fn remote_tip(
    root: &Path,
    cancel: &CancelFlag,
    opts: &GitOptions,
) -> Result<Option<String>, SyncError> {
    check_root(root).map_err(from_git)?;
    let branch = current_branch(root, opts)?;
    let up = upstream_of(root, &branch, opts)?;
    if cancel.is_cancelled() {
        return Err(SyncError::Cancelled);
    }
    let args = ["ls-remote", "--heads", "--", &up.remote, &up.remote_ref];
    let out = run::git(&args, root, opts).map_err(from_git)?;
    if cancel.is_cancelled() {
        return Err(SyncError::Cancelled);
    }
    if !out.status.success() {
        return Err(failed(redact_url(
            String::from_utf8_lossy(&out.stderr).trim(),
        )));
    }
    // The pattern matches by suffix, so only the line naming exactly the upstream ref counts.
    Ok(String::from_utf8_lossy(&out.stdout).lines().find_map(|l| {
        let (oid, name) = l.split_once('\t')?;
        (name == up.remote_ref).then(|| oid.to_owned())
    }))
}

/// The remote-tracking ref's commit as the last fetch left it; `None` when it does not exist.
pub fn tracking_tip(root: &Path, opts: &GitOptions) -> Result<Option<String>, SyncError> {
    check_root(root).map_err(from_git)?;
    let branch = current_branch(root, opts)?;
    let up = upstream_of(root, &branch, opts)?;
    verify(root, &format!("{}^{{commit}}", up.tracking), opts)
}

/// How many commits of the current branch its remote-tracking ref lacks.
pub fn unpushed(root: &Path, opts: &GitOptions) -> Result<usize, SyncError> {
    check_root(root).map_err(from_git)?;
    let branch = current_branch(root, opts)?;
    let up = upstream_of(root, &branch, opts)?;
    let (ahead, _) = counts(root, &format!("refs/heads/{branch}"), &up.tracking, opts)?;
    Ok(ahead)
}

/// Put the working tree back at `HEAD`: edits are reset and untracked files and folders removed,
/// while ignored files such as `.env` stay.
pub fn discard_changes(root: &Path, opts: &GitOptions) -> Result<(), GitError> {
    check_root(root)?;
    write(&["reset", "-q", "--hard", "HEAD"], root, opts)?;
    write(&["clean", "-q", "-f", "-d"], root, opts)
}

/// Whether `status --porcelain` lists any change; ignored files do not count.
pub fn is_dirty(root: &Path, opts: &GitOptions) -> Result<bool, GitError> {
    check_root(root)?;
    let args = ["status", "--porcelain"];
    let out = run::git(&args, root, opts)?;
    if !out.status.success() {
        return Err(out.failed(&args));
    }
    Ok(!out.stdout.is_empty())
}

/// One call that changes the repository, its failure carrying git's text.
fn write(args: &[&str], root: &Path, opts: &GitOptions) -> Result<(), GitError> {
    let out = run::git_write(args, &[], root, opts)?;
    if out.status.success() {
        Ok(())
    } else {
        Err(out.failed(args))
    }
}
