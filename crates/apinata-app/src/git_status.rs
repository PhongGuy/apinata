//! The app's view of a collection's git status: app-owned mirrors of core's status types, the
//! letters and words the sidebar, tabs and Changes dialog show, and what a refresh leaves.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use apinata_core::{
    CommitError, FileStatus, GIT_FILE_CAP, GitError, GitState, ItemPath, StageState,
};

/// How one changed file differs from `HEAD`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitChange {
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

impl GitChange {
    /// The app's change for a core status; a status added to core later reads as modified.
    pub fn from_core(status: &FileStatus) -> GitChange {
        match status {
            FileStatus::Added => GitChange::Added,
            FileStatus::Deleted => GitChange::Deleted,
            FileStatus::Untracked => GitChange::Untracked,
            FileStatus::Renamed { from } => GitChange::Renamed { from: from.clone() },
            FileStatus::Conflicted => GitChange::Conflicted,
            _ => GitChange::Modified,
        }
    }

    /// The one-letter marker shown on sidebar rows and tabs.
    pub fn letter(&self) -> &'static str {
        match self {
            GitChange::Modified => "M",
            GitChange::Added => "A",
            GitChange::Deleted => "D",
            GitChange::Untracked => "U",
            GitChange::Renamed { .. } => "R",
            GitChange::Conflicted => "C",
        }
    }

    /// The status word the Changes dialog lists, as the CLI prints it.
    pub fn word(&self) -> &'static str {
        match self {
            GitChange::Modified => "modified",
            GitChange::Added => "added",
            GitChange::Deleted => "deleted",
            GitChange::Untracked => "untracked",
            GitChange::Renamed { .. } => "renamed",
            GitChange::Conflicted => "conflicted",
        }
    }

    /// The marker's meaning in words, for tooltips and accessibility labels.
    pub fn description(&self) -> &'static str {
        match self {
            GitChange::Modified => "Modified since last commit",
            GitChange::Added => "Added since last commit",
            GitChange::Deleted => "Deleted since last commit",
            GitChange::Untracked => "Untracked",
            GitChange::Renamed { .. } => "Renamed since last commit",
            GitChange::Conflicted => "Conflicted",
        }
    }
}

/// Whether a changed file's change is in the index, the working tree, or both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitStage {
    /// Fully staged.
    Staged,
    /// Not staged at all.
    Unstaged,
    /// Staged, then edited again.
    Both,
    /// Staged, then gone from disk; a commit records the indexed copy.
    StagedGone,
}

impl GitStage {
    /// The app's stage for a core state; a state added to core later reads as unstaged.
    pub fn from_core(state: &StageState) -> GitStage {
        match state {
            StageState::Staged => GitStage::Staged,
            StageState::Both => GitStage::Both,
            StageState::StagedGone => GitStage::StagedGone,
            _ => GitStage::Unstaged,
        }
    }

    /// Whether a commit takes all there is of the file, which is what a ticked box says.
    pub fn is_full(self) -> bool {
        matches!(self, GitStage::Staged | GitStage::StagedGone)
    }
}

/// A collection's changed files, keyed by `/`-separated collection-relative path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitChanges {
    /// Each changed file and how it changed.
    pub files: BTreeMap<PathBuf, GitChange>,
    /// Core kept only the first changed files because more changed than its cap.
    pub truncated: bool,
    /// The stage of each changed file; has the same keys as `files`.
    pub stage: BTreeMap<PathBuf, GitStage>,
    /// The current branch, `None` when detached or unborn without a name.
    pub branch: Option<String>,
    /// Where the file of a staged path gone from disk went, when core knows.
    pub kept_at: BTreeMap<PathBuf, PathBuf>,
    /// The other file a staged path gone from disk now opens through a link, when core knows.
    pub opens_at: BTreeMap<PathBuf, PathBuf>,
}

impl GitChanges {
    /// The changes of a collection inside a repository; `None` without git or a repository.
    pub fn from_state(state: &GitState) -> Option<GitChanges> {
        match state {
            GitState::Repo(repo) => Some(GitChanges {
                files: repo
                    .files
                    .iter()
                    .map(|(path, status)| (path.clone(), GitChange::from_core(status)))
                    .collect(),
                truncated: repo.truncated,
                stage: repo
                    .files
                    .keys()
                    .map(|path| {
                        let stage = repo
                            .stage
                            .get(path)
                            .map_or(GitStage::Unstaged, GitStage::from_core);
                        (path.clone(), stage)
                    })
                    .collect(),
                branch: repo.branch.clone(),
                kept_at: repo.kept_at.clone(),
                opens_at: repo.opens_at.clone(),
            }),
            _ => None,
        }
    }

    /// How many files have something staged, fully or not.
    pub fn staged_count(&self) -> usize {
        self.stage
            .values()
            .filter(|s| !matches!(s, GitStage::Unstaged))
            .count()
    }

    /// The change of the file at `rel`, a collection-relative `/`-separated path.
    pub fn get(&self, rel: &str) -> Option<&GitChange> {
        self.files.get(Path::new(rel))
    }

    /// The count for the branch label: `None` when nothing changed, `"N+ changed"` when truncated.
    pub fn count_label(&self) -> Option<String> {
        if self.files.is_empty() {
            return None;
        }
        let more = if self.truncated { "+" } else { "" };
        Some(format!("{}{more} changed", self.files.len()))
    }

    /// The Changes dialog's note when the list stops at core's cap.
    pub fn truncated_note(&self) -> Option<String> {
        if !self.truncated {
            return None;
        }
        // Core's cap with its thousands grouped by commas.
        let digits = GIT_FILE_CAP.to_string();
        let groups: Vec<_> = digits
            .as_bytes()
            .rchunks(3)
            .rev()
            .map(String::from_utf8_lossy)
            .collect();
        Some(format!(
            "Showing the first {} changed files",
            groups.join(",")
        ))
    }
}

/// The Commit button's label.
pub fn commit_label(branch: Option<&str>) -> String {
    match branch {
        Some(branch) => format!("Commit to {branch}"),
        None => "Commit".to_owned(),
    }
}

/// Git's output, with a hint on how to set an identity when git complained it has none.
pub fn with_identity_hint(output: &str) -> String {
    if output.contains("Author identity unknown") || output.contains("empty ident") {
        format!(
            "{output}\n\nSet your identity with `git config user.name` and `git config user.email`."
        )
    } else {
        output.to_owned()
    }
}

/// The text the Changes dialog's notice shows for a refused or failed commit.
pub fn commit_failure_text(error: &CommitError) -> String {
    match error {
        CommitError::EmptyMessage => "The commit message is empty.".to_owned(),
        CommitError::Conflicted => "Resolve the conflicted files before committing.".to_owned(),
        CommitError::NothingStaged => "Nothing is staged.".to_owned(),
        CommitError::StagedOutside { paths, count } => {
            let mut text = String::from(
                "Files outside this collection are staged. Unstage them elsewhere before committing:",
            );
            for path in paths {
                text.push('\n');
                text.push_str(&path.to_string_lossy());
            }
            if *count > paths.len() {
                text.push_str(&format!("\nand {} more", count - paths.len()));
            }
            text
        }
        CommitError::Failed { output } => with_identity_hint(output),
        CommitError::Timeout => "git commit timed out".to_owned(),
        other => other.to_string(),
    }
}

/// The title bar's branch label: the branch, the changed-file count, or both joined by a
/// comma; `None` when there is neither.
pub fn branch_text(branch: Option<&str>, changes: Option<&GitChanges>) -> Option<String> {
    match (branch, changes.and_then(GitChanges::count_label)) {
        (Some(branch), Some(count)) => Some(format!("{branch}, {count}")),
        (Some(branch), None) => Some(branch.to_owned()),
        (None, count) => count,
    }
}

/// The file whose status a sidebar row shows: a request's own file, a folder's `folder.yml`.
pub fn item_file(path: &ItemPath, folder: bool) -> String {
    // Joined with `/` rather than `Path::join` so the key matches core's on every platform.
    if folder {
        format!("{}/folder.yml", path.as_str())
    } else {
        path.as_str().to_owned()
    }
}

/// What the Changes dialog says when git is missing or the collection is not in a repository.
pub const QUIET_NOTE: &str =
    "Changes are not available: git is missing or this collection is not in a repository.";

/// The state a refresh leaves: a repository's changes, the previous state after a timeout,
/// and `None` (quiet) for no git, no repository, a missing root or any other git failure.
pub fn after_refresh(
    prev: Option<Arc<GitChanges>>,
    result: Result<GitState, GitError>,
) -> Option<Arc<GitChanges>> {
    match result {
        Ok(state) => GitChanges::from_state(&state).map(Arc::new),
        Err(GitError::Timeout) => prev,
        // A missing root (`Io` NotFound) is as quiet as missing git; neither is reported.
        Err(_) => None,
    }
}

/// Lets one status run go at a time per collection; requests during a run collapse into one
/// more run, so a burst of saves or watcher batches costs at most two git calls.
#[derive(Debug, Default)]
pub struct RefreshGate {
    running: bool,
    again: bool,
}

impl RefreshGate {
    /// Asks for a run: `true` means start one now, `false` that one is running and one more
    /// is remembered.
    pub fn request(&mut self) -> bool {
        if self.running {
            self.again = true;
            false
        } else {
            self.running = true;
            true
        }
    }

    /// Ends the current run: `true` means start the remembered one now, which counts as running.
    pub fn finish(&mut self) -> bool {
        self.running = std::mem::take(&mut self.again);
        self.running
    }
}
