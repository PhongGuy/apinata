//! Operations that change a repository: stage, unstage and commit collection files.

use std::ffi::OsString;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use super::{
    FileStatus, GitError, GitOptions, StageState, check_root, is_not_a_repo, run, show_prefix,
    status,
};

/// The most paths `CommitError::StagedOutside` lists.
const OUTSIDE_LIST_CAP: usize = 20;

/// The most bytes of git's output `CommitError::Failed` carries.
const OUTPUT_CAP: usize = 8192;

/// The least time a write may take: hooks, filters and a cold cache can be slow.
const MIN_WRITE_TIMEOUT: Duration = Duration::from_secs(120);

/// The commit that [`commit`] recorded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct CommitOutcome {
    /// The abbreviated id of the new `HEAD`.
    pub short_id: String,
    /// The first line of the message.
    pub subject: String,
}

/// Why [`commit`] did not record a commit.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CommitError {
    /// The message is empty or only whitespace.
    #[error("the commit message is empty")]
    EmptyMessage,
    /// Some file in the collection is conflicted.
    #[error("resolve the conflicted files before committing")]
    Conflicted,
    /// Files outside the collection are staged, and a commit would record them too.
    #[error("{count} staged file(s) lie outside the collection")]
    StagedOutside {
        /// Repository-root-relative paths as git prints them, at most 20.
        paths: Vec<PathBuf>,
        /// The total number of staged files outside the collection.
        count: usize,
    },
    /// No file inside the collection is staged.
    #[error("nothing in the collection is staged")]
    NothingStaged,
    /// Git refused the commit; its output, hooks included.
    #[error("{output}")]
    Failed {
        /// Git's stdout then stderr, trimmed.
        output: String,
    },
    /// The commit outlived its time limit.
    #[error("git did not finish within the time limit; the commit may have completed")]
    Timeout,
    /// A preflight git call failed.
    #[error(transparent)]
    Git(#[from] GitError),
}

/// Commit what is staged in the collection at `root`, using git's own identity, hooks and
/// message cleanup. The author and committer are whatever git is configured with, and hooks run
/// as git runs them, their output returned in `Failed`. Refused with `EmptyMessage`,
/// `Conflicted`, `StagedOutside` or `NothingStaged` before git commits. A `Timeout` may still
/// have produced the commit.
pub fn commit(
    root: &Path,
    message: &str,
    options: &GitOptions,
) -> Result<CommitOutcome, CommitError> {
    if message.trim().is_empty() {
        return Err(CommitError::EmptyMessage);
    }
    check_root(root)?;
    let prefix = show_prefix(root, options)?;
    let current = status(root, &prefix, usize::MAX, options)?;
    if current
        .files
        .values()
        .any(|f| matches!(f, FileStatus::Conflicted))
    {
        return Err(CommitError::Conflicted);
    }

    // Everything staged is committed, so the index must hold only collection files.
    let args = ["diff", "--cached", "--name-only", "--no-renames", "-z"];
    let out = run::git(&args, root, options)?;
    if !out.status.success() {
        return Err(out.failed(&args).into());
    }
    let staged: Vec<String> = out
        .stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect();
    let outside: Vec<&String> = staged.iter().filter(|p| !p.starts_with(&prefix)).collect();
    if !outside.is_empty() {
        return Err(CommitError::StagedOutside {
            paths: outside
                .iter()
                .take(OUTSIDE_LIST_CAP)
                .map(PathBuf::from)
                .collect(),
            count: outside.len(),
        });
    }
    if staged.is_empty() {
        return Err(CommitError::NothingStaged);
    }

    // `-m` and the message are separate entries so a leading dash stays data.
    let args: Vec<OsString> = vec![
        "commit".into(),
        "-q".into(),
        "--cleanup=whitespace".into(),
        "-m".into(),
        message.into(),
    ];
    let commit_options = options.clone().timeout(write_timeout(options.timeout));
    let out = match run::git_write(&args, &["user.useConfigOnly=true"], root, &commit_options) {
        Ok(out) => out,
        Err(GitError::Timeout) => return Err(CommitError::Timeout),
        Err(e) => return Err(e.into()),
    };
    if !out.status.success() {
        // Hook output is returned verbatim and never filtered.
        return Err(CommitError::Failed {
            output: cap_output(&out.stdout, &out.stderr),
        });
    }

    let args = ["rev-parse", "--short", "HEAD"];
    let out = run::git(&args, root, options)?;
    if !out.status.success() {
        return Err(out.failed(&args).into());
    }
    Ok(CommitOutcome {
        short_id: String::from_utf8_lossy(&out.stdout).trim().to_owned(),
        subject: message
            .trim()
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned(),
    })
}

/// stdout, newline, stderr, lossy UTF-8, trimmed; the last 8192 bytes on a char boundary.
fn cap_output(stdout: &[u8], stderr: &[u8]) -> String {
    let mut text = String::from_utf8_lossy(stdout).into_owned();
    text.push('\n');
    text.push_str(&String::from_utf8_lossy(stderr));
    let text = text.trim();
    if text.len() <= OUTPUT_CAP {
        return text.to_owned();
    }
    // Git's verdict and hook failures come last, so the tail is what survives.
    let mut start = text.len() - OUTPUT_CAP;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_owned()
}

/// A write's time limit: the configured one, but at least 120 seconds.
fn write_timeout(configured: Duration) -> Duration {
    configured.max(MIN_WRITE_TIMEOUT)
}

/// Stage `paths` (relative to the collection `root`), including deletions and new files, as
/// `git add -A`. A path that is absolute, empty or contains `..` is `Malformed` and git does not
/// run; names are literal, never pathspec patterns. No paths is a no-op. A path gone from disk
/// whose deletion is already wholly staged is left out, since git would refuse it and with it the
/// rest; if that leaves nothing the call is a no-op. A path under a symlink is dropped from the
/// index, since to git it is gone whatever the link's target holds.
pub fn stage(
    root: &Path,
    paths: &[impl AsRef<Path>],
    options: &GitOptions,
) -> Result<(), GitError> {
    let Some(mut specs) = checked(root, paths)? else {
        return Ok(());
    };
    let gone = |path: &Path| {
        matches!(root.join(path).symlink_metadata(),
            Err(e) if matches!(e.kind(), io::ErrorKind::NotFound | io::ErrorKind::NotADirectory))
    };
    if paths.iter().any(|p| gone(p.as_ref())) {
        let prefix = show_prefix(root, options)?;
        let current = status(root, &prefix, usize::MAX, options)?;
        let settled = |path: &Path| {
            gone(path)
                && current.files.get(path) == Some(&FileStatus::Deleted)
                && current.stage.get(path) == Some(&StageState::Staged)
        };
        specs.retain(|spec| !settled(Path::new(spec)));
        if specs.is_empty() {
            return Ok(());
        }
    }
    // `git add` refuses a path beyond a symlink, and with it the whole call, so those go apart.
    let (linked, specs): (Vec<OsString>, Vec<OsString>) = specs
        .into_iter()
        .partition(|spec| under_link(root, Path::new(spec)));
    if !specs.is_empty() {
        // Git waits on a pipe where it reads attributes or ignore rules. A dry run under the
        // caller's own limit ends that wait long before the write's floor would.
        let mut probe: Vec<OsString> = ["--literal-pathspecs", "add", "-n", "-A", "--"]
            .into_iter()
            .map(OsString::from)
            .collect();
        probe.extend(specs.iter().cloned());
        run::git(&probe, root, options)?;
        let mut args: Vec<OsString> = vec!["add".into(), "-A".into(), "--".into()];
        args.extend(specs);
        run_write(&args, root, options)?;
    }
    if !linked.is_empty() {
        // `-f` lets go of staged content nothing else holds; `--cached` leaves the disk alone.
        let rm = ["rm", "-r", "-f", "-q", "--cached", "--ignore-unmatch", "--"];
        let mut args: Vec<OsString> = rm.into_iter().map(OsString::from).collect();
        args.extend(linked);
        run_write(&args, root, options)?;
    }
    Ok(())
}

/// Whether a parent of `path`, inside `root`, is a symlink.
fn under_link(root: &Path, path: &Path) -> bool {
    path.ancestors()
        .skip(1)
        .filter(|parent| !parent.as_os_str().is_empty())
        .any(|parent| {
            root.join(parent)
                .symlink_metadata()
                .is_ok_and(|m| m.file_type().is_symlink())
        })
}

/// Unstage `paths` (relative to the collection `root`) as `git reset -q`; the old name of a
/// staged rename among them is reset too, so the pair separates. Path rules as for [`stage`].
/// Outside a repository the error is `Failed`; without git, `Io` of kind `NotFound`.
pub fn unstage(
    root: &Path,
    paths: &[impl AsRef<Path>],
    options: &GitOptions,
) -> Result<(), GitError> {
    let Some(mut specs) = checked(root, paths)? else {
        return Ok(());
    };
    // A staged rename shows as one entry under its new name; resetting only that name would
    // leave the deletion of the old one staged, so the old names join the reset.
    let prefix = match show_prefix(root, options) {
        Ok(prefix) => prefix,
        Err(e) if is_not_a_repo(&e) => {
            return Err(GitError::Failed {
                command: "git rev-parse --show-prefix".to_owned(),
                stderr: "not a git repository".to_owned(),
            });
        }
        Err(e) => return Err(e),
    };
    let current = status(root, &prefix, usize::MAX, options)?;
    for (path, file) in &current.files {
        if let FileStatus::Renamed { from } = file
            && paths.iter().any(|p| {
                let p = p.as_ref();
                p.as_os_str() == "." || path.starts_with(p)
            })
        {
            specs.push(from.clone().into_os_string());
        }
    }
    let mut args: Vec<OsString> = vec!["reset".into(), "-q".into(), "--".into()];
    args.extend(specs);
    run_write(&args, root, options)
}

/// The pathspecs for `paths` after the root and path checks; `None` when there are none.
fn checked(root: &Path, paths: &[impl AsRef<Path>]) -> Result<Option<Vec<OsString>>, GitError> {
    check_root(root)?;
    for path in paths {
        check_relative(path.as_ref())?;
    }
    if paths.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        paths
            .iter()
            .map(|p| p.as_ref().as_os_str().to_owned())
            .collect(),
    ))
}

/// Lexical check: non-empty, relative, and never leaving the collection.
fn check_relative(path: &Path) -> Result<(), GitError> {
    let bad = path.as_os_str().is_empty()
        || path.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        });
    if bad {
        return Err(GitError::Malformed(format!(
            "{} is not a path inside the collection",
            path.display()
        )));
    }
    Ok(())
}

/// One stage or unstage call, under the same time floor as a commit.
fn run_write(args: &[OsString], root: &Path, options: &GitOptions) -> Result<(), GitError> {
    let options = options.clone().timeout(write_timeout(options.timeout));
    let out = run::git_write(args, &[], root, &options)?;
    if out.status.success() {
        Ok(())
    } else {
        Err(out.failed(args))
    }
}

#[cfg(test)]
mod tests {
    use super::{GitOptions, cap_output, stage, unstage, write_timeout};
    use std::time::Duration;

    #[test]
    fn output_joins_trims_and_keeps_the_tail() {
        assert_eq!(cap_output(b"out\n", b"err\n"), "out\n\nerr");
        assert_eq!(cap_output(b"", b"  only err \n"), "only err");
        let long = format!("{}TAIL", "x".repeat(10_000));
        let capped = cap_output(b"", long.as_bytes());
        assert!(capped.len() <= 8192 && capped.ends_with("TAIL"));
        let wide = "\u{e9}".repeat(5_000); // two bytes each
        let capped = cap_output(wide.as_bytes(), b"");
        assert!(capped.len() <= 8192 && capped.chars().all(|c| c == '\u{e9}'));
    }

    #[test]
    fn every_write_gets_at_least_two_minutes() {
        assert_eq!(
            write_timeout(Duration::from_secs(10)),
            Duration::from_secs(120)
        );
        assert_eq!(
            write_timeout(Duration::from_secs(300)),
            Duration::from_secs(300)
        );
    }

    /// Sleeps one second in `add` and `reset`; answers the reads stage and unstage make first,
    /// and the dry run of `add`, which runs no filters.
    const SLOW_WRITE_GIT: &str = r#"#!/bin/sh
case "$*" in
  *rev-parse*) printf '\n' ;;
  *status*) printf '# branch.oid (initial)\000'; printf '# branch.head main\000' ;;
  *" add -n "*) ;;
  *" add "*|*" reset "*) sleep 1 ;;
esac
"#;

    #[cfg(unix)]
    #[test]
    fn stage_and_unstage_run_under_the_write_floor() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let git = bin.join("git");
        std::fs::write(&git, SLOW_WRITE_GIT).unwrap();
        std::fs::set_permissions(&git, std::fs::Permissions::from_mode(0o755)).unwrap();
        let options = GitOptions::new()
            .timeout(Duration::from_millis(300))
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()));
        let root = dir.path();
        let r = stage(root, &["a.yml"], &options);
        assert!(r.is_ok(), "stage: {r:?}");
        let r = unstage(root, &["a.yml"], &options);
        assert!(r.is_ok(), "unstage: {r:?}");
    }
}
