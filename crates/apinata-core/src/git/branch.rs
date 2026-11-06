//! Remotes, fetching and the remote branches of a collection's repository.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::redact_url;
use super::stream::{CancelFlag, StreamError, git_stream};
use super::{GitError, GitOptions, busy_message, check_root, head_paths, run};

/// A branch on a remote and the local branch that tracks it, if any.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct RemoteBranch {
    /// The remote's name, such as `origin`.
    pub remote: String,
    /// The branch name on that remote.
    pub name: String,
    /// The local branch whose upstream this is.
    pub local: Option<String>,
}

impl RemoteBranch {
    /// A remote branch `remote/name` tracked by `local`.
    pub fn new(
        remote: impl Into<String>,
        name: impl Into<String>,
        local: Option<String>,
    ) -> RemoteBranch {
        RemoteBranch {
            remote: remote.into(),
            name: name.into(),
            local,
        }
    }
}

/// A row of the branch list: a local branch with its upstream, or a remote branch no local
/// branch tracks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Branch {
    /// The local branch, `None` for a remote branch nothing tracks.
    pub local: Option<String>,
    /// The remote branch the local one tracks, or the remote branch of a remote-only row.
    pub upstream: Option<RemoteBranch>,
    /// Whether HEAD is on this local branch.
    pub head: bool,
}

/// Why a fetch did not finish.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FetchError {
    /// The fetch was cancelled.
    #[error("fetch cancelled")]
    Cancelled,
    /// Another git process holds the repository's lock file at `lock`.
    #[error("{}", busy_message(lock))]
    Busy {
        /// The lock file, such as `.git/HEAD.lock`.
        lock: PathBuf,
    },
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

/// Run `git <args>` in `root` and return its stdout, or the failure.
pub(super) fn lines(root: &Path, args: &[&str], opts: &GitOptions) -> Result<String, GitError> {
    let out = run::git(args, root, opts)?;
    if !out.status.success() {
        return Err(out.failed(args));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The names of the repository's remotes, one per `git remote` line.
pub fn remotes(root: &Path, opts: &GitOptions) -> Result<Vec<String>, GitError> {
    check_root(root)?;
    let text = lines(root, &["remote"], opts)?;
    Ok(text.lines().map(str::to_owned).collect())
}

/// `git fetch --prune` from `remote`, stopping on `cancel`. Only the name of a configured remote
/// is accepted; anything else is refused without running the fetch.
pub fn fetch(
    root: &Path,
    remote: &str,
    cancel: &CancelFlag,
    opts: &GitOptions,
) -> Result<(), FetchError> {
    if remote.is_empty() || remote.starts_with('-') {
        return Err(FetchError::Failed {
            stderr: "invalid remote name".to_owned(),
        });
    }
    // Only a configured name: a URL, path or `ext::` command is never handed to git as a remote.
    let configured = remotes(root, opts).map_err(|e| match e {
        GitError::Io(e) => FetchError::Io(e),
        other => FetchError::Failed {
            stderr: redact_url(&other.to_string()),
        },
    })?;
    if !configured.iter().any(|name| name == remote) {
        return Err(FetchError::Failed {
            stderr: format!("{} is not a configured remote", redact_url(remote)),
        });
    }
    match git_stream(
        &["fetch", "--prune", "--", remote],
        root,
        opts,
        cancel,
        &mut |_| {},
    ) {
        Ok(done) if done.success => Ok(()),
        Ok(done) => Err(match run::refused_lock(&done.stderr, root) {
            Some(lock) => FetchError::Busy { lock },
            None => FetchError::Failed {
                stderr: redact_url(&done.stderr),
            },
        }),
        // A git that obeyed TERM released its locks; one killed after the grace may have left one.
        Err(StreamError::Cancelled { killed: true }) => {
            Err(match stale_fetch_lock(root, opts, remote, &configured) {
                Some(lock) => FetchError::Busy { lock },
                None => FetchError::Cancelled,
            })
        }
        Err(StreamError::Cancelled { killed: false }) => Err(FetchError::Cancelled),
        Err(StreamError::Io(e)) => Err(FetchError::Io(e)),
    }
}

/// The first lock a killed fetch from `remote` may have left: a `*.lock` under that remote's
/// ref directory (sorted), then `packed-refs.lock`. Never deleted: whose it is cannot be proven.
fn stale_fetch_lock(
    root: &Path,
    opts: &GitOptions,
    remote: &str,
    configured: &[String],
) -> Option<PathBuf> {
    let names = [
        format!("refs/remotes/{remote}"),
        "packed-refs.lock".to_owned(),
    ];
    let paths = run::git_paths(root, opts, &names)?;
    let [refs, packed] = paths.as_slice() else {
        return None;
    };
    let mut found = Vec::new();
    ref_locks(refs, remote, configured, &mut found);
    found.sort();
    found
        .into_iter()
        .next()
        .or_else(|| packed.exists().then(|| packed.clone()))
}

/// Collect the `*.lock` files under `dir`, the ref directory of remote `name`. A subdirectory
/// that is itself the ref directory of another configured remote (`name/...`) is not this
/// fetch's and is skipped.
fn ref_locks(dir: &Path, name: &str, configured: &[String], found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => {
                let nested = format!("{name}/{file}");
                if !configured.contains(&nested) {
                    ref_locks(&entry.path(), &nested, configured, found);
                }
            }
            Ok(_) if file.ends_with(".lock") => found.push(entry.path()),
            _ => {}
        }
    }
}

/// Every remote-tracking branch except `HEAD`, ordered by ref name (so by remote, then branch),
/// each with the local branch that has it as upstream.
pub fn remote_branches(root: &Path, opts: &GitOptions) -> Result<Vec<RemoteBranch>, GitError> {
    check_root(root)?;
    // A local branch with no upstream prints an empty second field and is left out of the map.
    let heads = lines(
        root,
        &[
            "for-each-ref",
            "--format=%(refname)%00%(upstream)",
            "refs/heads",
        ],
        opts,
    )?;
    let tracked: HashMap<&str, &str> = heads
        .lines()
        .filter_map(|l| {
            let (local, upstream) = l.split_once('\0')?;
            Some((upstream, local.strip_prefix("refs/heads/")?))
        })
        .filter(|(upstream, _)| !upstream.is_empty())
        .collect();
    let refs = lines(
        root,
        &["for-each-ref", "--format=%(refname)", "refs/remotes"],
        opts,
    )?;
    // A remote name may hold '/', so the ref is split at the longest configured name that
    // prefixes it, not at the first '/'.
    let mut names = remotes(root, opts)?;
    names.sort_by_key(|n| std::cmp::Reverse(n.len()));
    Ok(refs
        .lines()
        .filter_map(|full| {
            let rest = full.strip_prefix("refs/remotes/")?;
            let (remote, name) = names.iter().find_map(|r| {
                let name = rest.strip_prefix(r.as_str())?.strip_prefix('/')?;
                Some((r.as_str(), name))
            })?;
            (name != "HEAD").then(|| {
                RemoteBranch::new(remote, name, tracked.get(full).map(|l| (*l).to_owned()))
            })
        })
        .collect())
}

/// Every local branch, ordered by name, then every remote branch no local branch tracks (in
/// [`remote_branches`] order). A local branch's upstream is `None` when it tracks nothing, a
/// gone branch or another local branch; `head` marks the branch HEAD is on.
pub fn branches(root: &Path, opts: &GitOptions) -> Result<Vec<Branch>, GitError> {
    let remote = remote_branches(root, opts)?;
    // `%(HEAD)` is `*` on the checked-out branch; a detached or unborn HEAD marks none.
    let heads = lines(
        root,
        &[
            "for-each-ref",
            "--format=%(refname)%00%(upstream)%00%(HEAD)",
            "refs/heads",
        ],
        opts,
    )?;
    let full = |b: &RemoteBranch| format!("refs/remotes/{}/{}", b.remote, b.name);
    let mut rows: Vec<Branch> = heads
        .lines()
        .filter_map(|l| {
            let mut fields = l.split('\0');
            let local = fields.next()?.strip_prefix("refs/heads/")?.to_owned();
            let upstream = fields.next().unwrap_or("");
            Some(Branch {
                local: Some(local),
                upstream: remote.iter().find(|b| full(b) == upstream).cloned(),
                head: fields.next() == Some("*"),
            })
        })
        .collect();
    // A remote branch some local branch tracks is already shown through that branch.
    rows.extend(
        remote
            .into_iter()
            .filter(|b| b.local.is_none())
            .map(|b| Branch {
                local: None,
                upstream: Some(b),
                head: false,
            }),
    );
    Ok(rows)
}

/// Switch to `row`: a local branch is checked out as it is, its upstream untouched; a remote
/// branch nothing tracks goes through [`switch_to_remote`]. Changes, a held lock or a branch
/// gone since the listing refuse the switch before git writes anything.
pub fn switch_branch(
    root: &Path,
    row: &Branch,
    opts: &GitOptions,
) -> Result<Switched, SwitchError> {
    let Some(local) = &row.local else {
        return match &row.upstream {
            Some(upstream) => switch_to_remote(root, upstream, opts),
            None => Err(SwitchError::Failed {
                stderr: "invalid branch".to_owned(),
            }),
        };
    };
    if local.starts_with('-') {
        return Err(SwitchError::Failed {
            stderr: "invalid branch name".to_owned(),
        });
    }
    ready(root, opts)?;
    let full = format!("refs/heads/{local}");
    let text =
        lines(root, &["for-each-ref", "--format=%(refname)", &full], opts).map_err(from_git)?;
    if !text.lines().any(|l| l == full) {
        return Err(SwitchError::Failed {
            stderr: format!("no local branch named {local}"),
        });
    }
    if let Some(lock) = held_lock(root, opts, local, &Mode::Existing) {
        return Err(SwitchError::Busy { lock });
    }
    let done = Switched {
        branch: local.clone(),
        upstream: Upstream::Unchanged,
    };
    let switch = strings(&["switch", "--", local]);
    run_steps(root, opts, &Mode::Existing, None, vec![switch], done)
}

/// The checks every switch makes first: `root` is in a repository and nothing in it changed.
/// Porcelain v1 reports the whole repository, so a change outside `root` also counts. An `add -N`
/// path gone from disk and missing from `HEAD` is no change: git switches over it, and refuses
/// in its own words if the target holds that path. A staged new file gone from disk (`AD`) is a
/// change with a row in the status, so it counts.
pub(super) fn ready(root: &Path, opts: &GitOptions) -> Result<(), SwitchError> {
    check_root(root).map_err(from_git)?;
    let args = ["status", "--porcelain=v1", "-z"];
    let status = run::git(&args, root, opts).map_err(from_git)?;
    if !status.status.success() {
        return Err(from_git(status.failed(&args)));
    }
    // Only ` D` can be excused, so any other entry ends the scan before a rename's second field.
    let mut gone: Vec<&[u8]> = Vec::new();
    for entry in status.stdout.split(|&b| b == 0).filter(|e| !e.is_empty()) {
        gone.push(entry.strip_prefix(b" D ").ok_or(SwitchError::Dirty)?);
    }
    if gone.is_empty() {
        return Ok(());
    }
    let born = run::git(&["rev-parse", "--verify", "-q", "HEAD"], root, opts).map_err(from_git)?;
    let in_head = match born.status.success() {
        true => head_paths(root, opts).ok_or(SwitchError::Dirty)?,
        false => HashSet::new(),
    };
    match gone.iter().any(|path| in_head.contains(*path)) {
        true => Err(SwitchError::Dirty),
        false => Ok(()),
    }
}

/// Why [`switch_branch`] or [`switch_to_remote`] did not switch.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SwitchError {
    /// The working tree has staged, unstaged or untracked changes.
    #[error("commit or discard your changes before switching branches")]
    Dirty,
    /// Another git process holds the repository's lock file at `lock`.
    #[error("{}", busy_message(lock))]
    Busy {
        /// The lock file, such as `.git/HEAD.lock`.
        lock: PathBuf,
    },
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

/// The local branch a switch landed on and what became of its upstream.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Switched {
    /// The local branch now checked out.
    pub branch: String,
    /// How that branch relates to the picked remote branch.
    pub upstream: Upstream,
}

/// What a switch did to the upstream of the local branch it landed on.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Upstream {
    /// The branch is new and tracks the picked remote branch.
    Created,
    /// The branch already tracked the picked remote branch.
    Kept,
    /// The branch tracked nothing and now tracks the picked remote branch.
    Set,
    /// The branch tracks another branch, `upstream` (such as `upstream/main`), left unchanged.
    Other {
        /// The short name of the branch it tracks.
        upstream: String,
    },
    /// A local branch was picked; its upstream was not touched.
    Unchanged,
}

/// Switch to `branch`: its tracking local branch if one exists, else a same-named local branch
/// (given the upstream when it has none, left as is when it tracks another), else a new local
/// branch that tracks it. Any change anywhere in the repository refuses the switch, and so does a
/// held lock before git writes anything. A step that fails undoes what this call made, so a
/// refused switch leaves no branch or upstream behind; nothing is stashed, forced or reset.
pub fn switch_to_remote(
    root: &Path,
    branch: &RemoteBranch,
    opts: &GitOptions,
) -> Result<Switched, SwitchError> {
    let failed = |stderr: &str| SwitchError::Failed {
        stderr: stderr.to_owned(),
    };
    let local_dash = branch.local.as_deref().is_some_and(|l| l.starts_with('-'));
    if branch.name.starts_with('-') || branch.remote.starts_with('-') || local_dash {
        return Err(failed("invalid branch name"));
    }
    ready(root, opts)?;
    let (local, mode) = plan(root, branch, opts)?;
    // Git rewrites the tree before it notices a held HEAD.lock, so locks are checked up front.
    if let Some(lock) = held_lock(root, opts, &local, &mode) {
        return Err(SwitchError::Busy { lock });
    }
    let target = format!("{}/{}", branch.remote, branch.name);
    let tip = match mode {
        Mode::Create => {
            let commit = format!("refs/remotes/{target}^{{commit}}");
            Some(lines(root, &["rev-parse", "--verify", &commit], opts).map_err(from_git)?)
        }
        _ => None,
    };
    let switch = vec!["switch".to_owned(), "--".to_owned(), local.clone()];
    let steps = match mode {
        Mode::Existing | Mode::Other(_) => vec![switch],
        Mode::SetUpstream => vec![
            vec![
                "branch".to_owned(),
                format!("--set-upstream-to={target}"),
                "--".to_owned(),
                local.clone(),
            ],
            switch,
        ],
        Mode::Create => vec![
            strings(&["branch", "--track", "--", &local, &target]),
            switch,
        ],
    };
    let upstream = match &mode {
        Mode::Existing => Upstream::Kept,
        Mode::SetUpstream => Upstream::Set,
        Mode::Create => Upstream::Created,
        Mode::Other(upstream) => Upstream::Other {
            upstream: upstream.clone(),
        },
    };
    let done = Switched {
        branch: local,
        upstream,
    };
    run_steps(root, opts, &mode, tip.as_deref(), steps, done)
}

/// Run the planned write steps in order and report `done` once they all succeed. A failing step
/// that still left HEAD on the new branch counts as switched; otherwise what this call made is
/// undone (see [`roll_back`]) and the step's error is returned.
fn run_steps(
    root: &Path,
    opts: &GitOptions,
    mode: &Mode,
    tip: Option<&str>,
    steps: Vec<Vec<String>>,
    done: Switched,
) -> Result<Switched, SwitchError> {
    let local = done.branch.as_str();
    let before = current_head(root, opts);
    for step in steps {
        let Err(e) = write_step(root, &step, opts) else {
            continue;
        };
        // A git that moved HEAD and then failed (a post-checkout hook, say) did switch.
        let head = current_head(root, opts);
        let on_local = head.as_deref() == Some(format!("refs/heads/{local}").as_str());
        if on_local && head != before {
            return Ok(done);
        }
        // A branch step refused because the branch exists means another process made it
        // after the plan; it is not this call's to undo.
        let theirs = step[0] == "branch"
            && matches!(&e, SwitchError::Failed { stderr } if stderr.contains("already exists"));
        if !on_local && !theirs {
            roll_back(root, opts, mode, local, tip);
        }
        return Err(e);
    }
    Ok(done)
}

/// How a switch reaches its local branch.
enum Mode {
    /// The branch exists and tracks the picked one (or is the caller's tracking branch).
    Existing,
    /// The branch exists with no upstream and gets the picked one.
    SetUpstream,
    /// The branch exists and tracks this other branch, which stays.
    Other(String),
    /// No such branch; a new one is made to track the picked one.
    Create,
}

/// The local branch the switch lands on and how, read fresh from the repository unless the
/// caller already named the tracking branch.
fn plan(
    root: &Path,
    branch: &RemoteBranch,
    opts: &GitOptions,
) -> Result<(String, Mode), SwitchError> {
    if let Some(local) = &branch.local {
        return Ok((local.clone(), Mode::Existing));
    }
    let full = format!("refs/heads/{}", branch.name);
    let format = "--format=%(refname)%00%(upstream)%00%(upstream:short)";
    let text = lines(root, &["for-each-ref", format, &full], opts).map_err(from_git)?;
    // The pattern also matches refs below `full`, so only the exact refname counts.
    let found = text.lines().find_map(|l| {
        let mut fields = l.split('\0');
        (fields.next()? == full).then(|| {
            let upstream = fields.next().unwrap_or("").to_owned();
            (upstream, fields.next().unwrap_or("").to_owned())
        })
    });
    let tracked = format!("refs/remotes/{}/{}", branch.remote, branch.name);
    let mode = match found {
        None => Mode::Create,
        Some((upstream, _)) if upstream == tracked => Mode::Existing,
        Some((upstream, _)) if upstream.is_empty() => Mode::SetUpstream,
        Some((_, short)) => Mode::Other(short),
    };
    Ok((branch.name.clone(), mode))
}

/// The first held lock the planned steps would need: the index and HEAD always, the config when
/// an upstream is written, and the new branch's ref when one is made. `None` when none is held or
/// the lookup fails; a refusal from git still reports a lock then.
fn held_lock(root: &Path, opts: &GitOptions, local: &str, mode: &Mode) -> Option<PathBuf> {
    let mut names = vec!["index.lock".to_owned(), "HEAD.lock".to_owned()];
    if matches!(mode, Mode::Create | Mode::SetUpstream) {
        names.push("config.lock".to_owned());
    }
    if matches!(mode, Mode::Create) {
        names.push(format!("refs/heads/{local}.lock"));
    }
    run::git_paths(root, opts, &names)?
        .into_iter()
        .find(|lock| lock.exists())
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| (*a).to_owned()).collect()
}

/// The ref HEAD points at, `None` when detached or unreadable.
fn current_head(root: &Path, opts: &GitOptions) -> Option<String> {
    let out = run::git(&["symbolic-ref", "-q", "HEAD"], root, opts).ok()?;
    out.status.success().then(|| {
        String::from_utf8_lossy(&out.stdout)
            .trim_end_matches('\n')
            .to_owned()
    })
}

/// Run one write step: the switch streams, so a long checkout has no time limit, while the
/// branch edits are short calls. A refusal naming a lock is `Busy`.
fn write_step(root: &Path, args: &[String], opts: &GitOptions) -> Result<(), SwitchError> {
    let refused = |stderr: &str| match run::refused_lock(stderr, root) {
        Some(lock) => SwitchError::Busy { lock },
        None => SwitchError::Failed {
            stderr: redact_url(stderr.trim()),
        },
    };
    if args.first().is_some_and(|a| a == "switch") {
        return match git_stream(args, root, opts, &CancelFlag::new(), &mut |_| {}) {
            Ok(done) if done.success => Ok(()),
            Ok(done) => Err(refused(&done.stderr)),
            Err(StreamError::Io(e)) => Err(SwitchError::Io(e)),
            // The flag is never set, so a cancel cannot happen; report it as a failure anyway.
            Err(StreamError::Cancelled { .. }) => Err(SwitchError::Failed {
                stderr: "switch cancelled".to_owned(),
            }),
        };
    }
    let out = run::git(args, root, opts).map_err(from_git)?;
    if out.status.success() {
        Ok(())
    } else {
        Err(refused(&String::from_utf8_lossy(&out.stderr)))
    }
}

/// Undo, best effort, what a failed switch made while HEAD stayed off the local branch: a new
/// branch is deleted only at the tip it was made on, then its two tracking keys go; an upstream
/// that was set is unset. Failures here are ignored; the step's own error is what counts.
fn roll_back(root: &Path, opts: &GitOptions, mode: &Mode, local: &str, tip: Option<&str>) {
    let quiet = |args: &[&str]| {
        run::git(args, root, opts)
            .ok()
            .is_some_and(|out| out.status.success())
    };
    match (mode, tip) {
        (Mode::Create, Some(tip)) => {
            let full = format!("refs/heads/{local}");
            if quiet(&["update-ref", "-d", &full, tip.trim()]) {
                for key in ["remote", "merge"] {
                    quiet(&[
                        "config",
                        "--local",
                        "--unset",
                        &format!("branch.{local}.{key}"),
                    ]);
                }
            }
        }
        (Mode::SetUpstream, _) => {
            quiet(&["branch", "--unset-upstream", "--", local]);
        }
        _ => {}
    }
}

/// A [`GitError`] as a switch error: I/O and a held lock keep their kind, everything else
/// carries its text.
fn from_git(e: GitError) -> SwitchError {
    match e {
        GitError::Io(e) => SwitchError::Io(e),
        GitError::Busy { lock } => SwitchError::Busy { lock },
        other => SwitchError::Failed {
            stderr: redact_url(&other.to_string()),
        },
    }
}

#[cfg(all(test, target_os = "linux"))]
mod cancel_tests {
    use super::*;
    use crate::git::run::stop_tests::{Repo, fake_git};
    use std::thread;
    use std::time::{Duration, Instant};

    /// A git that ignores TERM: `remote` lists `remotes`, the global config lookup fails, each
    /// `--git-path X` maps to `.git/X`, and `fetch` creates `.git/<lock>` (none when empty) and
    /// hangs.
    fn stubborn_fetch(remotes: &str, lock: &str) -> String {
        let take = if lock.is_empty() {
            String::new()
        } else {
            format!("mkdir -p \"$(dirname .git/{lock})\"; : > .git/{lock}; ")
        };
        format!(
            r#"#!/bin/sh
trap '' TERM
case "$*" in
  *" fetch "*) {take}: > .git/started; sleep 30 ;;
  *" config "*) exit 1 ;;
  *" remote") printf '{remotes}' ;;
  *rev-parse*) prev=; for a in "$@"; do [ "$prev" = --git-path ] && printf '.git/%s\n' "$a"; prev=$a; done ;;
esac
"#
        )
    }

    /// Fetches `origin` under `script` and cancels once the fake fetch has started, checking the
    /// call took no longer than the grace, the lookups and some slack.
    fn cancelled_fetch(repo: &Repo, script: &str) -> Result<(), FetchError> {
        let options = fake_git(repo, script);
        let cancel = CancelFlag::new();
        let started = repo.root.join(".git/started");
        let begun = Instant::now();
        let r = thread::scope(|s| {
            s.spawn(|| {
                let seen = (0..250).any(|_| {
                    thread::sleep(Duration::from_millis(20));
                    started.exists()
                });
                cancel.cancel();
                assert!(seen, "the fake fetch never started");
            });
            fetch(&repo.root, "origin", &cancel, &options)
        });
        assert!(
            begun.elapsed() < Duration::from_secs(12),
            "took {:?}",
            begun.elapsed()
        );
        r
    }

    fn held(repo: &Repo, lock: &str) -> PathBuf {
        repo.root.join(".git").join(lock)
    }

    #[test]
    fn a_killed_fetch_reports_the_ref_or_packed_refs_lock_it_left_and_keeps_it() {
        for name in ["refs/remotes/origin/main.lock", "packed-refs.lock"] {
            let repo = Repo::new();
            match cancelled_fetch(&repo, &stubborn_fetch("origin\\n", name)) {
                Err(FetchError::Busy { lock }) => assert_eq!(lock, held(&repo, name), "{name}"),
                other => panic!("{name}: expected Busy, got {other:?}"),
            }
            assert!(held(&repo, name).exists(), "{name} was deleted");
        }
    }

    #[test]
    fn a_killed_fetch_ignores_locks_of_other_remotes() {
        for name in [
            "refs/remotes/other/x.lock",
            "refs/remotes/origin/sub/x.lock",
        ] {
            let repo = Repo::new();
            let script = stubborn_fetch("origin\\nother\\norigin/sub\\n", name);
            let r = cancelled_fetch(&repo, &script);
            assert!(matches!(r, Err(FetchError::Cancelled)), "{name}: {r:?}");
            assert!(held(&repo, name).exists(), "{name} was deleted");
        }
    }

    #[test]
    fn a_killed_fetch_with_no_lock_left_is_cancelled() {
        let repo = Repo::new();
        let r = cancelled_fetch(&repo, &stubborn_fetch("origin\\n", ""));
        assert!(matches!(r, Err(FetchError::Cancelled)), "{r:?}");
    }
}
