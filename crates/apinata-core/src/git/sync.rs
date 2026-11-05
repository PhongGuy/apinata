//! Pull (fetch, then fast-forward only) and push (never forced) for the branch HEAD is on.

use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::branch::{self, SwitchError, lines};
use super::stream::{CancelFlag, StreamError, git_stream, git_stream_capture};
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

/// What a push sent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Pushed {
    /// The local branch pushed.
    pub branch: String,
    /// Where it went, as `<remote>/<branch>`.
    pub upstream: String,
    /// The remote branch's id before; `None` for a new branch, equal to `to` when up to date.
    pub from: Option<String>,
    /// The full id pushed.
    pub to: String,
    /// Whether `--set-upstream` made it the branch's upstream.
    pub upstream_set: bool,
}

/// Why a pull or push did not finish.
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
    let Upstream {
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

/// Push the branch HEAD is on to its upstream, or with `set_upstream` to the same name on that
/// remote and make it the upstream. The refspec carries no `+`, so a push is never forced; a
/// detached HEAD, an unborn branch, a missing or gone upstream or an unknown remote refuses
/// before git pushes. Uncommitted changes do not matter.
pub fn push(
    root: &Path,
    set_upstream: Option<&str>,
    cancel: &CancelFlag,
    opts: &GitOptions,
) -> Result<Pushed, SyncError> {
    check_root(root).map_err(from_git)?;
    let branch = current_branch(root, opts)?;
    let full = format!("refs/heads/{branch}");
    let Some(to) = verify(root, &format!("{full}^{{commit}}"), opts)? else {
        return Err(failed(format!("{branch} has no commits yet; commit first")));
    };
    let (remote, dst, upstream) = match set_upstream {
        Some(remote) => {
            configured_remote(root, remote, opts)?;
            (
                remote.to_owned(),
                full.clone(),
                format!("{remote}/{branch}"),
            )
        }
        None => {
            let up = upstream_of(root, &branch, opts)?;
            if up.gone {
                return Err(SyncError::NoUpstream {
                    branch,
                    gone: Some(up.name),
                });
            }
            configured_remote(root, &up.remote, opts)?;
            (up.remote, up.remote_ref, up.name)
        }
    };
    let refspec = format!("{full}:{dst}");
    let mut args = vec![
        "push",
        "--porcelain",
        "--no-recurse-submodules",
        "--no-follow-tags",
    ];
    if set_upstream.is_some() {
        args.push("--set-upstream");
    }
    args.extend(["--", &remote, &refspec]);
    let done = match git_stream_capture(&args, root, opts, cancel, &mut |_| {}) {
        Ok(done) => done,
        Err(StreamError::Io(e)) => return Err(SyncError::Io(e)),
        Err(StreamError::Cancelled { .. }) => return Err(SyncError::Cancelled),
    };
    let from = match (done.success, porcelain_ref(&done.stdout)) {
        (true, Some((' ', summary))) => {
            let Some((before, _)) = summary.split_once("..") else {
                return Err(failed(redact_url(&done.stderr)));
            };
            match verify(root, &format!("{before}^{{commit}}"), opts)? {
                Some(oid) => Some(oid),
                None => {
                    return Err(failed(format!(
                        "unknown commit {before} in the push result"
                    )));
                }
            }
        }
        (true, Some(('=', _))) => Some(to.clone()),
        (true, Some(('*', _))) => None,
        _ => return Err(failed(redact_url(&done.stderr))),
    };
    Ok(Pushed {
        branch,
        upstream,
        from,
        to,
        upstream_set: set_upstream.is_some(),
    })
}

/// The flag and summary of the one ref line in `push --porcelain` output: the line whose second
/// byte is a tab. The `To` and tracking lines are skipped.
fn porcelain_ref(stdout: &str) -> Option<(char, &str)> {
    let line = stdout
        .lines()
        .find(|l| l.as_bytes().get(1) == Some(&b'\t'))?;
    let flag = line.chars().next()?;
    let summary = line.splitn(3, '\t').nth(2)?;
    Some((flag, summary.trim()))
}

/// Refuse anything but the name of a configured remote, so a URL, path or option never reaches
/// git as one.
fn configured_remote(root: &Path, remote: &str, opts: &GitOptions) -> Result<(), SyncError> {
    if remote.is_empty() || remote.starts_with('-') {
        return Err(failed("invalid remote name".to_owned()));
    }
    let configured = branch::remotes(root, opts).map_err(from_git)?;
    if !configured.iter().any(|name| name == remote) {
        return Err(failed(format!(
            "{} is not a configured remote",
            redact_url(remote)
        )));
    }
    Ok(())
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

/// A branch's upstream as git configures it.
struct Upstream {
    remote: String,
    /// The remote-tracking ref, such as `refs/remotes/origin/main`.
    tracking: String,
    /// The branch on the remote, such as `refs/heads/main`.
    remote_ref: String,
    /// `<remote>/<branch>`, such as `origin/main`.
    name: String,
    /// Whether the remote-tracking ref no longer exists.
    gone: bool,
}

/// The upstream of `branch`. A branch with no upstream, or one that tracks a local branch, has
/// none.
fn upstream_of(root: &Path, branch: &str, opts: &GitOptions) -> Result<Upstream, SyncError> {
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
    let (tracking, remote, remote_ref, track) = (field(1), field(2), field(3), field(4));
    if tracking.is_empty() || remote.is_empty() || remote == "." {
        return Err(SyncError::NoUpstream {
            branch: branch.to_owned(),
            gone: None,
        });
    }
    let name = remote_ref.strip_prefix("refs/heads/").unwrap_or(remote_ref);
    Ok(Upstream {
        remote: remote.to_owned(),
        tracking: tracking.to_owned(),
        remote_ref: remote_ref.to_owned(),
        name: format!("{remote}/{name}"),
        gone: track == "[gone]",
    })
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
