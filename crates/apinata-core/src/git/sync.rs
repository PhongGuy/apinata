//! Pull (fetch, then fast-forward only) for the branch HEAD is on.

use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::branch::{self, SwitchError, lines};
use super::stream::{CancelFlag, StreamError, git_stream};
use super::{FetchError, GitError, GitOptions, busy_message, check_root, redact_url, run};

/// What a pull did to the current branch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Pulled {
    /// The local branch, such as `main`.
    pub branch: String,
    /// Its upstream as `<remote>/<branch>`, such as `origin/main`.
    pub upstream: String,
    /// The full commit id the branch was at before.
    pub from: String,
    /// The full commit id it is at now; equal to `from` when nothing was new.
    pub to: String,
    /// How many local commits the upstream does not have.
    pub ahead: usize,
}

/// Why a pull did not finish.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SyncError {
    /// HEAD is not on a branch.
    #[error("HEAD is detached; switch to a branch first")]
    Detached,
    /// The branch tracks no remote branch, or the one it tracks is gone.
    #[error("{}", no_upstream_text(branch, gone.as_deref()))]
    NoUpstream {
        /// The local branch.
        branch: String,
        /// The upstream (`<remote>/<branch>`) when it was set but no longer exists.
        gone: Option<String>,
    },
    /// The working tree has staged, unstaged or untracked changes.
    #[error("commit or discard your changes before pulling")]
    Dirty,
    /// Both the branch and its upstream have commits the other lacks.
    #[error(
        "the branch and {upstream} have diverged ({ahead} local and {behind} upstream commits)"
    )]
    Diverged {
        /// The upstream as `<remote>/<branch>`.
        upstream: String,
        /// Local commits the upstream lacks.
        ahead: usize,
        /// Upstream commits the branch lacks.
        behind: usize,
    },
    /// The remote refused the push.
    #[error("the remote rejected the push ({reason})")]
    Rejected {
        /// Git's reason, such as `non-fast-forward`.
        reason: String,
        /// Git's error output.
        stderr: String,
    },
    /// Another git process holds the repository's lock file at `lock`.
    #[error("{}", busy_message(lock))]
    Busy {
        /// The lock file, such as `.git/index.lock`.
        lock: PathBuf,
    },
    /// The fast-forward failed after it had already changed files.
    #[error("the fast-forward to {upstream} stopped after it changed the files: {stderr}")]
    Partial {
        /// The upstream as `<remote>/<branch>`.
        upstream: String,
        /// Git's error output.
        stderr: String,
    },
    /// The call was cancelled.
    #[error("cancelled")]
    Cancelled,
    /// Git refused or failed; this is its text.
    #[error("{stderr}")]
    Failed {
        /// Git's error output.
        stderr: String,
    },
    /// Git could not be started or read.
    #[error(transparent)]
    Io(#[from] io::Error),
}

fn no_upstream_text(branch: &str, gone: Option<&str>) -> String {
    match gone {
        None => format!("{branch} has no upstream branch"),
        Some(upstream) => format!("the upstream {upstream} of {branch} is gone from the remote"),
    }
}

/// Fetch the upstream of the branch HEAD is on and fast-forward to it. A detached HEAD, an
/// unborn branch, a missing upstream or any change refuses before the fetch; a diverged branch
/// is refused after it, with nothing touched. Only `cancel` stops the fetch early.
pub fn pull(root: &Path, cancel: &CancelFlag, opts: &GitOptions) -> Result<Pulled, SyncError> {
    check_root(root).map_err(from_git)?;
    let branch = current_branch(root, opts)?;
    let full = format!("refs/heads/{branch}");
    let Some(head) = verify(root, &full, opts)? else {
        return Err(failed(format!("{branch} has no commits yet; commit first")));
    };
    let (remote, tracking, upstream) = upstream_of(root, &branch, opts)?;
    branch::ready(root, opts).map_err(from_switch)?;
    branch::fetch(root, &remote, cancel, opts).map_err(from_fetch)?;
    if cancel.is_cancelled() {
        return Err(SyncError::Cancelled);
    }
    let Some(oid) = verify(root, &format!("{tracking}^{{commit}}"), opts)? else {
        return Err(SyncError::NoUpstream {
            branch,
            gone: Some(upstream),
        });
    };
    let (ahead, behind) = counts(root, &full, &oid, opts)?;
    let pulled = |to: String| Pulled {
        branch: branch.clone(),
        upstream: upstream.clone(),
        from: head.clone(),
        to,
        ahead,
    };
    match (ahead, behind) {
        (_, 0) => return Ok(pulled(head.clone())),
        (0, _) => {}
        _ => {
            return Err(SyncError::Diverged {
                upstream,
                ahead,
                behind,
            });
        }
    }
    // The fetch may have run while another git process took a lock; refuse before touching it.
    if let Some(lock) = held_lock(root, &branch, opts) {
        return Err(SyncError::Busy { lock });
    }
    // A fresh flag: stopping a checkout halfway would leave the tree half moved.
    match git_stream(
        &["merge", "--ff-only", "--no-autostash", &oid],
        root,
        opts,
        &CancelFlag::new(),
        &mut |_| {},
    ) {
        Ok(done) if done.success => Ok(pulled(oid)),
        Ok(done) => Err(merge_failed(root, &upstream, &done.stderr, opts)),
        Err(StreamError::Io(e)) => Err(SyncError::Io(e)),
        Err(StreamError::Cancelled { .. }) => Err(failed("merge cancelled".to_owned())),
    }
}

/// The first lock the fast-forward would need that some other process already holds.
fn held_lock(root: &Path, branch: &str, opts: &GitOptions) -> Option<PathBuf> {
    let names = [
        "index.lock".to_owned(),
        "HEAD.lock".to_owned(),
        format!("refs/heads/{branch}.lock"),
    ];
    run::git_paths(root, opts, &names)?
        .into_iter()
        .find(|lock| lock.exists())
}

/// Why a failed fast-forward failed. Staged changes come first: a late lock refusal also
/// leaves the checkout half written, and that is what the user must know about.
fn merge_failed(root: &Path, upstream: &str, stderr: &str, opts: &GitOptions) -> SyncError {
    let staged = run::git(&["diff", "--cached", "--quiet"], root, opts)
        .is_ok_and(|out| out.status.code() == Some(1));
    if staged {
        return SyncError::Partial {
            upstream: upstream.to_owned(),
            stderr: redact_url(stderr),
        };
    }
    match run::refused_lock(stderr, root) {
        Some(lock) => SyncError::Busy { lock },
        None => failed(redact_url(stderr)),
    }
}

/// The branch HEAD is on; exit 1 from `symbolic-ref -q` means HEAD is detached.
fn current_branch(root: &Path, opts: &GitOptions) -> Result<String, SyncError> {
    let args = ["symbolic-ref", "-q", "HEAD"];
    let out = run::git(&args, root, opts).map_err(from_git)?;
    if out.status.code() == Some(1) {
        return Err(SyncError::Detached);
    }
    if !out.status.success() {
        return Err(from_git(out.failed(&args)));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    match text.trim_end_matches('\n').strip_prefix("refs/heads/") {
        Some(name) if !name.is_empty() => Ok(name.to_owned()),
        _ => Err(failed(format!("HEAD points at {}", text.trim()))),
    }
}

/// The full id `name` resolves to, `None` when it does not.
fn verify(root: &Path, name: &str, opts: &GitOptions) -> Result<Option<String>, SyncError> {
    let out = run::git(&["rev-parse", "--verify", "-q", name], root, opts).map_err(from_git)?;
    Ok(out
        .status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned()))
}

/// The upstream of `branch` as (remote, tracking ref, `<remote>/<branch>`). A branch with no
/// upstream, or one that tracks a local branch, has none.
fn upstream_of(
    root: &Path,
    branch: &str,
    opts: &GitOptions,
) -> Result<(String, String, String), SyncError> {
    let full = format!("refs/heads/{branch}");
    let format = "--format=%(refname)%00%(upstream)%00%(upstream:remotename)%00\
                  %(upstream:remoteref)%00%(upstream:track)";
    let text = lines(root, &["for-each-ref", format, &full], opts).map_err(from_git)?;
    // The pattern also matches refs below `full`, so only the exact refname counts.
    let fields: Vec<&str> = text
        .lines()
        .map(|l| l.split('\0').collect::<Vec<_>>())
        .find(|f| f.first() == Some(&full.as_str()))
        .unwrap_or_default();
    let field = |i: usize| fields.get(i).copied().unwrap_or("");
    let (tracking, remote, remote_ref) = (field(1), field(2), field(3));
    if tracking.is_empty() || remote.is_empty() || remote == "." {
        return Err(SyncError::NoUpstream {
            branch: branch.to_owned(),
            gone: None,
        });
    }
    let name = remote_ref.strip_prefix("refs/heads/").unwrap_or(remote_ref);
    Ok((
        remote.to_owned(),
        tracking.to_owned(),
        format!("{remote}/{name}"),
    ))
}

/// Commits only on `full` (ahead) and only on `oid` (behind).
fn counts(
    root: &Path,
    full: &str,
    oid: &str,
    opts: &GitOptions,
) -> Result<(usize, usize), SyncError> {
    let range = format!("{full}...{oid}");
    let text =
        lines(root, &["rev-list", "--left-right", "--count", &range], opts).map_err(from_git)?;
    let mut parts = text.split_whitespace().map(str::parse::<usize>);
    match (parts.next(), parts.next()) {
        (Some(Ok(ahead)), Some(Ok(behind))) => Ok((ahead, behind)),
        _ => Err(failed(format!(
            "unexpected rev-list output: {}",
            text.trim()
        ))),
    }
}

fn failed(stderr: String) -> SyncError {
    SyncError::Failed { stderr }
}

/// A [`GitError`] as a sync error: I/O and a held lock keep their kind, everything else carries
/// its text.
fn from_git(e: GitError) -> SyncError {
    match e {
        GitError::Io(e) => SyncError::Io(e),
        GitError::Busy { lock } => SyncError::Busy { lock },
        other => failed(redact_url(&other.to_string())),
    }
}

fn from_switch(e: SwitchError) -> SyncError {
    match e {
        SwitchError::Dirty => SyncError::Dirty,
        SwitchError::Busy { lock } => SyncError::Busy { lock },
        SwitchError::Failed { stderr } => SyncError::Failed { stderr },
        SwitchError::Io(e) => SyncError::Io(e),
    }
}

fn from_fetch(e: FetchError) -> SyncError {
    match e {
        FetchError::Cancelled => SyncError::Cancelled,
        FetchError::Busy { lock } => SyncError::Busy { lock },
        FetchError::Failed { stderr } => SyncError::Failed { stderr },
        FetchError::Io(e) => SyncError::Io(e),
    }
}
