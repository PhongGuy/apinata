//! A collection's git status and diffs, read through the `git` binary with fixed flags, plus the
//! stage, unstage and commit operations, and the clone, fetch and branch operations. Status, diff
//! and commit stay local; clone and fetch use the network. Git's global config is read for
//! `core.sshCommand`, and no call writes config.

mod branch;
mod clone;
mod diff;
pub(crate) use diff::hunks_within;
pub use diff::text_hunks;
mod porcelain;
mod run;
mod stream;
pub use branch::{
    Branch, FetchError, RemoteBranch, SwitchError, Switched, Upstream, branches, fetch,
    remote_branches, remotes, switch_branch, switch_to_remote,
};
pub use clone::{CloneError, CloneProgress, CloneRequest, clone_repo};
pub use stream::CancelFlag;
mod url;
pub(crate) use url::redact_header_url;
pub use url::{
    CloneUrlKind, default_folder_name, redact_url, redact_url_userinfo, validate_clone_url,
    validate_folder_name,
};
mod write;

use std::collections::{BTreeMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

pub use write::{CommitError, CommitOutcome, commit, stage, unstage};

/// The most changed files [`collection_status`] reports before it sets `truncated`.
pub const GIT_FILE_CAP: usize = 5_000;

/// How many gone paths one listing checks for a surviving copy, at two git calls each. Past it a
/// row has no `kept_at`, which only leaves it the wording that says less.
const KEPT_CHECK_CAP: usize = 100;

/// How long one git call may run before it is stopped.
pub const GIT_TIMEOUT: Duration = Duration::from_secs(10);

/// Where a collection stands with respect to git.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
#[non_exhaustive]
pub enum GitState {
    /// No `git` binary was found on `PATH`.
    Unavailable,
    /// The collection is not inside a git repository.
    NotARepo,
    /// The collection is inside a repository with this status.
    Repo(RepoStatus),
}

/// The branch, commit and changed files of a collection inside a repository.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct RepoStatus {
    /// The checked-out branch, `None` on a detached `HEAD`.
    pub branch: Option<String>,
    /// The first seven characters of `HEAD`'s id, `None` before the first commit.
    pub head: Option<String>,
    /// Changed files, keyed by their `/`-separated path relative to the collection root.
    pub files: BTreeMap<PathBuf, FileStatus>,
    /// Where each change sits relative to the index; exactly the keys of `files`.
    #[serde(skip)]
    pub stage: BTreeMap<PathBuf, StageState>,
    /// More than [`GIT_FILE_CAP`] files changed and only the first ones, in path order, are kept.
    pub truncated: bool,
    /// Where a [`StageState::StagedGone`] path's staged copy still is on disk: the new path of a
    /// worktree rename, or the target of a symlinked parent, whichever holds exactly that content.
    /// Relative to the collection root when inside it. Set by the collection status only, inside
    /// the repository, for a limited number of paths, and only under
    /// [`GitOptions::check_kept`]; no entry says nothing about the disk.
    #[serde(skip)]
    pub kept_at: BTreeMap<PathBuf, PathBuf>,
    /// The regular file in the repository that a [`StageState::StagedGone`] path now opens
    /// through a symlinked parent, when it holds other content than the staged copy. Filled
    /// alongside `kept_at`, for the paths it checks; no entry says nothing about the disk.
    #[serde(skip)]
    pub opens_at: BTreeMap<PathBuf, PathBuf>,
}

/// Where a changed file's change sits relative to the index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum StageState {
    /// Only the index differs from `HEAD`.
    Staged,
    /// Only the working tree differs from the index.
    Unstaged,
    /// Both differ.
    Both,
    /// The index holds a change to a file gone from disk; a commit records the indexed copy, and
    /// staging or unstaging drops that copy from the index.
    StagedGone,
}

/// How one file differs from `HEAD`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum FileStatus {
    /// Changed in the index or the working tree.
    Modified,
    /// New in the index.
    Added,
    /// Removed from the index or the working tree.
    Deleted,
    /// Not tracked and not ignored.
    Untracked,
    /// Renamed from another collection-relative path.
    Renamed {
        /// The path the file had in `HEAD`.
        from: PathBuf,
    },
    /// Left with unresolved merge conflicts.
    Conflicted,
}

/// One request file compared with its version in `HEAD`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct FileDiff {
    /// The file's path relative to the collection root.
    pub path: PathBuf,
    /// How the file differs from `HEAD`, `None` when it is unchanged.
    pub status: Option<FileStatus>,
    /// The text in `HEAD`, `None` when the file is not there or is binary.
    pub old: Option<String>,
    /// The text on disk, or in the index for an added file gone from disk; `None` when the file
    /// is gone or is binary.
    pub new: Option<String>,
    /// Either side is not UTF-8, so no text, sections or hunks are given.
    pub binary: bool,
    /// The request sections whose YAML values differ, in writer order.
    pub sections: Vec<Section>,
    /// Line hunks with three lines of context.
    pub hunks: Vec<Hunk>,
}

/// A run of changed lines with their context.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Hunk {
    /// The first old line, 1-based; the line before an empty range, 0 for an empty file.
    pub old_start: usize,
    /// The first new line, numbered like `old_start`.
    pub new_start: usize,
    /// The hunk's lines in order.
    pub lines: Vec<DiffLine>,
}

/// One line of a hunk, without its trailing newline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct DiffLine {
    /// Whether the line is context, added or removed.
    pub kind: LineKind,
    /// The line's text.
    pub text: String,
    /// Whether the file ends here without a newline, git's "No newline at end of file".
    /// Omitted from JSON when false.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub no_newline: bool,
}

/// The role of a line within a hunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LineKind {
    /// Present on both sides.
    Context,
    /// Only in the new text.
    Added,
    /// Only in the old text.
    Removed,
}

/// A part of a request file, named after the keys that hold it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Section {
    /// Top-level `info`.
    Info,
    /// `method` of `http` or `graphql`.
    Method,
    /// `url` of `http` or `graphql`.
    Url,
    /// `params` of `http` or `graphql`.
    Params,
    /// `headers` of `http` or `graphql`.
    Headers,
    /// `body` of `http` or `graphql`.
    Body,
    /// `auth` of `http` or `graphql`.
    Auth,
    /// `variables` of `runtime`.
    Vars,
    /// `scripts` of `runtime`.
    Scripts,
    /// `assertions` of `runtime`.
    Assertions,
    /// `actions` of `runtime`.
    Actions,
    /// Top-level `docs`.
    Docs,
    /// Top-level `settings`.
    Settings,
    /// Top-level `examples`.
    Examples,
    /// Any other key.
    Other,
}

/// Why git could not report a status.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GitError {
    /// A git call exited unsuccessfully.
    #[error("{command} failed: {stderr}")]
    Failed {
        /// The command line that failed.
        command: String,
        /// Git's trimmed error output.
        stderr: String,
    },
    /// A git call outlived its time limit.
    #[error("git did not finish within the time limit")]
    Timeout,
    /// Another git process holds the repository's lock file at `lock`.
    #[error("{}", busy_message(lock))]
    Busy {
        /// The lock file, such as `.git/index.lock`.
        lock: PathBuf,
    },
    /// Git's output could not be understood.
    #[error("{0}")]
    Malformed(String),
    /// Git could not be started or read.
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// The text of every busy error on this platform.
pub(crate) fn busy_message(lock: &Path) -> String {
    busy_text(
        lock,
        if cfg!(windows) {
            Os::Windows
        } else {
            Os::Other
        },
    )
}

/// Which platform's advice a busy error gives.
#[derive(Clone, Copy)]
enum Os {
    Windows,
    Other,
}

/// Which lock is held, and when it is safe to delete it. On Windows a timed-out git is
/// terminated outright, so the lock may be a leftover rather than another git's.
fn busy_text(lock: &Path, os: Os) -> String {
    let lock = lock.display();
    match os {
        Os::Other => format!(
            "git is busy: another git process is using this repository ({lock} exists; delete \
             it if no git process is running)"
        ),
        Os::Windows => format!(
            "git is busy: {lock} exists. Another git process may be using this repository, or a \
             git stopped at its time limit left it behind. That stop gives git no chance to \
             clean up. Once no git.exe is running (check Task Manager), delete \
             {lock} and try again."
        ),
    }
}

/// How git is run: extra child environment, the time limit of each call, and whether a status
/// looks for the surviving copies of gone rows.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct GitOptions {
    env: Vec<(OsString, OsString)>,
    timeout: Duration,
    check_kept: bool,
}

impl Default for GitOptions {
    fn default() -> GitOptions {
        GitOptions {
            env: Vec::new(),
            timeout: GIT_TIMEOUT,
            check_kept: false,
        }
    }
}

impl GitOptions {
    /// No extra environment, [`GIT_TIMEOUT`] and no check for kept copies.
    pub fn new() -> GitOptions {
        GitOptions::default()
    }

    /// Set `key` in git's environment; a later call for the same key wins. The fixed
    /// variables apinata always sets, such as `GIT_OPTIONAL_LOCKS`, cannot be overridden.
    pub fn env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> GitOptions {
        self.env
            .push((key.as_ref().to_owned(), value.as_ref().to_owned()));
        self
    }

    /// Stop each git call after `limit`.
    pub fn timeout(mut self, limit: Duration) -> GitOptions {
        self.timeout = limit;
        self
    }

    /// Have [`collection_status_with`] fill [`RepoStatus::kept_at`], at a capped number of pairs
    /// of git calls. Off by default: `kept_at` stays empty.
    pub fn check_kept(mut self, on: bool) -> GitOptions {
        self.check_kept = on;
        self
    }
}

/// [`collection_status_with`] with [`GitOptions::new`].
pub fn collection_status(root: &Path) -> Result<GitState, GitError> {
    collection_status_with(root, &GitOptions::new())
}

/// The git status of the collection at `root`, limited to files under it. Without git the state
/// is `Unavailable` and outside a repository `NotARepo`; other git failures are errors.
pub fn collection_status_with(root: &Path, options: &GitOptions) -> Result<GitState, GitError> {
    check_root(root)?;
    let prefix = match show_prefix(root, options) {
        Ok(prefix) => prefix,
        Err(e) if is_not_a_repo(&e) => return Ok(GitState::NotARepo),
        Err(GitError::Io(e)) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(GitState::Unavailable);
        }
        Err(e) => return Err(e),
    };
    let mut repo = status(root, &prefix, GIT_FILE_CAP, options)?;
    if options.check_kept {
        settle_kept(root, &prefix, &mut repo, options);
    } else {
        repo.kept_at.clear();
        repo.opens_at.clear();
    }
    Ok(GitState::Repo(repo))
}

/// [`file_diff_with`] with [`GitOptions::new`].
pub fn file_diff(root: &Path, rel_path: &Path) -> Result<FileDiff, GitError> {
    file_diff_with(root, rel_path, &GitOptions::new())
}

/// The request file at `rel_path` under the collection `root` compared with its version in
/// `HEAD`. A path that is not a `.yml`/`.yaml` file inside the collection is `Malformed` before
/// git runs, and one neither on disk, staged as new nor in `HEAD` is `Malformed` too. Without git
/// the error is `Io` of kind `NotFound`; outside a repository `Failed`.
pub fn file_diff_with(
    root: &Path,
    rel_path: &Path,
    options: &GitOptions,
) -> Result<FileDiff, GitError> {
    check_root(root)?;
    diff::file_diff(root, rel_path, options)
}

/// The collection-wide status under `prefix`, keeping at most `cap` changed files.
fn status(
    root: &Path,
    prefix: &str,
    cap: usize,
    options: &GitOptions,
) -> Result<RepoStatus, GitError> {
    let args = [
        "status",
        "--porcelain=v2",
        "-z",
        "--branch",
        "--untracked-files=all",
        "--",
        ".",
    ];
    let out = run::git(&args, root, options)?;
    if !out.status.success() {
        return Err(out.failed(&args));
    }
    porcelain::parse(&out.stdout, prefix, cap, || head_paths(root, options))
}

/// Settle `kept_at` for the listing: each staged path gone from disk, at the path git paired it
/// with or through a symlinked parent, keeps an entry only for a regular file in the repository
/// that holds the staged copy. A path that opens such a file through a link with other content
/// lands in `opens_at` instead. Nothing else is opened.
fn settle_kept(root: &Path, prefix: &str, repo: &mut RepoStatus, options: &GitOptions) {
    let paired = std::mem::take(&mut repo.kept_at);
    let Ok(base) = root.canonicalize() else {
        return;
    };
    let Some(top) = base.ancestors().nth(Path::new(prefix).components().count()) else {
        return;
    };
    let candidates: Vec<(&PathBuf, PathBuf)> = repo
        .stage
        .iter()
        .filter(|(_, at)| **at == StageState::StagedGone)
        .filter_map(|(rel, _)| {
            let real = base
                .join(paired.get(rel).unwrap_or(rel))
                .canonicalize()
                .ok()?;
            let usable = real.starts_with(top) && real.is_file() && real != base.join(rel);
            usable.then_some((rel, real))
        })
        .take(KEPT_CHECK_CAP)
        .collect();
    let (mut kept, mut opens) = (BTreeMap::new(), BTreeMap::new());
    for (rel, real) in candidates {
        let shown = real.strip_prefix(&base).unwrap_or(&real).to_path_buf();
        if holds_staged_copy(root, prefix, rel, &real, options) {
            kept.insert(rel.clone(), shown);
        } else if !paired.contains_key(rel) {
            // Only a path read through its own link opens the file; a paired one points elsewhere.
            opens.insert(rel.clone(), shown);
        }
    }
    repo.kept_at = kept;
    repo.opens_at = opens;
}

/// Whether the file at `at` is what staging `rel` would store: git hashes it under `rel`, so the
/// filters that cleaned the staged blob apply. A git failure claims nothing.
fn holds_staged_copy(
    root: &Path,
    prefix: &str,
    rel: &Path,
    at: &Path,
    options: &GitOptions,
) -> bool {
    let rel = rel.to_string_lossy();
    let hash = [
        OsString::from("hash-object"),
        OsString::from(format!("--path={rel}")),
        OsString::from("--"),
        at.as_os_str().to_owned(),
    ];
    let staged = format!(":0:{prefix}{rel}");
    let staged = git_stdout(&["rev-parse", staged.as_str()], root, options);
    staged.is_some() && staged == git_stdout(&hash, root, options)
}

/// What `args` print, `None` when git failed.
fn git_stdout<S: AsRef<OsStr>>(args: &[S], root: &Path, options: &GitOptions) -> Option<Vec<u8>> {
    let out = run::git(args, root, options).ok()?;
    out.status.success().then_some(out.stdout)
}

/// Every repository path in `HEAD`, relative to its top wherever `root` sits, or `None` when git
/// cannot say, so a caller keeps what it would otherwise hide.
fn head_paths(root: &Path, options: &GitOptions) -> Option<HashSet<Vec<u8>>> {
    let args = ["ls-tree", "-r", "-z", "--name-only", "--full-tree", "HEAD"];
    let out = run::git(&args, root, options).ok()?;
    out.status.success().then(|| {
        out.stdout
            .split(|&b| b == 0)
            .filter(|p| !p.is_empty())
            .map(<[u8]>::to_vec)
            .collect()
    })
}

/// `Io` naming `root` unless it is a directory. A spawn in a missing directory also fails with
/// `NotFound`, which would otherwise read as git missing from `PATH`.
fn check_root(root: &Path) -> Result<(), GitError> {
    let named = |kind, why: &dyn std::fmt::Display| {
        GitError::Io(io::Error::new(
            kind,
            format!("collection root {}: {why}", root.display()),
        ))
    };
    match std::fs::metadata(root) {
        Ok(meta) if meta.is_dir() => Ok(()),
        Ok(_) => Err(named(io::ErrorKind::NotADirectory, &"not a directory")),
        Err(e) => Err(named(e.kind(), &e)),
    }
}

/// The collection's path inside its repository: `"col/"`, or `""` at the top.
fn show_prefix(root: &Path, options: &GitOptions) -> Result<String, GitError> {
    let args = ["rev-parse", "--show-prefix"];
    let out = run::git(&args, root, options)?;
    if !out.status.success() {
        return Err(out.failed(&args));
    }
    let prefix = String::from_utf8(out.stdout)
        .map_err(|_| GitError::Malformed("the collection's path in git is not UTF-8".to_owned()))?;
    Ok(prefix.trim_end_matches('\n').to_owned())
}

/// Whether `show_prefix` failed because `root` is outside any repository. Only this message
/// counts; a refused unsafe repository stays an error.
fn is_not_a_repo(error: &GitError) -> bool {
    matches!(error, GitError::Failed { stderr, .. } if stderr.contains("not a git repository"))
}

#[cfg(test)]
mod tests {
    use super::{Os, busy_text};
    use std::path::Path;

    #[test]
    fn busy_text_elsewhere_names_the_lock_and_when_to_delete_it() {
        assert_eq!(
            busy_text(Path::new("/r/.git/index.lock"), Os::Other),
            "git is busy: another git process is using this repository \
             (/r/.git/index.lock exists; delete it if no git process is running)"
        );
    }

    #[test]
    fn busy_text_on_windows_says_a_stopped_git_leaves_the_lock_and_how_to_clear_it() {
        assert_eq!(
            busy_text(Path::new("C:/r/.git/index.lock"), Os::Windows),
            "git is busy: C:/r/.git/index.lock exists. Another git process may be using this \
              repository, or a git stopped at its time limit left it behind. That stop gives git \
              no chance to clean up. Once no git.exe is running (check Task \
              Manager), delete C:/r/.git/index.lock and try again."
        );
    }
}
